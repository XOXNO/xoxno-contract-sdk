use core::cell::{Cell, RefCell};

use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::{
    token, vec, Address, BytesN, Env, IntoVal, String, Symbol, TryFromVal, Val, Vec,
};

use super::governance::{
    self, AdminOperation, AssetOracle, ConfigureAssetOracleArgs, CreatePoolArgs,
    DeployPositionNftArgs, FeedNature, FeedSource, IndependencePolicy, MarketParamsRaw,
    MultiFeedRef, OracleAssetRef, OracleReadMode, PriceKey, PriceSource, ProviderRef,
    ReflectorFeedRef, SpokeAssetArgs, SpokeLiquidationCurveArgs,
};
use super::{mock_redstone, mock_reflector, Market, MarketConfig, RateModel, RiskConfig};
use crate::lending::constants::{NEW_ACCOUNT, WAD};
use crate::lending::controller::HubAssetKey;
use crate::lending::{
    controller, pool, position_nft, price_aggregator, ControllerClient, LendingAddresses,
    PoolClient, PositionNftClient, PriceAggregatorClient,
};

const MIN_TIMESTAMP: u64 = 1_000_000;
const MIN_SEQUENCE: u32 = 100;
const MIN_PERSISTENT_TTL: u32 = 10_000_000;
const SECONDS_PER_LEDGER: u64 = 5;
const TIMELOCK_DELAY: u32 = 1;
const TOKEN_DECIMALS: u32 = 7;
const REFLECTOR_DECIMALS: u32 = 14;
const REDSTONE_DECIMALS: u32 = 8;
const TWAP_RECORDS: u32 = 3;
const PRICE_STALE_SECONDS: u64 = 86_400;
const TOLERANCE_BPS: u32 = 500;
const MAX_PRICE_WAD: i128 = 1_000_000_000 * WAD;
const TARGET_HF_WAD: i128 = WAD * 115 / 100;
const HF_FOR_MAX_BONUS_WAD: i128 = WAD * 90 / 100;
const LIQUIDATION_BONUS_FACTOR_BPS: u32 = 1_500;

/// XOXNO Lending deployed into a test [`Env`] from the embedded WASM.
///
/// `deploy` changes the ledger of `env`: it raises the timestamp to at least
/// 1,000,000 (the TWAP feed needs price history before "now"), raises the
/// sequence to at least 100 (the timelock reserves ledger 1), raises the
/// minimum persistent entry TTL to 10,000,000 ledgers and the maximum entry
/// TTL above it (about 1.6 years, so long
/// [`advance_time`](Self::advance_time) jumps do not archive the protocol's
/// entries), and moves the sequence forward by one ledger for each timelocked
/// operation.
///
/// The fixture authorizes its own admin calls per call, so it does not change
/// the auth mode of `env`. It does set the budget of `env` to unlimited, as the
/// fixture's own setup exceeds the default test budget. To check one call
/// against the network limits, call `env.cost_estimate().budget().reset_default()`
/// just before it and read `env.cost_estimate()` after it.
pub struct LendingFixture<'a> {
    pub env: Env,
    /// Owner of governance; holds every governance role.
    pub admin: Address,
    pub governance: governance::Client<'a>,
    pub controller: ControllerClient<'a>,
    pub pool: PoolClient<'a>,
    pub position_nft: PositionNftClient<'a>,
    pub price_aggregator: PriceAggregatorClient<'a>,
    pub reflector: mock_reflector::Client<'a>,
    pub redstone: mock_redstone::Client<'a>,
    /// The default hub; [`create_market`](Self::create_market) lists markets here.
    pub hub_id: u32,
    /// The default spoke; [`create_market`](Self::create_market) lists markets here.
    pub spoke_id: u32,
    prices: RefCell<std::vec::Vec<(Address, String, i128)>>,
    markets: RefCell<std::vec::Vec<HubAssetKey>>,
    next_salt: Cell<u32>,
}

