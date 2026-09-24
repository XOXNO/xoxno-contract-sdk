//! Units and limits of XOXNO Lending.
//!
//! Token amounts and caps are in token base units. USD values, prices and the
//! health factor are WAD. Shares, indexes and rates are RAY. Risk parameters
//! and fees are BPS.

/// Fixed-point scale of shares, indexes and rates: 1e27.
pub const RAY: i128 = 1_000_000_000_000_000_000_000_000_000;

/// Fixed-point scale of USD values, prices and the health factor: 1e18.
pub const WAD: i128 = 1_000_000_000_000_000_000;

/// Scale of risk parameters and fees: 10_000 is 100%.
pub const BPS: i128 = 10_000;

/// Decimal places of [`RAY`].
pub const RAY_DECIMALS: u32 = 27;

/// Decimal places of [`WAD`].
pub const WAD_DECIMALS: u32 = 18;

/// Account id that opens a new account in `supply`, `multiply`,
/// `flash_position` and `migrate_from_blend`.
pub const NEW_ACCOUNT: u64 = 0;

/// Withdraw amount that withdraws the whole supply position.
pub const WITHDRAW_ALL: i128 = 0;

/// Maximum number of delegates on one account.
pub const MAX_DELEGATES: u32 = 16;

/// Maximum number of inputs to one controller view call.
pub const MAX_VIEW_INPUTS: u32 = 256;

/// Upper bound of the per-account supply and borrow position limits.
pub const POSITION_LIMIT_MAX: u32 = 5;
