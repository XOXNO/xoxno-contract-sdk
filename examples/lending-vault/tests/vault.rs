use lending_vault::{LendingVault, LendingVaultClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{vec, Address, Bytes, Env};
use xoxno_contract_sdk::lending::constants::NEW_ACCOUNT;
use xoxno_contract_sdk::testutils::{LendingFixture, Market, MarketConfig};

const UNIT: i128 = 10_000_000;
const DAY: u64 = 86_400;

struct Setup<'a> {
    env: Env,
    fixture: LendingFixture<'a>,
    usdc: Market<'a>,
    vault: LendingVaultClient<'a>,
    owner: Address,
}

fn setup<'a>() -> Setup<'a> {
    let env = Env::default();
    env.mock_all_auths();
    let fixture = LendingFixture::deploy(&env, &Address::generate(&env));
    let usdc = fixture.create_market(&MarketConfig::usdc());
    let owner = Address::generate(&env);
    let vault = env.register(
        LendingVault,
        (
            owner.clone(),
            fixture.controller.address.clone(),
            usdc.key.clone(),
            fixture.spoke_id,
        ),
    );
    Setup {
        vault: LendingVaultClient::new(&env, &vault),
        env,
        fixture,
        usdc,
        owner,
    }
}

#[test]
fn deposits_earn_interest_and_withdraw_in_full() {
    let s = setup();
    let depositor = Address::generate(&s.env);
    s.usdc.sac.mint(&depositor, &(1_000 * UNIT));
    let account_id = s.vault.deposit(&depositor, &(1_000 * UNIT));
    assert_eq!(
        s.fixture.position_nft.owner_of(&(account_id as u32)),
        s.vault.address
    );
    assert_eq!(s.vault.balance(), 1_000 * UNIT);

    let borrower = Address::generate(&s.env);
    s.usdc.sac.mint(&borrower, &(1_000_000 * UNIT));
    let borrower_account = s.fixture.controller.supply(
        &borrower,
        &NEW_ACCOUNT,
        &s.fixture.spoke_id,
        &vec![&s.env, (s.usdc.key.clone(), 1_000_000 * UNIT)],
    );
    s.fixture.controller.borrow(
        &borrower,
        &borrower_account,
        &vec![&s.env, (s.usdc.key.clone(), 700_000 * UNIT)],
        &None,
    );
    s.fixture.advance_time(90 * DAY);

    let grown = s.vault.balance();
    assert!(grown > 1_000 * UNIT, "balance {grown} earned no interest");
    let withdrawn = s.vault.withdraw_all();
    assert!(
        grown - withdrawn <= 1,
        "withdrew {withdrawn}, view showed {grown}"
    );
    assert_eq!(s.usdc.token.balance(&s.owner), withdrawn);
}

#[test]
fn a_flash_loan_is_repaid_with_the_fee() {
    let s = setup();
    s.usdc.sac.mint(&s.vault.address, &(100 * UNIT));
    let pool_before = s.usdc.token.balance(&s.fixture.pool.address);

    s.fixture.controller.flash_loan(
        &s.owner,
        &s.usdc.key,
        &(100_000 * UNIT),
        &s.vault.address,
        &Bytes::new(&s.env),
    );

    let fee = s.usdc.token.balance(&s.fixture.pool.address) - pool_before;
    assert!(fee > 0, "the pool received no fee");
    assert_eq!(fee, 90 * UNIT);
    assert_eq!(s.usdc.token.balance(&s.vault.address), 10 * UNIT);
}

#[test]
fn a_flash_loan_from_a_stranger_is_rejected() {
    let s = setup();
    s.usdc.sac.mint(&s.vault.address, &(10 * UNIT));

    let result = s.fixture.controller.try_flash_loan(
        &Address::generate(&s.env),
        &s.usdc.key,
        &(100_000 * UNIT),
        &s.vault.address,
        &Bytes::new(&s.env),
    );

    assert!(result.is_err());
    assert_eq!(s.usdc.token.balance(&s.vault.address), 10 * UNIT);
}
