# Bugfix Requirements Document

## Introduction

The Revora-Contracts Soroban contract (`RevoraRevenueShare`) is the authoritative source of truth for the chain leg of revenue reconciliation. An off-chain service (e.g. `revenueReconciliationService`) must be able to reconstruct every revenue-affecting state transition solely from the on-chain event log.

Two classes of defect exist today:

1. **Missing `EVENT_INDEXED_V2` coverage** — several revenue-affecting transitions (`deposit_revenue`, `claim`, `set_holder_share`) do not emit the canonical `EVENT_INDEXED_V2` / `EventIndexTopicV2` structured topic that indexers rely on, making those transitions invisible to the reconciliation backend.

2. **Mismatched test bodies** — the tests named `set_admin_emits_event` and `set_platform_fee_emits_event` in `src/test.rs` contain blacklist-manipulation logic instead of asserting the events they claim to cover. This means the event-emission contracts for `initialize`/`set_admin` and `set_platform_fee` are untested, and regressions in those paths would go undetected.

Together these gaps mean a backend cannot prove "every revenue-affecting transition has a corresponding auditable, indexable event," violating the reconciliation-completeness guarantee described in `docs/reconciliation-event-completeness.md`.

---

## Bug Analysis

### Current Behavior (Defect)

1.1 WHEN `deposit_revenue` is called with a valid period and positive amount THEN the system writes `PeriodRevenue`, `PeriodEntry`, and `PeriodDepositTime` to persistent storage but does NOT emit an `EVENT_INDEXED_V2` topic with `event_type = "rv_dep"`, leaving the deposit invisible to structured indexers.

1.2 WHEN `claim` is called and a holder successfully receives a payout THEN the system emits `EVENT_CLAIM` but does NOT emit an `EVENT_INDEXED_V2` topic with `event_type = "claim"` that carries the payout amount in the structured payload, so the claim cannot be correlated with the offering's revenue lifecycle by an indexer consuming only `EVENT_INDEXED_V2` events.

1.3 WHEN `set_holder_share` is called THEN the system writes `HolderShare` to persistent storage and emits `EVENT_SHARE_SET` but does NOT emit an `EVENT_INDEXED_V2` topic, so share-allocation changes are absent from the structured event stream.

1.4 WHEN the test named `set_admin_emits_event` is executed THEN the test body performs blacklist-add operations and asserts blacklist state instead of asserting that `initialize` or `set_admin` emits `EVENT_ADMIN_SET`, providing no coverage of the admin-set event path.

1.5 WHEN the test named `set_platform_fee_emits_event` is executed THEN the test body performs blacklist-add and blacklist-remove operations and asserts blacklist state instead of asserting that the platform-fee setter emits `EVENT_PLATFORM_FEE_SET`, providing no coverage of the platform-fee event path.

### Expected Behavior (Correct)

2.1 WHEN `deposit_revenue` is called with a valid period and positive amount THEN the system SHALL emit an `EVENT_INDEXED_V2` event whose `EventIndexTopicV2` payload has `event_type = symbol_short!("rv_dep")`, the correct `issuer`, `namespace`, `token`, and `period_id`, allowing indexers to observe every revenue deposit in the structured event stream.

2.2 WHEN `claim` is called and a holder successfully receives a payout THEN the system SHALL emit an `EVENT_INDEXED_V2` event whose `EventIndexTopicV2` payload has `event_type = symbol_short!("claim")` and whose data payload includes `total_payout`, so the claim is fully auditable via the structured event stream.

2.3 WHEN `set_holder_share` is called THEN the system SHALL emit an `EVENT_INDEXED_V2` event whose `EventIndexTopicV2` payload has `event_type = symbol_short!("sh_set")`, the correct `issuer`, `namespace`, `token`, and `period_id = 0`, so share-allocation changes are present in the structured event stream.

