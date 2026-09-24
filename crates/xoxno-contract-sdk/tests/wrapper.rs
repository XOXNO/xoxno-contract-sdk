use soroban_sdk::testutils::Address as _;
use soroban_sdk::{contract, contractimpl, vec, Address, Bytes, Env};
use xoxno_contract_sdk::lending::constants::{RAY, WAD};
use xoxno_contract_sdk::lending::helpers::approve_flash_repayment;
use xoxno_contract_sdk::lending::FlashLoanReceiver;
use xoxno_contract_sdk::networks;
use xoxno_contract_sdk::testutils::{LendingFixture, Market, MarketConfig};
use xoxno_contract_sdk::{LendingAddresses, XoxnoLending};

const UNIT: i128 = 10_000_000;

#[contract]
struct Integrator;

#[contractimpl]
impl Integrator {}

#[contract]
struct Receiver;

#[contractimpl]
impl FlashLoanReceiver for Receiver {
    fn execute_flash_loan(
        env: Env,
        _initiator: Address,
        asset: Address,
        amount: i128,
        fee: i128,
        pool: Address,
        _data: Bytes,
    ) {
        approve_flash_repayment(&env, &asset, &pool, amount + fee);
    }
}

struct Setup<'a> {
    env: Env,
    fixture: LendingFixture<'a>,
    usdc: Market<'a>,
    xlm: Market<'a>,
    integrator: Address,
}

/// No global auth mock: every authorization the wrapper creates is checked.
fn setup<'a>() -> Setup<'a> {
    let env = Env::default();
    let fixture = LendingFixture::deploy(&env, &Address::generate(&env));
    let usdc = fixture.create_market(&MarketConfig::usdc());
    let xlm = fixture.create_market(&MarketConfig::xlm());
    let integrator = env.register(Integrator, ());
    usdc.sac
        .mock_all_auths()
        .mint(&integrator, &(10_000 * UNIT));
    Setup {
        env,
        fixture,
        usdc,
        xlm,
        integrator,
    }
}

/// Runs `f` as the integrator contract, with a wrapper around the fixture.
fn as_integrator<T>(s: &Setup, f: impl FnOnce(&XoxnoLending) -> T) -> T {
    let lending = XoxnoLending::new(&s.env, &s.fixture.addresses());
    s.env.as_contract(&s.integrator, || f(&lending))
}

#[test]
fn network_constructors_use_the_published_addresses() {
    let env = Env::default();
    let mainnet = LendingAddresses::mainnet(&env);
    assert_eq!(
        mainnet.controller,
        Address::from_str(&env, networks::mainnet::CONTROLLER)
    );
    assert_eq!(
        mainnet.pool,
        Address::from_str(&env, networks::mainnet::POOL)
    );
    assert_eq!(
        XoxnoLending::testnet(&env).addresses().position_nft,
        Address::from_str(&env, networks::testnet::POSITION_NFT)
    );
}

#[test]
fn account_views_follow_the_nft() {
    let s = setup();
    let account = as_integrator(&s, |l| {
        l.open_account(s.fixture.spoke_id, &s.usdc.key, 1_000 * UNIT)
    });

    as_integrator(&s, |l| {
        assert!(l.owns(account));
        assert!(!l.owns(account + 1));
        assert_eq!(l.owner_of(account), s.integrator);
        assert_eq!(l.account_count(&s.integrator), 1);
        assert_eq!(l.accounts_of(&s.integrator, 0, 10).get(0), Some(account));
        assert_eq!(l.accounts_of(&s.integrator, 1, 10).len(), 0);
        assert_eq!(l.account_spoke(account), s.fixture.spoke_id);
        assert_eq!(l.resolve_account(Some(account)), account);
        l.renew_account(account);
    });
}

