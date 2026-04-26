# Payment Token Decimal Compatibility

## Overview

Different payment tokens on the Stellar network use different decimal precisions. For example:

| Token | Decimals | Example raw amount | Canonical (7-dec) |
|-------|----------|-------------------|-------------------|
| XLM   | 7        | 1_000_000_0       | 1_000_000_0       |
| USDC  | 6        | 1_000_000         | 10_000_000        |
| WBTC  | 8        | 1_000_000_00      | 1_000_000_0       |

Without normalization, reporting USDC revenue in raw amounts and then computing holder shares produces silent arithmetic errors — holders receive 10× too little or too much.

## How It Works

This contract stores a per-offering decimal configuration for the payout asset. Before any holder share computation (in `claim`, `get_claimable`, and `get_claimable_chunk`), the raw revenue amount is normalized to Stellar's canonical 7-decimal precision.

### Normalization Rules

- **`from_decimals == 7`**: no-op, amount returned unchanged.
- **`from_decimals < 7`** (e.g., USDC at 6): scale **up** by `10^(7 - from_decimals)`.
- **`from_decimals > 7`** (e.g., WBTC at 8): scale **down** by `10^(from_decimals - 7)` using integer truncation.
- **Overflow protection**: if multiplication overflows `i128`, the function returns `0` to prevent fund inflation. This is logged as a zero-payout distribution.

## API

### `set_payment_token_decimals(issuer, namespace, token, decimals: u32)`

Sets the decimal precision of the payout asset for an offering. Requires issuer authorization.

- **Range**: `0..=18`. Values outside this range return `RevoraError::LimitReached`.
- **Default**: If not set, `7` (canonical Stellar stroops) is assumed.
- **Event**: Emits `dec_set` event with the configured value.

### `get_payment_token_decimals(issuer, namespace, token) -> u32`

Returns the configured decimal precision or `7` if not set.

## Security Assumptions

1. **Issuer responsibility**: The `issuer` is trusted to supply the correct on-chain token decimal value. An incorrect value directly affects all future claim payouts. Issuers should verify the decimal on-chain before calling this function.
2. **Immutable after set**: There is no restriction on updating decimals after the fact, but changing decimals mid-offering will affect future claims inconsistently with past revenue reports. Issuers should set decimals before the first revenue report.
3. **Overflow is safe**: All multiplications are guarded with `checked_mul`. Overflow returns `0`, preventing fund inflation but potentially causing zero payouts for extremely large amounts with low-decimal tokens.
4. **Scope**: Decimals are per-offering, not per-asset globally. Two offerings with the same payout asset may have different decimal configurations.
5. **Read-Side Pagination**: When computing holder shares iteratively off-chain, be aware that paginated endpoints (e.g., `get_offerings_page`, `get_blacklist_page`) are capped at a `MAX_PAGE_LIMIT` of 20 to prevent unbounded execution. Indexers fetching multiple properties must handle this cursor-based traversal safely.

## Example

```rust
// Register offering with USDC (6 decimals) as payout asset
client.register_offering(&issuer, &ns, &token, &shares_bps, &usdc_address, &0);

// Configure decimals
client.set_payment_token_decimals(&issuer, &ns, &token, &6);

// Report 1,000,000 raw USDC units = 0.1 USDC
client.deposit_revenue(&issuer, &ns, &token, &usdc_address, &1_000_000, &1);

// After normalization: 1_000_000 (6-dec) → 10_000_000 (7-dec)
// Holder with 50% share receives: 10_000_000 * 5_000 / 10_000 = 5_000_000 canonical units
```
