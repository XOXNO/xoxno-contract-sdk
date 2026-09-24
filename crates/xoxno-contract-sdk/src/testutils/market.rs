use soroban_sdk::{token, Address, String};

use crate::lending::constants::{RAY, WAD};
use crate::lending::controller::HubAssetKey;

/// Interest rate curve and flash-loan settings of a market. Rates and
/// utilizations are RAY; the reserve factor and the flash-loan fee are BPS.
#[derive(Clone, Debug)]
pub struct RateModel {
    pub max_borrow_rate: i128,
    pub base_borrow_rate: i128,
    pub slope1: i128,
    pub slope2: i128,
    pub slope3: i128,
    pub mid_utilization: i128,
    pub optimal_utilization: i128,
    pub max_utilization: i128,
    pub reserve_factor: u32,
    pub is_flashloanable: bool,
    pub flashloan_fee: u32,
}

/// Risk settings of a market in the fixture's spoke. Ratios are BPS; caps are
/// token base units.
#[derive(Clone, Debug)]
pub struct RiskConfig {
    pub ltv: u32,
    pub liquidation_threshold: u32,
    pub liquidation_bonus: u32,
    pub liquidation_fees: u32,
    pub can_be_collateral: bool,
    pub can_be_borrowed: bool,
    pub supply_cap: i128,
    pub borrow_cap: i128,
}

/// A market for [`LendingFixture::create_market`](super::LendingFixture::create_market).
///
/// The token is a Stellar Asset Contract with 7 decimals.
#[derive(Clone, Debug)]
pub struct MarketConfig {
    /// Symbol of the token; also the RedStone feed id.
    pub symbol: &'static str,
    /// Initial USD price, WAD.
    pub price_wad: i128,
    /// Tokens a liquidity provider supplies when the market is created, in
    /// base units. 0 creates an empty market.
    pub initial_liquidity: i128,
    pub rates: RateModel,
    pub risk: RiskConfig,
}

const UNIT: i128 = 10_000_000;

impl MarketConfig {
    /// USDC at $1 with the mainnet rate curve and the mainnet "Blue Chip"
    /// spoke settings, seeded with 1,000,000 USDC.
    pub fn usdc() -> Self {
        Self {
            symbol: "USDC",
            price_wad: WAD,
            initial_liquidity: 1_000_000 * UNIT,
            rates: RateModel {
                max_borrow_rate: RAY / 4,
                base_borrow_rate: 0,
                slope1: RAY / 40,
                slope2: RAY / 40,
                slope3: RAY / 5,
                mid_utilization: RAY * 46 / 100,
                optimal_utilization: RAY * 92 / 100,
                max_utilization: RAY * 95 / 100,
                reserve_factor: 1_000,
                is_flashloanable: true,
                flashloan_fee: 9,
            },
            risk: RiskConfig {
                ltv: 7_600,
                liquidation_threshold: 8_000,
                liquidation_bonus: 400,
                liquidation_fees: 1_000,
                can_be_collateral: true,
                can_be_borrowed: true,
                supply_cap: 5_000_000 * UNIT,
                borrow_cap: 3_750_000 * UNIT,
            },
        }
    }

    /// XLM at $0.10 with the mainnet rate curve and the mainnet "Blue Chip"
    /// spoke settings, seeded with 10,000,000 XLM.
    pub fn xlm() -> Self {
        Self {
            symbol: "XLM",
            price_wad: WAD / 10,
            initial_liquidity: 10_000_000 * UNIT,
            rates: RateModel {
                max_borrow_rate: RAY * 64 / 100,
                base_borrow_rate: 0,
                slope1: RAY / 50,
                slope2: RAY / 50,
                slope3: RAY * 6 / 10,
                mid_utilization: RAY * 4 / 10,
                optimal_utilization: RAY * 8 / 10,
                max_utilization: RAY * 9 / 10,
                reserve_factor: 2_000,
                is_flashloanable: true,
                flashloan_fee: 9,
            },
            risk: RiskConfig {
                ltv: 7_500,
                liquidation_threshold: 7_800,
                liquidation_bonus: 900,
                liquidation_fees: 1_200,
                can_be_collateral: true,
                can_be_borrowed: true,
                supply_cap: 50_000_000 * UNIT,
                borrow_cap: 30_000_000 * UNIT,
            },
        }
    }
}

/// A market created by the fixture.
pub struct Market<'a> {
    /// Token contract address.
    pub asset: Address,
    /// Market key for controller calls.
    pub key: HubAssetKey,
    /// Token client for balances and transfers.
    pub token: token::TokenClient<'a>,
    /// Stellar Asset Contract admin client, for `mint`.
    pub sac: token::StellarAssetClient<'a>,
    /// RedStone feed id of the market.
    pub feed_id: String,
}
