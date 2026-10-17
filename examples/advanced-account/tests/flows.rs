use advanced_account::{
    AdvancedAccount, AdvancedAccountClient, BlendPositions, BlendRequest, FlashData, FlashPosition,
    Multiply, RepayCollateral, Swap,
};
use soroban_sdk::testutils::{Address as _, Ledger as _, MockAuth, MockAuthInvoke};
use soroban_sdk::{
    contract, contractimpl, token, vec,
    xdr::{FromXdr, ToXdr},
    Address, Bytes, BytesN, Env, IntoVal, Map, Symbol, Val, Vec,
};
use xoxno_contract_sdk::lending::constants::WAD;
use xoxno_contract_sdk::lending::controller::PositionMode;
use xoxno_contract_sdk::testutils::{
    governance::AdminOperation, LendingFixture, Market, MarketConfig,
};

const U: i128 = 10_000_000;

#[contract]
struct Router;
#[contractimpl]
impl Router {
    pub fn execute_strategy(env: Env, sender: Address, total_in: i128, swap_xdr: Bytes) -> i128 {
        sender.require_auth();
        let (input, output, amount): (Address, Address, i128) =
            FromXdr::from_xdr(&env, &swap_xdr).unwrap();
        assert!(total_in > 0 && amount > 0);
        token::Client::new(&env, &input).transfer(
            &sender,
            env.current_contract_address(),
            &total_in,
        );
        token::Client::new(&env, &output).transfer(
            &env.current_contract_address(),
            &sender,
            &amount,
        );
        amount
    }
}

struct Setup<'a> {
    env: Env,
    f: LendingFixture<'a>,
    a: Market<'a>,
    b: Market<'a>,
    owner: Address,
    c: AdvancedAccountClient<'a>,
    router: Address,
}
fn setup<'a>() -> Setup<'a> {
    let env = Env::default();
    let f = LendingFixture::deploy(&env, &Address::generate(&env));
    let a = f.create_market(&MarketConfig::usdc());
    let mut cfg = MarketConfig::usdc();
    cfg.symbol = "OTHER";
    let b = f.create_market(&cfg);
    let owner = Address::generate(&env);
    let router = env.register(Router, ());
    admin(
        &f,
        AdminOperation::SetSwapAggregator(router.clone()),
        "set_swap_aggregator",
        vec![&env, router.clone().into_val(&env)],
    );
    let address = env.register(
        AdvancedAccount,
        (owner.clone(), f.addresses(), router.clone(), owner.clone()),
    );
    for who in [&owner, &router] {
        a.sac.mock_all_auths().mint(who, &(10_000 * U));
        b.sac.mock_all_auths().mint(who, &(10_000 * U));
    }
    Setup {
        c: AdvancedAccountClient::new(&env, &address),
        env,
        f,
        a,
        b,
        owner,
        router,
    }
}
fn admin(f: &LendingFixture, op: AdminOperation, function: &str, args: Vec<Val>) {
    let env = &f.env;
    let salt = BytesN::from_array(env, &[99; 32]);
    let id = f.governance.mock_all_auths().propose(&f.admin, &op, &salt);
    let ledger = f.governance.get_operation_ledger(&id);
    env.ledger().with_mut(|l| l.sequence_number = ledger);
    f.governance.execute(
        &None,
        &f.controller.address,
        &Symbol::new(env, function),
        &args,
        &BytesN::from_array(env, &[0; 32]),
        &salt,
    );
}
fn route(s: &Setup, a: &Market, b: &Market, output: i128) -> Bytes {
    (a.asset.clone(), b.asset.clone(), output).to_xdr(&s.env)
}
fn open(s: &Setup) -> u64 {
    s.c.mock_all_auths()
        .deposit_batch(&0, &s.f.spoke_id, &vec![&s.env, (s.a.key.clone(), 100 * U)])
}
fn idle(s: &Setup) {
    for m in [&s.a, &s.b] {
        assert_eq!(m.token.balance(&s.c.address), 0);
    }
}

