//! Owner-controlled batch and strategy integration. Every contribution belongs
//! to the owner; this contract issues no shares. Exact-transfer tokens only.
//! Deploy a separate instance as the flash-position receiver, with the driver's
//! address as `initiator`. Idle receiver collateral becomes position equity.
#![no_std]
#![allow(clippy::too_many_arguments)] // SDK callback ABI.

use soroban_sdk::auth::{ContractContext, InvokerContractAuthEntry, SubContractInvocation};
use soroban_sdk::{
    contract, contractclient, contractimpl, contracttype, token, vec, xdr::FromXdr, Address, Bytes,
    Env, Vec,
};
use soroban_sdk::{IntoVal, Map, Symbol};
use xoxno_contract_sdk::lending::controller::{HubAssetKey, PositionMode};
use xoxno_contract_sdk::lending::helpers::authorize_transfer_as_current;
use xoxno_contract_sdk::lending::FlashPositionReceiver;
use xoxno_contract_sdk::{LendingAddresses, Position, Withdrawals, XoxnoLending};

#[contracttype]
#[derive(Clone)]
pub struct Config {
    pub owner: Address,
    pub lending: LendingAddresses,
    pub router: Address,
    pub initiator: Address,
}

#[contracttype]
#[derive(Clone)]
pub struct Multiply {
    pub account_id: u64,
    pub spoke_id: u32,
    pub collateral: HubAssetKey,
    pub debt: HubAssetKey,
    pub amount: i128,
    pub mode: PositionMode,
    pub swap: Bytes,
    pub initial: Vec<(HubAssetKey, i128)>,
    pub convert: Option<Bytes>,
}

#[contracttype]
#[derive(Clone)]
pub struct Swap {
    pub account_id: u64,
    pub current: HubAssetKey,
    pub next: HubAssetKey,
    pub amount: i128,
    pub swap: Bytes,
}

#[contracttype]
#[derive(Clone)]
pub struct RepayCollateral {
    pub account_id: u64,
    pub collateral: HubAssetKey,
    pub debt: HubAssetKey,
    pub amount: i128,
    pub swap: Bytes,
    pub close: bool,
}

#[contracttype]
#[derive(Clone)]
pub struct FlashPosition {
    pub account_id: u64,
    pub spoke_id: u32,
    pub mode: PositionMode,
    pub debt: HubAssetKey,
    pub amount: i128,
    pub receiver: Address,
    pub data: Bytes,
    pub collaterals: Vec<(HubAssetKey, i128)>,
    pub refunds: Vec<Address>,
}

#[contracttype]
#[derive(Clone)]
pub struct FlashData {
    pub collateral: Address,
    pub swap: Bytes,
}

#[contracttype]
#[derive(Clone)]
pub struct BlendRequest {
    pub request_type: u32,
    pub address: Address,
    pub amount: i128,
}

#[contracttype]
#[derive(Clone)]
pub struct BlendPositions {
    pub liabilities: Map<u32, i128>,
    pub collateral: Map<u32, i128>,
    pub supply: Map<u32, i128>,
}

#[contracttype]
#[derive(Clone)]
pub struct BlendMigration {
    pub account_id: u64,
    pub spoke_id: u32,
    pub hub_id: u32,
    pub pool: Address,
    pub collateral: Vec<Address>,
    pub supply: Vec<Address>,
    pub debts: Vec<(Address, i128)>,
}

#[contractclient(name = "BlendClient")]
pub trait BlendPool {
    fn submit(
        env: Env,
        from: Address,
        spender: Address,
        to: Address,
        requests: Vec<BlendRequest>,
    ) -> BlendPositions;
    fn get_positions(env: Env, address: Address) -> BlendPositions;
}

#[contractclient(name = "RouterClient")]
pub trait Router {
    fn execute_strategy(env: Env, sender: Address, total_in: i128, swap_xdr: Bytes) -> i128;
}

#[contract]
pub struct AdvancedAccount;

#[contractimpl]
impl AdvancedAccount {
    pub fn __constructor(
        env: Env,
        owner: Address,
        lending: LendingAddresses,
        router: Address,
        initiator: Address,
    ) {
        env.storage().instance().set(
            &0u32,
            &Config {
                owner,
                lending,
                router,
                initiator,
            },
        );
    }