#[test]
fn partial_withdraw_and_debt_views() {
    let s = setup();
    let account = as_integrator(&s, |l| {
        l.open_account(s.fixture.spoke_id, &s.usdc.key, 10_000 * UNIT)
    });

    as_integrator(&s, |l| {
        l.borrow(account, &s.xlm.key, 10_000 * UNIT);
        assert_eq!(l.debt(account, &s.xlm.key), 10_000 * UNIT);
        assert_eq!(l.debt_usd(account), 1_000 * WAD);
        assert_eq!(l.collateral_usd(account), 10_000 * WAD);
        let withdrawal = l.withdraw(account, &s.usdc.key, 1_000 * UNIT);
        assert_eq!(withdrawal.amount, 1_000 * UNIT);
        assert!(!withdrawal.account_closed);
        assert_eq!(l.collateral(account, &s.usdc.key), 9_000 * UNIT);
    });
    assert_eq!(s.usdc.token.balance(&s.integrator), 1_000 * UNIT);
}

#[test]
fn market_views_find_hubs_and_spokes() {
    let s = setup();
    let hub_2 = s.fixture.add_hub();
    let eurc = s
        .fixture
        .create_market_in(&MarketConfig::usdc(), hub_2, s.fixture.spoke_id);

    as_integrator(&s, |l| {
        assert_eq!(
            l.find_market(&eurc.asset, &[s.fixture.hub_id, hub_2]),
            Some(eurc.key.clone())
        );
        assert_eq!(l.find_market(&eurc.asset, &[s.fixture.hub_id]), None);
        assert!(l
            .spoke(s.fixture.spoke_id)
            .is_some_and(|spoke| !spoke.is_deprecated));
        assert!(l.spoke(99).is_none());
        assert!(l.can_borrow(s.fixture.spoke_id, &s.xlm.key));
        assert_eq!(l.supply_index(&s.usdc.key), RAY);
    });
}

#[test]
fn flash_loan_goes_to_another_contract() {
    let s = setup();
    let receiver = s.env.register(Receiver, ());
    s.usdc.sac.mock_all_auths().mint(&receiver, &(100 * UNIT));

    as_integrator(&s, |l| {
        l.flash_loan(&s.usdc.key, 10_000 * UNIT, &receiver, &Bytes::new(&s.env));
    });

    assert_eq!(s.usdc.token.balance(&receiver), 100 * UNIT - 9 * UNIT);
}

#[test]
fn repay_authorizes_the_transfer_it_pays() {
    let s = setup();
    let account = as_integrator(&s, |l| {
        l.open_account(s.fixture.spoke_id, &s.usdc.key, 10_000 * UNIT)
    });

    as_integrator(&s, |l| {
        l.borrow(account, &s.xlm.key, 10_000 * UNIT);
        assert_eq!(l.repay(account, &s.xlm.key, 4_000 * UNIT), 4_000 * UNIT);
        assert_eq!(l.debt(account, &s.xlm.key), 6_000 * UNIT);
    });
    assert_eq!(s.xlm.token.balance(&s.integrator), 6_000 * UNIT);
}

#[test]
fn repay_above_the_debt_returns_the_amount_repaid_and_keeps_the_refund() {
    let s = setup();
    let account = as_integrator(&s, |l| {
        l.open_account(s.fixture.spoke_id, &s.usdc.key, 10_000 * UNIT)
    });
    s.xlm
        .sac
        .mock_all_auths()
        .mint(&s.integrator, &(5_000 * UNIT));

    as_integrator(&s, |l| {
        l.borrow(account, &s.xlm.key, 10_000 * UNIT);
        let debt = l.debt(account, &s.xlm.key);
        assert_eq!(l.repay(account, &s.xlm.key, 15_000 * UNIT), debt);
        assert_eq!(l.debt(account, &s.xlm.key), 0);
    });
    assert_eq!(s.xlm.token.balance(&s.integrator), 5_000 * UNIT);
}

