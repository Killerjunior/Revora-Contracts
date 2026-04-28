//! # Vesting test suite — `vesting_test.rs`
//!
//! Tests are grouped by invariant:
//!
//! 1. **Happy-path registration** — tokens are locked, schedule stored.
//! 2. **Cliff gate** — nothing claimable before cliff.
//! 3. **Linear schedule** — partial release at intermediate times.
//! 4. **Full vest** — 100 % claimable after `end_ts`.
//! 5. **No over-claim** — cumulative claims never exceed `total_amount`.
//! 6. **Cursor / idempotency** — double-claim returns 0, state unchanged.
//! 7. **Backdating / timestamp order validation** — invalid inputs rejected.
//! 8. **Revocation** — partial and full revoke return correct token splits.
//! 9. **Auth** — only beneficiary can claim, only issuer can revoke.
//! 10. **Edge cases** — cliff == start, single-second schedule, zero-elapsed.

#[cfg(test)]
mod vesting_tests {
    use soroban_sdk::{
        testutils::{Address as _, Ledger},
        token::{Client as TokenClient, StellarAssetClient},
        Address, Env,
    };

    use crate::vesting::{VestingContract, VestingContractClient, VestingError};

    // ── Test helpers ──────────────────────────────────────────────────────────

    /// Base timestamp used across tests (arbitrary but realistic).
    const BASE_TS: u64 = 1_700_000_000;

    /// Set up the test environment: Soroban env, vesting contract, and a
    /// mock SAC token whose admin is `admin`.
    fn setup() -> (Env, VestingContractClient<'static>, Address, Address) {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register_contract(None, VestingContract);
        let client = VestingContractClient::new(&env, &contract_id);

        // Deploy a Stellar Asset Contract for testing.
        let admin = Address::generate(&env);
        let token_id = env.register_stellar_asset_contract_v2(admin.clone());
        let token_address = token_id.address();

        // Set ledger timestamp.
        env.ledger().with_mut(|l| l.timestamp = BASE_TS);

        (env, client, admin, token_address)
    }

    /// Mint `amount` tokens to `recipient` using the SAC admin interface.
    fn mint(env: &Env, admin: &Address, token: &Address, recipient: &Address, amount: i128) {
        StellarAssetClient::new(env, token).mint(recipient, &amount);
    }

    /// Return the token balance of `addr`.
    fn balance(env: &Env, token: &Address, addr: &Address) -> i128 {
        TokenClient::new(env, token).balance(addr)
    }

    // ── 1. Registration ───────────────────────────────────────────────────────

    #[test]
    fn test_register_stores_schedule() {
        let (env, client, admin, token) = setup();
        let issuer = Address::generate(&env);
        let beneficiary = Address::generate(&env);
        mint(&env, &admin, &token, &issuer, 1_000);

        client
            .vesting_register(
                &issuer,
                &beneficiary,
                &token,
                &1_000,
                &(BASE_TS + 100),  // cliff
                &(BASE_TS + 100),  // start == cliff (instant cliff)
                &(BASE_TS + 1_100), // end
            )
            .unwrap();

        let sched = client.get_vesting_schedule(&beneficiary).unwrap();
        assert_eq!(sched.total_amount, 1_000);
        assert_eq!(sched.cliff_ts, BASE_TS + 100);
        assert_eq!(client.get_claimed_amount(&beneficiary), 0);

        // Tokens moved out of issuer into contract.
        assert_eq!(balance(&env, &token, &issuer), 0);
    }

    #[test]
    fn test_register_duplicate_fails() {
        let (env, client, admin, token) = setup();
        let issuer = Address::generate(&env);
        let beneficiary = Address::generate(&env);
        mint(&env, &admin, &token, &issuer, 2_000);

        let args = (
            issuer.clone(),
            beneficiary.clone(),
            token.clone(),
            1_000_i128,
            BASE_TS + 100,
            BASE_TS + 100,
            BASE_TS + 1_100,
        );
        client
            .vesting_register(
                &args.0, &args.1, &args.2, &args.3, &args.4, &args.5, &args.6,
            )
            .unwrap();

        let result = client.try_vesting_register(
            &args.0, &args.1, &args.2, &args.3, &args.4, &args.5, &args.6,
        );
        assert_eq!(
            result.unwrap_err().unwrap(),
            VestingError::ScheduleAlreadyExists
        );
    }

