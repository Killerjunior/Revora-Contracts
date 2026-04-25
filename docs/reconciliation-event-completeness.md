# Reconciliation Event Completeness (#188 / #289)

## Overview

This document describes the **Reconciliation Event Completeness** capability and its
extension for on-chain milestone signal integrity (issue #289). The feature ensures that
every persistent state mutation in `RevoraRevenueShare` emits a deterministic on-chain
`env.events().publish(...)` call, allowing off-chain indexers, accounting systems, and
`hardenedMilestoneValidation` consumers to reconstruct contract state entirely from the
event log.

## Motivation

Prior to this feature, 8 critical configuration-level functions wrote to persistent
storage without emitting observable events. Any indexer or reconciliation job that relied
solely on events would experience blind spots, leading to state drift between on-chain
data and off-chain models.

## Events Emitted on Every State Mutation

| Event constant | Function | Emitted data |
|---|---|---|
| `EVENT_CONC_LIMIT_SET` | `set_concentration_limit` | `(max_bps, enforce)` |
| `EVENT_ROUNDING_MODE_SET` | `set_rounding_mode` | `mode` |
| `EVENT_META_SIGNER_SET` | `register_meta_signer_key` | `pub_key` |
| `EVENT_META_DELEGATE_SET` | `set_meta_delegate` | `delegate` |
| `EVENT_MULTISIG_INIT` | `init_multisig` | `(members, threshold)` |
| `EVENT_ADMIN_SET` | `initialize` / `set_admin` | `admin` |
| `EVENT_PLATFORM_FEE_SET` | `set_platform_fee` | `fee_bps` |

## Milestone Gate Signal Completeness (#289)

For `hardenedMilestoneValidation` consumers the following invariants are guaranteed and
covered by the `milestone_signals` test module (`src/milestone_signals.rs`):

### Invariant table

| Invariant | Guarantee | Test |
|---|---|---|
| `offer_reg` precedes `rev_rep` | Event log ordering is deterministic | `milestone_event_ordering_offer_before_rev_rep` |
| `period_id` strictly increasing | Duplicate / lower IDs rejected with `InvalidPeriodId` | `milestone_period_id_must_be_strictly_increasing` |
| `period_id = 0` rejected | Always invalid | `milestone_period_id_zero_rejected` |
| Audit summary accumulates | `total_revenue` = Σ accepted amounts; `report_count` = accepted calls | `milestone_audit_summary_accumulates_correctly` |
| Rejected reports don't mutate summary | Summary unchanged on error | `milestone_audit_summary_not_updated_on_rejected_report` |
| Concentration enforcement blocks report | `ConcentrationLimitExceeded`; summary unchanged | `milestone_concentration_enforcement_blocks_revenue_report` |
| At-limit allows report | Succeeds when `concentration_bps == max_bps` | `milestone_concentration_at_limit_allows_revenue_report` |
| Warning-only doesn't block | `conc_wrn` emitted; report proceeds | `milestone_concentration_warning_does_not_block_report` |
| Blacklist snapshot at report time | `get_blacklist` reflects state at each report | `milestone_blacklist_snapshot_captured_at_report_time` |
| `ev_idx2` topic on every report | Correct `event_type`, `period_id`, identity fields | `milestone_indexed_v2_topic_emitted_on_report_revenue` |
| Fixture covers 6 canonical types | Stable order; all types present | `milestone_fixture_covers_all_canonical_event_types` |
| Audit summary isolated per offering | Cross-offering isolation | `milestone_audit_summary_isolated_per_offering` |

### How a backend milestone gate should use these signals

```
1. Listen for rev_rep events on the offering's (issuer, namespace, token).
2. Verify the ev_idx2 topic: event_type == rv_rep, period_id is the expected value.
3. Read get_audit_summary(issuer, namespace, token):
   - report_count must equal the number of expected accepted reports.
   - total_revenue must equal the expected cumulative sum.
4. If concentration enforcement is configured, verify no ConcentrationLimitExceeded
   error was returned for this period (i.e. the report was accepted).
5. Only advance the milestone once all of the above checks pass.
```

## Security Assumptions

- Events are **informational only** — they carry no authority and cannot replay state.
- All existing authorization requirements (`issuer.require_auth()`, multisig threshold
  checks, etc.) remain in force before an event can be emitted.
- Decimal normalization applies to `AuditSummary.total_revenue` so reconciliation figures
  match payout math exactly.
- Concentration values are issuer-reported; see
  [concentration-reporting-integrity.md](./concentration-reporting-integrity.md) for the
  associated trust model and risk note.

## Running the Tests

```bash
cargo test milestone_
```

All 12 milestone signal tests must pass. The full suite:

```bash
cargo test
cargo clippy --all-targets -- -D warnings
```
