# xoxno-contract-sdk

Build Soroban contracts on top of [XOXNO Lending].

The crate has two layers:

- **`XoxnoLending`**, a wrapper for the calling contract. It opens and reuses
  accounts, supplies, borrows, repays, withdraws, liquidates, reads positions,
  markets and prices. It creates the token authorizations the protocol needs,
  and it returns token amounts, not raw shares.
- **Generated clients** for the controller, pool, position NFT and price
  aggregator, for anything the wrapper does not cover. They hold only the
  functions a builder calls, and their signatures come from WASM that has the
  same code as the contracts deployed on Stellar mainnet.

It also has callback traits for flash loans, constants, the mainnet and testnet
addresses, and a `testutils` fixture that deploys the whole protocol into a
test `Env`.

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

## Accounts

An account is a position NFT. The first supply opens it: the controller mints
the NFT to your contract and returns its token id, which is the account id.
Store the id and pass it to every later call.

```rust,ignore
use xoxno_contract_sdk::lending::constants::NEW_ACCOUNT;
use xoxno_contract_sdk::{LendingAddresses, XoxnoLending};

let lending = XoxnoLending::new(&env, &addresses); // or XoxnoLending::mainnet(&env)

// First supply, or reuse the stored account.
let stored: Option<u64> = env.storage().instance().get(&Key::Account);
let account_id = match lending.resolve_account(stored) {
    NEW_ACCOUNT => lending.open_account(spoke_id, &market, amount),
    account_id => lending.supply(account_id, &market, amount),
};
env.storage().instance().set(&Key::Account, &account_id);

lending.borrow(account_id, &debt_market, borrow_amount);   // tokens come to your contract
lending.repay(account_id, &debt_market, repay_amount);     // paid from your contract
let withdrawn = lending.withdraw_all(account_id, &market);
let position = lending.position(account_id);               // amounts per market, interest included
let health = lending.health_factor(account_id);            // WAD; i128::MAX without debt
```

Your contract is always the caller, payer and receiver, and it owns the NFT.
`resolve_account` matters: a full withdrawal or repayment that leaves an
account empty deletes it and burns the NFT, and the next supply must open a new
account.

Other account reads: `owns`, `owner_of`, `account_count`, `accounts_of`,
`account_spoke`, `collateral`, `debt`, `collateral_usd`, `debt_usd`,
`borrowable_usd`, `is_liquidatable`. `renew_account` extends the account's TTL.

## Markets: hubs and spokes

A market is a token in a hub: `HubAssetKey { asset, hub_id }`. The same token
can be listed in several hubs, and each hub market is isolated. A spoke is a
risk profile; an account opens in one spoke and keeps it. A spoke lists some
markets, each with its own LTV, liquidation threshold and caps.

```rust,ignore
use xoxno_contract_sdk::networks::mainnet;

let market = lending.find_market(&token, mainnet::HUB_IDS).unwrap(); // first hub that lists it
let spoke = mainnet::SPOKES.iter().map(|(id, _name)| *id)
    .find(|spoke| lending.can_supply(*spoke, &market)).unwrap();
let config = lending.spoke_asset(spoke, &market).unwrap();          // LTV, threshold, caps, flags
let rate = lending.supply_rate(&market);                            // RAY
```

`can_supply` and `can_borrow` check the listing flags. The controller checks
caps and the account's state when you act.

## Prices

```rust,ignore
let price = lending.price(&token);                 // USD, WAD; panics if the price is not usable
let maybe = lending.try_price(&token);             // None if stale, deviating or not configured
let status = lending.quote(&token);                // validity, staleness, deviation, both sources
let value = lending.value_usd(&token, amount);     // USD, WAD
```

Use `price` when your contract acts on the price, so it stops on a bad price.
Use `try_price` or `quote` when it can decide what to do without one.

## Liquidation

```rust,ignore
if lending.is_liquidatable(account_id) {
    let estimate = lending.liquidation_estimate(account_id, &debt_market, offer);
    let paid = lending.liquidate(account_id, &debt_market, offer); // seized collateral comes to your contract
}
```

The controller can use less than the offer. `liquidate` reads the same plan
first, so it authorizes exactly the amount the controller pulls.

## Flash loans

A flash-loan receiver implements `FlashLoanReceiver` and repays by approving
the pool:

