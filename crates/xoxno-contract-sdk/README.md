# xoxno-contract-sdk

Call [XOXNO Lending] from a Soroban contract, and test that integration against
embedded protocol WASM. `XoxnoLending` handles calls and token authorizations;
generated clients cover the remaining protocol methods.

## Install

```toml
[lib]
crate-type = ["cdylib", "rlib"]

[profile.release]
overflow-checks = true

[dependencies]
soroban-sdk = "28"
xoxno-contract-sdk = "0.3"

[dev-dependencies]
soroban-sdk = { version = "28", features = ["testutils"] }
xoxno-contract-sdk = { version = "0.3", features = ["testutils"] }
```

Requires Rust 1.91 or later. Build contracts with `stellar contract build`
(stellar-cli 25.2 or later). The crate is `no_std`; enable `testutils` only for
tests.

## Ownership, authorization and units

`XoxnoLending` acts as the current contract: it pays and receives tokens and
owns the accounts it opens. Each account has a `u64` id and a position NFT;
the NFT owner owns the account. Sending tokens to an integrator does not give
the payer ownership of its account or a right to withdraw.

A public entrypoint that spends the contract's funds or borrows against its
account must authenticate an owner or policy stored by the contract. Requiring
authentication from an arbitrary address passed by the caller is insufficient.
The wrapper creates the exact token-transfer authorizations for its protocol
calls; it does not enforce your application's access policy.

| Term | Meaning |
|---|---|
| Hub | An isolated group of markets; one token can be listed in several hubs. |
| Market | One token in one hub: `lending::controller::HubAssetKey { asset, hub_id }`. |
| Spoke | An account's fixed risk profile: eligible markets, LTV, liquidation threshold and caps. |

| Unit | Scale | Used for |
|---|---|---|
| Token base units | Token decimals | Amounts and caps |
| WAD | 10^18 | USD values, prices and health factor |
| RAY | 10^27 | Rates, indexes and utilization |
| BPS | 10,000 = 100% | LTV, liquidation threshold, fees and bonuses |

Constants are in `lending::constants`. Market discovery checks listing flags;
the controller still checks liquidity, caps and account eligibility when an
action executes. A conversion of `borrowable_usd` into tokens is only a
headroom estimate, not a promise that a borrow succeeds.

## One owner-controlled account

This complete contract accepts contributions from any authenticated payer.
All withdrawals belong to the stored owner; there are no depositor balances
or shares. It checks exact receipt on payer funding, protocol withdrawal and
owner payout. A mismatch reverts the whole invocation before existing contract
funds can subsidize a withdrawal's receipt tax.

Inject `LendingAddresses` at construction: use `LendingAddresses::mainnet`,
`LendingAddresses::testnet`, or a fixture's `addresses()` for the intended
deployment. The first deposit opens an account; full withdrawal closes it
when no debt or other collateral remains; the next deposit opens another.

