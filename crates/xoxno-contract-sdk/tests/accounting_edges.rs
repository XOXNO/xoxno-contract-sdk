use soroban_sdk::testutils::Address as _;
use soroban_sdk::{contract, contractimpl, vec, Address, Env, Vec};
use xoxno_contract_sdk::lending::constants::WAD;
use xoxno_contract_sdk::lending::controller::{HubAssetKey, SeizeMode};
use xoxno_contract_sdk::testutils::{LendingFixture, Market, MarketConfig};
use xoxno_contract_sdk::{LendingAddresses, XoxnoLending};

const UNIT: i128 = 10_000_000;

#[contract]
struct Liquidator;

#[contractimpl]
impl Liquidator {
    pub fn liquidate(
        env: Env,
        addresses: LendingAddresses,
        account: u64,
        offers: Vec<(HubAssetKey, i128)>,
    ) -> Vec<(HubAssetKey, i128)> {
        XoxnoLending::new(&env, &addresses).liquidate_batch(account, &offers)
    }
}

fn victim(fixture: &LendingFixture, collateral: &Market, debts: &Vec<(HubAssetKey, i128)>) -> u64 {
    let env = &fixture.env;
    let owner = Address::generate(env);
    collateral
        .sac
        .mock_all_auths()
        .mint(&owner, &(10_000 * UNIT));
    let account = fixture.controller.mock_all_auths().supply(
        &owner,
        &0,
        &fixture.spoke_id,
        &vec![env, (collateral.key.clone(), 10_000 * UNIT)],
    );
    fixture
        .controller
        .mock_all_auths()
        .borrow(&owner, &account, debts, &None);
    account
}

#[test]
fn multidebt_original_offers_match_direct_protocol_for_partial_full_and_insolvent_plans() {
    for price_cents in [13, 14, 18, 30] {
        let env = Env::default();
        let fixture = LendingFixture::deploy(&env, &Address::generate(&env));
        let usdc = fixture.create_market(&MarketConfig::usdc());
        let xlm = fixture.create_market(&MarketConfig::xlm());
        let mut eurc_config = MarketConfig::usdc();
        eurc_config.symbol = "EURC";
        let eurc = fixture.create_market(&eurc_config);
        let debts = vec![
            &env,
            (xlm.key.clone(), 35_000 * UNIT),
            (eurc.key.clone(), 3_500 * UNIT),
        ];
        let direct_account = victim(&fixture, &usdc, &debts);
        let sdk_account = victim(&fixture, &usdc, &debts);
        fixture.advance_time(3600);
        fixture.set_price(&xlm, WAD * price_cents / 100);
        let direct = Address::generate(&env);
        let sdk = env.register(Liquidator, ());
        for liquidator in [&direct, &sdk] {
            xlm.sac.mock_all_auths().mint(liquidator, &(100_000 * UNIT));
            eurc.sac.mock_all_auths().mint(liquidator, &(10_000 * UNIT));
        }
        let offers = vec![
            &env,
            (xlm.key.clone(), 50_000 * UNIT),
            (eurc.key.clone(), 10_000 * UNIT),
            (xlm.key.clone(), 50_000 * UNIT),
        ];
        fixture.controller.mock_all_auths().liquidate(
            &direct,
            &direct_account,
            &offers,
            &SeizeMode::Transfer,
        );
        let spent = LiquidatorClient::new(&env, &sdk).liquidate(
            &fixture.addresses(),
            &sdk_account,
            &offers,
        );
        assert_eq!(
            usdc.token.balance(&direct),
            usdc.token.balance(&sdk),
            "collateral: price {price_cents}"
        );
        for (market, initial) in [(&xlm, 100_000 * UNIT), (&eurc, 10_000 * UNIT)] {
            let direct_spent = initial - market.token.balance(&direct);
            assert_eq!(
                direct_spent,
                initial - market.token.balance(&sdk),
                "spend: price {price_cents}"
            );
            let reported = spent.iter().find(|(key, _)| *key == market.key);
            if direct_spent == 0 {
                assert!(reported.is_none(), "unused leg: price {price_cents}");
            } else {
                assert_eq!(
                    reported.unwrap().1,
                    direct_spent,
                    "reported spend: price {price_cents}"
                );
            }
            assert_eq!(
                fixture
                    .controller
                    .get_borrow_amount(&direct_account, &market.key),
                fixture
                    .controller
                    .get_borrow_amount(&sdk_account, &market.key),
                "debt: price {price_cents}",
            );
        }
        if price_cents == 13 {
            assert_eq!(spent.len(), 1);
            assert!(fixture.controller.get_borrow_amount(&sdk_account, &xlm.key) > 0);
            assert!(
                fixture
                    .controller
                    .get_borrow_amount(&sdk_account, &eurc.key)
                    > 0
            );
        } else if price_cents == 14 {
            assert_eq!(spent.len(), 2);
            assert_eq!(
                fixture.controller.get_borrow_amount(&sdk_account, &xlm.key),
                0
            );
            assert!(
                fixture
                    .controller
                    .get_borrow_amount(&sdk_account, &eurc.key)
                    > 0
            );
        } else if price_cents == 18 {
            assert_eq!(
                fixture.controller.get_borrow_amount(&sdk_account, &xlm.key),
                0
            );
            assert_eq!(
                fixture
                    .controller
                    .get_borrow_amount(&sdk_account, &eurc.key),
                0
            );
        } else {
            assert_eq!(spent.len(), 1);
        }
    }
}

#[test]
fn same_token_collateral_can_make_one_liquidation_leg_report_negative_spend() {
    let env = Env::default();
    let fixture = LendingFixture::deploy(&env, &Address::generate(&env));
    let usdc = fixture.create_market(&MarketConfig::usdc());
    let xlm = fixture.create_market(&MarketConfig::xlm());
    let offers = vec![
        &env,
        (usdc.key.clone(), 100 * UNIT),
        (xlm.key.clone(), 69_000 * UNIT),
    ];
    let account = victim(&fixture, &usdc, &offers);
    fixture.set_price(&xlm, WAD * 14 / 100);
    let sdk = env.register(Liquidator, ());
    usdc.sac.mock_all_auths().mint(&sdk, &(100 * UNIT));
    xlm.sac.mock_all_auths().mint(&sdk, &(69_000 * UNIT));
    let spent =
        LiquidatorClient::new(&env, &sdk).liquidate(&fixture.addresses(), &account, &offers);
    let usdc_spent = spent.iter().find(|(key, _)| *key == usdc.key).unwrap().1;
    let xlm_spent = spent.iter().find(|(key, _)| *key == xlm.key).unwrap().1;
    assert!(usdc_spent < 0);
    assert_eq!(usdc_spent, 100 * UNIT - usdc.token.balance(&sdk));
    assert_eq!(xlm_spent, 69_000 * UNIT - xlm.token.balance(&sdk));
}
