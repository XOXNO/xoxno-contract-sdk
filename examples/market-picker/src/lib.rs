//! Example: choose where to supply a token.
//!
//! A token can be listed in several hubs, and each hub market in several
//! spokes (risk profiles). This contract finds the first hub/spoke pair that
//! accepts the token as new collateral, opens an account there, and reads the
//! market's rates.
//! Contributions belong to the configured owner, who alone can withdraw;
//! there are no depositor shares. Tokens must transfer exact amounts.

#![no_std]

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, panic_with_error, token, Address, Env, Vec,
};
use xoxno_contract_sdk::lending::controller::HubAssetKey;
use xoxno_contract_sdk::{LendingAddresses, Withdrawal, XoxnoLending};

/// Where to supply: a spoke and a market key (the token in one hub).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Placement {
    pub spoke_id: u32,
    pub market: HubAssetKey,
}

/// A market's rates, RAY, and its lendable cash, token base units.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarketInfo {
    pub supply_rate: i128,
    pub borrow_rate: i128,
    pub utilization: i128,
    pub liquidity: i128,
}

#[contracttype]
enum Key {
    Lending,
    Owner,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum PickerError {
    NoPlacement = 1,
}

#[contract]
pub struct MarketPicker;

#[contractimpl]
impl MarketPicker {
    pub fn __constructor(env: Env, owner: Address, lending: LendingAddresses) {
        env.storage().instance().set(&Key::Lending, &lending);
        env.storage().instance().set(&Key::Owner, &owner);
    }

    /// The first hub in `hub_ids` that lists `asset`, paired with the first
    /// spoke in `spoke_ids` that accepts a new supply of it.
    pub fn pick(
        env: Env,
        asset: Address,
        hub_ids: Vec<u32>,
        spoke_ids: Vec<u32>,
    ) -> Option<Placement> {
        let lending = lending(&env);
        for hub_id in hub_ids.iter() {
            let market = lending.market(hub_id, &asset);
            if !lending.is_listed(&market) {
                continue;
            }
            for spoke_id in spoke_ids.iter() {
                if lending.can_supply(spoke_id, &market) {
                    return Some(Placement { spoke_id, market });
                }
            }
        }
        None
    }

    /// Opens an account for this contract at the placement `pick` finds, with a
    /// first supply of `amount` from `from`, and returns the account id.
    pub fn supply_best(
        env: Env,
        from: Address,
        asset: Address,
        amount: i128,
        hub_ids: Vec<u32>,
        spoke_ids: Vec<u32>,
    ) -> u64 {
        from.require_auth();
        assert!(amount > 0, "amount must be positive");
        let placement = Self::pick(env.clone(), asset.clone(), hub_ids, spoke_ids)
            .unwrap_or_else(|| panic_with_error!(&env, PickerError::NoPlacement));
        let token = token::Client::new(&env, &asset);
        let this = env.current_contract_address();
        let before = token.balance(&this);
        token.transfer(&from, &this, &amount);
        assert_eq!(token.balance(&this) - before, amount, "receipt tax");
        lending(&env).open_account(placement.spoke_id, &placement.market, amount)
    }

    /// Withdraws the entire market supply to the configured owner. Closing
    /// burns the account NFT; `supply_best` can open a new account afterwards.
    pub fn withdraw_all(env: Env, account_id: u64, market: HubAssetKey) -> Withdrawal {
        let owner: Address = env.storage().instance().get(&Key::Owner).unwrap();
        owner.require_auth();
        let token = token::Client::new(&env, &market.asset);
        let this = env.current_contract_address();
        let before = token.balance(&this);
        let withdrawal = lending(&env).withdraw_all(account_id, &market);
        assert_eq!(
            token.balance(&this) - before,
            withdrawal.amount,
            "receipt tax"
        );
        let owner_before = token.balance(&owner);
        token.transfer(&this, &owner, &withdrawal.amount);
        assert_eq!(
            token.balance(&owner) - owner_before,
            withdrawal.amount,
            "receipt tax"
        );
        withdrawal
    }

    /// The market's supply and borrow rates, utilization and cash.
    pub fn market_info(env: Env, market: HubAssetKey) -> MarketInfo {
        let lending = lending(&env);
        MarketInfo {
            supply_rate: lending.supply_rate(&market),
            borrow_rate: lending.borrow_rate(&market),
            utilization: lending.utilization(&market),
            liquidity: lending.liquidity(&market),
        }
    }
}

fn lending(env: &Env) -> XoxnoLending {
    XoxnoLending::new(env, &env.storage().instance().get(&Key::Lending).unwrap())
}