```rust,ignore
use soroban_sdk::{contract, contractimpl, contracttype, token, Address, Env};
use xoxno_contract_sdk::lending::controller::HubAssetKey;
use xoxno_contract_sdk::{LendingAddresses, Withdrawal, XoxnoLending};

#[contracttype]
#[derive(Clone)]
pub struct Config {
    pub owner: Address,
    pub lending: LendingAddresses,
    pub spoke_id: u32,
    pub market: HubAssetKey,
}

#[contracttype]
pub enum Key {
    Config,
    Account,
}

#[contract]
pub struct OwnerAccount;

#[contractimpl]
impl OwnerAccount {
    pub fn __constructor(env: Env, config: Config) {
        env.storage().instance().set(&Key::Config, &config);
    }

    pub fn deposit(env: Env, from: Address, amount: i128) -> u64 {
        assert!(amount > 0, "positive amount required");
        from.require_auth();
        let config: Config = env.storage().instance().get(&Key::Config).unwrap();
        let this = env.current_contract_address();
        let token = token::Client::new(&env, &config.market.asset);
        let before = token.balance(&this);
        token.transfer(&from, &this, &amount);
        assert_eq!(token.balance(&this).checked_sub(before), Some(amount));

        let lending = XoxnoLending::new(&env, &config.lending);
        let stored = env.storage().instance().get(&Key::Account);
        let id = lending.deposit(
            lending.resolve_account(stored), config.spoke_id, &config.market, amount,
        );
        env.storage().instance().set(&Key::Account, &id);
        id
    }

    pub fn withdraw_all(env: Env) -> Withdrawal {
        let config: Config = env.storage().instance().get(&Key::Config).unwrap();
        config.owner.require_auth();
        let id: u64 = env.storage().instance().get(&Key::Account).unwrap();
        let this = env.current_contract_address();
        let token = token::Client::new(&env, &config.market.asset);
        let before = token.balance(&this);
        let withdrawal = XoxnoLending::new(&env, &config.lending)
            .withdraw_all(id, &config.market);
        assert_eq!(token.balance(&this).checked_sub(before), Some(withdrawal.amount));
        if withdrawal.account_closed {
            env.storage().instance().remove(&Key::Account);
        }
        let before = token.balance(&config.owner);
        token.transfer(&this, &config.owner, &withdrawal.amount);
        assert_eq!(token.balance(&config.owner).checked_sub(before), Some(withdrawal.amount));
        withdrawal
    }
}

#[cfg(test)]
mod test {
    use super::{Config, OwnerAccount, OwnerAccountClient};
    use soroban_sdk::{testutils::Address as _, Address, Env};
    use xoxno_contract_sdk::testutils::{LendingFixture, MarketConfig};

    #[test]
    fn deposit_withdraw_close_and_reopen() {
        let env = Env::default();
        // Behavior test only: exact authorization needs separate tests.
        env.mock_all_auths();
        let fixture = LendingFixture::deploy(&env, &Address::generate(&env));
        let market = fixture.create_market(&MarketConfig::usdc());
        let owner = Address::generate(&env);
        let payer = Address::generate(&env);
        let address = env.register(OwnerAccount, (Config {
            owner: owner.clone(), lending: fixture.addresses(),
            spoke_id: fixture.spoke_id, market: market.key.clone(),
        },));
        let account = OwnerAccountClient::new(&env, &address);
        let amount = 1_000 * 10_000_000;
        market.sac.mint(&payer, &(2 * amount));

        let first = account.deposit(&payer, &amount);
        assert_eq!(fixture.position_nft.owner_of(&(first as u32)), address);
        let withdrawal = account.withdraw_all();
        assert!(withdrawal.account_closed);
        assert!(!fixture.controller.account_exists(&first));
        assert!(withdrawal.amount <= amount && amount - withdrawal.amount <= 1);
        assert_eq!(market.token.balance(&owner), withdrawal.amount);
        let second = account.deposit(&payer, &amount);
        assert_ne!(first, second);
    }
}
```

A repayment never closes an account. A withdrawal or strategy that leaves no
supply and no debt deletes it and burns its NFT. Clear the stored id when
`Withdrawal::account_closed` (or `Withdrawals::account_closed`) is true.
`resolve_account` checks existence only; it does not check NFT ownership.
`collateral` rounds half-up, while `withdraw_all` pays the floored claim, so
the pool output can be one base unit less than the collateral read, before
any token transfer tax.

See [account-basics source][account] and [tests][account-tests] for borrowing,
repayment refunds and account reuse. Keep account and application storage
alive with an appropriate TTL policy; `renew_account` renews protocol state,
not your integrator's storage.

## API and advanced integrations

The [API reference][api] documents actions, account reads, market discovery,
rates and prices. These are the important accounting and authorization rules:

- `deposit`, `supply`, `borrow`, `repay`, `withdraw` and `liquidate` have batch
  forms taking `&Vec<(HubAssetKey, i128)>`. `flash_loan` has no batch form.
  Repeated markets are merged; `WITHDRAW_ALL` (0) wins in a withdrawal batch.
  Returned amount vectors preserve merged request order; liquidation omits
  offers the plan does not use.
  Non-withdrawal batch amounts must each be positive, including duplicates.
