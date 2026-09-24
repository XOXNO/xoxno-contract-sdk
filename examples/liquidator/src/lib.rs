//! Example: a contract that liquidates XOXNO Lending accounts.
//!
//! The contract holds the debt tokens it repays with and receives the seized
//! collateral. `XoxnoLending::liquidate` reads the liquidation plan first, so
//! it authorizes exactly the amount the controller pulls, which can be less
//! than the amount offered.

#![no_std]

use soroban_sdk::{contract, contractimpl, contracttype, token, Address, Env};
use xoxno_contract_sdk::lending::controller::{HubAssetKey, LiquidationEstimate};
use xoxno_contract_sdk::{LendingAddresses, XoxnoLending};

#[contracttype]
#[derive(Clone)]
pub struct Config {
    pub owner: Address,
    pub lending: LendingAddresses,
}

#[contracttype]
enum Key {
    Config,
}

#[contract]
pub struct Liquidator;

#[contractimpl]
impl Liquidator {
    pub fn __constructor(env: Env, owner: Address, lending: LendingAddresses) {
        env.storage()
            .instance()
            .set(&Key::Config, &Config { owner, lending });
    }

    /// What repaying up to `amount` of the account's debt in `market` would
    /// seize, refund and cost in fees.
    pub fn estimate(
        env: Env,
        account_id: u64,
        market: HubAssetKey,
        amount: i128,
    ) -> LiquidationEstimate {
        lending(&env).liquidation_estimate(account_id, &market, amount)
    }

    /// Liquidates the account if it is liquidatable, repaying up to `amount`
    /// of its debt in `market`. Returns the amount paid; 0 if the account is
    /// healthy.
    pub fn liquidate(env: Env, account_id: u64, market: HubAssetKey, amount: i128) -> i128 {
        config(&env).owner.require_auth();
        let lending = lending(&env);
        if !lending.is_liquidatable(account_id) {
            return 0;
        }
        lending.liquidate(account_id, &market, amount)
    }

    /// Sends `amount` of `asset` held by this contract to the owner.
    pub fn sweep(env: Env, asset: Address, amount: i128) {
        let cfg = config(&env);
        cfg.owner.require_auth();
        token::Client::new(&env, &asset).transfer(
            &env.current_contract_address(),
            &cfg.owner,
            &amount,
        );
    }
}

fn config(env: &Env) -> Config {
    env.storage().instance().get(&Key::Config).unwrap()
}

fn lending(env: &Env) -> XoxnoLending {
    XoxnoLending::new(env, &config(env).lending)
}
