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

// Legacy event symbols (for backward compatibility)
const EVENT_VESTING_CREATED: Symbol = symbol_short!("vest_crt");
const EVENT_VESTING_CLAIMED: Symbol = symbol_short!("vest_clm");
const EVENT_VESTING_CANCELLED: Symbol = symbol_short!("vest_can");
const EVENT_VESTING_AMENDED: Symbol = symbol_short!("vest_amd");
const EVENT_VESTING_PCLAIM: Symbol = symbol_short!("vest_pcl");

/// Shared schema version for vesting events.
pub const VESTING_EVENT_SCHEMA_VERSION: u32 = 1;

const EVENT_VESTING_CREATED_V1: Symbol = symbol_short!("vst_crt1");
const EVENT_VESTING_CLAIMED_V1: Symbol = symbol_short!("vst_clm1");
const EVENT_VESTING_CANCELLED_V1: Symbol = symbol_short!("vst_can1");
const EVENT_VESTING_PCLAIM_V1: Symbol = symbol_short!("vst_pcl1");

// Versioned event symbols (v1 schema)
const EVENT_VESTING_CREATED_V1: Symbol = symbol_short!("vst_crt1");
const EVENT_VESTING_CLAIMED_V1: Symbol = symbol_short!("vst_clm1");
const EVENT_VESTING_CANCELLED_V1: Symbol = symbol_short!("vst_can1");
const EVENT_VESTING_AMENDED_V1: Symbol = symbol_short!("vst_amd1");

// Partial claim event
const EVENT_VESTING_PCLAIM: Symbol = symbol_short!("vest_pcl");

// Event schema version
pub const VESTING_EVENT_SCHEMA_VERSION: u32 = 1;

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

        env.events().publish(
            (EVENT_VESTING_AMENDED, admin.clone(), beneficiary.clone()),
            (schedule_index, new_total_amount, new_start_time, new_cliff_time, new_end_time),
        );
        env.events().publish(
            (EVENT_VESTING_AMENDED_V1, admin, beneficiary),
            (VESTING_EVENT_SCHEMA_VERSION, schedule_index, new_total_amount, new_start_time, new_cliff_time, new_end_time),
        );

        Ok(())
    }

    /// Compute currently vested amount (linear from cliff to end).
    fn vested_amount(env: &Env, schedule: &VestingSchedule) -> i128 {
        let now = env.ledger().timestamp();
        if now < schedule.cliff_time || schedule.cancelled {
            return 0;
        }
        if now >= schedule.end_time {
            return schedule.total_amount;
        }
        let vesting_duration = schedule.end_time - schedule.cliff_time;
        let elapsed = now - schedule.cliff_time;
        let vested = (schedule.total_amount as u128)
            .saturating_mul(elapsed as u128)
            .checked_div(vesting_duration as u128)
            .unwrap_or(0) as i128;
        core::cmp::min(vested, schedule.total_amount)
    }

    /// Claim vested tokens. Callable by beneficiary.
    ///
    /// Claim accounting is checked before storage is updated so the claimed
    /// balance can never exceed the schedule total, even if a partial claim
    /// arrives after other state changes.
    /// Renamed to `claim_vesting` to avoid symbol conflicts with other contracts.
    pub fn claim_vesting(
        env: Env,
        beneficiary: Address,
        admin: Address,
        schedule_index: u32,
    ) -> Result<i128, VestingError> {
        beneficiary.require_auth();
        let key = VestingDataKey::Schedule(admin.clone(), schedule_index);
        let mut schedule: VestingSchedule =
            env.storage().persistent().get(&key).ok_or(VestingError::ScheduleNotFound)?;
        if schedule.beneficiary != beneficiary {
            return Err(VestingError::ScheduleNotFound);
        }
        if schedule.cancelled {
            return Err(VestingError::ScheduleNotFound);
        }
        let vested = Self::vested_amount(&env, &schedule);
        let claimable = vested.saturating_sub(schedule.claimed_amount);
        if claimable <= 0 {
            return Err(VestingError::NothingToClaim);
        }
        let new_claimed =
            schedule.claimed_amount.checked_add(claimable).ok_or(VestingError::InvalidAmount)?;
        if new_claimed > schedule.total_amount {
            return Err(VestingError::InvalidAmount);
        }
        schedule.claimed_amount = new_claimed;
        env.storage().persistent().set(&key, &schedule);

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

    /// Claim a specific amount of currently claimable tokens (partial claim).
    ///
    /// Partial claims are append-only. Each success writes one ledger record,
    /// advances the schedule cursor, and emits both legacy and versioned
    /// partial-claim events.
    pub fn claim_vesting_partial(
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

        let new_claimed =
            schedule.claimed_amount.checked_add(amount).ok_or(VestingError::InvalidAmount)?;
        if new_claimed > schedule.total_amount {
            return Err(VestingError::InvalidAmount);
        }

        // Update claimed amount first so the schedule stays internally consistent
        // if later checks or transfers fail and the transaction rolls back.
        schedule.claimed_amount = new_claimed;
        env.storage().persistent().set(&key, &schedule);

        // Transfer tokens from this contract to beneficiary
        let contract_addr = env.current_contract_address();
        token::Client::new(&env, &schedule.token).transfer(&contract_addr, &beneficiary, &amount);
        let token = schedule.token.clone();

        // Clean up storage.
        env.storage().persistent().remove(&sched_key);
        env.storage().persistent().remove(&claimed_key);

        // Emit events for partial claim.
        env.events().publish(
            (EVENT_VESTING_PCLAIM, beneficiary.clone(), admin),
            (schedule_index, token.clone(), amount, count),
        );
        env.events().publish(
            (EVENT_VESTING_PCLAIM_V1, beneficiary, admin),
            (VESTING_EVENT_SCHEMA_VERSION, schedule_index, token, amount, count),
        );

    /// Return the append-only cursor for partial-claim records.
    ///
    /// The value is also the current record count. The next successful
    /// partial claim is written at this index.
    pub fn get_partial_claim_count(env: Env, admin: Address, schedule_index: u32) -> u32 {
        env.storage()
            .persistent()
            .get(&VestingDataKey::ClaimCount(admin, schedule_index))
            .unwrap_or(0)
    }

    /// Return a partial-claim ledger record `(timestamp, amount)` by cursor index.
    pub fn get_partial_claim_record(
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