- `repay` and `repay_batch` return the contract's net token spend, after pool
  refunds. This can differ from debt retired when a token taxes transfers.
  Forward refunds using the returned spend, not a before/after debt change.
  Receiving payer funds is a separate transfer and needs its own accounting;
  see [account-basics][account] and [batch/refund tests][wrapper-tests].
- `Withdrawal::amount` and `Withdrawals::amounts` report the pool's output
  before receipt tax, which can exceed the tokens the contract receives.
  Measure its balance change or reject a mismatch before forwarding funds;
  the tutorial rejects mismatches. Batch withdrawals can include one token
  in multiple hubs; measure receipt per token across those market outputs.
- `liquidate` and `liquidate_batch` preserve the full offered amounts, which
  must be available in the current contract. The controller may take the
  full offer on a debt close or only a planned amount on a partial repayment.
  Results measure net token spend, including refunds and seized collateral;
  they can be negative when seized collateral includes the payment token.
- Repayment and liquidation batches accept only one market per token. Using
  one token in two hubs in the same call panics with
  `GenericError::InvalidPayments`; use separate calls.
- `price` fails on invalid, stale or disagreeing prices. Use `try_price` or
  `quote` only when your application handles invalid data explicitly.
  A diagnostic quote's price must not be used without checking its status.
- Discovery must search all candidate hubs and spokes. The
  [market-picker][picker] implements the nested search, including a later hub
  when the first listed market cannot accept supply.

Flash receivers must authenticate the pool address stored at setup with
`require_auth()`, then check the authenticated `initiator`. Callback arguments
alone are forgeable. Use `approve_flash_repayment` for `amount + fee`; the
pool pulls repayment after the callback, so transferring it directly fails.
The receiver must be a WASM contract, distinct from the initiating contract.
During the callback, it cannot call the controller or pool, including views,
because they are already on the call stack. See [receiver source][vault],
[tests][vault-tests], [flash-loan callback rules][callbacks] and the distinct
[flash-position callback rules][flash-position].

Generated clients are available through `controller()`, `pool()`,
`position_nft()` and `price_aggregator()`. Each generated module has its own
Rust types: `controller::HubAssetKey` and `pool::HubAssetKey` are distinct.
Use the target module's types. Generated clients do not create token-transfer
authorizations: authorize each exact transfer with
`lending::helpers::authorize_transfer_as_current` or its plural form before
a call that pulls the current contract's tokens. Delegation, `multiply`,
`swap_*` and `flash_position` use the controller client; strategy calls need
route bytes from the XOXNO quote service.

Examples are repository source, excluded from the published crate:

| Source | Tests | Shows |
|---|---|---|
| [simple-deposit][simple] | [deposit tests][simple-tests] | Injected deployment, deposit and owner withdrawal |
| [account-basics][account] | [account tests][account-tests] | Owner account, contributions, borrow, repay, close and reopen |
| [market-picker][picker] | [placement tests][picker-tests] | Hub/spoke search, rates and utilization |
| [price-reader][prices] | [price tests][price-tests] | Strict/fallible prices, USD values and headroom estimates |
| [liquidator][liquidator] | [liquidation tests][liquidator-tests] | Liquidation offers, refunds and seized collateral |
| [lending-vault][vault] | [vault tests][vault-tests] | Owner account funded by contributors and a flash receiver; no shares |

## Fixture limits

`LendingFixture::deploy` deploys governance, controller, pool, position NFT,
price aggregator and two mock oracles, with one hub and one spoke.
Configuration goes through the governance timelock. `create_market` creates
a 7-decimal Stellar Asset Contract, configures both oracle prices and supplies
initial liquidity. Presets are snapshots, not a live network configuration.
See the [fixture reference][fixture] and [fixture tests][fixture-tests] for
additional hubs, spokes, listings, prices and time advancement.

The fixture changes the `Env`: timestamp is at least 1,000,000, ledger
sequence at least 100, persistent TTL at least 10,000,000 ledgers, and budget
unlimited. Reset `env.cost_estimate().budget().reset_default()` immediately
before the invocation whose resource use you want to measure. Long fixture
TTLs do not test expiry, archival or restoration under network settings.