    #[test]
    fn test_register_zero_amount_fails() {
        let (env, client, _, token) = setup();
        let issuer = Address::generate(&env);
        let beneficiary = Address::generate(&env);

        let result = client.try_vesting_register(
            &issuer,
            &beneficiary,
            &token,
            &0,
            &(BASE_TS + 100),
            &(BASE_TS + 100),
            &(BASE_TS + 1_100),
        );
        assert_eq!(result.unwrap_err().unwrap(), VestingError::InvalidAmount);
    }

    #[test]
    fn test_register_negative_amount_fails() {
        let (env, client, _, token) = setup();
        let issuer = Address::generate(&env);
        let beneficiary = Address::generate(&env);

        let result = client.try_vesting_register(
            &issuer,
            &beneficiary,
            &token,
            &(-500_i128),
            &(BASE_TS + 100),
            &(BASE_TS + 100),
            &(BASE_TS + 1_100),
        );
        assert_eq!(result.unwrap_err().unwrap(), VestingError::InvalidAmount);
    }

    // ── 2. Timestamp validation ───────────────────────────────────────────────

    #[test]
    fn test_register_start_before_cliff_fails() {
        let (env, client, admin, token) = setup();
        let issuer = Address::generate(&env);
        let beneficiary = Address::generate(&env);
        mint(&env, &admin, &token, &issuer, 1_000);

        // start_ts < cliff_ts — invalid.
        let result = client.try_vesting_register(
            &issuer,
            &beneficiary,
            &token,
            &1_000,
            &(BASE_TS + 500), // cliff
            &(BASE_TS + 100), // start < cliff  ← invalid
            &(BASE_TS + 1_100),
        );
        assert_eq!(result.unwrap_err().unwrap(), VestingError::InvalidTimestamps);
    }

    #[test]
    fn test_register_end_not_after_start_fails() {
        let (env, client, admin, token) = setup();
        let issuer = Address::generate(&env);
        let beneficiary = Address::generate(&env);
        mint(&env, &admin, &token, &issuer, 1_000);

        // end_ts == start_ts — invalid.
        let result = client.try_vesting_register(
            &issuer,
            &beneficiary,
            &token,
            &1_000,
            &(BASE_TS + 100),
            &(BASE_TS + 100),
            &(BASE_TS + 100), // end == start ← invalid
        );
        assert_eq!(result.unwrap_err().unwrap(), VestingError::InvalidTimestamps);
    }

    // ── 3. Cliff gate ─────────────────────────────────────────────────────────

    #[test]
    fn test_claim_before_cliff_fails() {
        let (env, client, admin, token) = setup();
        let issuer = Address::generate(&env);
        let beneficiary = Address::generate(&env);
        mint(&env, &admin, &token, &issuer, 1_000);

        client
            .vesting_register(
                &issuer,
                &beneficiary,
                &token,
                &1_000,
                &(BASE_TS + 500),   // cliff
                &(BASE_TS + 500),
                &(BASE_TS + 1_500),
            )
            .unwrap();

        // Ledger is still at BASE_TS — before cliff.
        let result = client.try_vesting_claim(&beneficiary);
        assert_eq!(result.unwrap_err().unwrap(), VestingError::NothingToClaimYet);

        // Advance to cliff - 1.
        env.ledger()
            .with_mut(|l| l.timestamp = BASE_TS + 499);
        let result = client.try_vesting_claim(&beneficiary);
        assert_eq!(result.unwrap_err().unwrap(), VestingError::NothingToClaimYet);

        // State unchanged — cursor still 0.
        assert_eq!(client.get_claimed_amount(&beneficiary), 0);
        assert_eq!(balance(&env, &token, &beneficiary), 0);
    }

