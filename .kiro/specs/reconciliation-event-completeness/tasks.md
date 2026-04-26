# Implementation Plan

- [ ] 1. Write bug condition exploration test
  - **Property 1: Bug Condition** - Missing EVENT_INDEXED_V2 on deposit_revenue and set_holder_share
  - **CRITICAL**: This test MUST FAIL on unfixed code - failure confirms the bug exists
  - **DO NOT attempt to fix the test or the code when it fails**
  - **NOTE**: This test encodes the expected behavior - it will validate the fix when it passes after implementation
  - **GOAL**: Surface counterexamples that demonstrate the bug exists
  - **Scoped PBT Approach**: Scope the property to the concrete failing cases: any valid deposit_revenue call and any valid set_holder_share call
  - In `src/test.rs` (or a new `src/test_reconciliation_bug.rs`), add a test module `reconciliation_bug_condition`
  - Test 1a: call `deposit_revenue` with valid args; scan `env.events().all()` for a topic whose first element equals `EVENT_INDEXED_V2` (`symbol_short!("ev_idx2")`) and whose second element has `event_type = symbol_short!("rv_dep")`; assert it is present — this assertion FAILS on unfixed code
  - Test 1b: call `set_holder_share` with valid args; scan `env.events().all()` for `EVENT_INDEXED_V2` with `event_type = symbol_short!("sh_set")`; assert it is present — this assertion FAILS on unfixed code
  - Run `cargo test reconciliation_bug_condition` on UNFIXED code
  - **EXPECTED OUTCOME**: Tests FAIL (this is correct — it proves the bug exists)
  - Document counterexamples found: e.g. "deposit_revenue emits EVENT_REV_DEPOSIT_V2 (ev_idx2 absent)", "set_holder_share emits EVENT_SHARE_SET (ev_idx2 absent)"
  - Mark task complete when tests are written, run, and failures are documented
  - _Requirements: 1.1, 1.3_

- [ ] 2. Write preservation property tests (BEFORE implementing fix)
  - **Property 2: Preservation** - Existing EVENT_INDEXED_V2 emissions for report_revenue, register_offering, and claim are unchanged
  - **IMPORTANT**: Follow observation-first methodology
  - Observe on UNFIXED code: `report_revenue` (initial) emits `EVENT_INDEXED_V2` with `event_type="rv_init"`
  - Observe on UNFIXED code: `report_revenue` (override) emits `EVENT_INDEXED_V2` with `event_type="rv_ovr"`
  - Observe on UNFIXED code: `register_offering` emits `EVENT_INDEXED_V2` with `event_type="offer"`
  - Observe on UNFIXED code: `deposit_revenue` emits `EVENT_REV_DEPOSIT_V2` (`symbol_short!("rev_dep2")`) — legacy event must survive the fix
  - Observe on UNFIXED code: `set_holder_share` emits `EVENT_SHARE_SET` (`symbol_short!("sh_set")`) — legacy event must survive the fix
  - Write property-based tests in a `reconciliation_preservation` module using `proptest!` macros and the helpers in `src/proptest_helpers.rs`
  - Property 2a: for any valid `(amount, period_id)` from `arb_deposit_revenue()`, after `deposit_revenue` succeeds, `EVENT_REV_DEPOSIT_V2` is present in `env.events().all()` (legacy event preserved)
  - Property 2b: for any valid `(holder_index, share_bps)` from `arb_set_holder_share()`, after `set_holder_share` succeeds, `EVENT_SHARE_SET` is present in `env.events().all()` (legacy event preserved)
  - Property 2c: for any valid `(amount, period_id)` from `arb_report_revenue()`, after `report_revenue` succeeds, `EVENT_INDEXED_V2` with `event_type="rv_init"` is present
  - Run `cargo test reconciliation_preservation` on UNFIXED code
  - **EXPECTED OUTCOME**: Tests PASS (this confirms baseline behavior to preserve)
  - Mark task complete when tests are written, run, and passing on unfixed code
  - _Requirements: 3.1, 3.2, 3.4, 3.11_

