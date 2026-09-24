//! Example integration: a vault that keeps every deposit in one XOXNO Lending
//! account and accepts flash loans that its owner starts.

#![no_std]

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, panic_with_error, token, vec, Address,
    Bytes, Env,
};
use xoxno_contract_sdk::lending::constants::{NEW_ACCOUNT, WITHDRAW_ALL};
use xoxno_contract_sdk::lending::controller::HubAssetKey;
use xoxno_contract_sdk::lending::helpers::{
    approve_flash_repayment, authorize_transfer_as_current,
};
use xoxno_contract_sdk::lending::{ControllerClient, FlashLoanReceiver};

const INSTANCE_TTL_THRESHOLD: u32 = 518_400;
const INSTANCE_TTL_EXTEND_TO: u32 = 3_110_400;

#[contracttype]
#[derive(Clone)]
pub struct Config {
    pub owner: Address,
    pub controller: Address,
    pub pool: Address,
    pub market: HubAssetKey,
    pub spoke_id: u32,
}

#[contracttype]
enum Key {
    Config,
    Account,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum VaultError {
    UnexpectedFlashLoan = 1,
}

#[contract]
pub struct LendingVault;

#[contractimpl]
impl LendingVault {
    pub fn __constructor(
        env: Env,
        owner: Address,
        controller: Address,
        market: HubAssetKey,
        spoke_id: u32,
    ) {
        let pool = ControllerClient::new(&env, &controller).get_pool_address();
        env.storage().instance().set(
            &Key::Config,
            &Config {
                owner,
                controller,
                pool,
                market,
                spoke_id,
            },
        );
    }

    /// Moves `amount` of the market token from `from` into the vault's
    /// lending account and returns the account id.
    pub fn deposit(env: Env, from: Address, amount: i128) -> u64 {
        from.require_auth();
        let cfg = config(&env);
        let this = env.current_contract_address();
        token::Client::new(&env, &cfg.market.asset).transfer(&from, &this, &amount);

        authorize_transfer_as_current(&env, &cfg.market.asset, &this, &cfg.pool, amount);
        let account_id = ControllerClient::new(&env, &cfg.controller).supply(
            &this,
            &account(&env),
            &cfg.spoke_id,
            &vec![&env, (cfg.market.clone(), amount)],
        );
        env.storage().instance().set(&Key::Account, &account_id);
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_TTL_THRESHOLD, INSTANCE_TTL_EXTEND_TO);
        account_id
    }

    /// Withdraws the whole position, interest included, to the owner.
    pub fn withdraw_all(env: Env) -> i128 {
        let cfg = config(&env);
        cfg.owner.require_auth();
        let withdrawn = ControllerClient::new(&env, &cfg.controller).withdraw(
            &env.current_contract_address(),
            &account(&env),
            &vec![&env, (cfg.market.clone(), WITHDRAW_ALL)],
            &Some(cfg.owner.clone()),
        );
        env.storage().instance().remove(&Key::Account);
        withdrawn.iter().map(|(_, amount)| amount).sum()
    }

    /// Value of the vault's position in the market token, interest included.
    pub fn balance(env: Env) -> i128 {
        let cfg = config(&env);
        match env.storage().instance().get::<_, u64>(&Key::Account) {
            Some(account_id) => ControllerClient::new(&env, &cfg.controller)
                .get_collateral_amount(&account_id, &cfg.market),
            None => 0,
        }
    }
}

#[contractimpl]
impl FlashLoanReceiver for LendingVault {
    fn execute_flash_loan(
        env: Env,
        initiator: Address,
        asset: Address,
        amount: i128,
        fee: i128,
        pool: Address,
        _data: Bytes,
    ) {
        let cfg = config(&env);
        cfg.pool.require_auth();
        if initiator != cfg.owner || pool != cfg.pool {
            panic_with_error!(&env, VaultError::UnexpectedFlashLoan);
        }
        approve_flash_repayment(&env, &asset, &pool, amount + fee);
    }
}

fn config(env: &Env) -> Config {
    env.storage().instance().get(&Key::Config).unwrap()
}

fn account(env: &Env) -> u64 {
    env.storage()
        .instance()
        .get(&Key::Account)
        .unwrap_or(NEW_ACCOUNT)
}