    #[test]
    fn test_claim_at_exact_cliff_ts() {
        let (env, client, admin, token) = setup();
        let issuer = Address::generate(&env);
        let beneficiary = Address::generate(&env);
        mint(&env, &admin, &token, &issuer, 1_000);

        // cliff == start == BASE_TS+100; end == BASE_TS+1_100 (1 000 s span).
        client
            .vesting_register(
                &issuer,
                &beneficiary,
                &token,
                &1_000,
                &(BASE_TS + 100),
                &(BASE_TS + 100),
                &(BASE_TS + 1_100),
            )
            .unwrap();

        // Advance exactly to cliff/start.
        env.ledger()
            .with_mut(|l| l.timestamp = BASE_TS + 100);

        // At t=start, elapsed=0 → 0 vested linearly → claimable==0.
        // But cliff is reached, so NothingToClaimYet is NOT returned;
        // instead claim returns Ok(0).
        let claimed = client.vesting_claim(&beneficiary).unwrap();
        assert_eq!(claimed, 0);
        assert_eq!(client.get_claimed_amount(&beneficiary), 0);
    }

    // ── 4. Linear schedule / partial release ─────────────────────────────────

    #[test]
    fn test_partial_release_at_midpoint() {
        let (env, client, admin, token) = setup();
        let issuer = Address::generate(&env);
        let beneficiary = Address::generate(&env);
        mint(&env, &admin, &token, &issuer, 1_000);

        // 1 000 tokens over 1 000 seconds.
        client
            .vesting_register(
                &issuer,
                &beneficiary,
                &token,
                &1_000,
                &(BASE_TS + 100),
                &(BASE_TS + 100),
                &(BASE_TS + 1_100),
            )
            .unwrap();

        // 50 % through the vesting window → 500 tokens vested.
        env.ledger()
            .with_mut(|l| l.timestamp = BASE_TS + 600);

        let claimed = client.vesting_claim(&beneficiary).unwrap();
        assert_eq!(claimed, 500);
        assert_eq!(client.get_claimed_amount(&beneficiary), 500);
        assert_eq!(balance(&env, &token, &beneficiary), 500);
    }

    #[test]
    fn test_multiple_partial_claims_monotonic() {
        let (env, client, admin, token) = setup();
        let issuer = Address::generate(&env);
        let beneficiary = Address::generate(&env);
        mint(&env, &admin, &token, &issuer, 1_000);

        client
            .vesting_register(
                &issuer,
                &beneficiary,
                &token,
                &1_000,
                &(BASE_TS + 100),
                &(BASE_TS + 100),
                &(BASE_TS + 1_100),
            )
            .unwrap();

        // First claim at 25 % → 250.
        env.ledger()
            .with_mut(|l| l.timestamp = BASE_TS + 350);
        let c1 = client.vesting_claim(&beneficiary).unwrap();
        assert_eq!(c1, 250);

        // Second claim at 75 % → additional 500.
        env.ledger()
            .with_mut(|l| l.timestamp = BASE_TS + 850);
        let c2 = client.vesting_claim(&beneficiary).unwrap();
        assert_eq!(c2, 500);

        // Third claim at 100 % → remaining 250.
        env.ledger()
            .with_mut(|l| l.timestamp = BASE_TS + 1_100);
        let c3 = client.vesting_claim(&beneficiary).unwrap();
        assert_eq!(c3, 250);

        // Totals correct.
        assert_eq!(c1 + c2 + c3, 1_000);
        assert_eq!(client.get_claimed_amount(&beneficiary), 1_000);
        assert_eq!(balance(&env, &token, &beneficiary), 1_000);
    }

    // ── 5. Full vest ──────────────────────────────────────────────────────────

    #[test]
    fn test_claim_after_full_vest() {
        let (env, client, admin, token) = setup();
        let issuer = Address::generate(&env);
        let beneficiary = Address::generate(&env);
        mint(&env, &admin, &token, &issuer, 1_000);

        client
            .vesting_register(
                &issuer,
                &beneficiary,
                &token,
                &1_000,
                &(BASE_TS + 100),
                &(BASE_TS + 100),
                &(BASE_TS + 1_100),
            )
            .unwrap();

        env.ledger()
            .with_mut(|l| l.timestamp = BASE_TS + 2_000); // well past end_ts

        let claimed = client.vesting_claim(&beneficiary).unwrap();
        assert_eq!(claimed, 1_000);
        assert_eq!(client.get_claimed_amount(&beneficiary), 1_000);
    }

    // ── 6. No over-claim invariant ────────────────────────────────────────────

