use soroban_sdk::testutils::Address as _;
use soroban_sdk::{contract, contractimpl, contracttype, vec, Address, Env, Error, Vec};
use xoxno_contract_sdk::lending::constants::{NEW_ACCOUNT, WAD};
use xoxno_contract_sdk::lending::controller::{GenericError, HubAssetKey};
use xoxno_contract_sdk::testutils::{LendingFixture, Market, MarketConfig};
use xoxno_contract_sdk::{LendingAddresses, Withdrawal, XoxnoLending};

const UNIT: i128 = 10_000_000;

#[contract]
struct Integrator;

#[contractimpl]
impl Integrator {
    pub fn deposit(
        env: Env,
        addresses: LendingAddresses,
        spoke: u32,
        payments: Vec<(HubAssetKey, i128)>,
    ) -> u64 {
        XoxnoLending::new(&env, &addresses).deposit_batch(NEW_ACCOUNT, spoke, &payments)
    }

    pub fn repay(
        env: Env,
        addresses: LendingAddresses,
        account: u64,
        payments: Vec<(HubAssetKey, i128)>,
    ) -> Vec<(HubAssetKey, i128)> {
        XoxnoLending::new(&env, &addresses).repay_batch(account, &payments)
    }

    pub fn liquidate(
        env: Env,
        addresses: LendingAddresses,
        account: u64,
        payments: Vec<(HubAssetKey, i128)>,
    ) -> Vec<(HubAssetKey, i128)> {
        XoxnoLending::new(&env, &addresses).liquidate_batch(account, &payments)
    }

    pub fn withdraw(
        env: Env,
        addresses: LendingAddresses,
        account: u64,
        market: HubAssetKey,
        amount: i128,
    ) -> Withdrawal {
        XoxnoLending::new(&env, &addresses).withdraw(account, &market, amount)
    }
}

// Receipt-tax test double; setup reuses the fixture's listed token and backing.
#[contract]
struct TaxToken;

#[contracttype]
#[derive(Clone)]
enum TokenKey {
    ReviewBalance(Address),
    Tax,
}

#[contractimpl]
impl TaxToken {
    pub fn balance(env: Env, owner: Address) -> i128 {
        env.storage()
            .persistent()
            .get(&TokenKey::ReviewBalance(owner))
            .unwrap_or(0)
    }

    pub fn decimals(_env: Env) -> u32 {
        7
    }

    pub fn mint(env: Env, owner: Address, amount: i128) {
        let balance = Self::balance(env.clone(), owner.clone());
        env.storage()
            .persistent()
            .set(&TokenKey::ReviewBalance(owner), &(balance + amount));
    }

    pub fn set_tax(env: Env, bps: i128) {
        assert!((0..=10_000).contains(&bps));
        env.storage().instance().set(&TokenKey::Tax, &bps);
    }

    pub fn transfer(env: Env, from: Address, to: Address, amount: i128) {
        from.require_auth();
        assert!(amount > 0);
        let balance = Self::balance(env.clone(), from.clone());
        assert!(balance >= amount);
        env.storage()
            .persistent()
            .set(&TokenKey::ReviewBalance(from), &(balance - amount));
        let tax: i128 = env.storage().instance().get(&TokenKey::Tax).unwrap_or(0);
        Self::mint(env, to, amount - amount * tax / 10_000);
    }
}

struct Setup<'a> {
    env: Env,
    fixture: LendingFixture<'a>,
    collateral: Market<'a>,
    debt: Market<'a>,
    integrator: Address,
    account: u64,
}

fn setup<'a>() -> Setup<'a> {
    let env = Env::default();
    let fixture = LendingFixture::deploy(&env, &Address::generate(&env));
    let collateral = fixture.create_market(&MarketConfig::usdc());
    let debt = fixture.create_market(&MarketConfig::xlm());
    let backing = debt.token.balance(&fixture.pool.address);
    env.register_at(&debt.asset, TaxToken, ());
    TaxTokenClient::new(&env, &debt.asset).mint(&fixture.pool.address, &backing);
    let integrator = env.register(Integrator, ());
    collateral
        .sac
        .mock_all_auths()
        .mint(&integrator, &(1_000 * UNIT));
    let account = IntegratorClient::new(&env, &integrator).deposit(
        &fixture.addresses(),
        &fixture.spoke_id,
        &vec![&env, (collateral.key.clone(), 1_000 * UNIT)],
    );
    env.as_contract(&integrator, || {
        XoxnoLending::new(&env, &fixture.addresses()).borrow(account, &debt.key, 7_000 * UNIT);
    });
    Setup {
        env,
        fixture,
        collateral,
        debt,
        integrator,
        account,
    }
}

