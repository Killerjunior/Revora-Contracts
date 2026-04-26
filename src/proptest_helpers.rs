/// # Proptest Helpers — Contract Fuzz Harness
///
/// Provides deterministic, composable strategies for property-based and fuzz testing
/// of the Revora revenue-share contract. All strategies are pure (no side effects)
/// and designed to be composed into larger operation sequences.
///
/// ## Security Assumptions
/// - Strategies generate both valid and invalid inputs to exercise rejection paths.
/// - `arb_valid_operation_sequence` filters to sequences that preserve key invariants
///   (period ordering, bps bounds) so the contract's own guards are the last line of defense.
/// - Strategies do NOT mock auth; callers must set up `env.mock_all_auths()` in tests.
///
/// ## Usage
/// ```ignore
/// proptest! {
///     #[test]
///     fn fuzz_register_offering(bps in 0u32..=10_000) {
///         let env = Env::default();
///         env.mock_all_auths();
///         let client = make_client(&env);
///         let issuer = Address::generate(&env);
///         let token  = Address::generate(&env);
///         client.register_offering(&issuer, &symbol_short!("def"), &token, &bps, &token, &0);
///     }
/// }
/// ```

#[cfg(test)]
use crate::ProposalAction;
use proptest::prelude::*;

// ── Primitive strategies ─────────────────────────────────────────────────────

/// Any valid basis-points value (0–10 000 inclusive).
pub fn arb_valid_bps() -> impl Strategy<Value = u32> {
    0u32..=10_000
}

/// Any invalid basis-points value (> 10 000).
pub fn arb_invalid_bps() -> impl Strategy<Value = u32> {
    10_001u32..=u32::MAX
}

/// Any strictly positive amount (1 .. 100 000 000).
pub fn any_positive_amount() -> impl Strategy<Value = i128> {
    1i128..=100_000_000
}

/// Any non-negative amount (0 .. 100 000 000).
pub fn arb_non_negative_amount() -> impl Strategy<Value = i128> {
    0i128..=100_000_000
}

/// Any negative amount (i128::MIN .. -1).
pub fn arb_negative_amount() -> impl Strategy<Value = i128> {
    i128::MIN..=-1i128
}

/// Boundary amounts that stress edge cases: MIN, -1, 0, 1, MAX.
pub fn arb_boundary_amount() -> impl Strategy<Value = i128> {
    prop_oneof![
        Just(i128::MIN),
        Just(i128::MIN + 1),
        Just(-1i128),
        Just(0i128),
        Just(1i128),
        Just(i128::MAX - 1),
        Just(i128::MAX),
    ]
}

/// Strictly positive period IDs (1 .. u64::MAX).
pub fn arb_positive_period_id() -> impl Strategy<Value = u64> {
    1u64..=u64::MAX
}

/// Boundary period IDs: 0, 1, 2, u64::MAX-1, u64::MAX.
pub fn arb_boundary_period_id() -> impl Strategy<Value = u64> {
    prop_oneof![
        Just(0u64),
        Just(1u64),
        Just(2u64),
        Just(u64::MAX - 1),
        Just(u64::MAX),
    ]
}

// ── Sequence strategies ──────────────────────────────────────────────────────

/// Generate a vector of `len` strictly-increasing u64 period IDs starting from 1.
/// Invariant: each element is strictly greater than the previous.
pub fn arb_strictly_increasing_periods(len: usize) -> impl Strategy<Value = Vec<u64>> {
    Just(
        (1..=len)
            .map(|i| (i as u64) * 10) // gaps of 10 to avoid off-by-one collisions
            .collect::<Vec<u64>>(),
    )
}

// ── Operation enum ───────────────────────────────────────────────────────────

/// Represents a single contract operation for sequence-based fuzz testing.
///
/// Each variant encodes the parameters needed to invoke the corresponding
/// contract entry point. Variants are designed to be generated independently
/// and composed into sequences.
#[derive(Debug, Clone)]
pub enum TestOperation {
    /// `register_offering(issuer, namespace, token, bps, payout_asset, supply_cap)`
    RegisterOffering { issuer: Address, namespace: Symbol, token: Address, bps: u32, payout_asset: Address, supply_cap: i128 },
    /// `report_revenue(issuer, namespace, token, payout_asset, amount, period_id, override_existing)`
    ReportRevenue { issuer: Address, namespace: Symbol, token: Address, payout_asset: Address, amount: i128, period_id: u64, override_existing: bool },
    /// `deposit_revenue(issuer, namespace, token, payment_token, amount, period_id)`
    DepositRevenue { issuer: Address, namespace: Symbol, token: Address, payment_token: Address, amount: i128, period_id: u64 },
    /// `set_holder_share(issuer, namespace, token, holder, share_bps)`
    SetHolderShare { issuer: Address, namespace: Symbol, token: Address, holder: Address, share_bps: u32 },
    /// `blacklist_add(caller, issuer, namespace, token, investor)`
    BlacklistAdd { caller: Address, issuer: Address, namespace: Symbol, token: Address, investor: Address },
    /// `blacklist_remove(caller, issuer, namespace, token, investor)`
    BlacklistRemove { caller: Address, issuer: Address, namespace: Symbol, token: Address, investor: Address },
    /// `set_concentration_limit(issuer, namespace, token, max_bps, enforce)`
    SetConcentrationLimit { max_bps: u32, enforce: bool },
    /// `report_concentration(issuer, namespace, token, concentration_bps)`
    ReportConcentration { concentration_bps: u32 },
    /// `freeze()` — admin-only global freeze
    Freeze,
    /// `pause_admin(caller)` — admin-only pause
    Pause { caller: Address },
    /// `set_claim_delay(issuer, namespace, token, delay_secs)`
    SetClaimDelay { issuer: Address, namespace: Symbol, token: Address, delay_secs: u64 },
}

