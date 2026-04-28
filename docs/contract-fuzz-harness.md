# Revora Fuzzing & Stress Harness

This document describes the automated stress testing story for the Revora-Contracts suite.

## Execution
- **Standard Unit Tests:** `cargo test`
- **Harden/Stress Tests (CI-Safe):** `cargo test --features stress-tests`

## Security Invariants
1. **Loop Bounding:** Input parameters like `period_id` are gated (max 1000) to prevent CPU instruction exhaustion.
2. **Panic-Free Execution:** The harness verifies that edge-case inputs return `Result::Err` rather than panicking.
3. **Deterministic Seeds:** Fixed seeds are used in CI to ensure reproducibility of failures.
