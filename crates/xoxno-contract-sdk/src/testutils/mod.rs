//! A test fixture that deploys XOXNO Lending from the embedded WASM.
//!
//! Enable the `testutils` feature in `[dev-dependencies]`:
//!
//! ```toml
//! [dev-dependencies]
//! xoxno-contract-sdk = { version = "0.2", features = ["testutils"] }
//! ```
//!
//! [`LendingFixture::deploy`] builds the protocol the way mainnet runs it:
//! governance owns the controller and the price aggregator, and every
//! configuration step goes through the governance timelock. Prices come from
//! two mock oracles, a Reflector-style feed and a RedStone-style feed, which
//! [`LendingFixture::set_price`] moves together.

mod fixture;
mod market;

pub use fixture::LendingFixture;
pub use market::{Market, MarketConfig, RateModel, RiskConfig};

/// Governance and timelock. The fixture's admin holds every role.
#[allow(clippy::too_many_arguments)]
pub mod governance {
    mod spec {
        soroban_sdk::contractimport!(file = "wasm/governance.wasm");
    }
    pub use spec::*;

    /// The deployed contract code.
    pub const WASM: &[u8] = include_bytes!("../../wasm/deploy/governance.wasm");
}

/// Reflector-style price oracle for tests: base USD, 14 decimals, 300 s
/// resolution.
#[allow(clippy::too_many_arguments)]
pub mod mock_reflector {
    soroban_sdk::contractimport!(file = "wasm/mock_reflector.wasm");
}

/// RedStone-style price feed for tests: 8 decimals.
#[allow(clippy::too_many_arguments)]
pub mod mock_redstone {
    soroban_sdk::contractimport!(file = "wasm/mock_redstone.wasm");
}