// ── Operation strategies ─────────────────────────────────────────────────────

/// Strategy for a single valid `RegisterOffering` operation.
pub fn arb_register_offering() -> impl Strategy<Value = TestOperation> {
    (arb_valid_bps(), 0i128..=1_000_000_000i128)
        .prop_map(|(bps, supply_cap)| TestOperation::RegisterOffering { bps, supply_cap })
}

/// Strategy for a single valid `ReportRevenue` operation.
pub fn arb_report_revenue() -> impl Strategy<Value = TestOperation> {
    (any_positive_amount(), arb_positive_period_id(), any::<bool>()).prop_map(
        |(amount, period_id, override_existing)| TestOperation::ReportRevenue {
            amount,
            period_id,
            override_existing,
        },
    )
}

/// Strategy for a single valid `DepositRevenue` operation.
pub fn arb_deposit_revenue() -> impl Strategy<Value = TestOperation> {
    (any_positive_amount(), arb_positive_period_id())
        .prop_map(|(amount, period_id)| TestOperation::DepositRevenue { amount, period_id })
}

/// Strategy for a single valid `SetHolderShare` operation.
pub fn arb_set_holder_share() -> impl Strategy<Value = TestOperation> {
    (any::<u8>(), arb_valid_bps())
        .prop_map(|(holder_index, share_bps)| TestOperation::SetHolderShare { holder_index, share_bps })
}

/// Strategy for a single `BlacklistAdd` operation.
pub fn arb_blacklist_add() -> impl Strategy<Value = TestOperation> {
    any::<u8>().prop_map(|target_index| TestOperation::BlacklistAdd { target_index })
}

/// Strategy for a single `BlacklistRemove` operation.
pub fn arb_blacklist_remove() -> impl Strategy<Value = TestOperation> {
    any::<u8>().prop_map(|target_index| TestOperation::BlacklistRemove { target_index })
}

/// Strategy for a single `SetConcentrationLimit` operation.
pub fn arb_set_concentration_limit() -> impl Strategy<Value = TestOperation> {
    (arb_valid_bps(), any::<bool>())
        .prop_map(|(max_bps, enforce)| TestOperation::SetConcentrationLimit { max_bps, enforce })
}

/// Strategy for a single `ReportConcentration` operation.
pub fn arb_report_concentration() -> impl Strategy<Value = TestOperation> {
    arb_valid_bps().prop_map(|concentration_bps| TestOperation::ReportConcentration { concentration_bps })
}

/// Strategy for any single valid operation (uniform distribution).
pub fn arb_any_operation() -> impl Strategy<Value = TestOperation> {
    prop_oneof![
        arb_register_offering(),
        arb_report_revenue(),
        arb_deposit_revenue(),
        arb_set_holder_share(),
        arb_blacklist_add(),
        arb_blacklist_remove(),
        arb_set_concentration_limit(),
        arb_report_concentration(),
        Just(TestOperation::Freeze),
        (0u64..=3600u64).prop_map(|d| TestOperation::SetClaimDelay { delay_secs: d }),
    ]
}

/// Strategy for a sequence of `len` valid operations.
///
/// Sequences are filtered to ensure period IDs are strictly increasing within
/// `ReportRevenue` and `DepositRevenue` operations, preserving the contract's
/// period-ordering invariant.
pub fn arb_valid_operation_sequence(len: usize) -> impl Strategy<Value = Vec<TestOperation>> {
    prop::collection::vec(arb_any_operation(), len).prop_map(|mut ops| {
        // Normalize period IDs to be strictly increasing across the sequence.
        let mut next_period: u64 = 1;
        for op in ops.iter_mut() {
            match op {
                TestOperation::ReportRevenue { period_id, .. }
                | TestOperation::DepositRevenue { period_id, .. } => {
                    *period_id = next_period;
                    next_period += 1;
                }
                _ => {}
            }
        }
        ops
    })
}

// ── Invariant validators ─────────────────────────────────────────────────────

/// Verify that a sequence of operations preserves the period-ordering invariant.
/// Returns true if all period IDs in report/deposit ops are strictly increasing.
pub fn sequence_has_valid_period_ordering(ops: &[TestOperation]) -> bool {
    let mut last_period: u64 = 0;
    for op in ops {
        match op {
            TestOperation::ReportRevenue { period_id, .. }
            | TestOperation::DepositRevenue { period_id, .. } => {
                if *period_id <= last_period {
                    return false;
                }
                last_period = *period_id;
            }
            _ => {}
        }
    }
    true
}

/// Verify that all bps values in a sequence are within valid range (0–10 000).
pub fn sequence_has_valid_bps(ops: &[TestOperation]) -> bool {
    for op in ops {
        match op {
            TestOperation::RegisterOffering { bps, .. }
            | TestOperation::SetConcentrationLimit { max_bps: bps, .. } => {
                if *bps > 10_000 {
                    return false;
                }
            }
            TestOperation::SetHolderShare { share_bps, .. } => {
                if *share_bps > 10_000 {
                    return false;
                }
            }
            _ => {}
        }
    }
    true
}
