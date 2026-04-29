# Reconciliation Event Completeness Bugfix Design

## Overview

`RevoraRevenueShare` must emit a deterministic `EVENT_INDEXED_V2` / `EventIndexTopicV2`
structured topic for every revenue-affecting state mutation so that off-chain indexers can
reconstruct contract state entirely from the event log.

Three functions currently mutate persistent storage without emitting `EVENT_INDEXED_V2`:
`deposit_revenue`, `set_holder_share`, and (partially) `claim`. Additionally, two tests
(`set_admin_emits_event`, `set_platform_fee_emits_event`) contain blacklist-manipulation
logic instead of asserting the events they claim to cover, leaving those event paths
effectively untested.

The fix adds the missing `EVENT_INDEXED_V2` emissions and replaces the mismatched test
bodies with correct assertions.

## Glossary

- **Bug_Condition (C)**: The set of contract function invocations that mutate persistent
  storage without emitting a corresponding `EVENT_INDEXED_V2` structured topic, or test
  functions whose bodies do not assert the event they are named for.
- **Property (P)**: After the fix, every invocation in C(X) must result in at least one
  `EVENT_INDEXED_V2` event in `env.events().all()` with the correct `event_type` symbol.
- **Preservation**: All existing event emissions (`EVENT_REV_DEPOSIT_V2`, `EVENT_SHARE_SET`,
  `EVENT_CLAIM`, `EVENT_ADMIN_SET`, `EVENT_PLATFORM_FEE_SET`, etc.) and all existing
  passing tests must remain unchanged by the fix.
- **`do_deposit_revenue`**: Internal helper in `src/lib.rs` that performs the actual token
  transfer and storage writes for `deposit_revenue`. Currently emits `EVENT_REV_DEPOSIT_V2`
  but not `EVENT_INDEXED_V2`.
- **`set_holder_share_internal`**: Internal helper in `src/lib.rs` that writes `HolderShare`
  to storage and emits `EVENT_SHARE_SET` (legacy). Does not emit `EVENT_INDEXED_V2`.
- **`claim`**: Public function in `src/lib.rs` that transfers payout tokens to a holder.
  Already emits `EVENT_INDEXED_V2` with `event_type = EVENT_TYPE_CLAIM`; the bug is that
  `period_id` is hardcoded to `0` rather than being omitted intentionally — this is
  acceptable per the spec (claim is not period-scoped in the topic), but the data payload
  must include `total_payout`.
- **`EventIndexTopicV2`**: Structured topic type used as the second element of the
  `EVENT_INDEXED_V2` publish tuple, consumed by off-chain indexers.
- **`EVENT_TYPE_REV_DEP`**: New `symbol_short!("rv_dep")` constant to be added for the
  deposit event type.
- **`EVENT_TYPE_SH_SET`**: New `symbol_short!("sh_set")` constant to be added for the
  share-set event type (distinct from the legacy `EVENT_SHARE_SET` topic symbol).

## Bug Details

### Bug Condition

The bug manifests when any of the following functions is called:
- `deposit_revenue` — writes `PeriodRevenue`, `PeriodEntry`, `PeriodDepositTime` but emits
  only `EVENT_REV_DEPOSIT_V2`, not `EVENT_INDEXED_V2`.
- `set_holder_share` (via `set_holder_share_internal`) — writes `HolderShare` but emits
  only `EVENT_SHARE_SET`, not `EVENT_INDEXED_V2`.
- The test `set_admin_emits_event` — body performs blacklist operations instead of asserting
  `EVENT_ADMIN_SET`.
- The test `set_platform_fee_emits_event` — body performs blacklist operations instead of
  asserting `EVENT_PLATFORM_FEE_SET`.

**Formal Specification:**
```
FUNCTION isBugCondition(X)
  INPUT: X — a contract function invocation or test execution
  OUTPUT: boolean

  RETURN (
    X.fn = deposit_revenue
    AND NOT exists e IN env.events().all() WHERE
          e.topic[0] = EVENT_INDEXED_V2
          AND e.topic[1].event_type = "rv_dep"
  )
  OR (
    X.fn = set_holder_share
    AND NOT exists e IN env.events().all() WHERE
          e.topic[0] = EVENT_INDEXED_V2
          AND e.topic[1].event_type = "sh_set"
  )
  OR (
    X.fn = set_admin_emits_event_test
    AND test_body_asserts_blacklist_state_instead_of_admin_set_event
  )
  OR (
    X.fn = set_platform_fee_emits_event_test
    AND test_body_asserts_blacklist_state_instead_of_fee_set_event
  )
END FUNCTION
```

