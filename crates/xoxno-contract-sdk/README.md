# xoxno-contract-sdk

Call [XOXNO Lending] from a Soroban contract, and test that contract against a
local deployment of the protocol.

- `XoxnoLending` calls the protocol as the current contract. It creates the
  token authorizations each call needs and returns token amounts.
- Generated clients for the controller, pool, position NFT and price
  aggregator cover the calls the wrapper does not.
- Flash-loan callback traits, unit constants, and the mainnet and testnet
  addresses.
- `testutils::LendingFixture` deploys the protocol into a test `Env`.

## Install

```toml
[dependencies]
soroban-sdk = "28"
xoxno-contract-sdk = "0.2"

[dev-dependencies]
soroban-sdk = { version = "28", features = ["testutils"] }
xoxno-contract-sdk = { version = "0.2", features = ["testutils"] }
```

Build contracts with `stellar contract build` (stellar-cli 25.2 or later). The
crate is `no_std`.

## Model

| Term | Meaning |
|---|---|
| Hub | An isolated group of markets. A token can be listed in several hubs. |
| Market | One token in one hub: `HubAssetKey { asset, hub_id }`. `lending.market(hub_id, &token)` builds it. |
| Spoke | A risk profile. It lists the markets an account can use, each with its LTV, liquidation threshold and caps. An account stays in the spoke it opens in. |
| Account | A `u64` id. Opening an account mints a position NFT with the same token id; the NFT owner owns the account. |

`XoxnoLending` acts as the current contract. The contract is the caller, pays
and receives the tokens, and owns the accounts it opens. Build it with
`XoxnoLending::mainnet(&env)`, `XoxnoLending::testnet(&env)`, or
`XoxnoLending::new(&env, &addresses)` for a `LendingAddresses` value.

Hub and spoke ids are in `networks::mainnet` and `networks::testnet`.

## Deposit

```rust
use soroban_sdk::{contract, contractimpl, token, Address, Env};
use xoxno_contract_sdk::XoxnoLending;

#[contract]
pub struct Vault;

#[contractimpl]
impl Vault {
    /// Moves `amount` of `token` from `from` into market (`hub_id`, `token`).
    /// `account_id` 0 opens an account in `spoke_id`. Returns the account id.
    pub fn deposit(
        env: Env,
        from: Address,
        token: Address,
        hub_id: u32,
        spoke_id: u32,
        account_id: u64,
        amount: i128,
    ) -> u64 {
        from.require_auth();
        token::Client::new(&env, &token).transfer(&from, env.current_contract_address(), &amount);

        let lending = XoxnoLending::mainnet(&env);
        let market = lending.market(hub_id, &token);
        lending.deposit(account_id, spoke_id, &market, amount)
    }
}
```

The call fails with `SpokeError::AssetNotInSpoke` when the spoke does not list
the market, and with `SpokeError::SpokeMismatch` when `spoke_id` is not the
spoke of an existing account.

## Keep one account

A contract that holds a single account stores its id. The protocol deletes an
account, and burns its NFT, when a withdrawal or a strategy call leaves it with
no supply and no debt. A repayment never deletes an account. `withdraw` and `withdraw_all` report this in `Withdrawal::account_closed`,
which the wrapper reads from the position NFT: the NFT no longer exists.
`resolve_account` returns the stored id if the account still exists, and 0
otherwise, so the next deposit opens a new account.