```rust,ignore
#[contractimpl]
impl FlashLoanReceiver for MyContract {
    fn execute_flash_loan(env: Env, initiator: Address, asset: Address, amount: i128,
                          fee: i128, pool: Address, data: Bytes) {
        let cfg = config(&env);
        cfg.lending.pool.require_auth();                  // only the pool can call this
        if initiator != cfg.owner || pool != cfg.lending.pool { panic!() }
        // ... use the funds ...
        approve_flash_repayment(&env, &asset, &pool, amount + fee);
    }
}
```

- Anyone can call a receiver directly with forged arguments. Call
  `require_auth()` on the stored pool address first; it passes only when the
  pool is the caller.
- The receiver cannot be the contract that starts the loan, and the callback
  cannot call the controller or the pool: Soroban rejects a call into a
  contract that is already on the call stack. Start the loan from an account
  or another contract (`XoxnoLending::flash_loan`).

`FlashPositionReceiver` is the same for flash positions; see its docs.

## Generated clients

`lending.controller()`, `lending.pool()`, `lending.position_nft()` and
`lending.price_aggregator()` return the generated clients. The modules
`lending::{controller, pool, position_nft, price_aggregator}` hold every type
and error of each contract. Each module has its own copy of the shared types:
use `lending::controller` types when you call the controller.

## Rules to know

- **Units.** Token amounts and caps are token base units. USD values, prices
  and the health factor are WAD (1e18). Shares, indexes and rates are RAY
  (1e27). Risk parameters and fees are BPS (10,000 = 100%).
- **Rounding.** `collateral` rounds half-up. A full withdrawal pays the floored
  claim, which can be 1 unit less.
- **Delegates and strategies.** Delegation, `multiply`, `swap_*` and
  `flash_position` are on the generated controller client. Strategy calls need
  swap route bytes from the XOXNO quote service.

## Test with `LendingFixture`

```rust,ignore
let env = Env::default();
env.mock_all_auths();
let fixture = LendingFixture::deploy(&env, &Address::generate(&env));
let usdc = fixture.create_market(&MarketConfig::usdc());
let xlm = fixture.create_market(&MarketConfig::xlm());
let my_contract = env.register(MyContract, (fixture.addresses(), fixture.spoke_id, usdc.key.clone()));

fixture.set_price(&xlm, WAD * 12 / 100);  // move a price
fixture.advance_time(30 * 86_400);        // move time and accrue interest
```

- `deploy` deploys governance, the controller, the pool, the position NFT, the
  price aggregator and two mock oracles, with one hub and one spoke. Governance
  owns the controller, as on mainnet, and every configuration step goes
  through the governance timelock.
- `create_market` lists a new 7-decimal token with a dual-source oracle and
  supplies initial liquidity. `MarketConfig::usdc()` and `MarketConfig::xlm()`
  use the mainnet market parameters and "Blue Chip" asset settings.
- `add_hub`, `add_spoke`, `create_market_in`, `list_market`,
  `add_market_to_hub` and `supply_liquidity` build more hubs, spokes and
  listings.
- `fixture.addresses()` gives the `LendingAddresses` your contract needs.

It changes the `Env`:

- It raises the timestamp to at least 1,000,000 and the sequence to at least
  100.
- It raises the minimum persistent entry TTL to 10,000,000 ledgers, and the
  maximum entry TTL above it.
- It sets the budget to unlimited. To check one call against the network
  limits, call `env.cost_estimate().budget().reset_default()` just before it.

It authorizes its own admin calls per call and does not change the auth mode of
the `Env`.

## Examples

Each example in the repository is a contract with tests on `LendingFixture`:

| Example | Shows |
|---|---|
| `account-basics` | First supply returns the NFT id; reuse it; borrow, repay, withdraw; reopen after the account closes |
| `market-picker` | Find the hub that lists a token and a spoke that accepts it; read rates and utilization |
| `price-reader` | Strict and tolerant prices, USD values, borrow headroom in tokens |
| `liquidator` | A contract liquidator that pays exactly what the plan uses |
| `lending-vault` | A vault on one account, with a flash-loan receiver |

## Networks

`xoxno_contract_sdk::networks::{mainnet, testnet}` hold the governance,
controller, pool and position NFT addresses, the hub ids, and the spoke ids with
their names. `LendingAddresses::mainnet(&env)` and `::testnet(&env)` build from
them. The price aggregator can change: the wrapper reads it from the controller.

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
