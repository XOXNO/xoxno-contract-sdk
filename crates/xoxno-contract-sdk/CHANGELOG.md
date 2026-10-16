# Changelog

All notable changes to this crate are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the crate uses
[semantic versioning](https://semver.org/): before 1.0, a minor version bump
means the embedded WASM, the ABI, the `soroban-sdk` major version or the
fixture API changed.

## [0.3.0] - 2026-10-16

### Changed

- Refreshed the embedded protocol, documentation specs and mocks from attested
  `v1.0.1`, commit `65761475c96ea2da4955b34d5cb3057f21e6a929`. Its five deploy
  artifacts match mainnet. The previous SDK fixture came from `v1.1.0`,
  commit `dc57a8562`, rather than the code mainnet runs. Public generated
  method signatures are unchanged; the fixture refresh requires this minor
  SDK version bump.
- The main guide starts with ownership, authorization and units, then shows a
  complete owner-controlled deposit, withdrawal, closure and reopening flow
  with constructor-injected deployment addresses. Advanced recipes link to
  repository examples and tests instead of duplicating the API reference.
- Clarified that `repay` and `repay_batch` return net payer spend after
  refunds, which can differ from debt retired for taxed tokens. Contributor
  deposits in the examples fund the stored owner; they do not create shares
  or a withdrawal entitlement.
- Corrected withdrawal amount documentation: `Withdrawal::amount` and
  `Withdrawals::amounts` are pool output before receipt tax, not measured
  recipient receipt. The tutorial checks actual receipt before forwarding
  funds; withdrawal batches still support one token across multiple hubs.
- Documented the fixture's authorization, budget and TTL limits, and separated
  embedded snapshot integrity from current deployment compatibility.

### Fixed

- The price-reader example uses wide arithmetic for headroom estimates, so
  18-decimal conversions do not saturate on intermediate overflow.
- Deposit examples expose owner-authorized recovery and close/reopen flows.
  Contributor examples reject receipt shortfalls before spending pre-funded
  balances; they remain owner accounts without depositor shares.
- Release attestations bind every artifact to one immutable source commit.
  Mainnet checks follow the active aggregator, and the pre-release drift
  override no longer suppresses RPC failures.
- Liquidation keeps the original repayment offer and authorizes both exact
  pull amounts the controller may choose, preserving overoffers needed to
  cover receipt tax on a full debt close. Results report measured net token
  spend, which can be negative if seized collateral includes a payment token.
- Non-withdrawal batches reject each zero or negative input before merging
  repeated markets, rather than allowing a positive sum to hide an invalid
  entry.
- Removed the README's overflow-prone `max_borrow` recipe and its first-hub-only
  placement recipe; linked the price and nested market-search examples.
- Corrected the repository release guide's crate changelog path and narrowed
  the batch-operation claim: `flash_loan` has no batch form.
- The install guide includes contract crate types and mandatory release
  overflow checks, verified by a fresh packaged consumer's WASM build.

## [0.2.2] - 2026-09-24

### Changed

- `wasm/MANIFEST.json` points at the re-cut rs-lending-xlm `v1.1.0` tag,
  commit `dc57a8562` (the first `v1.1.0` tag, at `1053ae033`, was deleted).
  Its attested build is byte-identical to the embedded WASM: every contract,
  file with docs, code hash and mock matches.

## [0.2.1] - 2026-09-24

### Added

- Batch operations on several markets in one controller call:
  `deposit_batch`, `supply_batch`, `borrow_batch`, `repay_batch` (net token
  spend per market), `withdraw_batch` (`Withdrawals { amounts,
  account_closed }`) and `liquidate_batch` (amount paid per market). A market
  listed twice is merged as the controller merges it. `repay_batch` and
  `liquidate_batch` take one market per token and panic with
  `InvalidPayments` otherwise.
- `lending::helpers::authorize_transfers_as_current`: one transfer
  authorization per `(token, amount)` for the next call.

### Changed

- The single-market operations call their batch forms with one market.

## [0.2.0] - 2026-09-24

### Added

- `Withdrawal { amount, account_closed }`. `account_closed` is true when the
  withdrawal left the account empty and the protocol burned its position NFT;
  the wrapper reads it from the NFT (`owner_of` fails with
  `NonExistentToken`).

### Fixed

- `XoxnoLending::liquidate` failed with `Error(Auth, InvalidAction)` when the
  offer was above a debt the liquidation closes in full: the controller then
  takes the whole offer, but the wrapper had authorized only the planned
  amount. It now offers and authorizes only the planned amount.
- `resolve_account` docs: a repayment never deletes an account; a withdrawal
  or a strategy call that leaves it empty does.

### Changed

- `XoxnoLending::withdraw` and `withdraw_all` return `Withdrawal` instead of
  the amount.
- `XoxnoLending::repay` returns net token spend. The pool refunds any amount
  above the debt to the current contract; the return value excludes it, so a
  contract that repays for a user can send the rest back. The README and the
  `account-basics` example do.
- The `account-basics` and `lending-vault` examples forget the stored account
  id only when the withdrawal closed the account.
- README: every example is a complete item with its own imports and typed
  inputs; operations, reads, markets and prices are reference tables; the
  testing section shows a contract and its fixture test. The borrow example
  requires the stored owner's authorization before it sends borrowed tokens.
- `scripts/check_readme.py` compiles every README example and runs its tests;
  CI runs it.

## [0.1.0] - 2026-09-24

### Added

- `XoxnoLending`, a wrapper for the calling contract: open and reuse accounts,
  supply, borrow, repay, withdraw, liquidate, flash loans, positions, market
  discovery across hubs and spokes, rates, and prices. `LendingAddresses` and
  `Position` go with it.
- `XoxnoLending::deposit`: supply to an account, or open one with account id 0.
- Examples: `simple-deposit`, `account-basics`, `market-picker`,
  `price-reader`, `liquidator` and `lending-vault`.
- Clients and types for the XOXNO Lending controller, pool, position NFT and
  price aggregator, generated from the attested rs-lending-xlm `v1.1.0`
  release. Each module's `WASM` is the release's deploy artifact, byte for
  byte.
- `FlashLoanReceiver` and `FlashPositionReceiver` callback traits.
- Unit and limit constants, and the `approve_flash_repayment` and
  `authorize_transfer_as_current` helpers.
- Mainnet and testnet addresses, hub ids and spoke ids.
- `testutils::LendingFixture`, with USDC and XLM market presets, extra hubs
  and spokes, and listings of one token in several hubs and spokes.
- Builder-facing clients generated from the embedded WASM, holding only the
  functions a builder calls.
- Built on `soroban-sdk` 28.
