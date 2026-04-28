//! # Token Vesting Core — `vesting.rs`
//!
//! Implements cliff + linear-schedule vesting for Soroban (Stellar).
//!
//! ## Security invariants (always maintained)
//!
//! 1. **No premature unlock** — nothing is claimable before `cliff_ts`.
//! 2. **No over-claim** — cumulative claimed tokens never exceed `total_amount`.
//! 3. **Cursor monotonicity** — `claimed_amount` only ever increases.
//! 4. **Idempotency** — calling `claim` when nothing new is vested is a no-op
//!    (returns 0, no state change).
//! 5. **Backdating prevention** — schedule parameters are validated at
//!    registration; `start_ts >= cliff_ts` is required and the contract
//!    reads `env.ledger().timestamp()` for the current time (consensus-set,
//!    not caller-supplied).
//! 6. **Auth-gated mutation** — only the registered `beneficiary` can call
//!    `vesting_claim`; only the `issuer` can register or revoke a schedule.
//!
//! ## Time source
//! All time checks use `env.ledger().timestamp()` — the Unix timestamp of
//! the closing ledger, set by Stellar consensus.  It is monotonically
//! non-decreasing and not manipulable per-transaction.

#![allow(clippy::too_many_arguments)]

use soroban_sdk::{
    contract, contractimpl, contracttype, token, Address, Env, Vec,
};

// ── Storage keys ─────────────────────────────────────────────────────────────

/// Persistent storage keys for vesting state.
#[contracttype]
#[derive(Clone)]
pub enum VestingKey {
    /// The full [`VestingSchedule`] for a given beneficiary.
    Schedule(Address),
    /// How many tokens the beneficiary has already claimed.
    Claimed(Address),
}

// ── Public types ──────────────────────────────────────────────────────────────

/// A single vesting tranche for a beneficiary.
///
/// # Fields
/// * `issuer`       – Address that funded and registered this schedule.
/// * `beneficiary`  – Recipient of vested tokens.
/// * `token`        – SEP-41 token contract address.
/// * `total_amount` – Total tokens to vest (must be > 0).
/// * `cliff_ts`     – Unix timestamp before which *nothing* unlocks.
/// * `start_ts`     – Vesting start for linear portion (must be ≥ `cliff_ts`).
/// * `end_ts`       – Full-vest timestamp (must be > `start_ts`).
///
/// Tokens vest linearly from `start_ts` to `end_ts`.  Between `cliff_ts`
/// and `start_ts` the vested amount is 0 (pure cliff).  After `end_ts`
/// the full `total_amount` is vested.
#[contracttype]
#[derive(Clone)]
pub struct VestingSchedule {
    pub issuer: Address,
    pub beneficiary: Address,
    pub token: Address,
    pub total_amount: i128,
    pub cliff_ts: u64,
    pub start_ts: u64,
    pub end_ts: u64,
}

/// Errors produced by the vesting module.
#[contracttype]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u32)]
pub enum VestingError {
    /// A schedule already exists for this beneficiary.
    ScheduleAlreadyExists = 100,
    /// No schedule found for the given beneficiary.
    ScheduleNotFound = 101,
    /// `total_amount` must be > 0.
    InvalidAmount = 102,
    /// Timestamp ordering violated (`cliff_ts > start_ts` or
    /// `start_ts >= end_ts`).
    InvalidTimestamps = 103,
    /// Nothing to claim at the current ledger time.
    NothingToClaimYet = 104,
    /// Caller is not authorised for this operation.
    Unauthorized = 105,
}

// ── Internal helpers ──────────────────────────────────────────────────────────

/// Compute how many tokens are vested at `now`, given the schedule.
///
/// Returns a value in `[0, total_amount]`.  Pure function — no storage I/O.
///
/// # Invariants
/// * Returns 0 if `now < cliff_ts` (cliff not reached).
/// * Returns `total_amount` if `now >= end_ts` (fully vested).
/// * Returns a linearly interpolated value otherwise.
pub fn vested_amount(schedule: &VestingSchedule, now: u64) -> i128 {
    if now < schedule.cliff_ts {
        // Before cliff — nothing unlocked.
        return 0;
    }
    if now >= schedule.end_ts {
        // Past or at full-vest — everything unlocked.
        return schedule.total_amount;
    }
    if now < schedule.start_ts {
        // After cliff but before linear start — still 0 (pure cliff period).
        return 0;
    }
    // Linear interpolation between start_ts and end_ts.
    // Use i128 arithmetic; duration and elapsed are u64 ≤ ~1.8e19, safe.
    let elapsed = (now - schedule.start_ts) as i128;
    let duration = (schedule.end_ts - schedule.start_ts) as i128;
    // Multiply first to avoid integer truncation.
    schedule.total_amount * elapsed / duration
}

