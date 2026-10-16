use soroban_sdk::testutils::storage::Persistent as _;
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::{contract, contractimpl, Address, Env, Symbol};
use xoxno_contract_sdk::lending::constants::WAD;
use xoxno_contract_sdk::testutils::{LendingFixture, Market, MarketConfig};
use xoxno_contract_sdk::XoxnoLending;

#[contract]
struct Integrator;

#[contractimpl]
impl Integrator {}

fn setup<'a>() -> (Env, LendingFixture<'a>, Market<'a>, Address) {
    let env = Env::default();
    let fixture = LendingFixture::deploy(&env, &Address::generate(&env));
    let market = fixture.create_market(&MarketConfig::usdc());
    let integrator = env.register(Integrator, ());
    (env, fixture, market, integrator)
}

#[test]
fn diagnostic_prices_reject_missing_and_disagreeing_sources() {
    let (env, fixture, market, integrator) = setup();
    env.as_contract(&integrator, || {
        let lending = XoxnoLending::new(&env, &fixture.addresses());
        assert_eq!(lending.try_price(&Address::generate(&env)), None);
        assert_eq!(lending.try_price(&market.asset), Some(WAD));
        fixture.redstone.set_price(&market.feed_id, &(2 * WAD));
        assert!(!lending.quote(&market.asset).valid);
        assert_eq!(lending.try_price(&market.asset), None);
        assert!(lending
            .price_aggregator()
            .try_prices(&soroban_sdk::vec![
                &env,
                xoxno_contract_sdk::lending::price_aggregator::PriceKey::Token(
                    market.asset.clone()
                )
            ])
            .is_err());
    });
}

#[test]
fn diagnostic_prices_reject_stale_and_future_observations() {
    let (env, fixture, market, integrator) = setup();
    let now = env.ledger().timestamp();
    env.ledger()
        .with_mut(|ledger| ledger.timestamp = now + 86_401);
    env.as_contract(&integrator, || {
        assert_eq!(
            XoxnoLending::new(&env, &fixture.addresses()).try_price(&market.asset),
            None
        );
    });
    env.ledger()
        .with_mut(|ledger| ledger.timestamp = now - 1_000);
    env.as_contract(&integrator, || {
        assert_eq!(
            XoxnoLending::new(&env, &fixture.addresses()).try_price(&market.asset),
            None
        );
    });
}

#[test]
fn a_wrapper_roundtrip_fits_the_default_execution_budget_after_setup() {
    let (env, fixture, market, integrator) = setup();
    market.sac.mock_all_auths().mint(&integrator, &10_000_000);
    let account = env.as_contract(&integrator, || {
        XoxnoLending::new(&env, &fixture.addresses()).open_account(
            fixture.spoke_id,
            &market.key,
            10_000_000,
        )
    });
    env.cost_estimate().budget().reset_default();
    let result = env.as_contract(&integrator, || {
        XoxnoLending::new(&env, &fixture.addresses()).withdraw_all(account, &market.key)
    });
    assert!(result.account_closed);
    assert!(result.amount >= 9_999_999);
}

#[test]
fn renewal_extends_near_expiry_protocol_entries_under_default_ttl_limits() {
    let (env, fixture, market, integrator) = setup();
    market.sac.mock_all_auths().mint(&integrator, &10_000_000);
    let account = env.as_contract(&integrator, || {
        XoxnoLending::new(&env, &fixture.addresses()).open_account(
            fixture.spoke_id,
            &market.key,
            10_000_000,
        )
    });
    let addresses = fixture.addresses();
    let mut snapshot = env.to_ledger_snapshot();
    let limits = Env::default().ledger().get();
    snapshot.min_persistent_entry_ttl = limits.min_persistent_entry_ttl;
    snapshot.min_temp_entry_ttl = limits.min_temp_entry_ttl;
    snapshot.max_entry_ttl = limits.max_entry_ttl;
    let near_expiry = snapshot.sequence_number + 100;
    for (_, (_, ttl)) in &mut snapshot.ledger_entries {
        if ttl.is_some() {
            *ttl = Some(near_expiry);
        }
    }
    let restored = Env::from_ledger_snapshot(snapshot);
    let addresses = xoxno_contract_sdk::LendingAddresses {
        controller: Address::from_str(&restored, &addresses.controller.to_string().to_string()),
        pool: Address::from_str(&restored, &addresses.pool.to_string().to_string()),
        position_nft: Address::from_str(&restored, &addresses.position_nft.to_string().to_string()),
    };
    let integrator = Address::from_str(&restored, &integrator.to_string().to_string());
    restored.as_contract(&integrator, || {
        let lending = XoxnoLending::new(&restored, &addresses);
        lending.renew_account(account);
        assert!(lending.owns(account));
    });
    // Restoring an archived footprint is an off-chain transaction concern;
    // this checks renewal while entries are live, with the fixture TTL removed.
    restored.as_contract(&addresses.controller, || {
        for key in ["AccountMeta", "SupplyPositions"] {
            assert!(
                restored
                    .storage()
                    .persistent()
                    .get_ttl(&(Symbol::new(&restored, key), account))
                    > 100
            );
        }
    });
}