The fixture scopes authorization mocks to its admin calls; it does not change
the environment's auth mode. The tutorial's `env.mock_all_auths()` also accepts
wrong authorization entries. Use explicit authorization trees and negative
tests to check your contract's access control; see [wrapper tests][wrapper-tests].

## Networks and compatibility

`networks::mainnet` and `networks::testnet` contain address and hub/spoke
snapshots. Governance can replace the price aggregator; `XoxnoLending` resolves
it from the controller. Check current deployment configuration before use.

The embedded fixture and generated specs are a protocol snapshot, recorded in
[wasm/MANIFEST.json][manifest] with an immutable source commit and artifact
hashes. Generated types come from documentation artifacts; each module's
`WASM` constant is the corresponding deploy artifact. This crate uses
`soroban-sdk` 28; its manifest records protocol source commit `65761475c`,
labeled `v1.0.1`. A release name alone does not establish current deployed
behavior.

`scripts/verify_wasm.py` checks embedded integrity; `scripts/gen_clients.py
--check` checks generated clients; `scripts/check_mainnet.py` checks live
artifact drift. A passing fixture test or unchanged ABI does not prove
execution parity with upgraded mainnet contracts. Verify current code hashes
and replay relevant integration tests against the deployment you target.

## License

MIT. [XOXNO Lending] contract source has its own license.

[XOXNO Lending]: https://github.com/XOXNO/rs-lending-xlm
[api]: https://docs.rs/xoxno-contract-sdk/latest/xoxno_contract_sdk/lending/struct.XoxnoLending.html
[callbacks]: https://docs.rs/xoxno-contract-sdk/latest/xoxno_contract_sdk/lending/trait.FlashLoanReceiver.html
[flash-position]: https://docs.rs/xoxno-contract-sdk/latest/xoxno_contract_sdk/lending/trait.FlashPositionReceiver.html
[fixture]: https://docs.rs/xoxno-contract-sdk/latest/xoxno_contract_sdk/testutils/struct.LendingFixture.html
[manifest]: https://github.com/XOXNO/xoxno-contract-sdk/blob/main/crates/xoxno-contract-sdk/wasm/MANIFEST.json
[simple]: https://github.com/XOXNO/xoxno-contract-sdk/blob/main/examples/simple-deposit/src/lib.rs
[simple-tests]: https://github.com/XOXNO/xoxno-contract-sdk/blob/main/examples/simple-deposit/tests/deposit.rs
[account]: https://github.com/XOXNO/xoxno-contract-sdk/blob/main/examples/account-basics/src/lib.rs
[account-tests]: https://github.com/XOXNO/xoxno-contract-sdk/blob/main/examples/account-basics/tests/account.rs
[picker]: https://github.com/XOXNO/xoxno-contract-sdk/blob/main/examples/market-picker/src/lib.rs
[picker-tests]: https://github.com/XOXNO/xoxno-contract-sdk/blob/main/examples/market-picker/tests/picker.rs
[prices]: https://github.com/XOXNO/xoxno-contract-sdk/blob/main/examples/price-reader/src/lib.rs
[price-tests]: https://github.com/XOXNO/xoxno-contract-sdk/blob/main/examples/price-reader/tests/prices.rs
[liquidator]: https://github.com/XOXNO/xoxno-contract-sdk/blob/main/examples/liquidator/src/lib.rs
[liquidator-tests]: https://github.com/XOXNO/xoxno-contract-sdk/blob/main/examples/liquidator/tests/liquidate.rs
[vault]: https://github.com/XOXNO/xoxno-contract-sdk/blob/main/examples/lending-vault/src/lib.rs
[vault-tests]: https://github.com/XOXNO/xoxno-contract-sdk/blob/main/examples/lending-vault/tests/vault.rs
[wrapper-tests]: https://github.com/XOXNO/xoxno-contract-sdk/blob/main/crates/xoxno-contract-sdk/tests/wrapper.rs
[fixture-tests]: https://github.com/XOXNO/xoxno-contract-sdk/blob/main/crates/xoxno-contract-sdk/tests/fixture.rs