### Examples

- `deposit_revenue(issuer, ns, token, payment_token, 1000, 1)` → writes storage, emits
  `EVENT_REV_DEPOSIT_V2`, but indexer sees no `EVENT_INDEXED_V2` with `event_type="rv_dep"`.
- `set_holder_share(issuer, ns, token, holder, 500)` → writes `HolderShare`, emits
  `EVENT_SHARE_SET`, but indexer sees no `EVENT_INDEXED_V2` with `event_type="sh_set"`.
- Running `set_admin_emits_event` passes (blacklist assertions succeed) but provides zero
  coverage of the `EVENT_ADMIN_SET` emission path.
- Running `set_platform_fee_emits_event` passes (blacklist assertions succeed) but provides
  zero coverage of the `EVENT_PLATFORM_FEE_SET` emission path.

## Expected Behavior

### Preservation Requirements

**Unchanged Behaviors:**
- `report_revenue` must continue to emit `EVENT_REVENUE_REPORT_INITIAL`,
  `EVENT_REVENUE_REPORTED`, and `EVENT_INDEXED_V2` with `event_type="rv_init"` / `"rv_ovr"` /
  `"rv_rej"` exactly as before.
- `register_offering` must continue to emit `EVENT_INDEXED_V2` with `event_type="offer"`.
- `claim` must continue to emit `EVENT_CLAIM` and `EVENT_INDEXED_V2` with
  `event_type="claim"` exactly as before (no change to the claim path is required).
- `deposit_revenue` must continue to emit `EVENT_REV_DEPOSIT_V2` in addition to the new
  `EVENT_INDEXED_V2` emission.
- `set_holder_share` must continue to emit `EVENT_SHARE_SET` in addition to the new
  `EVENT_INDEXED_V2` emission.
- All configuration-level events (`EVENT_CONC_LIMIT_SET`, `EVENT_ROUNDING_MODE_SET`,
  `EVENT_META_SIGNER_SET`, `EVENT_META_DELEGATE_SET`, `EVENT_MULTISIG_INIT`,
  `EVENT_ADMIN_SET`, `EVENT_PLATFORM_FEE_SET`) must continue to be emitted unchanged.
- All existing blacklist/whitelist tests must continue to pass.

**Scope:**
All inputs that do NOT involve `deposit_revenue`, `set_holder_share`, or the two mismatched
tests are completely unaffected by this fix. This includes:
- All `report_revenue` paths (initial, override, rejected).
- All `register_offering` calls.
- All `claim` calls (claim already emits `EVENT_INDEXED_V2` correctly).
- All configuration setters not listed in the bug condition.
- All blacklist and whitelist operations.

## Hypothesized Root Cause

1. **Incremental feature addition without full audit**: `EVENT_INDEXED_V2` was introduced as
   a structured indexer topic for a subset of functions. `deposit_revenue` and
   `set_holder_share` were not updated at that time, leaving them with only legacy event
   symbols.

2. **Test copy-paste error**: The tests `set_admin_emits_event` and
   `set_platform_fee_emits_event` appear to have been created by copying an existing
   blacklist test and renaming it without replacing the body, resulting in tests that pass
   for the wrong reason.

3. **`claim` partial coverage**: The `claim` function already emits `EVENT_INDEXED_V2` with
   `event_type = EVENT_TYPE_CLAIM` and `(total_payout,)` as data. The requirements document
   describes this as a bug, but the code already has the emission. The fix for `claim` is
   therefore a no-op on the production path; the indexer fixture test coverage for `claim`
   in `src/test_indexer_fixtures.rs` should be verified to include the payout amount.

4. **Missing event type constants**: `EVENT_TYPE_REV_DEP` (`"rv_dep"`) and
   `EVENT_TYPE_SH_SET` (`"sh_set"`) are not yet defined as named constants in `src/lib.rs`,
   so the `EVENT_INDEXED_V2` emissions for those paths cannot be added without first
   declaring them.

