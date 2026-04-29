# Requirements: Multisig Owner Removal Safety (#296)

## Functional Requirements

### REQ-1: Threshold invariant on removal

**When** `execute_action(RemoveOwner(addr))` is called,
**Then** the contract MUST reject the action if `(current_owner_count - 1) < threshold`,
returning `LimitReached`.

**Rationale:** Allowing removal below threshold permanently bricks the multisig — no
future proposal can ever reach threshold.

### REQ-2: Membership check on removal

**When** `execute_action(RemoveOwner(addr))` is called with an address not in the owner list,
**Then** the contract MUST reject the action, returning `NotAuthorized`.

**Rationale:** Silently succeeding on a phantom removal wastes a proposal slot and
misleads off-chain tooling.

### REQ-3: Removal requires threshold approvals

**When** a `RemoveOwner` proposal is executed,
**Then** it MUST have at least `threshold` approvals, same as any other action.

**Rationale:** Owner removal is a privileged governance action; it must not be
executable by a single owner below threshold.

### REQ-4: Prior approvals on other proposals are preserved after removal

**When** an owner is removed,
**Then** their existing approvals on other proposals MUST remain recorded and count
toward threshold for those proposals.

**Rationale:** Retroactively invalidating approvals would allow a griefing attack where
removing an approver blocks unrelated pending proposals.

## Non-Functional Requirements

### REQ-5: CI falsifiability

All invariants above MUST have at least one automated test that would fail if the
invariant were violated. Tests run via `cargo test` with no special flags.

### REQ-6: Documentation

The security assumptions and threat model MUST be documented in
`docs/multisig-owner-removal-safety.md`.
