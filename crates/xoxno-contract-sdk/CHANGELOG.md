# Changelog

All notable changes to this crate are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the crate uses
[semantic versioning](https://semver.org/): before 1.0, a minor version bump
means the embedded WASM, the ABI, the `soroban-sdk` major version or the
fixture API changed.

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