impl<'a> LendingFixture<'a> {
    /// Deploys governance, the controller, the pool, the position NFT, the
    /// price aggregator and both mock oracles, creates one hub and one spoke
    /// with the liquidation curve of the mainnet "Blue Chip" spoke, and
    /// unpauses the controller.
    pub fn deploy(env: &Env, admin: &Address) -> Self {
        env.cost_estimate().budget().reset_unlimited();
        env.ledger().with_mut(|l| {
            l.timestamp = l.timestamp.max(MIN_TIMESTAMP);
            l.sequence_number = l.sequence_number.max(MIN_SEQUENCE);
            l.min_persistent_entry_ttl = l.min_persistent_entry_ttl.max(MIN_PERSISTENT_TTL);
            l.max_entry_ttl = l.max_entry_ttl.max(MIN_PERSISTENT_TTL + 1);
        });

        let deployer = env.deployer();
        let controller_hash = deployer.upload_contract_wasm(controller::WASM);
        let pool_hash = deployer.upload_contract_wasm(pool::WASM);
        let nft_hash = deployer.upload_contract_wasm(position_nft::WASM);
        let aggregator_hash = deployer.upload_contract_wasm(price_aggregator::WASM);

        let governance_address = env.register(governance::WASM, (admin.clone(), TIMELOCK_DELAY));
        let governance = governance::Client::new(env, &governance_address);
        let controller_address = governance
            .mock_all_auths()
            .deploy_controller(&controller_hash);
        let aggregator_address = governance
            .mock_all_auths()
            .deploy_price_aggregator(&aggregator_hash);

        let fixture = Self {
            env: env.clone(),
            admin: admin.clone(),
            governance,
            controller: ControllerClient::new(env, &controller_address),
            pool: PoolClient::new(env, &controller_address),
            position_nft: PositionNftClient::new(env, &controller_address),
            price_aggregator: PriceAggregatorClient::new(env, &aggregator_address),
            reflector: mock_reflector::Client::new(env, &env.register(mock_reflector::WASM, ())),
            redstone: mock_redstone::Client::new(env, &env.register(mock_redstone::WASM, ())),
            hub_id: 0,
            spoke_id: 0,
            prices: RefCell::new(std::vec::Vec::new()),
            markets: RefCell::new(std::vec::Vec::new()),
            next_salt: Cell::new(1),
        };

        let pool_address: Address = fixture.decode(fixture.execute(
            AdminOperation::DeployPool(pool_hash.clone()),
            &controller_address,
            "deploy_pool",
            vec![env, pool_hash.into_val(env)],
        ));
        let nft_args = DeployPositionNftArgs {
            wasm_hash: nft_hash,
            uri: String::from_str(env, "https://test.xoxno.com/lending/"),
            name: String::from_str(env, "XOXNO Lending Position"),
            symbol: String::from_str(env, "XLP"),
        };
        let nft_address: Address = fixture.decode(fixture.execute(
            AdminOperation::DeployPositionNft(nft_args.clone()),
            &controller_address,
            "deploy_position_nft",
            vec![
                env,
                nft_args.wasm_hash.into_val(env),
                nft_args.uri.into_val(env),
                nft_args.name.into_val(env),
                nft_args.symbol.into_val(env),
            ],
        ));
        let hub_id = fixture.add_hub();
        let spoke_id = fixture.add_spoke();
        fixture.execute(
            AdminOperation::Unpause,
            &controller_address,
            "unpause",
            vec![env],
        );

        Self {
            pool: PoolClient::new(env, &pool_address),
            position_nft: PositionNftClient::new(env, &nft_address),
            hub_id,
            spoke_id,
            ..fixture
        }
    }

    /// The deployment's addresses, for [`XoxnoLending`](crate::XoxnoLending)
    /// inside the contract under test.
    pub fn addresses(&self) -> LendingAddresses {
        LendingAddresses {
            controller: self.controller.address.clone(),
            pool: self.pool.address.clone(),
            position_nft: self.position_nft.address.clone(),
        }
    }

    /// Creates a new hub and returns its id.
    pub fn add_hub(&self) -> u32 {
        self.governance.mock_all_auths().create_hub(&self.admin)
    }

    /// Creates a new spoke with the mainnet "Blue Chip" liquidation curve and
    /// returns its id.
    pub fn add_spoke(&self) -> u32 {
        let spoke_id = self.governance.mock_all_auths().add_spoke(&self.admin);
        let env = &self.env;
        let curve = SpokeLiquidationCurveArgs {
            hf_for_max_bonus_wad: HF_FOR_MAX_BONUS_WAD,
            liquidation_bonus_factor_bps: LIQUIDATION_BONUS_FACTOR_BPS,
            spoke_id,
            target_hf_wad: TARGET_HF_WAD,
        };
        self.execute(
            AdminOperation::SetSpokeLiquidationCurve(curve.clone()),
            &self.controller.address,
            "set_spoke_liquidation_curve",
            vec![
                env,
                curve.spoke_id.into_val(env),
                curve.target_hf_wad.into_val(env),
                curve.hf_for_max_bonus_wad.into_val(env),
                curve.liquidation_bonus_factor_bps.into_val(env),
            ],
        );
        spoke_id
    }