```rust
use soroban_sdk::{contracttype, Env};
use xoxno_contract_sdk::lending::controller::HubAssetKey;
use xoxno_contract_sdk::{Withdrawal, XoxnoLending};

#[contracttype]
pub enum DataKey {
    Account,
}

/// Supplies `amount` of `market` from the current contract's balance to its
/// account, and returns the account id.
pub fn supply(env: &Env, spoke_id: u32, market: &HubAssetKey, amount: i128) -> u64 {
    let lending = XoxnoLending::mainnet(env);
    let stored = env.storage().instance().get(&DataKey::Account);
    let account_id = lending.deposit(lending.resolve_account(stored), spoke_id, market, amount);
    env.storage().instance().set(&DataKey::Account, &account_id);
    account_id
}

/// Withdraws the whole supply of `market` to the current contract. Forgets the
/// account id when the withdrawal closed the account and burned its NFT.
pub fn withdraw_all(env: &Env, account_id: u64, market: &HubAssetKey) -> Withdrawal {
    let withdrawal = XoxnoLending::mainnet(env).withdraw_all(account_id, market);
    if withdrawal.account_closed {
        env.storage().instance().remove(&DataKey::Account);
    }
    withdrawal
}
```

## Operations

Amounts are token base units. "Contract" is the current contract.

| Method | Tokens | Returns |
|---|---|---|
| `deposit(account_id, spoke_id, &market, amount)` | contract → protocol | Account id. Opens an account when `account_id` is 0. |
| `open_account(spoke_id, &market, amount)` | contract → protocol | New account id |
| `supply(account_id, &market, amount)` | contract → protocol | Account id |
| `borrow(account_id, &market, amount)` | protocol → contract | |
| `repay(account_id, &market, amount)` | contract → protocol | Amount repaid. The pool refunds any amount above the debt to the contract. |
| `withdraw(account_id, &market, amount)` | protocol → contract | `Withdrawal { amount, account_closed }`. `account_closed`: the account was deleted and its NFT burned. |
| `withdraw_all(account_id, &market)` | protocol → contract | `Withdrawal`, as `withdraw` |
| `liquidate(account_id, &market, amount)` | contract → protocol, seized collateral → contract | Amount paid |
| `flash_loan(&market, amount, &receiver, &data)` | protocol → receiver → protocol | |
| `renew_account(account_id)` | | Extends the TTL of the account, its positions and its NFT. |

