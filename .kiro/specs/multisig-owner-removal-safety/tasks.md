# Implementation Plan: Multisig Owner Removal Safety (#296)

- [x] 1. Fix `RemoveOwner` membership check (`&addr` → `&old_owner`)
  - File: `src/lib.rs`, `execute_action`, `RemoveOwner` branch
  - _Requirements: REQ-2_

- [x] 2. Fix `RemoveOwner` storage write (remove immutable reassignment)
  - Replace `owners = new_owners; env.storage()...set(..., &owners)` with
    `env.storage()...set(..., &new_owners)`
  - File: `src/lib.rs`, `execute_action`, `RemoveOwner` branch
  - _Requirements: REQ-1_

- [x] 3. Remove duplicated guard block in `execute_action`
  - The first block set `executed = true` and saved; the second block re-read and
    re-checked the same fields. Remove the duplicate.
  - File: `src/lib.rs`, `execute_action`

- [x] 4. Add test: remove non-existent owner fails
  - `multisig_remove_nonexistent_owner_fails`
  - _Requirements: REQ-2, REQ-5_

- [x] 5. Add test: exact threshold boundary succeeds
  - `multisig_remove_owner_exact_threshold_boundary_succeeds`
  - _Requirements: REQ-1, REQ-5_

- [x] 6. Add test: below threshold is rejected
  - `multisig_remove_owner_below_threshold_is_rejected`
  - _Requirements: REQ-1, REQ-5_

- [x] 7. Add test: proposer removed, pending proposal still executable
  - `multisig_remove_proposer_pending_proposal_still_executable`
  - _Requirements: REQ-4, REQ-5_

- [x] 8. Add test: last approver removed blocks execution
  - `multisig_remove_last_approver_blocks_execution`
  - _Requirements: REQ-4, REQ-5_

- [x] 9. Add test: self-removal with threshold met
  - `multisig_owner_self_removal_succeeds_with_threshold`
  - _Requirements: REQ-3, REQ-5_

- [x] 10. Create `docs/multisig-owner-removal-safety.md`
  - _Requirements: REQ-6_

- [x] 11. Create `.kiro/specs/multisig-owner-removal-safety/` spec files
  - `requirements.md`, `design.md`, `tasks.md`
