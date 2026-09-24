use soroban_sdk::testutils::Address as _;
use soroban_sdk::{vec, Address, Env, Error};
use xoxno_contract_sdk::lending::constants::{NEW_ACCOUNT, WAD};
use xoxno_contract_sdk::lending::controller::{CollateralError, SeizeMode};
use xoxno_contract_sdk::testutils::{LendingFixture, Market, MarketConfig};

const UNIT: i128 = 10_000_000;
const DAY: u64 = 86_400;

struct Setup<'a> {
    env: Env,
    fixture: LendingFixture<'a>,
    usdc: Market<'a>,
    xlm: Market<'a>,
}

fn setup<'a>() -> Setup<'a> {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let fixture = LendingFixture::deploy(&env, &admin);
    let usdc = fixture.create_market(&MarketConfig::usdc());
    let xlm = fixture.create_market(&MarketConfig::xlm());
    Setup {
        env,
        fixture,
        usdc,
        xlm,
    }
}

/// Supplies `usdc` from a new user into a new account and returns both.
fn open_account(s: &Setup, usdc: i128) -> (Address, u64) {
    let user = Address::generate(&s.env);
    s.usdc.sac.mint(&user, &usdc);
    let account_id = s.fixture.controller.supply(
        &user,
        &NEW_ACCOUNT,
        &s.fixture.spoke_id,
        &vec![&s.env, (s.usdc.key.clone(), usdc)],
    );
    (user, account_id)
}

#[test]
fn deploys_and_creates_markets() {
    let s = setup();

    assert_eq!(s.fixture.hub_id, 1);
    assert_eq!(s.fixture.spoke_id, 1);
    assert_eq!(
        s.fixture.controller.get_pool_address(),
        s.fixture.pool.address
    );
    assert_eq!(
        s.fixture.controller.price_aggregator(),
        s.fixture.price_aggregator.address
    );
    assert_eq!(
        s.usdc.token.balance(&s.fixture.pool.address),
        1_000_000 * UNIT
    );
    assert_eq!(
        s.xlm.token.balance(&s.fixture.pool.address),
        10_000_000 * UNIT
    );
}

#[test]
fn supply_opens_an_account_owned_through_the_position_nft() {
    let s = setup();
    let (user, account_id) = open_account(&s, 10_000 * UNIT);

    assert_eq!(s.fixture.position_nft.owner_of(&(account_id as u32)), user);
    assert_eq!(
        s.fixture
            .controller
            .get_collateral_amount(&account_id, &s.usdc.key),
        10_000 * UNIT
    );
    s.fixture.controller.renew_account(&user, &account_id);
}

#[test]
fn borrowed_debt_grows_as_time_passes() {
    let s = setup();
    let (user, account_id) = open_account(&s, 10_000 * UNIT);
    let borrowed = 50_000 * UNIT;
    s.fixture.controller.borrow(
        &user,
        &account_id,
        &vec![&s.env, (s.xlm.key.clone(), borrowed)],
        &None,
    );
    assert_eq!(s.xlm.token.balance(&user), borrowed);

    s.fixture.advance_time(30 * DAY);

    let debt = s
        .fixture
        .controller
        .get_borrow_amount(&account_id, &s.xlm.key);
    assert!(debt > borrowed, "debt {debt} did not grow past {borrowed}");
}

#[test]
fn a_borrow_above_ltv_fails_with_a_typed_error() {
    let s = setup();
    let (user, account_id) = open_account(&s, 10_000 * UNIT);

    let result = s.fixture.controller.try_borrow(
        &user,
        &account_id,
        &vec![&s.env, (s.xlm.key.clone(), 80_000 * UNIT)],
        &None,
    );

    assert_eq!(
        result.err().and_then(Result::ok),
        Some(Error::from_contract_error(
            CollateralError::InsufficientCollateral as u32
        ))
    );
}

#[test]
fn a_price_rise_of_the_debt_makes_the_account_liquidatable() {
    let s = setup();
    let (user, account_id) = open_account(&s, 10_000 * UNIT);
    s.fixture.controller.borrow(
        &user,
        &account_id,
        &vec![&s.env, (s.xlm.key.clone(), 70_000 * UNIT)],
        &None,
    );
    assert!(!s.fixture.controller.is_liquidatable(&account_id));

    s.fixture.set_price(&s.xlm, WAD * 12 / 100);
    assert!(s.fixture.controller.is_liquidatable(&account_id));

    let liquidator = Address::generate(&s.env);
    s.xlm.sac.mint(&liquidator, &(20_000 * UNIT));
    s.fixture.controller.liquidate(
        &liquidator,
        &account_id,
        &vec![&s.env, (s.xlm.key.clone(), 20_000 * UNIT)],
        &SeizeMode::Transfer,
    );

    assert!(s.usdc.token.balance(&liquidator) > 0);
    assert!(
        s.fixture
            .controller
            .get_borrow_amount(&account_id, &s.xlm.key)
            < 70_000 * UNIT
    );
}