#[test]
fn repayment_reports_spend_separately_from_receipt_and_debt_reduction() {
    let s = setup();
    let token = TaxTokenClient::new(&s.env, &s.debt.asset);
    token.set_tax(&1_000);
    let debt_before = s
        .fixture
        .controller
        .get_borrow_amount(&s.account, &s.debt.key);
    let cash_before = s.debt.token.balance(&s.fixture.pool.address);
    let before = token.balance(&s.integrator);
    let spent = IntegratorClient::new(&s.env, &s.integrator).repay(
        &s.fixture.addresses(),
        &s.account,
        &vec![&s.env, (s.debt.key.clone(), 100 * UNIT)],
    );
    assert_eq!(spent.get(0).unwrap().1, 100 * UNIT);
    assert_eq!(before - token.balance(&s.integrator), 100 * UNIT);
    assert_eq!(
        s.debt.token.balance(&s.fixture.pool.address) - cash_before,
        90 * UNIT
    );
    assert_eq!(
        debt_before
            - s.fixture
                .controller
                .get_borrow_amount(&s.account, &s.debt.key),
        90 * UNIT
    );
}

#[test]
fn liquidation_keeps_tax_cover_and_returns_actual_spend() {
    let s = setup();
    s.fixture.set_price(&s.debt, WAD * 14 / 100);
    let token = TaxTokenClient::new(&s.env, &s.debt.asset);
    token.set_tax(&1_000);
    token.mint(&s.integrator, &(3_000 * UNIT));
    let before = token.balance(&s.integrator);
    let spent = IntegratorClient::new(&s.env, &s.integrator).liquidate(
        &s.fixture.addresses(),
        &s.account,
        &vec![&s.env, (s.debt.key.clone(), 10_000 * UNIT)],
    );
    assert_eq!(
        s.fixture
            .controller
            .get_borrow_amount(&s.account, &s.debt.key),
        0
    );
    // Pool receives 9,000, retires 7,000 and refunds 2,000 (1,800 received).
    assert_eq!(spent.get(0).unwrap().1, 8_200 * UNIT);
    assert_eq!(before - token.balance(&s.integrator), 8_200 * UNIT);
    assert!(s.collateral.token.balance(&s.integrator) > 0);
}

#[test]
fn paying_batches_reject_invalid_legs_before_merging() {
    let s = setup();
    s.fixture.set_price(&s.debt, WAD * 14 / 100);
    let client = IntegratorClient::new(&s.env, &s.integrator);
    for invalid in [-1, 0] {
        let payments = vec![
            &s.env,
            (s.debt.key.clone(), UNIT),
            (s.debt.key.clone(), invalid),
        ];
        let error = Error::from_contract_error(GenericError::AmountMustBePositive as u32);
        assert_eq!(
            client.try_deposit(&s.fixture.addresses(), &s.fixture.spoke_id, &payments),
            Err(Ok(error))
        );
        assert_eq!(
            client.try_repay(&s.fixture.addresses(), &s.account, &payments),
            Err(Ok(error))
        );
        assert_eq!(
            client.try_liquidate(&s.fixture.addresses(), &s.account, &payments),
            Err(Ok(error))
        );
    }
}

#[test]
fn withdrawal_reports_pool_output_separately_from_taxed_receipt() {
    let s = setup();
    let client = IntegratorClient::new(&s.env, &s.integrator);
    let account = client.deposit(
        &s.fixture.addresses(),
        &s.fixture.spoke_id,
        &vec![&s.env, (s.debt.key.clone(), 1_000 * UNIT)],
    );
    let token = TaxTokenClient::new(&s.env, &s.debt.asset);
    token.set_tax(&1_000);
    let before = token.balance(&s.integrator);
    let result = client.withdraw(&s.fixture.addresses(), &account, &s.debt.key, &(100 * UNIT));
    assert_eq!(result.amount, 100 * UNIT);
    assert_eq!(token.balance(&s.integrator) - before, 90 * UNIT);
    assert!(!result.account_closed);
}