Each operation that moves tokens also has a `_batch` form for several markets
in one call; see [Batches](#batches).

The wrapper spends the contract's tokens and borrows against the contract's
accounts. A public entrypoint that calls it must check who may start that
action, with an address the contract stored, not one the caller passes.

Borrow for the owner, and repay with a payer's tokens:

```rust
use soroban_sdk::{contracttype, token, Address, Env};
use xoxno_contract_sdk::lending::controller::HubAssetKey;
use xoxno_contract_sdk::XoxnoLending;

#[contracttype]
pub enum DataKey {
    Owner,
}

/// Borrows `amount` of `market` against the contract's account and sends it to
/// the owner the contract stored at construction.
pub fn borrow_to_owner(env: &Env, account_id: u64, market: &HubAssetKey, amount: i128) {
    let owner: Address = env.storage().instance().get(&DataKey::Owner).unwrap();
    owner.require_auth();
    XoxnoLending::mainnet(env).borrow(account_id, market, amount);
    token::Client::new(env, &market.asset).transfer(&env.current_contract_address(), &owner, &amount);
}

/// Repays up to `amount` of the account's debt in `market` with tokens from
/// `from`, sends the part above the debt back, and returns the amount repaid.
pub fn repay_from(env: &Env, account_id: u64, market: &HubAssetKey, amount: i128, from: &Address) -> i128 {
    from.require_auth();
    let this = env.current_contract_address();
    let token = token::Client::new(env, &market.asset);
    token.transfer(from, &this, &amount);
    let repaid = XoxnoLending::mainnet(env).repay(account_id, market, amount);
    if repaid < amount {
        token.transfer(&this, from, &(amount - repaid));
    }
    repaid
}
```

## Batches

The `_batch` methods take `&Vec<(HubAssetKey, i128)>` and act on several
markets in one controller call.

| Method | Returns |
|---|---|
| `deposit_batch(account_id, spoke_id, &assets)` | Account id. Opens an account when `account_id` is 0. |
| `supply_batch(account_id, &assets)` | Account id |
| `borrow_batch(account_id, &assets)` | |
| `repay_batch(account_id, &payments)` | Amount repaid per market |
| `withdraw_batch(account_id, &assets)` | `Withdrawals { amounts, account_closed }`: amount received per market |
| `liquidate_batch(account_id, &payments)` | Amount paid per market the plan uses |

- A market listed twice is merged, as the controller does: its amounts are
  summed, and for `withdraw_batch` a `WITHDRAW_ALL` (0) wins. Results have one
  entry per market, in request order.
- `repay_batch` and `liquidate_batch` take one market per token, because the
  refunds and the liquidation plan are reported per token. A token listed in
  two hubs needs two calls; otherwise the call panics with
  `GenericError::InvalidPayments`.
- The paying batches authorize one exact transfer per market
  (`lending::helpers::authorize_transfers_as_current`).

Repay several debts with a payer's tokens, and return each refund:

```rust
use soroban_sdk::{token, Address, Env, Vec};
use xoxno_contract_sdk::lending::controller::HubAssetKey;
use xoxno_contract_sdk::XoxnoLending;

/// Repays up to each `(market, amount)` of `payments` with tokens from `from`,
/// sends the part above each debt back, and returns the amount repaid per market.
pub fn repay_batch_from(
    env: &Env,
    account_id: u64,
    payments: &Vec<(HubAssetKey, i128)>,
    from: &Address,
) -> Vec<(HubAssetKey, i128)> {
    from.require_auth();
    let this = env.current_contract_address();
    for (market, amount) in payments.iter() {
        token::Client::new(env, &market.asset).transfer(from, &this, &amount);
    }
    let repaid = XoxnoLending::mainnet(env).repay_batch(account_id, payments);
    for (market, repaid_amount) in repaid.iter() {
        let sent: i128 = payments
            .iter()
            .filter(|(paid_market, _)| *paid_market == market)
            .map(|(_, amount)| amount)
            .sum();
        if repaid_amount < sent {
            token::Client::new(env, &market.asset).transfer(&this, from, &(sent - repaid_amount));
        }
    }
    repaid
}
```

## Account reads

| Method | Returns |
|---|---|
| `position(account_id)` | `Position { collateral, debt }`: `(HubAssetKey, amount)` per market, interest included |
| `collateral(account_id, &market)` | Supplied amount, interest included, rounded half-up |
| `debt(account_id, &market)` | Borrowed amount, interest included |
| `health_factor(account_id)` | WAD. `i128::MAX` without debt. Below `WAD`, the account can be liquidated. |
| `is_liquidatable(account_id)` | `bool` |
| `collateral_usd(account_id)`, `debt_usd(account_id)`, `borrowable_usd(account_id)` | USD, WAD |
| `account_exists(account_id)`, `owns(account_id)`, `owner_of(account_id)`, `account_spoke(account_id)` | Existence, ownership by the contract, owner, spoke |
| `account_count(&owner)`, `accounts_of(&owner, start, limit)` | Accounts held by `owner` |

`withdraw_all` pays the floored claim, so its `amount` can be 1 unit less than
`collateral`.

## Choose a market

```rust
use soroban_sdk::{Address, Env};
use xoxno_contract_sdk::lending::controller::HubAssetKey;
use xoxno_contract_sdk::networks::mainnet;
use xoxno_contract_sdk::XoxnoLending;

/// The first mainnet hub that lists `token`, and the first spoke that accepts
/// a new supply of it.
pub fn placement(env: &Env, token: &Address) -> Option<(u32, HubAssetKey)> {
    let lending = XoxnoLending::mainnet(env);
    let market = lending.find_market(token, mainnet::HUB_IDS)?;
    let spoke_id = mainnet::SPOKES
        .iter()
        .map(|(spoke_id, _name)| *spoke_id)
        .find(|spoke_id| lending.can_supply(*spoke_id, &market))?;
    Some((spoke_id, market))
}
```

| Method | Returns |
|---|---|
| `is_listed(&market)` | Whether a hub lists the token |
| `can_supply(spoke_id, &market)`, `can_borrow(spoke_id, &market)` | Whether the listing flags allow a new supply or borrow. The controller checks caps and the account when you act. |
| `spoke(spoke_id)` | `Option<SpokeConfig>` |
| `spoke_asset(spoke_id, &market)` | `Option<SpokeAssetConfig>`: LTV, liquidation threshold, caps, flags |
| `supply_rate(&market)`, `borrow_rate(&market)` | Annual rate, RAY |
| `utilization(&market)`, `supply_index(&market)` | RAY |
| `liquidity(&market)` | Cash available to borrow, token base units |

## Prices

```rust
use soroban_sdk::{token, Address, Env};
use xoxno_contract_sdk::XoxnoLending;

/// How many base units of `asset` the account can still borrow, rounded down.
pub fn max_borrow(env: &Env, account_id: u64, asset: &Address) -> i128 {
    let lending = XoxnoLending::mainnet(env);
    let headroom_usd = lending.borrowable_usd(account_id);
    let price = lending.price(asset);
    let unit = 10i128.pow(token::Client::new(env, asset).decimals());
    headroom_usd.checked_mul(unit).map_or(i128::MAX, |scaled| scaled / price)
}
```

| Method | Returns |
|---|---|
| `price(&asset)` | USD per whole token, WAD. Panics when the price is stale, deviates between sources, or is not configured. |
| `try_price(&asset)` | `Option<i128>`: `None` where `price` panics |
| `quote(&asset)` | `PriceStatus`: validity, staleness, deviation, final price and error code |
| `value_usd(&asset, amount)` | USD value of `amount` base units, WAD |

Use `price` when the contract acts on the price, so the call stops on a bad
price. Use `try_price` or `quote` when the contract has a fallback.

## Liquidate

```rust
use soroban_sdk::Env;
use xoxno_contract_sdk::lending::controller::HubAssetKey;
use xoxno_contract_sdk::XoxnoLending;

/// Repays up to `max_repay` of the account's debt in `debt_market` from the
/// contract's balance. Returns the amount paid, or 0 if the account is healthy.
pub fn liquidate(env: &Env, account_id: u64, debt_market: &HubAssetKey, max_repay: i128) -> i128 {
    let lending = XoxnoLending::mainnet(env);
    if !lending.is_liquidatable(account_id) {
        return 0;
    }
    lending.liquidate(account_id, debt_market, max_repay)
}
```

The controller can use less than `max_repay`. `liquidate` reads the plan first
(`liquidation_estimate`), then offers and authorizes only the planned amount.
The rest stays in the contract, and the seized collateral goes to it.
`liquidate_batch` does the same with offers in several debt markets.

## Flash loans

The contract that starts the loan calls
`lending.flash_loan(&market, amount, &receiver, &data)`. The receiver is a
different contract that implements `FlashLoanReceiver`:

```rust
use soroban_sdk::{contract, contractimpl, contracttype, Address, Bytes, Env};
use xoxno_contract_sdk::lending::helpers::approve_flash_repayment;
use xoxno_contract_sdk::lending::FlashLoanReceiver;
use xoxno_contract_sdk::LendingAddresses;

#[contracttype]
pub enum DataKey {
    Owner,
}

#[contract]
pub struct Receiver;

#[contractimpl]
impl Receiver {
    /// `owner` is the only account that can start a loan to this contract.
    pub fn __constructor(env: Env, owner: Address) {
        env.storage().instance().set(&DataKey::Owner, &owner);
    }
}

#[contractimpl]
impl FlashLoanReceiver for Receiver {
    fn execute_flash_loan(
        env: Env,
        initiator: Address,
        asset: Address,
        amount: i128,
        fee: i128,
        pool: Address,
        _data: Bytes,
    ) {
        let expected_pool = LendingAddresses::mainnet(&env).pool;
        expected_pool.require_auth();
        let owner: Address = env.storage().instance().get(&DataKey::Owner).unwrap();
        assert!(pool == expected_pool && initiator == owner, "unexpected flash loan");

        // Use `amount` of `asset` here.
        approve_flash_repayment(&env, &asset, &pool, amount + fee);
    }
}
```

- Anyone can call `execute_flash_loan` with forged arguments.
  `require_auth()` on the pool address passes only when the pool is the
  caller. Check it before you trust `initiator`.
- Repay with `approve_flash_repayment`. The pool takes `amount + fee` after
  the callback returns.
- The receiver cannot start its own loan, and the callback cannot call the
  controller or the pool: Soroban rejects a call into a contract that is
  already on the call stack.

`FlashPositionReceiver` is the callback for `flash_position`; see its rustdoc.

## Generated clients

`lending.controller()`, `lending.pool()`, `lending.position_nft()` and
`lending.price_aggregator()` return the generated clients. The modules
`lending::{controller, pool, position_nft, price_aggregator}` hold each
contract's types and errors. Each module has its own copy of the shared types:
use the `lending::controller` types with the controller.

A generated client does not create token authorizations. Before a controller
call that takes tokens from the contract, call
`lending::helpers::authorize_transfer_as_current` for that transfer.
Delegation, `multiply`, `swap_*` and `flash_position` are on the controller
client. Strategy calls need swap route bytes from the XOXNO quote service.

## Units

| Unit | Scale | Used for |
|---|---|---|
| Token base units | Token decimals | Amounts, caps |
| WAD | 10^18 | USD values, prices, health factor |
| RAY | 10^27 | Rates, indexes, utilization |
| BPS | 10,000 = 100% | LTV, liquidation threshold, fees, bonuses |

The constants are in `lending::constants`.

## Test with `LendingFixture`

A contract that runs against the fixture takes the protocol addresses as a
constructor argument instead of calling `XoxnoLending::mainnet`. Deploy it
with `LendingAddresses::mainnet(&env)` on mainnet and `fixture.addresses()` in
tests.

```rust,ignore
use soroban_sdk::{contract, contractimpl, contracttype, token, Address, Env};
use xoxno_contract_sdk::{LendingAddresses, XoxnoLending};

#[contracttype]
pub enum DataKey {
    Lending,
}

#[contract]
pub struct Vault;

#[contractimpl]
impl Vault {
    pub fn __constructor(env: Env, lending: LendingAddresses) {
        env.storage().instance().set(&DataKey::Lending, &lending);
    }

    pub fn deposit(
        env: Env,
        from: Address,
        token: Address,
        hub_id: u32,
        spoke_id: u32,
        account_id: u64,
        amount: i128,
    ) -> u64 {
        from.require_auth();
        token::Client::new(&env, &token).transfer(&from, env.current_contract_address(), &amount);

        let addresses: LendingAddresses = env.storage().instance().get(&DataKey::Lending).unwrap();
        let lending = XoxnoLending::new(&env, &addresses);
        let market = lending.market(hub_id, &token);
        lending.deposit(account_id, spoke_id, &market, amount)
    }
}

#[cfg(test)]
mod test {
    use super::{Vault, VaultClient};
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::{Address, Env};
    use xoxno_contract_sdk::lending::constants::NEW_ACCOUNT;
    use xoxno_contract_sdk::testutils::{LendingFixture, MarketConfig};

    const UNIT: i128 = 10_000_000;

    #[test]
    fn first_deposit_opens_an_account() {
        let env = Env::default();
        env.mock_all_auths();
        let fixture = LendingFixture::deploy(&env, &Address::generate(&env));
        let usdc = fixture.create_market(&MarketConfig::usdc());
        let vault = VaultClient::new(&env, &env.register(Vault, (fixture.addresses(),)));
        let user = Address::generate(&env);
        usdc.sac.mint(&user, &(1_000 * UNIT));

        let account_id = vault.deposit(
            &user,
            &usdc.asset,
            &fixture.hub_id,
            &fixture.spoke_id,
            &NEW_ACCOUNT,
            &(1_000 * UNIT),
        );

        assert_eq!(fixture.position_nft.owner_of(&(account_id as u32)), vault.address);
        assert_eq!(fixture.controller.get_collateral_amount(&account_id, &usdc.key), 1_000 * UNIT);
    }
}
```

`LendingFixture::deploy` deploys governance, the controller, the pool, the
position NFT, the price aggregator and two mock oracles, with one hub
(`fixture.hub_id`) and one spoke (`fixture.spoke_id`). Governance owns the
controller, and every configuration step goes through the governance
timelock, as on mainnet.

| Method | Effect |
|---|---|
| `create_market(&MarketConfig::usdc())` | Lists a new 7-decimal token with a dual-source oracle and initial liquidity. `MarketConfig::usdc()` and `MarketConfig::xlm()` use the mainnet parameters. Returns a `Market`: `asset`, `key`, `token`, `sac`. |
| `create_market_in(&config, hub_id, spoke_id)` | The same, in another hub and spoke |
| `add_hub()`, `add_spoke()` | New hub or spoke id |
| `add_market_to_hub`, `list_market`, `supply_liquidity` | Build other listings |
| `set_price(&market, price_wad)` | Moves both oracle prices |
| `advance_time(seconds)` | Moves the ledger and accrues interest |

The fixture changes the `Env`:

- timestamp at least 1,000,000 and ledger sequence at least 100;
- minimum persistent entry TTL 10,000,000 ledgers;
- unlimited budget. To check one call against the network limits, call
  `env.cost_estimate().budget().reset_default()` just before it.

The fixture authorizes its own admin calls and does not change the auth mode
of the `Env`. `env.mock_all_auths()` also accepts wrong authorization entries;
leave it off to test the contract's own authorization.

## Examples

Each example in the repository is a contract with tests on `LendingFixture`.

| Example | Shows |
|---|---|
| `simple-deposit` | Deposit with the token, hub, spoke and account as parameters |
| `account-basics` | One stored account: deposit, borrow, repay, withdraw, reopen after it closes |
| `market-picker` | Find the hub and spoke for a token; read rates and utilization |
| `price-reader` | Strict and fallible prices, USD values, borrow headroom in tokens |
| `liquidator` | A liquidator contract that pays exactly what the plan uses |
| `lending-vault` | A vault on one account, with a flash-loan receiver |

## Networks

`networks::mainnet` and `networks::testnet` hold the governance, controller,
pool and position NFT addresses, the hub ids, and the spoke ids with their
names. Governance can replace the price aggregator, so the wrapper reads its
address from the controller.

## WASM and compatibility

`wasm/MANIFEST.json` records, for each contract, the source release and
commit, the SHA-256 of the embedded file, its code hash without custom
sections, and the SHA-256 of the deploy artifact in `wasm/deploy/`.

The generated types come from the files with contract spec docs. Each module's
`WASM` constant, and so the fixture, is the deploy artifact byte for byte, so
its SHA-256 is the on-chain WASM hash. `scripts/check_mainnet.py` compares
those hashes with the live mainnet contracts.

| xoxno-contract-sdk | soroban-sdk | rs-lending-xlm |
|---|---|---|
| 0.1.x, 0.2.x | 28 | v1.1.0 (`dc57a8562`) |

0.1.0 and 0.2.0 were published before the mainnet upgrade to rs-lending-xlm
v1.1.0.

## License

MIT. The XOXNO Lending contract source is in
[XOXNO/rs-lending-xlm](https://github.com/XOXNO/rs-lending-xlm) under its own
license.

[XOXNO Lending]: https://github.com/XOXNO/rs-lending-xlm