#[test]
fn liquidate_authorizes_exactly_what_the_plan_pulls() {
    let s = setup();
    let borrower = Address::generate(&s.env);
    s.usdc
        .sac
        .mock_all_auths()
        .mint(&borrower, &(10_000 * UNIT));
    let victim = s.fixture.controller.mock_all_auths().supply(
        &borrower,
        &0,
        &s.fixture.spoke_id,
        &vec![&s.env, (s.usdc.key.clone(), 10_000 * UNIT)],
    );
    s.fixture.controller.mock_all_auths().borrow(
        &borrower,
        &victim,
        &vec![&s.env, (s.xlm.key.clone(), 70_000 * UNIT)],
        &None,
    );
    s.fixture.set_price(&s.xlm, WAD * 12 / 100);
    s.xlm
        .sac
        .mock_all_auths()
        .mint(&s.integrator, &(200_000 * UNIT));

    let paid = as_integrator(&s, |l| l.liquidate(victim, &s.xlm.key, 200_000 * UNIT));

    assert!(paid > 0 && paid < 200_000 * UNIT);
    assert_eq!(s.xlm.token.balance(&s.integrator), 200_000 * UNIT - paid);
    assert!(s.usdc.token.balance(&s.integrator) > 0);
}

#[test]
fn withdraw_all_reports_the_closed_account_and_its_burned_nft() {
    let s = setup();
    let account = as_integrator(&s, |l| {
        l.open_account(s.fixture.spoke_id, &s.usdc.key, 1_000 * UNIT)
    });

    let withdrawal = as_integrator(&s, |l| l.withdraw_all(account, &s.usdc.key));

    assert!(1_000 * UNIT - withdrawal.amount <= 1);
    assert!(withdrawal.account_closed);
    assert!(!s.fixture.controller.account_exists(&account));
    assert!(s
        .fixture
        .position_nft
        .try_owner_of(&(account as u32))
        .is_err());
}

#[test]
fn withdraw_all_with_another_supply_keeps_the_account() {
    let s = setup();
    let account = as_integrator(&s, |l| {
        l.open_account(s.fixture.spoke_id, &s.usdc.key, 1_000 * UNIT)
    });
    s.xlm
        .sac
        .mock_all_auths()
        .mint(&s.integrator, &(1_000 * UNIT));

    let withdrawal = as_integrator(&s, |l| {
        l.deposit(account, s.fixture.spoke_id, &s.xlm.key, 1_000 * UNIT);
        l.withdraw_all(account, &s.usdc.key)
    });

    assert!(!withdrawal.account_closed);
    assert!(s.fixture.controller.account_exists(&account));
    assert_eq!(
        s.fixture.position_nft.owner_of(&(account as u32)),
        s.integrator
    );
}

#[test]
fn liquidate_with_an_offer_above_the_whole_debt_pays_only_the_debt() {
    let s = setup();
    let borrower = Address::generate(&s.env);
    s.usdc
        .sac
        .mock_all_auths()
        .mint(&borrower, &(10_000 * UNIT));
    let victim = s.fixture.controller.mock_all_auths().supply(
        &borrower,
        &0,
        &s.fixture.spoke_id,
        &vec![&s.env, (s.usdc.key.clone(), 10_000 * UNIT)],
    );
    s.fixture.controller.mock_all_auths().borrow(
        &borrower,
        &victim,
        &vec![&s.env, (s.xlm.key.clone(), 70_000 * UNIT)],
        &None,
    );
    s.fixture.set_price(&s.xlm, WAD * 14 / 100);
    s.xlm
        .sac
        .mock_all_auths()
        .mint(&s.integrator, &(100_000 * UNIT));

    let paid = as_integrator(&s, |l| l.liquidate(victim, &s.xlm.key, 100_000 * UNIT));

    assert!((70_000 * UNIT..100_000 * UNIT).contains(&paid));
    assert_eq!(s.xlm.token.balance(&s.integrator), 100_000 * UNIT - paid);
    assert_eq!(
        s.fixture.controller.get_borrow_amount(&victim, &s.xlm.key),
        0
    );
    assert!(s.usdc.token.balance(&s.integrator) > 0);
}
