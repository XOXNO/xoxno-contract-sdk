use price_reader::{PriceReader, PriceReaderClient};
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
    reader: PriceReaderClient<'a>,
}

fn setup<'a>() -> Setup<'a> {
    let env = Env::default();
    env.mock_all_auths();
    let fixture = LendingFixture::deploy(&env, &Address::generate(&env));
    let usdc = fixture.create_market(&MarketConfig::usdc());
    let xlm = fixture.create_market(&MarketConfig::xlm());
    let reader = env.register(PriceReader, (fixture.addresses(),));
    Setup {
        reader: PriceReaderClient::new(&env, &reader),
        env,
        fixture,
        usdc,
        xlm,
    }
}

#[test]
fn reads_the_current_price() {
    let s = setup();
    assert_eq!(s.reader.price(&s.usdc.asset), WAD);
    assert_eq!(s.reader.price(&s.xlm.asset), WAD / 10);

    s.fixture.set_price(&s.xlm, WAD * 12 / 100);

    assert_eq!(s.reader.price(&s.xlm.asset), WAD * 12 / 100);
    assert!(s.reader.quote(&s.xlm.asset).valid);
}

#[test]
fn a_token_without_a_price_has_no_safe_price() {
    let s = setup();
    let unpriced = s
        .env
        .register_stellar_asset_contract_v2(Address::generate(&s.env))
        .address();

    assert_eq!(s.reader.safe_price(&unpriced), None);
    assert!(s.reader.try_price(&unpriced).is_err());
    assert_eq!(s.reader.safe_price(&s.usdc.asset), Some(WAD));
}

#[test]
fn converts_amounts_to_usd() {
    let s = setup();
    assert_eq!(s.reader.value_usd(&s.usdc.asset, &(100 * UNIT)), 100 * WAD);
    assert_eq!(s.reader.value_usd(&s.xlm.asset, &(1_000 * UNIT)), 100 * WAD);
}

#[test]
fn max_borrow_converts_the_usd_headroom_to_tokens() {
    let s = setup();
    let user = Address::generate(&s.env);
    s.usdc.sac.mint(&user, &(10_000 * UNIT));
    let account = s.fixture.controller.supply(
        &user,
        &NEW_ACCOUNT,
        &s.fixture.spoke_id,
        &vec![&s.env, (s.usdc.key.clone(), 10_000 * UNIT)],
    );

    let max = s.reader.max_borrow(&account, &s.xlm.asset);

    assert_eq!(max, 76_000 * UNIT);
    s.fixture.controller.borrow(
        &user,
        &account,
        &vec![&s.env, (s.xlm.key.clone(), max)],
        &None,
    );
    assert_eq!(s.reader.max_borrow(&account, &s.xlm.asset), 0);
}