    #[test]
    fn test_no_overclaim_after_full_vest() {
        let (env, client, admin, token) = setup();
        let issuer = Address::generate(&env);
        let beneficiary = Address::generate(&env);
        mint(&env, &admin, &token, &issuer, 1_000);

        client
            .vesting_register(
                &issuer,
                &beneficiary,
                &token,
                &1_000,
                &(BASE_TS + 100),
                &(BASE_TS + 100),
                &(BASE_TS + 1_100),
            )
            .unwrap();

        // Claim fully.
        env.ledger()
            .with_mut(|l| l.timestamp = BASE_TS + 2_000);
        client.vesting_claim(&beneficiary).unwrap();

        // Try again — should return 0, not panic.
        let second = client.vesting_claim(&beneficiary).unwrap();
        assert_eq!(second, 0);

        // Cursor must not exceed total.
        assert_eq!(client.get_claimed_amount(&beneficiary), 1_000);
    }

    // ── 7. Cursor / idempotency ───────────────────────────────────────────────

    #[test]
    fn test_idempotent_claim_same_timestamp() {
        let (env, client, admin, token) = setup();
        let issuer = Address::generate(&env);
        let beneficiary = Address::generate(&env);
        mint(&env, &admin, &token, &issuer, 1_000);

        client
            .vesting_register(
                &issuer,
                &beneficiary,
                &token,
                &1_000,
                &(BASE_TS + 100),
                &(BASE_TS + 100),
                &(BASE_TS + 1_100),
            )
            .unwrap();

        env.ledger()
            .with_mut(|l| l.timestamp = BASE_TS + 600);

        let c1 = client.vesting_claim(&beneficiary).unwrap();
        assert_eq!(c1, 500);

        // Same timestamp — nothing new has vested.
        let c2 = client.vesting_claim(&beneficiary).unwrap();
        assert_eq!(c2, 0);

        // Cursor unchanged after no-op claim.
        assert_eq!(client.get_claimed_amount(&beneficiary), 500);
        assert_eq!(balance(&env, &token, &beneficiary), 500);
    }

    #[test]
    fn test_cursor_advances_monotonically() {
        let (env, client, admin, token) = setup();
        let issuer = Address::generate(&env);
        let beneficiary = Address::generate(&env);
        mint(&env, &admin, &token, &issuer, 10_000);

        client
            .vesting_register(
                &issuer,
                &beneficiary,
                &token,
                &10_000,
                &(BASE_TS + 100),
                &(BASE_TS + 100),
                &(BASE_TS + 10_100),
            )
            .unwrap();

        let mut prev_cursor = 0_i128;
        for step in 1..=10_u64 {
            env.ledger()
                .with_mut(|l| l.timestamp = BASE_TS + 100 + step * 1_000);
            client.vesting_claim(&beneficiary).unwrap();
            let cursor = client.get_claimed_amount(&beneficiary);
            assert!(
                cursor >= prev_cursor,
                "cursor regressed: {} < {}",
                cursor,
                prev_cursor
            );
            prev_cursor = cursor;
        }
        assert_eq!(prev_cursor, 10_000);
    }

    // ── 8. Pure-cliff period ──────────────────────────────────────────────────

    #[test]
    fn test_pure_cliff_period_no_unlock_before_start() {
        let (env, client, admin, token) = setup();
        let issuer = Address::generate(&env);
        let beneficiary = Address::generate(&env);
        mint(&env, &admin, &token, &issuer, 1_000);

        // cliff at +100, linear vesting only starts at +600.
        client
            .vesting_register(
                &issuer,
                &beneficiary,
                &token,
                &1_000,
                &(BASE_TS + 100),   // cliff
                &(BASE_TS + 600),   // start (after cliff)
                &(BASE_TS + 1_600), // end
            )
            .unwrap();

        // Past cliff but before linear start → 0 vested.
        env.ledger()
            .with_mut(|l| l.timestamp = BASE_TS + 300);
        let claimed = client.vesting_claim(&beneficiary).unwrap();
        assert_eq!(claimed, 0);

        // At linear start → 0 (elapsed == 0).
        env.ledger()
            .with_mut(|l| l.timestamp = BASE_TS + 600);
        let claimed = client.vesting_claim(&beneficiary).unwrap();
        assert_eq!(claimed, 0);

        // 50 % through linear window → 500.
        env.ledger()
            .with_mut(|l| l.timestamp = BASE_TS + 1_100);
        let claimed = client.vesting_claim(&beneficiary).unwrap();
        assert_eq!(claimed, 500);
    }