#[test]
fn strict_owner_auth_duplicate_batches_refunds_and_close() {
    let s = setup();
    let env = &s.env;
    let assets = vec![
        env,
        (s.a.key.clone(), 40 * U),
        (s.b.key.clone(), 20 * U),
        (s.a.key.clone(), 60 * U),
    ];
    assert!(s.c.try_deposit_batch(&0, &s.f.spoke_id, &assets).is_err());
    let transfers = [(&s.a, 40 * U), (&s.b, 20 * U), (&s.a, 60 * U)];
    let children: std::vec::Vec<_> = transfers
        .iter()
        .map(|(m, n)| MockAuthInvoke {
            contract: &m.asset,
            fn_name: "transfer",
            args: (s.owner.clone(), s.c.address.clone(), *n).into_val(env),
            sub_invokes: &[],
        })
        .collect();
    let auth = MockAuth {
        address: &s.owner,
        invoke: &MockAuthInvoke {
            contract: &s.c.address,
            fn_name: "deposit_batch",
            args: (0u64, s.f.spoke_id, assets.clone()).into_val(env),
            sub_invokes: &children,
        },
    };
    let account =
        s.c.mock_auths(&[auth])
            .deposit_batch(&0, &s.f.spoke_id, &assets);
    let p = s.c.position(&account);
    assert_eq!(p.collateral.len(), 2);
    assert_eq!(s.a.token.balance(&s.owner), 9_900 * U);
    assert_eq!(s.b.token.balance(&s.owner), 9_980 * U);
    idle(&s);
    assert_eq!(
        s.c.mock_all_auths().supply_batch(
            &account,
            &vec![env, (s.a.key.clone(), U), (s.a.key.clone(), U)]
        ),
        account
    );
    let borrows = vec![env, (s.a.key.clone(), U), (s.b.key.clone(), 2 * U)];
    s.c.mock_all_auths().borrow_batch(&account, &borrows);
    assert_eq!(s.c.position(&account).debt.len(), 2);
    let before = s.a.token.balance(&s.owner);
    let paid = s.c.mock_all_auths().repay_batch(
        &account,
        &vec![env, (s.a.key.clone(), 2 * U), (s.b.key.clone(), 3 * U)],
    );
    assert_eq!(paid.len(), 2);
    assert_eq!(before - s.a.token.balance(&s.owner), U);
    assert!(s.c.position(&account).debt.is_empty());
    let result = s.c.mock_all_auths().withdraw_batch(
        &account,
        &vec![
            env,
            (s.a.key.clone(), U),
            (s.a.key.clone(), 0),
            (s.b.key.clone(), 0),
        ],
    );
    assert!(result.account_closed);
    assert_eq!(result.amounts.len(), 2);
    assert!(!s.f.controller.account_exists(&account));
    assert_eq!(s.f.position_nft.balance(&s.c.address), 0);
    idle(&s);
}

#[test]
fn failed_batch_is_atomic_and_liquidation_returns_measured_spend() {
    let s = setup();
    let env = &s.env;
    let account = open(&s);
    let before = s.c.position(&account);
    let wallet = s.b.token.balance(&s.owner);
    assert!(s
        .c
        .mock_all_auths()
        .try_supply_batch(
            &account,
            &vec![env, (s.b.key.clone(), U), (s.a.key.clone(), -1)]
        )
        .is_err());
    assert_eq!(s.c.position(&account), before);
    assert_eq!(s.b.token.balance(&s.owner), wallet);
    idle(&s);
    s.c.mock_all_auths()
        .borrow_batch(&account, &vec![env, (s.b.key.clone(), 70 * U)]);
    s.f.set_price(&s.a, WAD * 8 / 10);
    let debt_before = s.f.controller.get_borrow_amount(&account, &s.b.key);
    let wallet_before = s.b.token.balance(&s.owner);
    let collateral_before = s.a.token.balance(&s.owner);
    let liq = env.register(
        AdvancedAccount,
        (
            s.owner.clone(),
            s.f.addresses(),
            s.router.clone(),
            s.owner.clone(),
        ),
    );
    let liq = AdvancedAccountClient::new(env, &liq);
    let spent = liq
        .mock_all_auths()
        .liquidate_batch(&account, &vec![env, (s.b.key.clone(), 100 * U)]);
    assert!(!spent.is_empty());
    let n = spent.get(0).unwrap().1;
    assert!(n > 0 && n < 100 * U);
    assert_eq!(wallet_before - s.b.token.balance(&s.owner), n);
    assert!(s.a.token.balance(&s.owner) > collateral_before);
    assert!(s.f.controller.get_borrow_amount(&account, &s.b.key) < debt_before);
    idle(&s);
}

