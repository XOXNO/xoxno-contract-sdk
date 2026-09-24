use simple_deposit::{SimpleDeposit, SimpleDepositClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env, Error};
use xoxno_contract_sdk::lending::constants::NEW_ACCOUNT;
use xoxno_contract_sdk::lending::controller::SpokeError;
use xoxno_contract_sdk::testutils::{LendingFixture, MarketConfig};

const UNIT: i128 = 10_000_000;

#[test]
fn deposit_opens_then_reuses_an_account() {
    let env = Env::default();
    env.mock_all_auths();
    let fixture = LendingFixture::deploy(&env, &Address::generate(&env));
    let usdc = fixture.create_market(&MarketConfig::usdc());
    let contract =
        SimpleDepositClient::new(&env, &env.register(SimpleDeposit, (fixture.addresses(),)));
    let alice = Address::generate(&env);
    usdc.sac.mint(&alice, &(1_500 * UNIT));

    let account = contract.deposit(
        &alice,
        &usdc.asset,
        &fixture.hub_id,
        &fixture.spoke_id,
        &NEW_ACCOUNT,
        &(1_000 * UNIT),
    );
    let same = contract.deposit(
        &alice,
        &usdc.asset,
        &fixture.hub_id,
        &fixture.spoke_id,
        &account,
        &(500 * UNIT),
    );

    assert_eq!(same, account);
    assert_eq!(
        fixture.position_nft.owner_of(&(account as u32)),
        contract.address
    );
    assert_eq!(
        fixture
            .controller
            .get_collateral_amount(&account, &usdc.key),
        1_500 * UNIT
    );
}

#[test]
fn a_wrong_spoke_for_an_existing_account_is_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let fixture = LendingFixture::deploy(&env, &Address::generate(&env));
    let usdc = fixture.create_market(&MarketConfig::usdc());
    let spoke_2 = fixture.add_spoke();
    let contract =
        SimpleDepositClient::new(&env, &env.register(SimpleDeposit, (fixture.addresses(),)));
    let alice = Address::generate(&env);
    usdc.sac.mint(&alice, &(2_000 * UNIT));
    let account = contract.deposit(
        &alice,
        &usdc.asset,
        &fixture.hub_id,
        &fixture.spoke_id,
        &NEW_ACCOUNT,
        &(1_000 * UNIT),
    );

    let result = contract.try_deposit(
        &alice,
        &usdc.asset,
        &fixture.hub_id,
        &spoke_2,
        &account,
        &(1_000 * UNIT),
    );

    assert_eq!(
        result.err().and_then(Result::ok),
        Some(Error::from_contract_error(SpokeError::SpokeMismatch as u32))
    );
}