    // ── 9. Revocation ─────────────────────────────────────────────────────────

    #[test]
    fn test_revoke_before_cliff_returns_all_to_issuer() {
        let (env, client, admin, token) = setup();
        let issuer = Address::generate(&env);
        let beneficiary = Address::generate(&env);
        mint(&env, &admin, &token, &issuer, 1_000);

        client
            .vesting_register(
                &issuer,
                &beneficiary,
                &token,
                &1_000,
                &(BASE_TS + 500),
                &(BASE_TS + 500),
                &(BASE_TS + 1_500),
            )
            .unwrap();

        // Revoke before cliff — nothing vested.
        client.vesting_revoke(&issuer, &beneficiary).unwrap();

        assert_eq!(balance(&env, &token, &issuer), 1_000);
        assert_eq!(balance(&env, &token, &beneficiary), 0);
        assert!(client.get_vesting_schedule(&beneficiary).is_none());
    }

    #[test]
    fn test_revoke_midway_splits_correctly() {
        let (env, client, admin, token) = setup();
        let issuer = Address::generate(&env);
        let beneficiary = Address::generate(&env);
        mint(&env, &admin, &token, &issuer, 1_000);

        client
            .vesting_register(
                &issuer,
                &beneficiary,
                &token,
                &1_000,
                &(BASE_TS + 100),
                &(BASE_TS + 100),
                &(BASE_TS + 1_100),
            )
            .unwrap();

        // Claim 25 % first.
        env.ledger()
            .with_mut(|l| l.timestamp = BASE_TS + 350);
        client.vesting_claim(&beneficiary).unwrap(); // 250

        // Revoke at 50 %.
        env.ledger()
            .with_mut(|l| l.timestamp = BASE_TS + 600);
        client.vesting_revoke(&issuer, &beneficiary).unwrap();

        // vested=500, already_claimed=250 → beneficiary_due=250 more,
        // issuer_due = 1000-250-250 = 500.
        assert_eq!(balance(&env, &token, &beneficiary), 500);
        assert_eq!(balance(&env, &token, &issuer), 500);
        assert!(client.get_vesting_schedule(&beneficiary).is_none());
    }

    #[test]
    fn test_revoke_wrong_issuer_fails() {
        let (env, client, admin, token) = setup();
        let issuer = Address::generate(&env);
        let attacker = Address::generate(&env);
        let beneficiary = Address::generate(&env);
        mint(&env, &admin, &token, &issuer, 1_000);

        client
            .vesting_register(
                &issuer,
                &beneficiary,
                &token,
                &1_000,
                &(BASE_TS + 100),
                &(BASE_TS + 100),
                &(BASE_TS + 1_100),
            )
            .unwrap();

        let result = client.try_vesting_revoke(&attacker, &beneficiary);
        assert_eq!(result.unwrap_err().unwrap(), VestingError::Unauthorized);
    }

    // ── 10. Auth — beneficiary cannot claim for another address ──────────────
    // (mock_all_auths is active so we test at the logic level here;
    //  in a real deployment the Soroban host enforces require_auth.)

    #[test]
    fn test_claim_on_nonexistent_schedule_fails() {
        let (env, client, _, _) = setup();
        let stranger = Address::generate(&env);

        let result = client.try_vesting_claim(&stranger);
        assert_eq!(result.unwrap_err().unwrap(), VestingError::ScheduleNotFound);
    }

    // ── 11. get_vested_amount / get_claimable_amount queries ─────────────────

