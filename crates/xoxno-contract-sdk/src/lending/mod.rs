//! Clients, types and WASM for the deployed XOXNO Lending contracts.
//!
//! Each contract module is generated with `contractimport!` from the WASM in
//! `wasm/`, which has the same code as the contract deployed on mainnet (see
//! `wasm/MANIFEST.json`). A module exports its `Client`, its `WASM` bytes and
//! every type and error in its contract spec.
//!
//! The types of each module are separate Rust types: `controller::HubAssetKey`
//! and `pool::HubAssetKey` do not convert into each other. Use the
//! [`controller`] types when you call the controller.
//!
//! Each module's `Client` holds only the functions a builder calls: the
//! controller's user, keeper and view functions, and read-only views of the
//! pool, the position NFT and the price aggregator. `scripts/gen_clients.py`
//! generates these clients from the same WASM, so their signatures match it.

mod callbacks;
pub mod constants;
pub mod helpers;

pub use callbacks::{
    FlashLoanReceiver, FlashLoanReceiverClient, FlashPositionReceiver, FlashPositionReceiverClient,
};

/// The controller: accounts, supply, borrow, repay, withdraw, liquidation,
/// flash loans, strategies and every account view.
#[allow(clippy::too_many_arguments)]
pub mod controller {
    mod spec {
        soroban_sdk::contractimport!(file = "wasm/controller.wasm");
    }
    pub use spec::*;

    include!("clients/controller.rs");
}

/// The liquidity pool. Integrators use its views only; every mutating
/// function is restricted to the controller.
#[allow(clippy::too_many_arguments)]
pub mod pool {
    mod spec {
        soroban_sdk::contractimport!(file = "wasm/pool.wasm");
    }
    pub use spec::*;

    include!("clients/pool.rs");
}

/// The position NFT. The token id is the account id, and the token owner owns
/// the account.
#[allow(clippy::too_many_arguments)]
pub mod position_nft {
    mod spec {
        soroban_sdk::contractimport!(file = "wasm/position_nft.wasm");
    }
    pub use spec::*;

    include!("clients/position_nft.rs");
}

/// The price aggregator. Read its address with
/// `ControllerClient::price_aggregator`, because governance can replace it.
#[allow(clippy::too_many_arguments)]
pub mod price_aggregator {
    mod spec {
        soroban_sdk::contractimport!(file = "wasm/price_aggregator.wasm");
    }
    pub use spec::*;

    include!("clients/price_aggregator.rs");
}

pub use controller::Client as ControllerClient;
pub use pool::Client as PoolClient;
pub use position_nft::Client as PositionNftClient;
pub use price_aggregator::Client as PriceAggregatorClient;
