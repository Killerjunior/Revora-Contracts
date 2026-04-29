# Design: Multisig Owner Removal Safety (#296)

## Problem

`ProposalAction::RemoveOwner` in `execute_action` had two compile-blocking bugs that
prevented the threshold invariant and membership check from running:

1. `!owners.contains(&addr)` — `addr` is undefined; should be `old_owner`.
2. `owners = new_owners` — `owners` is an immutable binding; the new list was never
   persisted to storage.

Additionally, `execute_action` contained a duplicated guard block (executed/expiry/
threshold checks + a second `get` of the same proposal key), which was dead code after
the first block set `executed = true` and saved.

## Fix

### RemoveOwner branch (`src/lib.rs`)

```
Before:
  if !owners.contains(&addr) { ... }          // addr undefined → compile error
  ...
  owners = new_owners;                          // immutable binding → compile error
  env.storage().persistent().set(..., &owners);

After:
  if !owners.contains(&old_owner) { ... }      // correct variable
  ...
  env.storage().persistent().set(..., &new_owners); // write new_owners directly
```

### Duplicated guard block removal

The first guard block (lines ~5497–5510 before fix) set `proposal.executed = true`,
saved it, then the second block re-read the same key and checked `executed` (always
true). The duplicate block was removed; the single authoritative check now precedes
the `match`.

## Invariant Enforcement

```
execute_action(RemoveOwner(old_owner)):
  1. Standard checks: not executed, not expired, approvals >= threshold
  2. owners.contains(old_owner)  → else NotAuthorized
  3. (owners.len() - 1) >= threshold  → else LimitReached
  4. Build new_owners excluding old_owner
  5. Persist new_owners
```

## Test Strategy

Six new tests cover the threat model (see `docs/multisig-owner-removal-safety.md`):

- Non-existent owner → error (REQ-2)
- Exact boundary (owners == threshold after removal) → success (REQ-1)
- Below threshold → error (REQ-1)
- Proposer removed mid-flight → pending proposal still executable (REQ-4)
- Last approver removed → execution blocked (REQ-4 corollary)
- Self-removal with threshold met → success (REQ-3)