    /// Creates a market in the default hub and spoke; see
    /// [`create_market_in`](Self::create_market_in).
    pub fn create_market(&self, cfg: &MarketConfig) -> Market<'a> {
        self.create_market_in(cfg, self.hub_id, self.spoke_id)
    }

    /// Creates a new token, lists it in `hub_id` and `spoke_id`, configures its
    /// dual-source oracle at `cfg.price_wad`, and supplies
    /// `cfg.initial_liquidity` from a new liquidity provider.
    pub fn create_market_in(&self, cfg: &MarketConfig, hub_id: u32, spoke_id: u32) -> Market<'a> {
        let env = &self.env;
        let asset = env
            .register_stellar_asset_contract_v2(self.admin.clone())
            .address();
        let key = self.create_pool(&asset, hub_id, &cfg.rates);
        self.list_market(&key, spoke_id, &cfg.risk);

        let feed_id = String::from_str(env, cfg.symbol);
        self.prices
            .borrow_mut()
            .push((asset.clone(), feed_id.clone(), cfg.price_wad));
        self.push_price(&asset, &feed_id, cfg.price_wad);
        self.configure_oracle(&asset, &feed_id);

        let market = Market {
            key,
            token: token::TokenClient::new(env, &asset),
            sac: token::StellarAssetClient::new(env, &asset),
            asset,
            feed_id,
        };
        if cfg.initial_liquidity > 0 {
            self.supply_liquidity(&market.key, spoke_id, cfg.initial_liquidity);
        }
        market
    }

    /// Lists the token of `market` in another hub with its own rate curve and
    /// returns the new market key. List it in a spoke with
    /// [`list_market`](Self::list_market) before using it.
    pub fn add_market_to_hub(
        &self,
        market: &Market,
        hub_id: u32,
        rates: &RateModel,
    ) -> HubAssetKey {
        self.create_pool(&market.asset, hub_id, rates)
    }

    /// Lists an existing market in `spoke_id` with `risk`.
    pub fn list_market(&self, market: &HubAssetKey, spoke_id: u32, risk: &RiskConfig) {
        let env = &self.env;
        let args = SpokeAssetArgs {
            asset: market.asset.clone(),
            bonus: risk.liquidation_bonus,
            borrow_cap: risk.borrow_cap,
            can_borrow: risk.can_be_borrowed,
            can_collateral: risk.can_be_collateral,
            frozen: false,
            hub_id: market.hub_id,
            liquidation_fees: risk.liquidation_fees,
            ltv: risk.ltv,
            no_seize: false,
            paused: false,
            spoke_id,
            supply_cap: risk.supply_cap,
            threshold: risk.liquidation_threshold,
        };
        self.execute(
            AdminOperation::AddAssetToSpoke(args.clone()),
            &self.controller.address,
            "add_asset_to_spoke",
            vec![env, args.into_val(env)],
        );
    }

    /// Supplies `amount` of `market` from a new liquidity provider's account in
    /// `spoke_id`, so the market has cash to lend.
    pub fn supply_liquidity(&self, market: &HubAssetKey, spoke_id: u32, amount: i128) {
        let provider = Address::generate(&self.env);
        token::StellarAssetClient::new(&self.env, &market.asset)
            .mock_all_auths()
            .mint(&provider, &amount);
        self.controller.mock_all_auths().supply(
            &provider,
            &NEW_ACCOUNT,
            &spoke_id,
            &vec![&self.env, (market.clone(), amount)],
        );
    }

    /// Sets the USD price of `market`, WAD, on both oracle feeds.
    pub fn set_price(&self, market: &Market, price_wad: i128) {
        for entry in self.prices.borrow_mut().iter_mut() {
            if entry.0 == market.asset {
                entry.2 = price_wad;
            }
        }
        self.push_price(&market.asset, &market.feed_id, price_wad);
    }

    /// Moves the ledger forward by `seconds` (and by one ledger per 5
    /// seconds), publishes every price again at the new time, and accrues
    /// interest on every market.
    pub fn advance_time(&self, seconds: u64) {
        self.env.ledger().with_mut(|l| {
            l.timestamp = l.timestamp.saturating_add(seconds);
            let ledgers = u32::try_from(seconds / SECONDS_PER_LEDGER).unwrap_or(u32::MAX);
            l.sequence_number = l.sequence_number.saturating_add(ledgers);
        });
        for (asset, feed_id, price_wad) in self.prices.borrow().iter() {
            self.push_price(asset, feed_id, *price_wad);
        }
        let mut keys = Vec::new(&self.env);
        for key in self.markets.borrow().iter() {
            keys.push_back(key.clone());
        }
        if !keys.is_empty() {
            self.controller
                .mock_all_auths()
                .update_indexes(&self.admin, &keys);
        }
    }

    fn create_pool(&self, asset: &Address, hub_id: u32, rates: &RateModel) -> HubAssetKey {
        let env = &self.env;
        let params = MarketParamsRaw {
            asset_decimals: TOKEN_DECIMALS,
            asset_id: asset.clone(),
            base_borrow_rate: rates.base_borrow_rate,
            flashloan_fee: rates.flashloan_fee,
            is_flashloanable: rates.is_flashloanable,
            max_borrow_rate: rates.max_borrow_rate,
            max_utilization: rates.max_utilization,
            mid_utilization: rates.mid_utilization,
            optimal_utilization: rates.optimal_utilization,
            reserve_factor: rates.reserve_factor,
            slope1: rates.slope1,
            slope2: rates.slope2,
            slope3: rates.slope3,
        };
        self.execute(
            AdminOperation::CreateLiquidityPool(CreatePoolArgs {
                asset: asset.clone(),
                hub_id,
                params: params.clone(),
            }),
            &self.controller.address,
            "create_liquidity_pool",
            vec![
                env,
                hub_id.into_val(env),
                asset.into_val(env),
                params.into_val(env),
            ],
        );
        let key = HubAssetKey {
            asset: asset.clone(),
            hub_id,
        };
        self.markets.borrow_mut().push(key.clone());
        key
    }

    fn push_price(&self, asset: &Address, feed_id: &String, price_wad: i128) {
        self.reflector.set_price(
            &mock_reflector::ReflectorAsset::Stellar(asset.clone()),
            &price_wad,
        );
        self.redstone.set_price(feed_id, &price_wad);
    }

    fn configure_oracle(&self, asset: &Address, feed_id: &String) {
        let env = &self.env;
        let reflector = PriceSource::Feed(FeedSource {
            decimals: REFLECTOR_DECIMALS,
            max_stale_seconds: PRICE_STALE_SECONDS,
            provider: ProviderRef::Reflector(ReflectorFeedRef {
                asset: OracleAssetRef::Stellar(asset.clone()),
                contract: self.reflector.address.clone(),
                read_mode: OracleReadMode::Twap(TWAP_RECORDS),
            }),
        });
        let redstone = PriceSource::Feed(FeedSource {
            decimals: REDSTONE_DECIMALS,
            max_stale_seconds: PRICE_STALE_SECONDS,
            provider: ProviderRef::RedStone(MultiFeedRef {
                contract: self.redstone.address.clone(),
                feed_id: feed_id.clone(),
                nature: FeedNature::Fundamental,
            }),
        });
        let oracle = AssetOracle {
            asset_decimals: 0,
            independence: IndependencePolicy::RequireDisjoint,
            max_price_stale_seconds: PRICE_STALE_SECONDS,
            max_sanity_price_wad: MAX_PRICE_WAD,
            min_sanity_price_wad: 1,
            sources: vec![env, reflector, redstone],
            tolerance: self.governance.resolve_oracle_tolerance(&TOLERANCE_BPS),
        };
        let key = PriceKey::Token(asset.clone());
        let resolved = self.governance.resolve_asset_oracle(&key, &oracle);
        self.execute(
            AdminOperation::ConfigureAssetOracle(ConfigureAssetOracleArgs {
                key: key.clone(),
                oracle,
            }),
            &self.price_aggregator.address,
            "set_oracle",
            vec![env, key.into_val(env), resolved.into_val(env)],
        );
    }

    /// Proposes `op`, waits out its timelock delay, and executes it. `target`,
    /// `function` and `args` must be the call governance resolves `op` to.
    fn execute(&self, op: AdminOperation, target: &Address, function: &str, args: Vec<Val>) -> Val {
        let env = &self.env;
        let salt = self.next_salt.get();
        self.next_salt.set(salt + 1);
        let mut bytes = [0u8; 32];
        bytes[28..].copy_from_slice(&salt.to_be_bytes());
        let salt = BytesN::from_array(env, &bytes);

        let id = self
            .governance
            .mock_all_auths()
            .propose(&self.admin, &op, &salt);
        let ready = self.governance.get_operation_ledger(&id);
        env.ledger()
            .with_mut(|l| l.sequence_number = l.sequence_number.max(ready));
        self.governance.execute(
            &None,
            target,
            &Symbol::new(env, function),
            &args,
            &BytesN::from_array(env, &[0; 32]),
            &salt,
        )
    }

    fn decode<T: TryFromVal<Env, Val>>(&self, value: Val) -> T {
        T::try_from_val(&self.env, &value)
            .unwrap_or_else(|_| panic!("unexpected governance result"))
    }
}
