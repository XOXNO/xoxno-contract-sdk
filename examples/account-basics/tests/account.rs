use account_basics::{AccountBasics, AccountBasicsClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env};
use xoxno_contract_sdk::lending::constants::WAD;
use xoxno_contract_sdk::testutils::{LendingFixture, Market, MarketConfig};

const UNIT: i128 = 10_000_000;

struct Setup<'a> {
    env: Env,
    fixture: LendingFixture<'a>,
    usdc: Market<'a>,
    xlm: Market<'a>,
    owner: Address,
    contract: AccountBasicsClient<'a>,
}

fn setup<'a>() -> Setup<'a> {
    let env = Env::default();
    env.mock_all_auths();
    let fixture = LendingFixture::deploy(&env, &Address::generate(&env));
    let usdc = fixture.create_market(&MarketConfig::usdc());
    let xlm = fixture.create_market(&MarketConfig::xlm());
    let owner = Address::generate(&env);
    let contract = env.register(
        AccountBasics,
        (
            owner.clone(),
            fixture.addresses(),
            fixture.spoke_id,
            usdc.key.clone(),
        ),
    );
    Setup {
        contract: AccountBasicsClient::new(&env, &contract),
        env,
        fixture,
        usdc,
        xlm,
        owner,
    }
}

fn deposit(s: &Setup, amount: i128) -> u64 {
    let from = Address::generate(&s.env);
    s.usdc.sac.mint(&from, &amount);
    s.contract.deposit(&from, &amount)
}

#[test]
fn the_first_deposit_opens_an_account_whose_nft_the_contract_owns() {
    let s = setup();

    let account_id = deposit(&s, 1_000 * UNIT);

    assert_eq!(s.contract.account_id(), Some(account_id));
    assert_eq!(
        s.fixture.position_nft.owner_of(&(account_id as u32)),
        s.contract.address
    );
    assert_eq!(
        s.fixture
            .controller
            .get_account_attributes(&account_id)
            .spoke_id,
        s.fixture.spoke_id
    );
}

#[test]
fn a_later_deposit_reuses_the_same_account() {
    let s = setup();
    let first = deposit(&s, 1_000 * UNIT);

    let second = deposit(&s, 500 * UNIT);

    assert_eq!(second, first);
    assert_eq!(s.fixture.position_nft.balance(&s.contract.address), 1);
    let position = s.contract.position();
    assert_eq!(position.collateral.len(), 1);
    assert_eq!(
        position.collateral.get(0).unwrap(),
        (s.usdc.key.clone(), 1_500 * UNIT)
    );
}

#[test]
fn borrow_and_repay_move_the_debt() {
    let s = setup();
    deposit(&s, 1_000 * UNIT);

    s.contract.borrow(&s.xlm.key, &(2_000 * UNIT));
    assert_eq!(s.xlm.token.balance(&s.owner), 2_000 * UNIT);
    let health = s.contract.health_factor();
    assert!(health > WAD && health < i128::MAX);

    assert_eq!(s.contract.repay(&s.xlm.key, &(2_000 * UNIT)), 2_000 * UNIT);
    assert!(s.contract.position().debt.is_empty());
    assert_eq!(s.contract.health_factor(), i128::MAX);
}

#[test]
fn a_repayment_above_the_debt_returns_the_rest_to_the_owner() {
    let s = setup();
    deposit(&s, 1_000 * UNIT);
    s.contract.borrow(&s.xlm.key, &(2_000 * UNIT));
    s.xlm.sac.mint(&s.owner, &(500 * UNIT));

    let repaid = s.contract.repay(&s.xlm.key, &(2_500 * UNIT));

    assert_eq!(repaid, 2_000 * UNIT);
    assert!(s.contract.position().debt.is_empty());
    assert_eq!(s.xlm.token.balance(&s.owner), 500 * UNIT);
    assert_eq!(s.xlm.token.balance(&s.contract.address), 0);
}

#[test]
fn a_closed_account_is_replaced_by_a_new_one() {
    let s = setup();
    let first = deposit(&s, 1_000 * UNIT);

    let withdrawal = s.contract.withdraw_all();
    assert!(1_000 * UNIT - withdrawal.amount <= 1);
    assert!(withdrawal.account_closed);
    assert_eq!(s.contract.account_id(), None);
    assert_eq!(s.usdc.token.balance(&s.owner), withdrawal.amount);
    assert!(!s.fixture.controller.account_exists(&first));
    assert!(s
        .fixture
        .position_nft
        .try_owner_of(&(first as u32))
        .is_err());

    let second = deposit(&s, 100 * UNIT);
    assert_ne!(second, first);
    assert_eq!(s.contract.account_id(), Some(second));
}
