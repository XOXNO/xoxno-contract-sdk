//! Example: a contract that keeps one XOXNO Lending account.
//!
//! - The first deposit opens the account: the controller mints a position NFT
//!   to this contract, and the NFT's token id is the account id.
//! - Later deposits reuse the stored account id.
//! - When the account is closed (a full withdrawal with no debt deletes it and
//!   burns the NFT), the next deposit opens a new one.

#![no_std]

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, panic_with_error, token, Address, Env,
};
use xoxno_contract_sdk::lending::constants::NEW_ACCOUNT;
use xoxno_contract_sdk::lending::controller::HubAssetKey;
use xoxno_contract_sdk::{LendingAddresses, Position, XoxnoLending};

#[contracttype]
#[derive(Clone)]
pub struct Config {
    pub owner: Address,
    pub lending: LendingAddresses,
    pub spoke_id: u32,
    pub market: HubAssetKey,
}

#[contracttype]
enum Key {
    Config,
    Account,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum AccountError {
    NoAccount = 1,
}

#[contract]
pub struct AccountBasics;

#[contractimpl]
impl AccountBasics {
    /// `spoke_id` is the risk profile the account opens in, and `market` is
    /// the token (and hub) this contract supplies.
    pub fn __constructor(
        env: Env,
        owner: Address,
        lending: LendingAddresses,
        spoke_id: u32,
        market: HubAssetKey,
    ) {
        env.storage().instance().set(
            &Key::Config,
            &Config {
                owner,
                lending,
                spoke_id,
                market,
            },
        );
    }

    /// Moves `amount` from `from` into the account and returns the account id.
    pub fn deposit(env: Env, from: Address, amount: i128) -> u64 {
        from.require_auth();
        let cfg = config(&env);
        token::Client::new(&env, &cfg.market.asset).transfer(
            &from,
            env.current_contract_address(),
            &amount,
        );

        let lending = XoxnoLending::new(&env, &cfg.lending);
        let stored = env.storage().instance().get(&Key::Account);
        let account_id = match lending.resolve_account(stored) {
            NEW_ACCOUNT => lending.open_account(cfg.spoke_id, &cfg.market, amount),
            account_id => lending.supply(account_id, &cfg.market, amount),
        };
        env.storage().instance().set(&Key::Account, &account_id);
        account_id
    }

    /// Borrows `amount` of `market` against the account and sends it to the
    /// owner.
    pub fn borrow(env: Env, market: HubAssetKey, amount: i128) {
        let cfg = config(&env);
        cfg.owner.require_auth();
        let lending = XoxnoLending::new(&env, &cfg.lending);
        lending.borrow(account(&env), &market, amount);
        token::Client::new(&env, &market.asset).transfer(
            &env.current_contract_address(),
            &cfg.owner,
            &amount,
        );
    }

    /// Repays `amount` of the debt in `market`, paid by the owner.
    pub fn repay(env: Env, market: HubAssetKey, amount: i128) {
        let cfg = config(&env);
        cfg.owner.require_auth();
        let this = env.current_contract_address();
        token::Client::new(&env, &market.asset).transfer(&cfg.owner, &this, &amount);
        XoxnoLending::new(&env, &cfg.lending).repay(account(&env), &market, amount);
    }

    /// Withdraws the whole supply to the owner and returns the amount.
    pub fn withdraw_all(env: Env) -> i128 {
        let cfg = config(&env);
        cfg.owner.require_auth();
        let lending = XoxnoLending::new(&env, &cfg.lending);
        let withdrawn = lending.withdraw_all(account(&env), &cfg.market);
        token::Client::new(&env, &cfg.market.asset).transfer(
            &env.current_contract_address(),
            &cfg.owner,
            &withdrawn,
        );
        withdrawn
    }

    /// The stored account id, if the contract has opened one.
    pub fn account_id(env: Env) -> Option<u64> {
        env.storage().instance().get(&Key::Account)
    }

    /// Every supplied and borrowed amount of the account.
    pub fn position(env: Env) -> Position {
        XoxnoLending::new(&env, &config(&env).lending).position(account(&env))
    }

    /// The account's health factor, WAD.
    pub fn health_factor(env: Env) -> i128 {
        XoxnoLending::new(&env, &config(&env).lending).health_factor(account(&env))
    }
}

fn config(env: &Env) -> Config {
    env.storage().instance().get(&Key::Config).unwrap()
}

fn account(env: &Env) -> u64 {
    env.storage()
        .instance()
        .get(&Key::Account)
        .unwrap_or_else(|| panic_with_error!(env, AccountError::NoAccount))
}