#[test]
fn routed_strategies_and_flash_callback_use_actual_backing() {
    let s = setup();
    let env = &s.env;
    let request = Multiply {
        account_id: 0,
        spoke_id: s.f.spoke_id,
        collateral: s.a.key.clone(),
        debt: s.b.key.clone(),
        amount: 10 * U,
        mode: PositionMode::Multiply,
        swap: route(&s, &s.b, &s.a, 10 * U),
        initial: vec![env, (s.a.key.clone(), 100 * U)],
        convert: None,
    };
    let account = s.c.mock_all_auths().multiply(&request);
    assert!(s.c.position(&account).collateral.get(0).unwrap().1 >= 110 * U - 2);
    s.c.mock_all_auths().swap_debt(&Swap {
        account_id: account,
        current: s.b.key.clone(),
        next: s.a.key.clone(),
        amount: 2 * U,
        swap: route(&s, &s.a, &s.b, 2 * U),
    });
    s.c.mock_all_auths().swap_collateral(&Swap {
        account_id: account,
        current: s.a.key.clone(),
        next: s.b.key.clone(),
        amount: U,
        swap: route(&s, &s.a, &s.b, U),
    });
    let before = s.f.controller.get_borrow_amount(&account, &s.b.key);
    s.c.mock_all_auths().repay_collateral(&RepayCollateral {
        account_id: account,
        collateral: s.a.key.clone(),
        debt: s.b.key.clone(),
        amount: U,
        swap: route(&s, &s.a, &s.b, U),
        close: false,
    });
    assert!(s.f.controller.get_borrow_amount(&account, &s.b.key) < before);
    idle(&s);
    let receiver = env.register(
        AdvancedAccount,
        (
            s.owner.clone(),
            s.f.addresses(),
            s.router.clone(),
            s.c.address.clone(),
        ),
    );
    let client = AdvancedAccountClient::new(env, &receiver);
    let data = FlashData {
        collateral: s.a.asset.clone(),
        swap: route(&s, &s.b, &s.a, 10 * U),
    }
    .to_xdr(env);
    assert!(client
        .try_execute_flash_position(
            &s.c.address,
            &0,
            &s.b.asset,
            &(10 * U),
            &0,
            &(10 * U),
            &s.f.controller.address,
            &data
        )
        .is_err());
    s.a.sac.mock_all_auths().mint(&receiver, &(100 * U));
    let request = FlashPosition {
        account_id: 0,
        spoke_id: s.f.spoke_id,
        mode: PositionMode::Multiply,
        debt: s.b.key.clone(),
        amount: 10 * U,
        receiver: receiver.clone(),
        data,
        collaterals: vec![env, (s.a.key.clone(), 1)],
        refunds: vec![env, s.b.asset.clone()],
    };
    let mut bad = request.clone();
    bad.collaterals = vec![env, (s.a.key.clone(), 1_000 * U)];
    let count = s.f.position_nft.balance(&s.c.address);
    assert!(s.c.mock_all_auths().try_flash_position(&bad).is_err());
    assert_eq!(s.f.position_nft.balance(&s.c.address), count);
    assert_eq!(s.a.token.balance(&receiver), 100 * U);
    assert_eq!(s.b.token.balance(&receiver), 0);
    let flash = s.c.mock_all_auths().flash_position(&request);
    assert!(s.c.position(&flash).collateral.get(0).unwrap().1 >= 110 * U - 2);
    assert_eq!(s.a.token.balance(&receiver), 0);
    assert_eq!(s.b.token.balance(&receiver), 0);
    idle(&s);
}

#[contract]
struct AggregateBlend;
#[contractimpl]
impl AggregateBlend {
    pub fn submit(
        env: Env,
        from: Address,
        spender: Address,
        _to: Address,
        requests: Vec<BlendRequest>,
    ) -> BlendPositions {
        from.require_auth();
        let mut total: Map<Address, i128> = Map::new(&env);
        for r in requests {
            total.set(
                r.address.clone(),
                total.get(r.address).unwrap_or(0) + r.amount,
            );
        }
        for (asset, amount) in total {
            token::Client::new(&env, &asset).transfer(
                &spender,
                env.current_contract_address(),
                &amount,
            );
        }
        BlendPositions {
            liabilities: Map::new(&env),
            collateral: Map::new(&env),
            supply: Map::new(&env),
        }
    }
}
#[test]
fn blend_funding_authorizes_aggregate_token_transfer() {
    let s = setup();
    let env = &s.env;
    let blend = env.register(AggregateBlend, ());
    let requests = vec![
        env,
        BlendRequest {
            request_type: 0,
            address: s.a.asset.clone(),
            amount: U,
        },
        BlendRequest {
            request_type: 2,
            address: s.a.asset.clone(),
            amount: 2 * U,
        },
    ];
    let before = s.a.token.balance(&s.owner);
    s.c.mock_all_auths().blend_submit(&blend, &requests);
    assert_eq!(before - s.a.token.balance(&s.owner), 3 * U);
    assert_eq!(s.a.token.balance(&blend), 3 * U);
    idle(&s);
}