    pub fn deposit_batch(
        env: Env,
        account_id: u64,
        spoke_id: u32,
        assets: Vec<(HubAssetKey, i128)>,
    ) -> u64 {
        let cfg = authorized(&env);
        let lending = XoxnoLending::new(&env, &cfg.lending);
        own_or_new(&lending, account_id);
        fund(&env, &cfg, &assets);
        lending.deposit_batch(account_id, spoke_id, &assets)
    }

    pub fn supply_batch(env: Env, account_id: u64, assets: Vec<(HubAssetKey, i128)>) -> u64 {
        let cfg = authorized(&env);
        let lending = XoxnoLending::new(&env, &cfg.lending);
        assert!(lending.owns(account_id), "foreign account");
        fund(&env, &cfg, &assets);
        lending.supply_batch(account_id, &assets)
    }

    pub fn borrow_batch(env: Env, account_id: u64, assets: Vec<(HubAssetKey, i128)>) {
        let cfg = authorized(&env);
        let lending = XoxnoLending::new(&env, &cfg.lending);
        assert!(lending.owns(account_id), "foreign account");
        lending.borrow_batch(account_id, &assets);
        pay_assets(&env, &cfg, &assets);
    }

    pub fn repay_batch(
        env: Env,
        account_id: u64,
        assets: Vec<(HubAssetKey, i128)>,
    ) -> Vec<(HubAssetKey, i128)> {
        let cfg = authorized(&env);
        let lending = XoxnoLending::new(&env, &cfg.lending);
        assert!(lending.owns(account_id), "foreign account");
        fund(&env, &cfg, &assets);
        let result = lending.repay_batch(account_id, &assets);
        pay_assets(&env, &cfg, &assets);
        result
    }

    pub fn withdraw_batch(
        env: Env,
        account_id: u64,
        assets: Vec<(HubAssetKey, i128)>,
    ) -> Withdrawals {
        let cfg = authorized(&env);
        let lending = XoxnoLending::new(&env, &cfg.lending);
        assert!(lending.owns(account_id), "foreign account");
        let result = lending.withdraw_batch(account_id, &assets);
        pay_assets(&env, &cfg, &assets);
        result
    }

    pub fn liquidate_batch(
        env: Env,
        victim: u64,
        assets: Vec<(HubAssetKey, i128)>,
    ) -> Vec<(HubAssetKey, i128)> {
        let cfg = authorized(&env);
        let lending = XoxnoLending::new(&env, &cfg.lending);
        let collateral = lending.position(victim).collateral;
        fund(&env, &cfg, &assets);
        let result = lending.liquidate_batch(victim, &assets);
        pay_assets(&env, &cfg, &assets);
        pay_assets(&env, &cfg, &collateral);
        result
    }

    pub fn multiply(env: Env, request: Multiply) -> u64 {
        let cfg = authorized(&env);
        let lending = XoxnoLending::new(&env, &cfg.lending);
        own_or_new(&lending, request.account_id);
        assert!(request.initial.len() <= 1, "at most one initial payment");
        if let Some((market, amount)) = request.initial.get(0) {
            fund(&env, &cfg, &vec![&env, (market.clone(), amount)]);
            authorize_transfer_as_current(
                &env,
                &market.asset,
                &env.current_contract_address(),
                &cfg.lending.controller,
                amount,
            );
        }
        let account = lending.controller().multiply(
            &env.current_contract_address(),
            &request.account_id,
            &request.spoke_id,
            &request.collateral,
            &request.amount,
            &request.debt,
            &request.mode,
            &request.swap,
            &request.initial.get(0),
            &request.convert,
        );
        pay(&env, &cfg, &request.collateral.asset);
        pay(&env, &cfg, &request.debt.asset);
        pay_assets(&env, &cfg, &request.initial);
        account
    }

    pub fn swap_debt(env: Env, request: Swap) {
        let cfg = authorized(&env);
        let lending = XoxnoLending::new(&env, &cfg.lending);
        assert!(lending.owns(request.account_id), "foreign account");
        lending.controller().swap_debt(
            &env.current_contract_address(),
            &request.account_id,
            &request.current,
            &request.amount,
            &request.next,
            &request.swap,
        );
        pay(&env, &cfg, &request.current.asset);
        pay(&env, &cfg, &request.next.asset);
    }