## Correctness Properties

Property 1: Bug Condition — deposit_revenue Emits EVENT_INDEXED_V2

_For any_ call to `deposit_revenue` where the offering exists, the amount is positive, and
the period_id is valid, the fixed `do_deposit_revenue` function SHALL emit an
`EVENT_INDEXED_V2` event whose `EventIndexTopicV2` payload has `event_type =
symbol_short!("rv_dep")`, the correct `issuer`, `namespace`, `token`, and `period_id`,
in addition to the existing `EVENT_REV_DEPOSIT_V2` emission.

**Validates: Requirements 2.1**

Property 2: Bug Condition — set_holder_share Emits EVENT_INDEXED_V2

_For any_ call to `set_holder_share` where the offering exists and `share_bps <= 10_000`,
the fixed `set_holder_share_internal` function SHALL emit an `EVENT_INDEXED_V2` event whose
`EventIndexTopicV2` payload has `event_type = symbol_short!("sh_set")`, the correct
`issuer`, `namespace`, `token`, and `period_id = 0`, in addition to the existing
`EVENT_SHARE_SET` emission.

**Validates: Requirements 2.3**

Property 3: Bug Condition — set_admin_emits_event Test Correctness

_For any_ execution of the test `set_admin_emits_event`, the test SHALL call `initialize`
(or `set_admin`) and assert that at least one event with topic symbol `"admin_set"` is
present in `env.events().all()`, confirming the admin-set event path is exercised.

**Validates: Requirements 2.4**

Property 4: Bug Condition — set_platform_fee_emits_event Test Correctness

_For any_ execution of the test `set_platform_fee_emits_event`, the test SHALL call the
platform-fee setter and assert that at least one event with topic symbol `"fee_set"` is
present in `env.events().all()`, confirming the platform-fee event path is exercised.

**Validates: Requirements 2.5**

Property 5: Preservation — Existing Event Emissions Unchanged

_For any_ input where the bug condition does NOT hold (i.e., any function call other than
`deposit_revenue` and `set_holder_share`, and any test other than the two mismatched ones),
the fixed code SHALL produce exactly the same event set as the original code, preserving all
existing event emissions and all existing test outcomes.

**Validates: Requirements 3.1, 3.2, 3.3, 3.4, 3.5, 3.6, 3.7, 3.8, 3.9, 3.10, 3.11, 3.12**

## Fix Implementation

### Changes Required

**File**: `src/lib.rs`

**Change 1 — Add missing event type constants** (near the existing `EVENT_TYPE_*` block,
around line 194):
```rust
const EVENT_TYPE_REV_DEP: Symbol = symbol_short!("rv_dep");
const EVENT_TYPE_SH_SET:  Symbol = symbol_short!("sh_set");
```

**Change 2 — Emit `EVENT_INDEXED_V2` in `do_deposit_revenue`** (after the existing
`EVENT_REV_DEPOSIT_V2` emission, before `Ok(())`):
```rust
env.events().publish(
    (
        EVENT_INDEXED_V2,
        EventIndexTopicV2 {
            version: INDEXER_EVENT_SCHEMA_VERSION,
            event_type: EVENT_TYPE_REV_DEP,
            issuer: issuer.clone(),
            namespace: namespace.clone(),
            token: token.clone(),
            period_id,
        },
    ),
    (amount,),
);
```

**Change 3 — Emit `EVENT_INDEXED_V2` in `set_holder_share_internal`** (after the existing
`EVENT_SHARE_SET` emission, before `Ok(())`):
```rust
env.events().publish(
    (
        EVENT_INDEXED_V2,
        EventIndexTopicV2 {
            version: INDEXER_EVENT_SCHEMA_VERSION,
            event_type: EVENT_TYPE_SH_SET,
            issuer: issuer.clone(),
            namespace: namespace.clone(),
            token: token.clone(),
            period_id: 0,
        },
    ),
    (holder.clone(), share_bps),
);
```

---

**File**: `src/test.rs`

