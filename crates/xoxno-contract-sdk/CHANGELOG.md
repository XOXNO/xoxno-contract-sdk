# Changelog

All notable changes to this crate are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the crate uses
[semantic versioning](https://semver.org/): before 1.0, a minor version bump
means the embedded WASM, the ABI, the `soroban-sdk` major version or the
fixture API changed.

## [0.2.1] - 2026-09-24

### Added

- Batch operations on several markets in one controller call:
  `deposit_batch`, `supply_batch`, `borrow_batch`, `repay_batch` (amount
  repaid per market), `withdraw_batch` (`Withdrawals { amounts,
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
- `XoxnoLending::repay` returns the amount repaid. The pool refunds any amount
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
