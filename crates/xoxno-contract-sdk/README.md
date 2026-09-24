# xoxno-contract-sdk

Clients, types, WASM and a test fixture for calling [XOXNO Lending] from your
own Soroban contracts.

- **Clients and types** for the controller, the position NFT, the price
  aggregator and the pool. They are generated with `contractimport!` from WASM
  that has the same code as the contracts deployed on Stellar mainnet.
- **Callback traits** for flash loans and flash positions.
- **Constants and helpers**: units, limits, and the authorization entries a
  contract needs to supply, repay, or repay a flash loan.
- **Network addresses** of the mainnet and testnet deployments.
- **`LendingFixture`** (feature `testutils`): deploys the whole protocol into a
  test `Env`, lists markets, moves prices and time.

## Install

```toml
[dependencies]
xoxno-contract-sdk = "0.1"

[dev-dependencies]
xoxno-contract-sdk = { version = "0.1", features = ["testutils"] }
```

The crate is `no_std` and uses `soroban-sdk` 28. Build your contract with
`stellar contract build` (stellar-cli 25.2 or newer), as `soroban-sdk` 28
requires.

## Call XOXNO Lending from a contract

```rust,ignore
use soroban_sdk::{vec, Address, Env};
use xoxno_contract_sdk::lending::constants::NEW_ACCOUNT;
use xoxno_contract_sdk::lending::controller::HubAssetKey;
use xoxno_contract_sdk::lending::helpers::authorize_transfer_as_current;
use xoxno_contract_sdk::lending::ControllerClient;

/// Supplies `amount` of the current contract's tokens and returns the account id.
fn supply(env: &Env, controller: &Address, market: &HubAssetKey, spoke_id: u32, amount: i128) -> u64 {
    let this = env.current_contract_address();
    let client = ControllerClient::new(env, controller);
    let pool = client.get_pool_address();
    // The controller moves the tokens with `transfer(this, pool, amount)`.
    authorize_transfer_as_current(env, &market.asset, &this, &pool, amount);
    client.supply(&this, &NEW_ACCOUNT, &spoke_id, &vec![env, (market.clone(), amount)])
}
```

Store the returned account id. The account is the position NFT with that id,
and the contract that owns the NFT owns the account.

A full working contract is in
[`examples/lending-vault`](https://github.com/XOXNO/xoxno-contract-sdk/tree/main/examples/lending-vault).

## Rules to know

- **Units.** Token amounts and caps are token base units. USD values, prices
  and the health factor are WAD (1e18). Shares, indexes and rates are RAY
  (1e27). Risk parameters and fees are BPS (10,000 = 100%).
- **Rounding.** `get_collateral_amount` rounds half-up. A full withdrawal
  (`WITHDRAW_ALL`) pays the floored claim, which can be 1 unit less.
- **Accounts.** Pass `NEW_ACCOUNT` to open an account. Moving the position NFT
  moves the account.
- **Flash callbacks.** Anyone can call a receiver's callback directly and
  forge its arguments. In the callback, call `require_auth()` on the pool (flash
  loan) or controller (flash position) address the contract stored, then check
  `initiator`. Repay a flash loan by approving the pool for `amount + fee`
  (`approve_flash_repayment`). The receiver cannot be the contract that calls
  `flash_loan`, and the callback cannot call the controller or the pool:
  Soroban rejects a call into a contract that is already on the call stack.
- **Types per module.** Each contract module has its own copy of the shared
  types. Use `lending::controller` types when you call the controller.
- **Admin functions** are present in the generated clients but revert unless
  the caller is the owner, which on mainnet is governance.

## Test with `LendingFixture`

```rust,ignore
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{vec, Address, Env};
use xoxno_contract_sdk::lending::constants::{NEW_ACCOUNT, WAD};
use xoxno_contract_sdk::testutils::{LendingFixture, MarketConfig};

#[test]
fn borrow_and_accrue() {
    let env = Env::default();
    env.mock_all_auths();
    let fixture = LendingFixture::deploy(&env, &Address::generate(&env));
    let usdc = fixture.create_market(&MarketConfig::usdc());
    let xlm = fixture.create_market(&MarketConfig::xlm());

    let user = Address::generate(&env);
    usdc.sac.mint(&user, &10_000_0000000);
    let account = fixture.controller.supply(
        &user, &NEW_ACCOUNT, &fixture.spoke_id, &vec![&env, (usdc.key.clone(), 10_000_0000000)],
    );
    fixture.controller.borrow(&user, &account, &vec![&env, (xlm.key.clone(), 50_000_0000000)], &None);

    fixture.advance_time(30 * 86_400);
    fixture.set_price(&xlm, WAD * 12 / 100);
}
```

What the fixture does:

- It deploys governance, the controller, the pool, the position NFT, the price
  aggregator and two mock oracles. Governance owns the controller, as on
  mainnet, and every configuration step goes through the governance timelock.
  The fixture's spoke uses the liquidation curve of the mainnet "Blue Chip"
  spoke.
- `create_market` lists a 7-decimal Stellar Asset Contract token and
  configures a dual-source oracle. It then supplies the initial liquidity from a
  new liquidity provider. `MarketConfig::usdc()` and `MarketConfig::xlm()` use
  the mainnet market parameters and "Blue Chip" asset settings. The oracle
  configuration is the fixture's own, not mainnet's.
- `set_price` moves both oracle feeds. `advance_time` moves the timestamp and
  the ledger sequence, publishes the prices again, and accrues interest.

It changes the `Env`:

- It raises the timestamp to at least 1,000,000 and the sequence to at least
  100.
- It raises the minimum persistent entry TTL to 10,000,000 ledgers, and the
  maximum entry TTL above it.
- It sets the budget to unlimited. To check one call against the network
  limits, call `env.cost_estimate().budget().reset_default()` just before it.

It authorizes its own admin calls per call and does not change the auth mode of
the `Env`.

## Networks

`xoxno_contract_sdk::networks::{mainnet, testnet}` hold the governance,
controller, pool and position NFT addresses, the hub ids, and the spoke ids with
their names. The price aggregator can change: read it with
`ControllerClient::price_aggregator`.

## WASM and compatibility

`wasm/MANIFEST.json` records, for each contract:

- the source repository, tag and commit;
- the SHA-256 of the embedded file;
- the hash of its code without custom sections;
- the hash of the deployed artifact.

The deployed artifact hash is checked against the live mainnet contract when
the WASM is synced. The embedded files keep their contract spec docs and error
enums, so the generated clients have rustdoc and typed errors. Their code is
identical to the deployed code.

Version 0.1.0 was synced from a clean rebuild of rs-lending-xlm `v1.0.0`
(`"method": "build-dir"` in the manifest). The rebuild reproduced every
mainnet hash. From the next rs-lending-xlm release on, the WASM comes from the
release's attested SDK bundle (`"method": "release"`).

| xoxno-contract-sdk | soroban-sdk | rs-lending-xlm |
|---|---|---|
| 0.1.x | 28 | v1.0.0 (`d26b93ebb`) |

## License

MIT. The XOXNO Lending contract source is in
[XOXNO/rs-lending-xlm](https://github.com/XOXNO/rs-lending-xlm) under its own
license.

[XOXNO Lending]: https://github.com/XOXNO/rs-lending-xlm