**Change 4 — Replace `set_admin_emits_event` body** with a test that calls `initialize` and
asserts `EVENT_ADMIN_SET` is present:
```rust
fn set_admin_emits_event() {
    let env = Env::default();
    env.mock_all_auths();
    let client = make_client(&env);
    let admin = Address::generate(&env);
    client.initialize(&admin, &None::<Address>, &None::<bool>);
    let evts = env.events().all();
    let found = evts.iter().any(|(topics, _)| {
        topics.get(0) == Some(EVENT_ADMIN_SET.into_val(&env))
    });
    assert!(found, "EVENT_ADMIN_SET not emitted by initialize");
}
```

**Change 5 — Replace `set_platform_fee_emits_event` body** with a test that calls
`set_platform_fee` and asserts `EVENT_PLATFORM_FEE_SET` is present:
```rust
fn set_platform_fee_emits_event() {
    let env = Env::default();
    env.mock_all_auths();
    let client = make_client(&env);
    let admin = Address::generate(&env);
    client.initialize(&admin, &None::<Address>, &None::<bool>);
    client.set_platform_fee(&admin, &500u32);
    let evts = env.events().all();
    let found = evts.iter().any(|(topics, _)| {
        topics.get(0) == Some(EVENT_PLATFORM_FEE_SET.into_val(&env))
    });
    assert!(found, "EVENT_PLATFORM_FEE_SET not emitted by set_platform_fee");
}
```

---

**File**: `src/test_indexer_fixtures.rs`

**Change 6 — Add fixture tests for `rv_dep` and `sh_set`** to verify the new
`EVENT_INDEXED_V2` emissions appear in the structured event stream with correct topic shape.

---

**File**: `docs/reconciliation-event-completeness.md`

**Change 7 — Update the New Events table** to include the three newly covered functions:

| Event Constant | Function | Emitted Data |
|---|---|---|
| `EVENT_INDEXED_V2` (type `rv_dep`) | `deposit_revenue` | `(amount,)` |
| `EVENT_INDEXED_V2` (type `sh_set`) | `set_holder_share` | `(holder, share_bps)` |
| `EVENT_INDEXED_V2` (type `claim`) | `claim` | `(total_payout,)` |

## Testing Strategy

### Validation Approach

The testing strategy follows a two-phase approach: first, surface counterexamples that
demonstrate the bug on unfixed code, then verify the fix works correctly and preserves
existing behavior.

### Exploratory Bug Condition Checking

**Goal**: Surface counterexamples that demonstrate the bug BEFORE implementing the fix.
Confirm or refute the root cause analysis.

**Test Plan**: Write tests that call `deposit_revenue` and `set_holder_share`, then scan
`env.events().all()` for an `EVENT_INDEXED_V2` topic with the expected `event_type`. Run
these tests on the UNFIXED code to observe failures and confirm the missing emissions.

**Test Cases**:
1. **deposit_revenue missing rv_dep**: Call `deposit_revenue` with valid args; assert
   `EVENT_INDEXED_V2` with `event_type="rv_dep"` is present. (will fail on unfixed code)
2. **set_holder_share missing sh_set**: Call `set_holder_share` with valid args; assert
   `EVENT_INDEXED_V2` with `event_type="sh_set"` is present. (will fail on unfixed code)
3. **set_admin_emits_event wrong body**: Run the test; observe it passes despite never
   calling `initialize` or asserting `EVENT_ADMIN_SET`. (passes for wrong reason on unfixed code)
4. **set_platform_fee_emits_event wrong body**: Same pattern. (passes for wrong reason on unfixed code)

**Expected Counterexamples**:
- `EVENT_INDEXED_V2` is absent from `env.events().all()` after `deposit_revenue`.
- `EVENT_INDEXED_V2` is absent from `env.events().all()` after `set_holder_share`.
- Possible causes: missing `env.events().publish(EVENT_INDEXED_V2, ...)` call in
  `do_deposit_revenue` and `set_holder_share_internal`; missing `EVENT_TYPE_REV_DEP` and
  `EVENT_TYPE_SH_SET` constants.

### Fix Checking

**Goal**: Verify that for all inputs where the bug condition holds, the fixed function
produces the expected behavior.

