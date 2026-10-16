use simple_deposit::{SimpleDeposit, SimpleDepositClient};
use soroban_sdk::testutils::{Address as _, MockAuth, MockAuthInvoke};
use soroban_sdk::{vec, Address, Env, Error, IntoVal};
use xoxno_contract_sdk::lending::constants::NEW_ACCOUNT;
use xoxno_contract_sdk::lending::controller::SpokeError;
use xoxno_contract_sdk::testutils::{LendingFixture, MarketConfig};

const UNIT: i128 = 10_000_000;

#[path = "../../account-basics/tests/support/receipt_tax.rs"]
mod receipt_tax;
use receipt_tax::{ReceiptTaxToken, ReceiptTaxTokenClient};

#[test]
fn deposit_opens_then_reuses_an_account() {
    let env = Env::default();
    env.mock_all_auths();
    let fixture = LendingFixture::deploy(&env, &Address::generate(&env));
    let usdc = fixture.create_market(&MarketConfig::usdc());
    let owner = Address::generate(&env);
    let contract = SimpleDepositClient::new(
        &env,
        &env.register(SimpleDeposit, (owner.clone(), fixture.addresses())),
    );
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

    env.set_auths(&[]);
    assert!(contract
        .try_withdraw_all(&account, &usdc.asset, &fixture.hub_id)
        .is_err());
    assert!(fixture.controller.account_exists(&account));
    env.mock_auths(&[MockAuth {
        address: &owner,
        invoke: &MockAuthInvoke {
            contract: &contract.address,
            fn_name: "withdraw_all",
            args: (account, usdc.asset.clone(), fixture.hub_id).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    let withdrawal = contract.withdraw_all(&account, &usdc.asset, &fixture.hub_id);
    assert!(withdrawal.account_closed);
    assert!(1_500 * UNIT - withdrawal.amount <= 1);
    assert_eq!(usdc.token.balance(&owner), withdrawal.amount);
    assert!(!fixture.controller.account_exists(&account));
    assert!(fixture
        .position_nft
        .try_owner_of(&(account as u32))
        .is_err());

    env.mock_all_auths();
    usdc.sac.mint(&alice, &(100 * UNIT));
    let reopened = contract.deposit(
        &alice,
        &usdc.asset,
        &fixture.hub_id,
        &fixture.spoke_id,
        &NEW_ACCOUNT,
        &(100 * UNIT),
    );
    assert_ne!(reopened, account);
    assert_eq!(
        fixture.position_nft.owner_of(&(reopened as u32)),
        contract.address
    );
}

#[test]
fn a_wrong_spoke_for_an_existing_account_is_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let fixture = LendingFixture::deploy(&env, &Address::generate(&env));
    let usdc = fixture.create_market(&MarketConfig::usdc());
    let spoke_2 = fixture.add_spoke();
    let contract = SimpleDepositClient::new(
        &env,
        &env.register(
            SimpleDeposit,
            (Address::generate(&env), fixture.addresses()),
        ),
    );
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

#[test]
fn taxed_funding_reverts_without_spending_prefunded_tokens() {
    let env = Env::default();
    env.mock_all_auths();
    let fixture = LendingFixture::deploy(&env, &Address::generate(&env));
    let usdc = fixture.create_market(&MarketConfig::usdc());
    let contract = SimpleDepositClient::new(
        &env,
        &env.register(
            SimpleDeposit,
            (Address::generate(&env), fixture.addresses()),
        ),
    );
    let from = Address::generate(&env);
    env.register_at(&usdc.asset, ReceiptTaxToken, (contract.address.clone(),));
    let token = ReceiptTaxTokenClient::new(&env, &usdc.asset);
    token.mint(&from, &(100 * UNIT));
    token.mint(&contract.address, &(10 * UNIT));

    assert!(contract
        .try_deposit(
            &from,
            &usdc.asset,
            &fixture.hub_id,
            &fixture.spoke_id,
            &NEW_ACCOUNT,
            &(100 * UNIT)
        )
        .is_err());

    assert_eq!(token.balance(&from), 100 * UNIT);
    assert_eq!(token.balance(&contract.address), 10 * UNIT);
    assert_eq!(fixture.position_nft.balance(&contract.address), 0);
}

#[test]
fn cannot_deposit_into_an_account_the_contract_does_not_own() {
    let env = Env::default();
    env.mock_all_auths();
    let fixture = LendingFixture::deploy(&env, &Address::generate(&env));
    let usdc = fixture.create_market(&MarketConfig::usdc());
    let contract = SimpleDepositClient::new(
        &env,
        &env.register(
            SimpleDeposit,
            (Address::generate(&env), fixture.addresses()),
        ),
    );
    let from = Address::generate(&env);
    usdc.sac.mint(&from, &(200 * UNIT));
    let foreign = fixture.controller.supply(
        &from,
        &NEW_ACCOUNT,
        &fixture.spoke_id,
        &vec![&env, (usdc.key.clone(), 100 * UNIT)],
    );

    assert!(contract
        .try_deposit(
            &from,
            &usdc.asset,
            &fixture.hub_id,
            &fixture.spoke_id,
            &foreign,
            &(100 * UNIT)
        )
        .is_err());

    assert_eq!(usdc.token.balance(&from), 100 * UNIT);
    assert_eq!(usdc.token.balance(&contract.address), 0);
    assert_eq!(
        fixture
            .controller
            .get_collateral_amount(&foreign, &usdc.key),
        100 * UNIT
    );
}