    pub fn swap_collateral(env: Env, request: Swap) {
        let cfg = authorized(&env);
        let lending = XoxnoLending::new(&env, &cfg.lending);
        assert!(lending.owns(request.account_id), "foreign account");
        lending.controller().swap_collateral(
            &env.current_contract_address(),
            &request.account_id,
            &request.current,
            &request.amount,
            &request.next,
            &request.swap,
        );
        pay(&env, &cfg, &request.current.asset);
        pay(&env, &cfg, &request.next.asset);
    }

    pub fn repay_collateral(env: Env, request: RepayCollateral) {
        let cfg = authorized(&env);
        let lending = XoxnoLending::new(&env, &cfg.lending);
        assert!(lending.owns(request.account_id), "foreign account");
        lending.controller().repay_debt_with_collateral(
            &env.current_contract_address(),
            &request.account_id,
            &request.collateral,
            &request.amount,
            &request.debt,
            &request.swap,
            &request.close,
        );
        pay(&env, &cfg, &request.collateral.asset);
        pay(&env, &cfg, &request.debt.asset);
    }

    pub fn flash_position(env: Env, request: FlashPosition) -> u64 {
        let cfg = authorized(&env);
        let lending = XoxnoLending::new(&env, &cfg.lending);
        own_or_new(&lending, request.account_id);
        assert!(
            request.receiver != env.current_contract_address(),
            "separate receiver required"
        );
        let account = lending.controller().flash_position(
            &env.current_contract_address(),
            &request.account_id,
            &request.spoke_id,
            &request.mode,
            &request.debt,
            &request.amount,
            &request.receiver,
            &request.data,
            &request.collaterals,
            &request.refunds,
        );
        for asset in request.refunds.iter() {
            pay(&env, &cfg, &asset);
        }
        account
    }

    /// Seeds or closes this contract's own real Blend position.
    pub fn blend_submit(env: Env, pool: Address, requests: Vec<BlendRequest>) -> BlendPositions {
        let cfg = authorized(&env);
        let this = env.current_contract_address();
        let mut totals: Map<Address, i128> = Map::new(&env);
        for request in requests.iter() {
            if matches!(request.request_type, 0 | 2 | 5) {
                receive(&env, &cfg, &request.address, request.amount);
                let total = totals.get(request.address.clone()).unwrap_or(0);
                totals.set(request.address, total.checked_add(request.amount).unwrap());
            }
        }
        let mut transfers = Vec::new(&env);
        for transfer in totals.iter() {
            transfers.push_back(transfer);
        }
        xoxno_contract_sdk::lending::helpers::authorize_transfers_as_current(
            &env, &this, &pool, &transfers,
        );
        let result = BlendClient::new(&env, &pool).submit(&this, &this, &this, &requests);
        for request in requests.iter() {
            pay(&env, &cfg, &request.address);
        }
        result
    }

    pub fn migrate_blend(env: Env, request: BlendMigration) -> u64 {
        let cfg = authorized(&env);
        let lending = XoxnoLending::new(&env, &cfg.lending);
        own_or_new(&lending, request.account_id);
        let mut repay = Vec::new(&env);
        for (asset, amount) in request.debts.iter() {
            assert!(amount > 0, "positive cap required");
            repay.push_back(BlendRequest {
                request_type: 5,
                address: asset,
                amount,
            });
        }
        let mut withdraw = Vec::new(&env);
        // Match the controller's overflow-safe Blend withdraw-all ABI exactly.
        let all = 1_000_000_000_000_000_000_000_000_000_000;
        for asset in request.collateral.iter() {
            withdraw.push_back(BlendRequest {
                request_type: 3,
                address: asset,
                amount: all,
            });
        }
        for asset in request.supply.iter() {
            withdraw.push_back(BlendRequest {
                request_type: 1,
                address: asset,
                amount: all,
            });
        }
        let mut authorizations = Vec::new(&env);
        for requests in [repay, withdraw] {
            if !requests.is_empty() {
                authorizations.push_back(InvokerContractAuthEntry::Contract(
                    SubContractInvocation {
                        context: ContractContext {
                            contract: request.pool.clone(),
                            fn_name: Symbol::new(&env, "submit"),
                            args: (
                                env.current_contract_address(),
                                cfg.lending.controller.clone(),
                                cfg.lending.controller.clone(),
                                requests,
                            )
                                .into_val(&env),
                        },
                        sub_invocations: Vec::new(&env),
                    },
                ));
            }
        }
        env.authorize_as_current_contract(authorizations);
        let account = lending.controller().migrate_from_blend(
            &env.current_contract_address(),
            &request.account_id,
            &request.spoke_id,
            &request.hub_id,
            &request.pool,
            &request.collateral,
            &request.supply,
            &request.debts,
        );
        for (asset, _) in request.debts.iter() {
            pay(&env, &cfg, &asset);
        }
        account
    }

