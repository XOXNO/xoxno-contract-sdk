//! Example: read XOXNO Lending prices inside a contract.
//!
//! The price aggregator gives two reads. `price` is strict: it panics when a
//! price is not usable, which is what a contract that acts on the price
//! wants. `try_price` and `quote` are tolerant: they return the price's status
//! so a contract can decide what to do with a stale or deviating price.

#![no_std]

use soroban_sdk::{contract, contractimpl, contracttype, token, Address, Env};
use xoxno_contract_sdk::lending::price_aggregator::PriceStatus;
use xoxno_contract_sdk::{LendingAddresses, XoxnoLending};

#[contracttype]
enum Key {
    Lending,
}

#[contract]
pub struct PriceReader;

#[contractimpl]
impl PriceReader {
    pub fn __constructor(env: Env, lending: LendingAddresses) {
        env.storage().instance().set(&Key::Lending, &lending);
    }

    /// The USD price of `asset`, WAD. Panics if the price is not usable.
    pub fn price(env: Env, asset: Address) -> i128 {
        lending(&env).price(&asset)
    }

    /// The USD price of `asset`, WAD, or `None` if it is not usable now.
    pub fn safe_price(env: Env, asset: Address) -> Option<i128> {
        lending(&env).try_price(&asset)
    }

    /// The full status of the price of `asset`.
    pub fn quote(env: Env, asset: Address) -> PriceStatus {
        lending(&env).quote(&asset)
    }

    /// The USD value of `amount` base units of `asset`, WAD.
    pub fn value_usd(env: Env, asset: Address, amount: i128) -> i128 {
        lending(&env).value_usd(&asset, amount)
    }

    /// How many base units of `asset` the account can still borrow: its USD
    /// borrow headroom converted at the current price, rounded down.
    pub fn max_borrow(env: Env, account_id: u64, asset: Address) -> i128 {
        let lending = lending(&env);
        let headroom_usd = lending.borrowable_usd(account_id);
        let price = lending.price(&asset);
        let unit = 10i128.pow(token::Client::new(&env, &asset).decimals());
        headroom_usd
            .checked_mul(unit)
            .map(|scaled| scaled / price)
            .unwrap_or(i128::MAX)
    }
}

fn lending(env: &Env) -> XoxnoLending {
    XoxnoLending::new(env, &env.storage().instance().get(&Key::Lending).unwrap())
}