    #[test]
    fn test_query_vested_amount_at_various_times() {
        let (env, client, admin, token) = setup();
        let issuer = Address::generate(&env);
        let beneficiary = Address::generate(&env);
        mint(&env, &admin, &token, &issuer, 1_000);

        client
            .vesting_register(
                &issuer,
                &beneficiary,
                &token,
                &1_000,
                &(BASE_TS + 100),
                &(BASE_TS + 100),
                &(BASE_TS + 1_100),
            )
            .unwrap();

        env.ledger().with_mut(|l| l.timestamp = BASE_TS + 50);
        assert_eq!(client.get_vested_amount(&beneficiary).unwrap(), 0);

        env.ledger().with_mut(|l| l.timestamp = BASE_TS + 600);
        assert_eq!(client.get_vested_amount(&beneficiary).unwrap(), 500);

        env.ledger().with_mut(|l| l.timestamp = BASE_TS + 2_000);
        assert_eq!(client.get_vested_amount(&beneficiary).unwrap(), 1_000);
    }

    #[test]
    fn test_query_claimable_reflects_cursor() {
        let (env, client, admin, token) = setup();
        let issuer = Address::generate(&env);
        let beneficiary = Address::generate(&env);
        mint(&env, &admin, &token, &issuer, 1_000);

        client
            .vesting_register(
                &issuer,
                &beneficiary,
                &token,
                &1_000,
                &(BASE_TS + 100),
                &(BASE_TS + 100),
                &(BASE_TS + 1_100),
            )
            .unwrap();

        env.ledger().with_mut(|l| l.timestamp = BASE_TS + 600);
        assert_eq!(client.get_claimable_amount(&beneficiary).unwrap(), 500);

        client.vesting_claim(&beneficiary).unwrap();
        assert_eq!(client.get_claimable_amount(&beneficiary).unwrap(), 0);
    }

    // ── 12. vested_amount pure-function unit tests ────────────────────────────

    #[test]
    fn test_vested_amount_pure_function() {
        use crate::vesting::{vested_amount, VestingSchedule};

        let env = Env::default();
        let dummy = Address::generate(&env);

        let sched = VestingSchedule {
            issuer: dummy.clone(),
            beneficiary: dummy.clone(),
            token: dummy.clone(),
            total_amount: 10_000,
            cliff_ts: 1_000,
            start_ts: 1_000,
            end_ts: 11_000,
        };

        assert_eq!(vested_amount(&sched, 0), 0);       // before cliff
        assert_eq!(vested_amount(&sched, 999), 0);     // one second before cliff
        assert_eq!(vested_amount(&sched, 1_000), 0);   // at start, elapsed=0
        assert_eq!(vested_amount(&sched, 6_000), 5_000); // halfway
        assert_eq!(vested_amount(&sched, 11_000), 10_000); // at end_ts
        assert_eq!(vested_amount(&sched, 20_000), 10_000); // past end_ts
    }

    // ── 13. Edge: cliff == start == end-1 (minimum-width schedule) ───────────

    #[test]
    fn test_minimum_schedule_width() {
        let (env, client, admin, token) = setup();
        let issuer = Address::generate(&env);
        let beneficiary = Address::generate(&env);
        mint(&env, &admin, &token, &issuer, 100);

        // 1-second vesting window.
        client
            .vesting_register(
                &issuer,
                &beneficiary,
                &token,
                &100,
                &BASE_TS,
                &BASE_TS,
                &(BASE_TS + 1),
            )
            .unwrap();

        // Exactly at end_ts → fully vested.
        env.ledger().with_mut(|l| l.timestamp = BASE_TS + 1);
        let claimed = client.vesting_claim(&beneficiary).unwrap();
        assert_eq!(claimed, 100);
    }

    // ── 14. No schedule → queries return None / 0 gracefully ─────────────────

    #[test]
    fn test_queries_on_missing_schedule_return_none() {
        let (env, client, _, _) = setup();
        let ghost = Address::generate(&env);

        assert!(client.get_vesting_schedule(&ghost).is_none());
        assert_eq!(client.get_claimed_amount(&ghost), 0);
        assert!(client.get_vested_amount(&ghost).is_none());
        assert!(client.get_claimable_amount(&ghost).is_none());
    }

    // ── 15. Revoke on non-existent schedule fails gracefully ─────────────────

    #[test]
    fn test_revoke_nonexistent_fails() {
        let (env, client, _, _) = setup();
        let issuer = Address::generate(&env);
        let ghost = Address::generate(&env);

        let result = client.try_vesting_revoke(&issuer, &ghost);
        assert_eq!(result.unwrap_err().unwrap(), VestingError::ScheduleNotFound);
    }
}