    pub fn position(env: Env, account_id: u64) -> Position {
        XoxnoLending::new(&env, &config(&env).lending).position(account_id)
    }

    pub fn renew_account(env: Env, account_id: u64) {
        let cfg = authorized(&env);
        let lending = XoxnoLending::new(&env, &cfg.lending);
        assert!(lending.owns(account_id), "foreign account");
        lending.renew_account(account_id);
    }

    pub fn sweep(env: Env, asset: Address) {
        let cfg = authorized(&env);
        pay(&env, &cfg, &asset);
    }
}

#[contractimpl]
impl FlashPositionReceiver for AdvancedAccount {
    fn execute_flash_position(
        env: Env,
        initiator: Address,
        _account_id: u64,
        asset: Address,
        _amount: i128,
        fee: i128,
        amount_received: i128,
        controller: Address,
        data: Bytes,
    ) {
        let cfg = config(&env);
        cfg.lending.controller.require_auth();
        assert!(
            controller == cfg.lending.controller && initiator == cfg.initiator && fee == 0,
            "unexpected callback"
        );
        assert!(amount_received > 0, "empty loan");
        let payload = FlashData::from_xdr(&env, &data).unwrap();
        assert!(payload.collateral != asset, "distinct collateral required");
        let input = token::Client::new(&env, &asset);
        let output = token::Client::new(&env, &payload.collateral);
        let this = env.current_contract_address();
        let before = output.balance(&this);
        authorize_transfer_as_current(&env, &asset, &this, &cfg.router, amount_received);
        RouterClient::new(&env, &cfg.router).execute_strategy(
            &this,
            &amount_received,
            &payload.swap,
        );
        assert!(output.balance(&this) > before, "empty route output");
        output.transfer(&this, &controller, &output.balance(&this));
        let refund = input.balance(&this);
        if refund > 0 {
            input.transfer(&this, &controller, &refund);
        }
    }
}

fn config(env: &Env) -> Config {
    env.storage().instance().get(&0u32).unwrap()
}
fn authorized(env: &Env) -> Config {
    let cfg = config(env);
    cfg.owner.require_auth();
    cfg
}
fn own_or_new(lending: &XoxnoLending, account: u64) {
    assert!(account == 0 || lending.owns(account), "foreign account");
}
fn fund(env: &Env, cfg: &Config, assets: &Vec<(HubAssetKey, i128)>) {
    assert!(!assets.is_empty(), "empty payments");
    for (market, amount) in assets.iter() {
        receive(env, cfg, &market.asset, amount);
    }
}
fn receive(env: &Env, cfg: &Config, asset: &Address, amount: i128) {
    assert!(amount > 0, "amount must be positive");
    let token = token::Client::new(env, asset);
    let this = env.current_contract_address();
    let before = token.balance(&this);
    token.transfer(&cfg.owner, &this, &amount);
    assert_eq!(token.balance(&this) - before, amount, "receipt tax");
}
fn pay_assets(env: &Env, cfg: &Config, assets: &Vec<(HubAssetKey, i128)>) {
    for (market, _) in assets.iter() {
        pay(env, cfg, &market.asset);
    }
}
fn pay(env: &Env, cfg: &Config, asset: &Address) {
    let token = token::Client::new(env, asset);
    let amount = token.balance(&env.current_contract_address());
    if amount > 0 {
        let before = token.balance(&cfg.owner);
        token.transfer(&env.current_contract_address(), &cfg.owner, &amount);
        assert_eq!(token.balance(&cfg.owner) - before, amount, "receipt tax");
    }
}