/// Amount claimable *now* (vested minus already claimed).
///
/// Always ≥ 0 by construction.
fn claimable_amount(schedule: &VestingSchedule, claimed: i128, now: u64) -> i128 {
    let vested = vested_amount(schedule, now);
    // Defensive: clamp to 0 (should never go negative given invariants).
    if vested > claimed { vested - claimed } else { 0 }
}

// ── Contract implementation ───────────────────────────────────────────────────

#[contract]
pub struct VestingContract;

#[contractimpl]
impl VestingContract {
    // ── Registration ─────────────────────────────────────────────────────────

    /// Register a new vesting schedule for `beneficiary`.
    ///
    /// The `issuer` must authorise this call and must have pre-approved the
    /// token contract to allow the vesting contract to pull `total_amount`.
    ///
    /// # Errors
    /// * [`VestingError::ScheduleAlreadyExists`] – a schedule is already
    ///   registered for this beneficiary.
    /// * [`VestingError::InvalidAmount`] – `total_amount` ≤ 0.
    /// * [`VestingError::InvalidTimestamps`] – ordering violated.
    pub fn vesting_register(
        env: Env,
        issuer: Address,
        beneficiary: Address,
        token: Address,
        total_amount: i128,
        cliff_ts: u64,
        start_ts: u64,
        end_ts: u64,
    ) -> Result<(), VestingError> {
        issuer.require_auth();

        // ── Validate inputs ──────────────────────────────────────────────────
        if total_amount <= 0 {
            return Err(VestingError::InvalidAmount);
        }
        // start_ts must be ≥ cliff_ts (cliff may precede or coincide with
        // linear start); end_ts must be strictly after start_ts.
        if start_ts < cliff_ts || end_ts <= start_ts {
            return Err(VestingError::InvalidTimestamps);
        }

        // ── Duplicate guard ──────────────────────────────────────────────────
        let key = VestingKey::Schedule(beneficiary.clone());
        if env.storage().persistent().has(&key) {
            return Err(VestingError::ScheduleAlreadyExists);
        }

        // ── Pull tokens from issuer into this contract ────────────────────────
        let tok = token::Client::new(&env, &token);
        tok.transfer(&issuer, &env.current_contract_address(), &total_amount);

        // ── Persist schedule & zero-initialise claimed cursor ────────────────
        let schedule = VestingSchedule {
            issuer,
            beneficiary: beneficiary.clone(),
            token,
            total_amount,
            cliff_ts,
            start_ts,
            end_ts,
        };
        env.storage().persistent().set(&key, &schedule);
        env.storage()
            .persistent()
            .set(&VestingKey::Claimed(beneficiary.clone()), &0_i128);

        // ── Emit event ───────────────────────────────────────────────────────
        env.events().publish(
            (soroban_sdk::symbol_short!("vest_reg"), beneficiary),
            (total_amount, cliff_ts, start_ts, end_ts),
        );

        Ok(())
    }

    // ── Claim ─────────────────────────────────────────────────────────────────

    /// Claim all tokens that have vested up to the current ledger timestamp.
    ///
    /// # Returns
    /// The number of tokens transferred to `beneficiary`.  Returns 0 (without
    /// error) when nothing new has vested — satisfying the idempotency
    /// invariant.
    ///
    /// # Errors
    /// * [`VestingError::ScheduleNotFound`] – no schedule for this address.
    /// * [`VestingError::NothingToClaimYet`] – cliff not yet reached.
    pub fn vesting_claim(
        env: Env,
        beneficiary: Address,
    ) -> Result<i128, VestingError> {
        beneficiary.require_auth();

        let sched_key = VestingKey::Schedule(beneficiary.clone());
        let claimed_key = VestingKey::Claimed(beneficiary.clone());

        let schedule: VestingSchedule = env
            .storage()
            .persistent()
            .get(&sched_key)
            .ok_or(VestingError::ScheduleNotFound)?;

        let already_claimed: i128 = env
            .storage()
            .persistent()
            .get(&claimed_key)
            .unwrap_or(0_i128);

        let now = env.ledger().timestamp();

        // Hard cliff gate — return a distinct error if we are before cliff.
        if now < schedule.cliff_ts {
            return Err(VestingError::NothingToClaimYet);
        }

        let claimable = claimable_amount(&schedule, already_claimed, now);

        // Idempotent: nothing new to send → return 0 without state change.
        if claimable == 0 {
            return Ok(0);
        }

        // ── Safety assertion: never exceed total_amount ──────────────────────
        let new_claimed = already_claimed
            .checked_add(claimable)
            .expect("vesting: claimed overflow");
        assert!(
            new_claimed <= schedule.total_amount,
            "vesting: invariant violated — claimed > total"
        );

        // ── Advance cursor first (checks-effects-interactions) ───────────────
        env.storage()
            .persistent()
            .set(&claimed_key, &new_claimed);

        // ── Transfer tokens to beneficiary ───────────────────────────────────
        let tok = token::Client::new(&env, &schedule.token);
        tok.transfer(
            &env.current_contract_address(),
            &beneficiary,
            &claimable,
        );

        // ── Emit event ───────────────────────────────────────────────────────
        env.events().publish(
            (soroban_sdk::symbol_short!("vest_clm"), beneficiary),
            (claimable, new_claimed, schedule.total_amount),
        );

        Ok(claimable)
    }

