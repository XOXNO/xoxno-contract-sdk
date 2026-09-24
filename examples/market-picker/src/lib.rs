//! Example: choose where to supply a token.
//!
//! A token can be listed in several hubs, and each hub market in several
//! spokes (risk profiles). This contract finds the first hub that lists the
//! token and the first spoke that accepts it as new collateral, opens an
//! account there, and reads the market's rates.

#![no_std]

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, panic_with_error, token, Address, Env, Vec,
};
use xoxno_contract_sdk::lending::controller::HubAssetKey;
use xoxno_contract_sdk::{LendingAddresses, XoxnoLending};

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
    pub fn __constructor(env: Env, lending: LendingAddresses) {
        env.storage().instance().set(&Key::Lending, &lending);
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
        let placement = Self::pick(env.clone(), asset.clone(), hub_ids, spoke_ids)
            .unwrap_or_else(|| panic_with_error!(&env, PickerError::NoPlacement));
        token::Client::new(&env, &asset).transfer(&from, env.current_contract_address(), &amount);
        lending(&env).open_account(placement.spoke_id, &placement.market, amount)
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
