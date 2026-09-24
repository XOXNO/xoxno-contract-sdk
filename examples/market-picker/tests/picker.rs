use market_picker::{MarketPicker, MarketPickerClient, Placement};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{vec, Address, Env};
use xoxno_contract_sdk::lending::constants::NEW_ACCOUNT;
use xoxno_contract_sdk::testutils::{LendingFixture, MarketConfig};

const UNIT: i128 = 10_000_000;

fn setup<'a>(env: &Env) -> (LendingFixture<'a>, MarketPickerClient<'a>) {
    env.mock_all_auths();
    let fixture = LendingFixture::deploy(env, &Address::generate(env));
    let picker = env.register(MarketPicker, (fixture.addresses(),));
    (fixture, MarketPickerClient::new(env, &picker))
}

#[test]
fn picks_the_hub_that_lists_the_token() {
    let env = Env::default();
    let (fixture, picker) = setup(&env);
    let hub_2 = fixture.add_hub();
    let spoke_2 = fixture.add_spoke();
    let eurc = fixture.create_market_in(&MarketConfig::usdc(), hub_2, spoke_2);

    let placement = picker.pick(
        &eurc.asset,
        &vec![&env, fixture.hub_id, hub_2],
        &vec![&env, fixture.spoke_id, spoke_2],
    );

    assert_eq!(
        placement,
        Some(Placement {
            spoke_id: spoke_2,
            market: eurc.key.clone(),
        })
    );
}

#[test]
fn skips_a_spoke_that_does_not_accept_the_token_as_collateral() {
    let env = Env::default();
    let (fixture, picker) = setup(&env);
    let spoke_2 = fixture.add_spoke();
    let mut cfg = MarketConfig::usdc();
    cfg.risk.can_be_collateral = false;
    cfg.initial_liquidity = 0;
    let usdc = fixture.create_market(&cfg);
    let mut collateral = MarketConfig::usdc().risk;
    collateral.can_be_collateral = true;
    fixture.list_market(&usdc.key, spoke_2, &collateral);

    let placement = picker.pick(
        &usdc.asset,
        &vec![&env, fixture.hub_id],
        &vec![&env, fixture.spoke_id, spoke_2],
    );

    assert_eq!(placement.map(|p| p.spoke_id), Some(spoke_2));
}

#[test]
fn finds_nothing_for_an_unlisted_token() {
    let env = Env::default();
    let (fixture, picker) = setup(&env);
    let unlisted = env
        .register_stellar_asset_contract_v2(Address::generate(&env))
        .address();

    let placement = picker.pick(
        &unlisted,
        &vec![&env, fixture.hub_id],
        &vec![&env, fixture.spoke_id],
    );

    assert_eq!(placement, None);
}

#[test]
fn supply_best_opens_the_account_in_the_picked_spoke() {
    let env = Env::default();
    let (fixture, picker) = setup(&env);
    let spoke_2 = fixture.add_spoke();
    let usdc = fixture.create_market_in(&MarketConfig::usdc(), fixture.hub_id, spoke_2);
    let from = Address::generate(&env);
    usdc.sac.mint(&from, &(1_000 * UNIT));

    let account_id = picker.supply_best(
        &from,
        &usdc.asset,
        &(1_000 * UNIT),
        &vec![&env, fixture.hub_id],
        &vec![&env, fixture.spoke_id, spoke_2],
    );

    assert_eq!(
        fixture
            .controller
            .get_account_attributes(&account_id)
            .spoke_id,
        spoke_2
    );
    assert_eq!(
        fixture.position_nft.owner_of(&(account_id as u32)),
        picker.address
    );
}

#[test]
fn market_info_shows_utilization_after_a_borrow() {
    let env = Env::default();
    let (fixture, picker) = setup(&env);
    let usdc = fixture.create_market(&MarketConfig::usdc());
    let xlm = fixture.create_market(&MarketConfig::xlm());
    assert_eq!(picker.market_info(&xlm.key).utilization, 0);

    let user = Address::generate(&env);
    usdc.sac.mint(&user, &(10_000 * UNIT));
    let account = fixture.controller.supply(
        &user,
        &NEW_ACCOUNT,
        &fixture.spoke_id,
        &vec![&env, (usdc.key.clone(), 10_000 * UNIT)],
    );
    fixture.controller.borrow(
        &user,
        &account,
        &vec![&env, (xlm.key.clone(), 50_000 * UNIT)],
        &None,
    );

    let info = picker.market_info(&xlm.key);
    assert!(info.utilization > 0);
    assert!(info.borrow_rate > info.supply_rate);
    assert_eq!(info.liquidity, 10_000_000 * UNIT - 50_000 * UNIT);
}
