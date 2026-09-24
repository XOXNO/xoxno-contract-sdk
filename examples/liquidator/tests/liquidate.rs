use liquidator::{Liquidator, LiquidatorClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{vec, Address, Env};
use xoxno_contract_sdk::lending::constants::{NEW_ACCOUNT, WAD};
use xoxno_contract_sdk::testutils::{LendingFixture, Market, MarketConfig};

const UNIT: i128 = 10_000_000;

struct Setup<'a> {
    env: Env,
    fixture: LendingFixture<'a>,
    usdc: Market<'a>,
    xlm: Market<'a>,
    liquidator: LiquidatorClient<'a>,
    account: u64,
}

/// A borrower with 10,000 USDC of collateral and 70,000 XLM of debt.
fn setup<'a>() -> Setup<'a> {
    let env = Env::default();
    env.mock_all_auths();
    let fixture = LendingFixture::deploy(&env, &Address::generate(&env));
    let usdc = fixture.create_market(&MarketConfig::usdc());
    let xlm = fixture.create_market(&MarketConfig::xlm());
    let borrower = Address::generate(&env);
    usdc.sac.mint(&borrower, &(10_000 * UNIT));
    let account = fixture.controller.supply(
        &borrower,
        &NEW_ACCOUNT,
        &fixture.spoke_id,
        &vec![&env, (usdc.key.clone(), 10_000 * UNIT)],
    );
    fixture.controller.borrow(
        &borrower,
        &account,
        &vec![&env, (xlm.key.clone(), 70_000 * UNIT)],
        &None,
    );
    let liquidator = env.register(Liquidator, (Address::generate(&env), fixture.addresses()));
    Setup {
        liquidator: LiquidatorClient::new(&env, &liquidator),
        env,
        fixture,
        usdc,
        xlm,
        account,
    }
}

#[test]
fn a_healthy_account_is_not_liquidated() {
    let s = setup();
    s.xlm.sac.mint(&s.liquidator.address, &(20_000 * UNIT));

    let paid = s
        .liquidator
        .liquidate(&s.account, &s.xlm.key, &(20_000 * UNIT));

    assert_eq!(paid, 0);
    assert_eq!(s.xlm.token.balance(&s.liquidator.address), 20_000 * UNIT);
}

#[test]
fn liquidation_repays_debt_and_seizes_collateral() {
    let s = setup();
    s.fixture.set_price(&s.xlm, WAD * 12 / 100);
    s.xlm.sac.mint(&s.liquidator.address, &(20_000 * UNIT));
    let estimate = s
        .liquidator
        .estimate(&s.account, &s.xlm.key, &(20_000 * UNIT));
    assert!(!estimate.seized_collaterals.is_empty());

    let paid = s
        .liquidator
        .liquidate(&s.account, &s.xlm.key, &(20_000 * UNIT));

    assert!(paid > 0 && paid <= 20_000 * UNIT);
    assert_eq!(
        s.xlm.token.balance(&s.liquidator.address),
        20_000 * UNIT - paid
    );
    assert!(s.usdc.token.balance(&s.liquidator.address) > 0);
    assert!(
        s.fixture
            .controller
            .get_borrow_amount(&s.account, &s.xlm.key)
            < 70_000 * UNIT
    );
}

#[test]
fn an_offer_above_what_the_plan_uses_pays_only_the_planned_amount() {
    let s = setup();
    s.fixture.set_price(&s.xlm, WAD * 12 / 100);
    s.xlm.sac.mint(&s.liquidator.address, &(200_000 * UNIT));

    let paid = s
        .liquidator
        .liquidate(&s.account, &s.xlm.key, &(200_000 * UNIT));

    assert!(paid > 0 && paid < 200_000 * UNIT);
    assert_eq!(
        s.xlm.token.balance(&s.liquidator.address),
        200_000 * UNIT - paid
    );
    let _ = &s.env;
}