2.4 WHEN the test named `set_admin_emits_event` is executed THEN the test SHALL call `initialize` (or `set_admin`) and assert that exactly one event with topic symbol `"admin_set"` is present in `env.events().all()`, confirming the admin-set event path is covered.

2.5 WHEN the test named `set_platform_fee_emits_event` is executed THEN the test SHALL call the platform-fee setter and assert that exactly one event with topic symbol `"fee_set"` is present in `env.events().all()`, confirming the platform-fee event path is covered.

### Unchanged Behavior (Regression Prevention)

3.1 WHEN `report_revenue` is called for an initial report THEN the system SHALL CONTINUE TO emit `EVENT_REVENUE_REPORT_INITIAL`, `EVENT_REVENUE_REPORTED`, and `EVENT_INDEXED_V2` with `event_type = "rv_init"` exactly as before.

3.2 WHEN `report_revenue` is called and an existing report is overridden THEN the system SHALL CONTINUE TO emit `EVENT_REVENUE_REPORT_OVERRIDE` and `EVENT_INDEXED_V2` with `event_type = "rv_ovr"` exactly as before.

3.3 WHEN `report_revenue` is called and the report is rejected THEN the system SHALL CONTINUE TO emit `EVENT_REVENUE_REPORT_REJECTED` and `EVENT_INDEXED_V2` with `event_type = "rv_rej"` exactly as before.

3.4 WHEN `register_offering` is called THEN the system SHALL CONTINUE TO emit the `"offer_reg"` event and `EVENT_INDEXED_V2` with `event_type = "offer"` exactly as before.

3.5 WHEN `set_concentration_limit` is called THEN the system SHALL CONTINUE TO emit `EVENT_CONC_LIMIT_SET` exactly as before.

3.6 WHEN `set_rounding_mode` is called THEN the system SHALL CONTINUE TO emit `EVENT_ROUNDING_MODE_SET` exactly as before.

3.7 WHEN `register_meta_signer_key` is called THEN the system SHALL CONTINUE TO emit `EVENT_META_SIGNER_SET` exactly as before.

3.8 WHEN `set_meta_delegate` is called THEN the system SHALL CONTINUE TO emit `EVENT_META_DELEGATE_SET` exactly as before.

3.9 WHEN `init_multisig` is called THEN the system SHALL CONTINUE TO emit `EVENT_MULTISIG_INIT` exactly as before.

3.10 WHEN `initialize` is called THEN the system SHALL CONTINUE TO emit `EVENT_ADMIN_SET` and `EVENT_INIT` exactly as before.

3.11 WHEN `blacklist_add` or `blacklist_remove` is called THEN the system SHALL CONTINUE TO emit `EVENT_BL_ADD` or `EVENT_BL_REM` exactly as before, and existing blacklist tests SHALL continue to pass.

3.12 WHEN `deposit_revenue_with_snapshot` is called THEN the system SHALL CONTINUE TO emit `EVENT_REV_DEP_SNAP_V2` exactly as before.

---

## Bug Condition Pseudocode

```pascal
FUNCTION isBugCondition(X)
  INPUT: X — a contract function invocation with its arguments
  OUTPUT: boolean

  RETURN (
    X.fn = deposit_revenue
    OR X.fn = claim
    OR X.fn = set_holder_share
    OR (X.fn = set_admin_emits_event_test AND test_body_does_not_assert_admin_set_event)
    OR (X.fn = set_platform_fee_emits_event_test AND test_body_does_not_assert_fee_set_event)
  )
END FUNCTION

// Property: Fix Checking
FOR ALL X WHERE isBugCondition(X) DO
  result ← F'(X)
  ASSERT EVENT_INDEXED_V2 in env.events().all()
         AND result.event_type matches expected_type(X.fn)
END FOR

// Property: Preservation Checking
FOR ALL X WHERE NOT isBugCondition(X) DO
  ASSERT F(X).events = F'(X).events
END FOR
```
