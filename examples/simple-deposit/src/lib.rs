//! Example: an owner-controlled XOXNO Lending account with caller-chosen markets.
//!
//! The caller chooses everything: the token, the hub, the spoke and the
//! account. `account_id` 0 opens a new account and returns its id (the
//! position NFT's token id); pass that id back to add to the same account.
//! Contributions belong to the configured owner; this contract tracks no
//! depositor balances or shares. Only the owner can withdraw. Tokens must
//! transfer exact amounts; fee-on-transfer tokens are unsupported. Receipt
//! checks reject shortfalls when funding this contract or paying its owner.
//!
//! Pass `LendingAddresses::mainnet(&env)` on mainnet. Tests inject their own
//! fixture addresses through the same constructor.

#![no_std]

use soroban_sdk::{contract, contractimpl, symbol_short, token, Address, Env, Symbol};
use xoxno_contract_sdk::lending::constants::NEW_ACCOUNT;
use xoxno_contract_sdk::{LendingAddresses, Withdrawal, XoxnoLending};

const LENDING: Symbol = symbol_short!("lending");
const OWNER: Symbol = symbol_short!("owner");

#[contract]
pub struct SimpleDeposit;

#[contractimpl]
impl SimpleDeposit {
    pub fn __constructor(env: Env, owner: Address, lending: LendingAddresses) {
        env.storage().instance().set(&LENDING, &lending);
        env.storage().instance().set(&OWNER, &owner);
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
        assert!(amount > 0, "amount must be positive");
        let lending = XoxnoLending::new(&env, &env.storage().instance().get(&LENDING).unwrap());
        assert!(
            account_id == NEW_ACCOUNT || lending.owns(account_id),
            "foreign account"
        );
        let token_client = token::Client::new(&env, &token);
        let this = env.current_contract_address();
        let before = token_client.balance(&this);
        token_client.transfer(&from, &this, &amount);
        assert_eq!(token_client.balance(&this) - before, amount, "receipt tax");

        let market = lending.market(hub_id, &token);
        lending.deposit(account_id, spoke_id, &market, amount)
    }

    /// Withdraws the market's entire supply to the owner. A closed account's
    /// NFT is burned; use account id 0 to open another account on the next deposit.
    pub fn withdraw_all(env: Env, account_id: u64, token: Address, hub_id: u32) -> Withdrawal {
        let owner: Address = env.storage().instance().get(&OWNER).unwrap();
        owner.require_auth();
        let lending = XoxnoLending::new(&env, &env.storage().instance().get(&LENDING).unwrap());
        let token_client = token::Client::new(&env, &token);
        let this = env.current_contract_address();
        let before = token_client.balance(&this);
        let withdrawal = lending.withdraw_all(account_id, &lending.market(hub_id, &token));
        assert_eq!(
            token_client.balance(&this) - before,
            withdrawal.amount,
            "receipt tax"
        );
        let owner_before = token_client.balance(&owner);
        token_client.transfer(&this, &owner, &withdrawal.amount);
        assert_eq!(
            token_client.balance(&owner) - owner_before,
            withdrawal.amount,
            "receipt tax"
        );
        withdrawal
    }
}