    // ── Revocation ────────────────────────────────────────────────────────────

    /// Revoke a vesting schedule.  Vested-but-unclaimed tokens are sent to
    /// `beneficiary`; unvested tokens are returned to `issuer`.
    ///
    /// Only the original `issuer` may call this.
    ///
    /// # Errors
    /// * [`VestingError::ScheduleNotFound`] – no schedule for this address.
    /// * [`VestingError::Unauthorized`]     – caller is not the issuer.
    pub fn vesting_revoke(
        env: Env,
        issuer: Address,
        beneficiary: Address,
    ) -> Result<(), VestingError> {
        issuer.require_auth();

        let sched_key = VestingKey::Schedule(beneficiary.clone());
        let claimed_key = VestingKey::Claimed(beneficiary.clone());

        let schedule: VestingSchedule = env
            .storage()
            .persistent()
            .get(&sched_key)
            .ok_or(VestingError::ScheduleNotFound)?;

        if schedule.issuer != issuer {
            return Err(VestingError::Unauthorized);
        }

        let already_claimed: i128 = env
            .storage()
            .persistent()
            .get(&claimed_key)
            .unwrap_or(0_i128);

        let now = env.ledger().timestamp();
        let vested = vested_amount(&schedule, now);

        // Tokens owed to beneficiary = vested minus already received.
        let beneficiary_due = if vested > already_claimed {
            vested - already_claimed
        } else {
            0
        };
        // Unvested remainder returns to issuer.
        let issuer_due = schedule.total_amount - already_claimed - beneficiary_due;

        let tok = token::Client::new(&env, &schedule.token);

        if beneficiary_due > 0 {
            tok.transfer(
                &env.current_contract_address(),
                &beneficiary,
                &beneficiary_due,
            );
        }
        if issuer_due > 0 {
            tok.transfer(
                &env.current_contract_address(),
                &issuer,
                &issuer_due,
            );
        }

        // Clean up storage.
        env.storage().persistent().remove(&sched_key);
        env.storage().persistent().remove(&claimed_key);

        env.events().publish(
            (soroban_sdk::symbol_short!("vest_rev"), beneficiary),
            (beneficiary_due, issuer_due),
        );

        Ok(())
    }

    // ── Read-only queries ─────────────────────────────────────────────────────

    /// Return the [`VestingSchedule`] for `beneficiary`, or `None`.
    pub fn get_vesting_schedule(
        env: Env,
        beneficiary: Address,
    ) -> Option<VestingSchedule> {
        env.storage()
            .persistent()
            .get(&VestingKey::Schedule(beneficiary))
    }

    /// Return the total tokens already claimed by `beneficiary`.
    pub fn get_claimed_amount(env: Env, beneficiary: Address) -> i128 {
        env.storage()
            .persistent()
            .get(&VestingKey::Claimed(beneficiary))
            .unwrap_or(0_i128)
    }

    /// Return the tokens vested (but not necessarily claimed) at the current
    /// ledger timestamp.
    ///
    /// Returns `None` if no schedule exists.
    pub fn get_vested_amount(env: Env, beneficiary: Address) -> Option<i128> {
        let schedule: VestingSchedule = env
            .storage()
            .persistent()
            .get(&VestingKey::Schedule(beneficiary))?;
        let now = env.ledger().timestamp();
        Some(vested_amount(&schedule, now))
    }

    /// Return the currently claimable amount for `beneficiary`.
    ///
    /// Returns `None` if no schedule exists, `Some(0)` if nothing is claimable
    /// yet.
    pub fn get_claimable_amount(env: Env, beneficiary: Address) -> Option<i128> {
        let schedule: VestingSchedule = env
            .storage()
            .persistent()
            .get(&VestingKey::Schedule(beneficiary.clone()))?;
        let claimed: i128 = env
            .storage()
            .persistent()
            .get(&VestingKey::Claimed(beneficiary))
            .unwrap_or(0_i128);
        let now = env.ledger().timestamp();
        Some(claimable_amount(&schedule, claimed, now))
    }

    /// Return all schedules for a batch of beneficiaries.
    /// Useful for off-chain dashboards.
    pub fn get_vesting_schedules(
        env: Env,
        beneficiaries: Vec<Address>,
    ) -> Vec<Option<VestingSchedule>> {
        let mut out = Vec::new(&env);
        for b in beneficiaries.iter() {
            let s = env
                .storage()
                .persistent()
                .get(&VestingKey::Schedule(b));
            out.push_back(s);
        }
        out
    }
}