- [ ] 3. Fix for missing EVENT_INDEXED_V2 emissions and mismatched test bodies

  - [ ] 3.1 Add EVENT_TYPE_REV_DEP and EVENT_TYPE_SH_SET constants to src/lib.rs
    - Near the existing `EVENT_TYPE_*` block (around line 194, after `EVENT_TYPE_CLAIM`), add:
      ```rust
      const EVENT_TYPE_REV_DEP: Symbol = symbol_short!("rv_dep");
      const EVENT_TYPE_SH_SET:  Symbol = symbol_short!("sh_set");
      ```
    - These are required before the EVENT_INDEXED_V2 publish calls can reference them
    - _Bug_Condition: isBugCondition(X) where X.fn = deposit_revenue OR X.fn = set_holder_share_
    - _Requirements: 2.1, 2.3_

  - [ ] 3.2 Emit EVENT_INDEXED_V2 in do_deposit_revenue in src/lib.rs
    - After the existing `Self::emit_v2_event(env, (EVENT_REV_DEPOSIT_V2, ...), ...)` call and before `Ok(())`, add:
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
    - The existing `EVENT_REV_DEPOSIT_V2` emission must remain unchanged (preservation)
    - _Bug_Condition: isBugCondition(X) where X.fn = deposit_revenue AND no ev_idx2 with event_type="rv_dep" in events_
    - _Expected_Behavior: env.events().all() contains EVENT_INDEXED_V2 with event_type=symbol_short!("rv_dep"), issuer, namespace, token, period_id; data=(amount,)_
    - _Preservation: EVENT_REV_DEPOSIT_V2 must still be emitted alongside the new EVENT_INDEXED_V2_
    - _Requirements: 2.1, 3.1_

  - [ ] 3.3 Emit EVENT_INDEXED_V2 in set_holder_share_internal in src/lib.rs
    - After the existing `env.events().publish((EVENT_SHARE_SET, ...), ...)` call and before `Ok(())`, add:
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
    - The existing `EVENT_SHARE_SET` emission must remain unchanged (preservation)
    - _Bug_Condition: isBugCondition(X) where X.fn = set_holder_share AND no ev_idx2 with event_type="sh_set" in events_
    - _Expected_Behavior: env.events().all() contains EVENT_INDEXED_V2 with event_type=symbol_short!("sh_set"), issuer, namespace, token, period_id=0; data=(holder, share_bps)_
    - _Preservation: EVENT_SHARE_SET must still be emitted alongside the new EVENT_INDEXED_V2_
    - _Requirements: 2.3, 3.1_

  - [ ] 3.4 Fix set_admin_emits_event test body in src/test.rs
    - Replace the entire body of `set_admin_emits_event` (which currently does blacklist operations) with:
      ```rust
      let env = Env::default();
      env.mock_all_auths();
      let cid = env.register_contract(None, RevoraRevenueShare);
      let client = RevoraRevenueShareClient::new(&env, &cid);
      let admin = Address::generate(&env);
      client.initialize(&admin, &None::<Address>, &None::<bool>);
      let evts = env.events().all();
      let found = evts.iter().any(|(topics, _)| {
          topics.get(0) == Some(EVENT_ADMIN_SET.into_val(&env))
      });
      assert!(found, "EVENT_ADMIN_SET not emitted by initialize");
      ```
    - _Bug_Condition: test body asserts blacklist state instead of asserting EVENT_ADMIN_SET_
    - _Expected_Behavior: test calls initialize and asserts EVENT_ADMIN_SET is present in env.events().all()_
    - _Requirements: 2.4_

  - [ ] 3.5 Fix set_platform_fee_emits_event test body in src/test.rs
    - Replace the entire body of `set_platform_fee_emits_event` (which currently does blacklist operations) with:
      ```rust
      let env = Env::default();
      env.mock_all_auths();
      let cid = env.register_contract(None, RevoraRevenueShare);
      let client = RevoraRevenueShareClient::new(&env, &cid);
      let admin = Address::generate(&env);
      client.initialize(&admin, &None::<Address>, &None::<bool>);
      client.set_platform_fee(&admin, &500u32);
      let evts = env.events().all();
      let found = evts.iter().any(|(topics, _)| {
          topics.get(0) == Some(EVENT_PLATFORM_FEE_SET.into_val(&env))
      });
      assert!(found, "EVENT_PLATFORM_FEE_SET not emitted by set_platform_fee");
      ```
    - _Bug_Condition: test body asserts blacklist state instead of asserting EVENT_PLATFORM_FEE_SET_
    - _Expected_Behavior: test calls set_platform_fee and asserts EVENT_PLATFORM_FEE_SET is present in env.events().all()_
    - _Requirements: 2.5_

  - [ ] 3.6 Add rv_dep and sh_set fixture tests in src/test_indexer_fixtures.rs
    - Extend `get_indexer_fixture_topics` (or add a new fixture function) to include `rv_dep` and `sh_set` topic shapes
    - Add test `fixture_rv_dep_topic_shape`: call `get_indexer_fixture_topics` and assert the returned list contains an entry with `event_type = symbol_short!("rv_dep")` and `period_id` matching the requested period
    - Add test `fixture_sh_set_topic_shape`: assert the returned list contains an entry with `event_type = symbol_short!("sh_set")` and `period_id = 0`
    - Verify `issuer`, `namespace`, `token`, and `version = 2` are correctly bound in both new fixture entries
    - _Requirements: 2.1, 2.3_

  - [ ] 3.7 Update docs/reconciliation-event-completeness.md
    - Extend the "New Events" table to include the three newly covered functions:
      | `EVENT_INDEXED_V2` (type `rv_dep`) | `deposit_revenue` | `(amount,)` |
      | `EVENT_INDEXED_V2` (type `sh_set`) | `set_holder_share` | `(holder, share_bps)` |
      | `EVENT_INDEXED_V2` (type `claim`) | `claim` | `(total_payout,)` |
    - Update the Testing section to reference the new test modules
    - _Requirements: 2.1, 2.2, 2.3_

  - [ ] 3.8 Verify bug condition exploration test now passes
    - **Property 1: Expected Behavior** - deposit_revenue and set_holder_share emit EVENT_INDEXED_V2
    - **IMPORTANT**: Re-run the SAME tests from task 1 - do NOT write new tests
    - Run `cargo test reconciliation_bug_condition` on FIXED code
    - **EXPECTED OUTCOME**: Tests PASS (confirms bug is fixed)
    - _Requirements: 2.1, 2.3_

  - [ ] 3.9 Verify preservation tests still pass
    - **Property 2: Preservation** - Legacy events and existing EVENT_INDEXED_V2 emissions unchanged
    - **IMPORTANT**: Re-run the SAME tests from task 2 - do NOT write new tests
    - Run `cargo test reconciliation_preservation` on FIXED code
    - **EXPECTED OUTCOME**: Tests PASS (confirms no regressions)
    - Confirm all tests still pass after fix (no regressions)

- [ ] 4. Checkpoint - Ensure all tests pass
  - Run `cargo test` and confirm the full suite passes
  - Run `cargo clippy` and confirm no new warnings
  - Ensure all tests pass; ask the user if questions arise