**Pseudocode:**
```
FOR ALL X WHERE isBugCondition(X) DO
  result := fixedFunction(X)
  ASSERT exists e IN env.events().all() WHERE
    e.topic[0] = EVENT_INDEXED_V2
    AND e.topic[1].event_type = expected_type(X.fn)
    AND e.topic[1].issuer    = X.issuer
    AND e.topic[1].namespace = X.namespace
    AND e.topic[1].token     = X.token
    AND e.topic[1].period_id = expected_period_id(X.fn, X.args)
END FOR
```

### Preservation Checking

**Goal**: Verify that for all inputs where the bug condition does NOT hold, the fixed
function produces the same result as the original function.

**Pseudocode:**
```
FOR ALL X WHERE NOT isBugCondition(X) DO
  ASSERT original_events(X) ⊆ fixed_events(X)
  AND    fixed_events(X) \ original_events(X) = ∅
END FOR
```

**Testing Approach**: Property-based testing is recommended for preservation checking
because it generates many test cases automatically across the input domain, catches edge
cases that manual unit tests might miss, and provides strong guarantees that behavior is
unchanged for all non-buggy inputs.

**Test Plan**: Observe behavior on UNFIXED code first for `report_revenue`, `register_offering`,
and `claim`, then write property-based tests capturing that behavior.

**Test Cases**:
1. **report_revenue preservation**: Verify `EVENT_INDEXED_V2` with `event_type="rv_init"` /
   `"rv_ovr"` / `"rv_rej"` continues to be emitted after the fix.
2. **register_offering preservation**: Verify `EVENT_INDEXED_V2` with `event_type="offer"`
   continues to be emitted.
3. **claim preservation**: Verify `EVENT_CLAIM` and `EVENT_INDEXED_V2` with
   `event_type="claim"` continue to be emitted.
4. **deposit_revenue legacy event preservation**: Verify `EVENT_REV_DEPOSIT_V2` is still
   emitted alongside the new `EVENT_INDEXED_V2`.
5. **set_holder_share legacy event preservation**: Verify `EVENT_SHARE_SET` is still emitted
   alongside the new `EVENT_INDEXED_V2`.

### Unit Tests

- Test `deposit_revenue` emits `EVENT_INDEXED_V2` with `event_type="rv_dep"` and correct
  `issuer`, `namespace`, `token`, `period_id`, and `amount` in data.
- Test `set_holder_share` emits `EVENT_INDEXED_V2` with `event_type="sh_set"` and correct
  fields; `period_id` must be `0`.
- Test `set_admin_emits_event` calls `initialize` and asserts `EVENT_ADMIN_SET` is present.
- Test `set_platform_fee_emits_event` calls `set_platform_fee` and asserts
  `EVENT_PLATFORM_FEE_SET` is present.
- Test edge case: `deposit_revenue` with supply cap reached still emits both
  `EVENT_SUPPLY_CAP_REACHED` and `EVENT_INDEXED_V2`.

### Property-Based Tests

- Generate random `(issuer, namespace, token, amount, period_id)` tuples and verify that
  every successful `deposit_revenue` call results in exactly one `EVENT_INDEXED_V2` with
  `event_type="rv_dep"` in `env.events().all()`.
- Generate random `(issuer, namespace, token, holder, share_bps)` tuples and verify that
  every successful `set_holder_share` call results in exactly one `EVENT_INDEXED_V2` with
  `event_type="sh_set"` in `env.events().all()`.
- Generate random sequences of `report_revenue` calls and verify the existing
  `EVENT_INDEXED_V2` emissions are unchanged (preservation).

### Integration Tests

- Full flow: `register_offering` → `deposit_revenue` → `set_holder_share` → `claim`;
  assert all four `EVENT_INDEXED_V2` event types (`"offer"`, `"rv_dep"`, `"sh_set"`,
  `"claim"`) appear in `env.events().all()` in the correct order.
- Extend `src/test_indexer_fixtures.rs` to include `rv_dep` and `sh_set` fixture topics
  in `get_indexer_fixture_topics` and assert their shape and field bindings.
- Verify that the updated `docs/reconciliation-event-completeness.md` accurately reflects
  the complete event surface by cross-referencing the table against the event constants
  defined in `src/lib.rs`.
