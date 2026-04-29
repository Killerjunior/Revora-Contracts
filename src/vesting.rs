//! Token vesting contract for team, advisors, and other stakeholders.
//!
//! Supports multiple vesting schedules per beneficiary, with linear and cliff-based vesting.
//! Standalone primitive that can integrate with Revora token or revenue-share logic.

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, symbol_short, token, Address, Env, Symbol,
};

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum VestingError {
    Unauthorized = 1,
    ScheduleNotFound = 2,
    ScheduleNotStarted = 3,
    NothingToClaim = 4,
    CancelNotAllowed = 5,
    InvalidAmount = 6,
    InvalidDuration = 7,
    InvalidCliff = 8,
    AmendmentNotAllowed = 9,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct VestingSchedule {
    pub beneficiary: Address,
    pub token: Address,
    pub total_amount: i128,
    pub claimed_amount: i128,
    pub start_time: u64,
    pub cliff_time: u64,
    pub end_time: u64,
    pub cancelled: bool,
}

#[contracttype]
pub enum VestingDataKey {
    Admin,
    ScheduleCount(Address),
    Schedule(Address, u32),
    /// Number of partial claim records stored for a schedule
    ClaimCount(Address, u32),
    /// Partial claim record for (admin, schedule_index, claim_index)
    ClaimRecord(Address, u32, u32),
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
pub struct RevoraVesting;

#[contractimpl]
impl RevoraVesting {
    /// Initialize the vesting contract with an admin.
    /// Renamed to `initialize_vesting` to avoid symbol conflicts with other contracts.
    pub fn initialize_vesting(env: Env, admin: Address) -> Result<(), VestingError> {
        if env.storage().persistent().has(&VestingDataKey::Admin) {
            return Err(VestingError::Unauthorized);
        }
        admin.require_auth();
        env.storage().persistent().set(&VestingDataKey::Admin, &admin);
        Ok(())
    }

    /// Create a vesting schedule. Admin only.
    /// Linear vesting: amount vests linearly from start_time to end_time.
    /// Cliff: nothing vests before cliff_time; after cliff, linear to end_time.
    #[allow(clippy::too_many_arguments)]
    pub fn create_schedule(
        env: Env,
        admin: Address,
        beneficiary: Address,
        token: Address,
        total_amount: i128,
        start_time: u64,
        cliff_duration_secs: u64,
        duration_secs: u64,
    ) -> Result<u32, VestingError> {
        admin.require_auth();
        let stored_admin: Address = env
            .storage()
            .persistent()
            .get(&VestingDataKey::Admin)
            .ok_or(VestingError::Unauthorized)?;
        if admin != stored_admin {
            return Err(VestingError::Unauthorized);
        }
        if total_amount <= 0 {
            return Err(VestingError::InvalidAmount);
        }
        if duration_secs == 0 {
            return Err(VestingError::InvalidDuration);
        }
        if cliff_duration_secs > duration_secs {
            return Err(VestingError::InvalidCliff);
        }

        let end_time = start_time.saturating_add(duration_secs);
        let cliff_time = start_time.saturating_add(cliff_duration_secs);

        let count_key = VestingDataKey::ScheduleCount(admin.clone());
        let count: u32 = env.storage().persistent().get(&count_key).unwrap_or(0);
        let schedule = VestingSchedule {
            beneficiary: beneficiary.clone(),
            token: token.clone(),
            total_amount,
            claimed_amount: 0,
            start_time,
            cliff_time,
            end_time,
            cancelled: false,
        };
        let schedule_key = VestingDataKey::Schedule(admin.clone(), count);
        env.storage().persistent().set(&schedule_key, &schedule);
        env.storage().persistent().set(&count_key, &(count + 1));

        env.events().publish(
            (EVENT_VESTING_CREATED, admin.clone(), beneficiary.clone()),
            (token.clone(), total_amount, start_time, cliff_time, end_time, count),
        );
        env.events().publish(
            (EVENT_VESTING_CREATED_V1, admin, beneficiary),
            (
                VESTING_EVENT_SCHEMA_VERSION,
                token,
                total_amount,
                start_time,
                cliff_time,
                end_time,
                count,
            ),
        );
        Ok(count)
    }

    /// Cancel a schedule (admin only). Business rules: only future unvested amount is forfeit.
    pub fn cancel_schedule(
        env: Env,
        admin: Address,
        beneficiary: Address,
        schedule_index: u32,
    ) -> Result<(), VestingError> {
        admin.require_auth();
        let stored_admin: Address = env
            .storage()
            .persistent()
            .get(&VestingDataKey::Admin)
            .ok_or(VestingError::Unauthorized)?;
        if admin != stored_admin {
            return Err(VestingError::Unauthorized);
        }
        let key = VestingDataKey::Schedule(admin.clone(), schedule_index);
        let mut schedule: VestingSchedule =
            env.storage().persistent().get(&key).ok_or(VestingError::ScheduleNotFound)?;
        if schedule.beneficiary != beneficiary {
            return Err(VestingError::ScheduleNotFound);
        }
        if schedule.cancelled {
            return Err(VestingError::CancelNotAllowed);
        }
        schedule.cancelled = true;
        env.storage().persistent().set(&key, &schedule);
        env.events().publish(
            (EVENT_VESTING_CANCELLED, admin.clone(), beneficiary.clone()),
            (schedule_index, schedule.token.clone()),
        );
        env.events().publish(
            (EVENT_VESTING_CANCELLED_V1, admin, beneficiary),
            (VESTING_EVENT_SCHEMA_VERSION, schedule_index, schedule.token.clone()),
        );
        Ok(())
    }

    /// Amend an existing vesting schedule. Admin only.
    /// Allows updating the total amount, start time, cliff, and duration.
    ///
    /// ### Parameters
    /// - `admin`: The authorized admin address.
    /// - `beneficiary`: The beneficiary of the schedule.
    /// - `schedule_index`: The index of the schedule to amend.
    /// - `new_total_amount`: The new total amount (cannot be less than `claimed_amount`).
    /// - `new_start_time`: The new start timestamp.
    /// - `new_cliff_duration_secs`: The new cliff duration in seconds.
    /// - `new_duration_secs`: The new total duration in seconds.
    ///
    /// ### Security Assumptions
    /// - Caller must be the authorized admin.
    /// - Schedule must exist and not be cancelled.
    /// - New total amount cannot be less than already claimed tokens to maintain accounting integrity.
    /// - Duration and cliff bounds are strictly enforced (duration > 0, cliff <= duration).
    #[allow(clippy::too_many_arguments)]
    pub fn amend_schedule(
        env: Env,
        admin: Address,
        beneficiary: Address,
        schedule_index: u32,
        new_total_amount: i128,
        new_start_time: u64,
        new_cliff_duration_secs: u64,
        new_duration_secs: u64,
    ) -> Result<(), VestingError> {
        admin.require_auth();
        let stored_admin: Address = env
            .storage()
            .persistent()
            .get(&VestingDataKey::Admin)
            .ok_or(VestingError::Unauthorized)?;
        if admin != stored_admin {
            return Err(VestingError::Unauthorized);
        }

        let key = VestingDataKey::Schedule(admin.clone(), schedule_index);
        let mut schedule: VestingSchedule =
            env.storage().persistent().get(&key).ok_or(VestingError::ScheduleNotFound)?;

        if schedule.beneficiary != beneficiary {
            return Err(VestingError::ScheduleNotFound);
        }
        if schedule.cancelled {
            return Err(VestingError::AmendmentNotAllowed);
        }

        // Validity checks
        if new_total_amount < schedule.claimed_amount {
            return Err(VestingError::InvalidAmount);
        }
        if new_duration_secs == 0 {
            return Err(VestingError::InvalidDuration);
        }
        if new_cliff_duration_secs > new_duration_secs {
            return Err(VestingError::InvalidCliff);
        }

        let new_end_time = new_start_time.saturating_add(new_duration_secs);
        let new_cliff_time = new_start_time.saturating_add(new_cliff_duration_secs);

        // Update schedule parameters
        schedule.total_amount = new_total_amount;
        schedule.start_time = new_start_time;
        schedule.cliff_time = new_cliff_time;
        schedule.end_time = new_end_time;

        env.storage().persistent().set(&key, &schedule);

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

        let contract_addr = env.current_contract_address();
        token::Client::new(&env, &schedule.token).transfer(
            &contract_addr,
            &beneficiary,
            &claimable,
        );

        env.events().publish(
            (EVENT_VESTING_CLAIMED, beneficiary.clone(), admin.clone()),
            (schedule_index, schedule.token.clone(), claimable),
        );
        env.events().publish(
            (EVENT_VESTING_CLAIMED_V1, beneficiary.clone(), admin),
            (VESTING_EVENT_SCHEMA_VERSION, schedule_index, schedule.token, claimable),
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
        beneficiary: Address,
        admin: Address,
        schedule_index: u32,
        amount: i128,
    ) -> Result<i128, VestingError> {
        beneficiary.require_auth();
        if amount <= 0 {
            return Err(VestingError::InvalidAmount);
        }
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
        if amount > claimable {
            return Err(VestingError::InvalidAmount);
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

        // Record claim history: append (timestamp, amount)
        let cnt_key = VestingDataKey::ClaimCount(admin.clone(), schedule_index);
        let count: u32 = env.storage().persistent().get(&cnt_key).unwrap_or(0);
        let rec_key = VestingDataKey::ClaimRecord(admin.clone(), schedule_index, count);
        let record: (u64, i128) = (env.ledger().timestamp(), amount);
        env.storage().persistent().set(&rec_key, &record);
        env.storage().persistent().set(&cnt_key, &(count + 1));

        // Emit events for partial claim.
        env.events().publish(
            (EVENT_VESTING_PCLAIM, beneficiary.clone(), admin),
            (schedule_index, token.clone(), amount, count),
        );
        env.events().publish(
            (EVENT_VESTING_PCLAIM_V1, beneficiary, admin),
            (VESTING_EVENT_SCHEMA_VERSION, schedule_index, token, amount, count),
        );
        Ok(amount)
    }

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
        admin: Address,
        schedule_index: u32,
        claim_index: u32,
    ) -> Option<(u64, i128)> {
        env.storage().persistent().get(&VestingDataKey::ClaimRecord(
            admin,
            schedule_index,
            claim_index,
        ))
    }

    /// Query a schedule by admin and index.
    pub fn get_schedule(
        env: Env,
        admin: Address,
        schedule_index: u32,
    ) -> Result<VestingSchedule, VestingError> {
        let key = VestingDataKey::Schedule(admin, schedule_index);
        env.storage().persistent().get(&key).ok_or(VestingError::ScheduleNotFound)
    }

    /// Claimable amount for a schedule (vested minus already claimed).
    /// Renamed to `get_claimable_vesting` to avoid symbol conflicts with other contracts.
    pub fn get_claimable_vesting(
        env: Env,
        admin: Address,
        schedule_index: u32,
    ) -> Result<i128, VestingError> {
        let schedule = Self::get_schedule(env.clone(), admin, schedule_index)?;
        let vested = Self::vested_amount(&env, &schedule);
        Ok(vested.saturating_sub(schedule.claimed_amount))
    }

    /// Number of schedules created by an admin.
    pub fn get_schedule_count(env: Env, admin: Address) -> u32 {
        env.storage().persistent().get(&VestingDataKey::ScheduleCount(admin)).unwrap_or(0)
    }

    /// Returns the current vesting event schema version.
    pub fn get_event_schema_version(env: Env) -> u32 {
        let _ = env;
        VESTING_EVENT_SCHEMA_VERSION
    }
}
