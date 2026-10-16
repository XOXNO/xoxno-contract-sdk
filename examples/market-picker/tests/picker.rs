use market_picker::{MarketPicker, MarketPickerClient, Placement};
use soroban_sdk::testutils::{Address as _, MockAuth, MockAuthInvoke};
use soroban_sdk::{vec, Address, Env, IntoVal};
use xoxno_contract_sdk::lending::constants::NEW_ACCOUNT;
use xoxno_contract_sdk::testutils::{LendingFixture, MarketConfig};

const UNIT: i128 = 10_000_000;

#[path = "../../account-basics/tests/support/receipt_tax.rs"]
mod receipt_tax;
use receipt_tax::{ReceiptTaxToken, ReceiptTaxTokenClient};

fn setup<'a>(env: &Env) -> (LendingFixture<'a>, MarketPickerClient<'a>, Address) {
    env.mock_all_auths();
    let fixture = LendingFixture::deploy(env, &Address::generate(env));
    let owner = Address::generate(env);
    let picker = env.register(MarketPicker, (owner.clone(), fixture.addresses()));
    (fixture, MarketPickerClient::new(env, &picker), owner)
}

#[test]
fn picks_the_hub_that_lists_the_token() {
    let env = Env::default();
    let (fixture, picker, _) = setup(&env);
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
    let (fixture, picker, _) = setup(&env);
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
fn skips_a_listed_hub_when_only_a_later_hub_accepts_supply() {
    let env = Env::default();
    let (fixture, picker, _) = setup(&env);
    let mut cfg = MarketConfig::usdc();
    cfg.risk.can_be_collateral = false;
    cfg.initial_liquidity = 0;
    let usdc = fixture.create_market(&cfg);
    let hub_2 = fixture.add_hub();
    let later = fixture.add_market_to_hub(&usdc, hub_2, &cfg.rates);
    fixture.list_market(&later, fixture.spoke_id, &MarketConfig::usdc().risk);

    assert_eq!(
        picker.pick(
            &usdc.asset,
            &vec![&env, fixture.hub_id, hub_2],
            &vec![&env, fixture.spoke_id]
        ),
        Some(Placement {
            spoke_id: fixture.spoke_id,
            market: later
        })
    );
}

#[test]
fn finds_nothing_for_an_unlisted_token() {
    let env = Env::default();
    let (fixture, picker, _) = setup(&env);
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
    let (fixture, picker, owner) = setup(&env);
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

    env.set_auths(&[]);
    assert!(picker.try_withdraw_all(&account_id, &usdc.key).is_err());
    assert!(fixture.controller.account_exists(&account_id));
    env.mock_auths(&[MockAuth {
        address: &owner,
        invoke: &MockAuthInvoke {
            contract: &picker.address,
            fn_name: "withdraw_all",
            args: (account_id, usdc.key.clone()).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    let withdrawal = picker.withdraw_all(&account_id, &usdc.key);
    assert!(withdrawal.account_closed);
    assert!(1_000 * UNIT - withdrawal.amount <= 1);
    assert_eq!(usdc.token.balance(&owner), withdrawal.amount);
    assert!(!fixture.controller.account_exists(&account_id));
    assert!(fixture
        .position_nft
        .try_owner_of(&(account_id as u32))
        .is_err());

    env.mock_all_auths();
    usdc.sac.mint(&from, &(100 * UNIT));
    let reopened = picker.supply_best(
        &from,
        &usdc.asset,
        &(100 * UNIT),
        &vec![&env, fixture.hub_id],
        &vec![&env, spoke_2],
    );
    assert_ne!(reopened, account_id);
    assert_eq!(
        fixture.position_nft.owner_of(&(reopened as u32)),
        picker.address
    );
}

#[test]
fn market_info_shows_utilization_after_a_borrow() {
    let env = Env::default();
    let (fixture, picker, _) = setup(&env);
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

#[test]
fn taxed_funding_reverts_without_spending_prefunded_tokens() {
    let env = Env::default();
    let (fixture, picker, _) = setup(&env);
    let usdc = fixture.create_market(&MarketConfig::usdc());
    let from = Address::generate(&env);
    env.register_at(&usdc.asset, ReceiptTaxToken, (picker.address.clone(),));
    let token = ReceiptTaxTokenClient::new(&env, &usdc.asset);
    token.mint(&from, &(100 * UNIT));
    token.mint(&picker.address, &(10 * UNIT));

    assert!(picker
        .try_supply_best(
            &from,
            &usdc.asset,
            &(100 * UNIT),
            &vec![&env, fixture.hub_id],
            &vec![&env, fixture.spoke_id]
        )
        .is_err());

    assert_eq!(token.balance(&from), 100 * UNIT);
    assert_eq!(token.balance(&picker.address), 10 * UNIT);
    assert_eq!(fixture.position_nft.balance(&picker.address), 0);
}