#[test]
fn multiply_accepts_debt_funding_converted_funding_and_same_token_across_hubs() {
    let s = setup();
    let env = &s.env;
    let account = s.c.mock_all_auths().multiply(&Multiply {
        account_id: 0,
        spoke_id: s.f.spoke_id,
        collateral: s.a.key.clone(),
        debt: s.b.key.clone(),
        amount: U,
        mode: PositionMode::Multiply,
        swap: route(&s, &s.b, &s.a, U),
        initial: vec![env, (s.a.key.clone(), 100 * U)],
        convert: None,
    });
    let before = s.b.token.balance(&s.owner);
    assert_eq!(
        s.c.mock_all_auths().multiply(&Multiply {
            account_id: account,
            spoke_id: s.f.spoke_id,
            collateral: s.a.key.clone(),
            debt: s.b.key.clone(),
            amount: U,
            mode: PositionMode::Multiply,
            swap: route(&s, &s.b, &s.a, 3 * U),
            initial: vec![env, (s.b.key.clone(), 2 * U)],
            convert: None,
        }),
        account
    );
    assert_eq!(before - s.b.token.balance(&s.owner), 2 * U);
    let mut cfg = MarketConfig::usdc();
    cfg.symbol = "THIRD";
    let c = s.f.create_market(&cfg);
    c.sac.mock_all_auths().mint(&s.owner, &(10 * U));
    let old = s.f.controller.get_collateral_amount(&account, &s.a.key);
    s.c.mock_all_auths().multiply(&Multiply {
        account_id: account,
        spoke_id: s.f.spoke_id,
        collateral: s.a.key.clone(),
        debt: s.b.key.clone(),
        amount: U,
        mode: PositionMode::Multiply,
        swap: route(&s, &s.b, &s.a, U),
        initial: vec![env, (c.key.clone(), 2 * U)],
        convert: Some(route(&s, &c, &s.a, 2 * U)),
    });
    assert!(s.f.controller.get_collateral_amount(&account, &s.a.key) >= old + 3 * U - 2);
    assert_eq!(c.token.balance(&s.owner), 8 * U);
    assert_eq!(c.token.balance(&s.c.address), 0);
    let hub = s.f.add_hub();
    let second = s.f.add_market_to_hub(&s.b, hub, &cfg.rates);
    s.f.list_market(&second, s.f.spoke_id, &cfg.risk);
    s.f.supply_liquidity(&second, s.f.spoke_id, 1000 * U);
    let cross = s.c.mock_all_auths().multiply(&Multiply {
        account_id: 0,
        spoke_id: s.f.spoke_id,
        collateral: s.b.key.clone(),
        debt: second.clone(),
        amount: U,
        mode: PositionMode::Multiply,
        swap: Bytes::new(env),
        initial: vec![env, (s.b.key.clone(), 100 * U)],
        convert: None,
    });
    assert_eq!(s.c.position(&cross).debt.get(0).unwrap().0, second);
    assert!(s.c.position(&cross).collateral.get(0).unwrap().1 > 100 * U);
    idle(&s);
}

#[test]
fn routed_collateral_repayment_closes_and_refunds_remaining_principal() {
    let s = setup();
    let env = &s.env;
    let account = open(&s);
    s.c.mock_all_auths()
        .borrow_batch(&account, &vec![env, (s.b.key.clone(), 20 * U)]);
    let before = s.a.token.balance(&s.owner);
    let debt_wallet = s.b.token.balance(&s.owner);
    s.c.mock_all_auths().repay_collateral(&RepayCollateral {
        account_id: account,
        collateral: s.a.key.clone(),
        debt: s.b.key.clone(),
        amount: 20 * U,
        swap: route(&s, &s.a, &s.b, 20 * U),
        close: true,
    });
    assert!(!s.f.controller.account_exists(&account));
    assert_eq!(s.f.position_nft.balance(&s.c.address), 0);
    assert_eq!(s.a.token.balance(&s.owner) - before, 80 * U);
    assert_eq!(s.b.token.balance(&s.owner), debt_wallet);
    idle(&s);
}
