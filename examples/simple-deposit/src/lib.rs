//! Example: the smallest contract function that deposits into XOXNO Lending.
//!
//! The caller chooses everything: the token, the hub, the spoke and the
//! account. `account_id` 0 opens a new account and returns its id (the
//! position NFT's token id); pass that id back to add to the same account.
//!
//! On mainnet, build the wrapper with `XoxnoLending::mainnet(&env)` and drop
//! the constructor. This example stores the addresses only so that its tests
//! can point it at the test fixture.

#![no_std]

use soroban_sdk::{contract, contractimpl, symbol_short, token, Address, Env, Symbol};
use xoxno_contract_sdk::{LendingAddresses, XoxnoLending};

const LENDING: Symbol = symbol_short!("lending");

#[contract]
pub struct SimpleDeposit;

#[contractimpl]
impl SimpleDeposit {
    pub fn __constructor(env: Env, lending: LendingAddresses) {
        env.storage().instance().set(&LENDING, &lending);
    }

    /// Moves `amount` of `token` from `from` into XOXNO Lending, in market
    /// (`hub_id`, `token`), and returns the account id.
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

        let lending = XoxnoLending::new(&env, &env.storage().instance().get(&LENDING).unwrap());
        let market = lending.market(hub_id, &token);
        lending.deposit(account_id, spoke_id, &market, amount)
    }
}
