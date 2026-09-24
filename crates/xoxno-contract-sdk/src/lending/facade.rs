use soroban_sdk::{
    contracttype, panic_with_error, token, vec, Address, Bytes, Env, Error, Vec, I256,
};

use super::constants::{NEW_ACCOUNT, WITHDRAW_ALL};
use super::controller::{
    self, GenericError, HubAssetKey, LiquidationEstimate, SeizeMode, SpokeAssetConfig, SpokeConfig,
};
use super::helpers::authorize_transfers_as_current;
use super::position_nft::NonFungibleTokenError;
use super::{pool, position_nft, price_aggregator};
use crate::networks;

/// The addresses of one XOXNO Lending deployment. Store it in your contract's
/// instance storage at construction and build [`XoxnoLending`] from it.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LendingAddresses {
    pub controller: Address,
    pub pool: Address,
    pub position_nft: Address,
}

impl LendingAddresses {
    /// The Stellar mainnet deployment.
    pub fn mainnet(env: &Env) -> Self {
        Self {
            controller: Address::from_str(env, networks::mainnet::CONTROLLER),
            pool: Address::from_str(env, networks::mainnet::POOL),
            position_nft: Address::from_str(env, networks::mainnet::POSITION_NFT),
        }
    }

    /// The Stellar testnet deployment.
    pub fn testnet(env: &Env) -> Self {
        Self {
            controller: Address::from_str(env, networks::testnet::CONTROLLER),
            pool: Address::from_str(env, networks::testnet::POOL),
            position_nft: Address::from_str(env, networks::testnet::POSITION_NFT),
        }
    }
}

/// Supplied and borrowed amounts of one account, in token base units.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Position {
    /// Supplied amount per market, interest included.
    pub collateral: Vec<(HubAssetKey, i128)>,
    /// Borrowed amount per market, interest included.
    pub debt: Vec<(HubAssetKey, i128)>,
}

/// The result of a withdrawal.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Withdrawal {
    /// Token base units the current contract received.
    pub amount: i128,
    /// The withdrawal left the account with no supply and no debt: the
    /// protocol deleted the account and burned its position NFT. The id is
    /// no longer valid.
    pub account_closed: bool,
}

/// The result of a batch withdrawal.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Withdrawals {
    /// Token base units the current contract received per market, one entry
    /// per market in request order.
    pub amounts: Vec<(HubAssetKey, i128)>,
    /// As [`Withdrawal::account_closed`].
    pub account_closed: bool,
}

/// XOXNO Lending as the calling contract sees it.
///
/// Every operation acts for the current contract: it is the caller, it pays,
/// and it receives. The account it opens is a position NFT that the current
/// contract owns. Methods that pay (`deposit`, `supply`, `repay`, `liquidate`
/// and their `_batch` forms) create the token-transfer authorizations the
/// controller needs. Each `_batch` method acts on several markets in one call.
///
/// Amounts are token base units. USD values and prices are WAD (1e18). Rates
/// and indexes are RAY (1e27). For a function this type does not wrap, use the
/// generated clients from [`XoxnoLending::controller`], [`XoxnoLending::pool`],
/// [`XoxnoLending::position_nft`] and [`XoxnoLending::price_aggregator`].
#[derive(Clone)]
pub struct XoxnoLending {
    env: Env,
    addresses: LendingAddresses,
}

impl XoxnoLending {
    /// A wrapper around the deployment at `addresses`.
    pub fn new(env: &Env, addresses: &LendingAddresses) -> Self {
        Self {
            env: env.clone(),
            addresses: addresses.clone(),
        }
    }

    /// A wrapper around the Stellar mainnet deployment.
    pub fn mainnet(env: &Env) -> Self {
        Self::new(env, &LendingAddresses::mainnet(env))
    }

    /// A wrapper around the Stellar testnet deployment.
    pub fn testnet(env: &Env) -> Self {
        Self::new(env, &LendingAddresses::testnet(env))
    }

    /// The deployment's addresses.
    pub fn addresses(&self) -> &LendingAddresses {
        &self.addresses
    }

    /// The generated controller client.
    pub fn controller(&self) -> controller::Client<'static> {
        controller::Client::new(&self.env, &self.addresses.controller)
    }

    /// The generated pool client (read-only views).
    pub fn pool(&self) -> pool::Client<'static> {
        pool::Client::new(&self.env, &self.addresses.pool)
    }

    /// The generated position NFT client (ownership views).
    pub fn position_nft(&self) -> position_nft::Client<'static> {
        position_nft::Client::new(&self.env, &self.addresses.position_nft)
    }

    /// The generated price aggregator client. Its address is read from the
    /// controller on every call, because governance can replace it.
    pub fn price_aggregator(&self) -> price_aggregator::Client<'static> {
        price_aggregator::Client::new(&self.env, &self.controller().price_aggregator())
    }

    // ----- Accounts -----------------------------------------------------------

    /// Opens a new account in `spoke_id` with a first supply of `amount` and
    /// returns its id. The id is the position NFT's token id, and the current
    /// contract owns the NFT. Store the id to reuse the account.
    pub fn open_account(&self, spoke_id: u32, market: &HubAssetKey, amount: i128) -> u64 {
        self.deposit(NEW_ACCOUNT, spoke_id, market, amount)
    }

    /// Supplies `amount` of `market` to `account_id` and returns the account
    /// id. With `account_id` = [`NEW_ACCOUNT`] it opens a new account in
    /// `spoke_id` (the returned id is the new position NFT's token id);
    /// otherwise `spoke_id` must be the account's spoke.
    pub fn deposit(
        &self,
        account_id: u64,
        spoke_id: u32,
        market: &HubAssetKey,
        amount: i128,
    ) -> u64 {
        self.deposit_batch(
            account_id,
            spoke_id,
            &vec![&self.env, (market.clone(), amount)],
        )
    }

    /// [`deposit`](Self::deposit) for several markets in one call. A market
    /// listed twice is supplied once with the sum.
    pub fn deposit_batch(
        &self,
        account_id: u64,
        spoke_id: u32,
        assets: &Vec<(HubAssetKey, i128)>,
    ) -> u64 {
        let assets = merge(&self.env, assets);
        self.authorize_pulls(&assets);
        self.controller().supply(
            &self.env.current_contract_address(),
            &account_id,
            &spoke_id,
            &assets,
        )
    }

    /// Supplies `amount` more to an existing account of the current contract.
    pub fn supply(&self, account_id: u64, market: &HubAssetKey, amount: i128) -> u64 {
        self.supply_batch(account_id, &vec![&self.env, (market.clone(), amount)])
    }

    /// [`supply`](Self::supply) for several markets in one call. A market
    /// listed twice is supplied once with the sum.
    pub fn supply_batch(&self, account_id: u64, assets: &Vec<(HubAssetKey, i128)>) -> u64 {
        let spoke_id = self.account_spoke(account_id);
        self.deposit_batch(account_id, spoke_id, assets)
    }

    /// Returns `stored` if that account still exists, otherwise
    /// [`NEW_ACCOUNT`]. A withdrawal or a strategy call that leaves an account
    /// with no supply and no debt deletes it and burns its NFT, so check a
    /// stored id before reuse. A repayment never deletes an account.
    pub fn resolve_account(&self, stored: Option<u64>) -> u64 {
        match stored {
            Some(account_id) if self.controller().account_exists(&account_id) => account_id,
            _ => NEW_ACCOUNT,
        }
    }

    /// Borrows `amount` of `market` against the account; the current contract
    /// receives the tokens.
    pub fn borrow(&self, account_id: u64, market: &HubAssetKey, amount: i128) {
        self.borrow_batch(account_id, &vec![&self.env, (market.clone(), amount)]);
    }

    /// [`borrow`](Self::borrow) for several markets in one call. A market
    /// listed twice is borrowed once with the sum.
    pub fn borrow_batch(&self, account_id: u64, assets: &Vec<(HubAssetKey, i128)>) {
        self.controller().borrow(
            &self.env.current_contract_address(),
            &account_id,
            assets,
            &None,
        );
    }

    /// Repays up to `amount` of the account's debt in `market` from the
    /// current contract, and returns the amount repaid. The pool refunds any
    /// amount above the debt to the current contract; the return value
    /// excludes that refund.
    pub fn repay(&self, account_id: u64, market: &HubAssetKey, amount: i128) -> i128 {
        self.repay_batch(account_id, &vec![&self.env, (market.clone(), amount)])
            .get(0)
            .map_or(0, |(_, repaid)| repaid)
    }

    /// [`repay`](Self::repay) for several markets in one call. Returns the
    /// amount repaid per market, in request order. A market listed twice is
    /// repaid once with the sum. Each token can appear in only one market,
    /// because the refund is measured per token; otherwise it panics with
    /// `InvalidPayments`.
    pub fn repay_batch(
        &self,
        account_id: u64,
        payments: &Vec<(HubAssetKey, i128)>,
    ) -> Vec<(HubAssetKey, i128)> {
        let this = self.env.current_contract_address();
        let payments = merge(&self.env, payments);
        require_one_market_per_token(&self.env, &payments);
        let mut before = Vec::new(&self.env);
        for (market, _) in payments.iter() {
            before.push_back(token::TokenClient::new(&self.env, &market.asset).balance(&this));
        }
        self.authorize_pulls(&payments);
        self.controller().repay(&this, &account_id, &payments);
        let mut repaid = Vec::new(&self.env);
        for (index, (market, _)) in payments.iter().enumerate() {
            let after = token::TokenClient::new(&self.env, &market.asset).balance(&this);
            repaid.push_back((market, before.get_unchecked(index as u32) - after));
        }
        repaid
    }

    /// Withdraws `amount` of `market` to the current contract. The result has
    /// the amount received and whether the withdrawal closed the account.
    pub fn withdraw(&self, account_id: u64, market: &HubAssetKey, amount: i128) -> Withdrawal {
        let withdrawals =
            self.withdraw_batch(account_id, &vec![&self.env, (market.clone(), amount)]);
        Withdrawal {
            amount: withdrawals.amounts.iter().map(|(_, amount)| amount).sum(),
            account_closed: withdrawals.account_closed,
        }
    }

    /// Withdraws the whole supply of `market`. The amount can be 1 unit below
    /// [`collateral`](Self::collateral), which rounds half-up.
    pub fn withdraw_all(&self, account_id: u64, market: &HubAssetKey) -> Withdrawal {
        self.withdraw(account_id, market, WITHDRAW_ALL)
    }

    /// [`withdraw`](Self::withdraw) for several markets in one call.
    /// [`WITHDRAW_ALL`] withdraws the whole supply of its market. A market
    /// listed twice is withdrawn once with the sum, or in full if either
    /// amount is [`WITHDRAW_ALL`].
    pub fn withdraw_batch(
        &self,
        account_id: u64,
        assets: &Vec<(HubAssetKey, i128)>,
    ) -> Withdrawals {
        let amounts = self.controller().withdraw(
            &self.env.current_contract_address(),
            &account_id,
            assets,
            &None,
        );
        Withdrawals {
            amounts,
            account_closed: self.nft_burned(account_id),
        }
    }

    /// Extends the TTL of the account, its positions and its NFT.
    pub fn renew_account(&self, account_id: u64) {
        self.controller()
            .renew_account(&self.env.current_contract_address(), &account_id);
    }

    /// Starts a flash loan of `amount` to `receiver`, which must be a
    /// different contract than the current one.
    pub fn flash_loan(&self, market: &HubAssetKey, amount: i128, receiver: &Address, data: &Bytes) {
        self.controller().flash_loan(
            &self.env.current_contract_address(),
            market,
            &amount,
            receiver,
            data,
        );
    }

    /// Liquidates `account_id` by repaying up to `amount` of its debt in
    /// `market` from the current contract, and returns the amount paid. The
    /// seized collateral goes to the current contract.
    ///
    /// The controller can use less than `amount`. This reads the plan with
    /// `get_liquidation_estimate` first, then offers and authorizes only the
    /// planned amount.
    pub fn liquidate(&self, account_id: u64, market: &HubAssetKey, amount: i128) -> i128 {
        self.liquidate_batch(account_id, &vec![&self.env, (market.clone(), amount)])
            .get(0)
            .map_or(0, |(_, paid)| paid)
    }

    /// [`liquidate`](Self::liquidate) with offers in several debt markets in
    /// one call. Returns the amount paid per market, for the markets the plan
    /// uses, in request order. A market listed twice is offered once with the
    /// sum. Each token can appear in only one market, because the plan reports
    /// refunds per token; otherwise it panics with `InvalidPayments`.
    pub fn liquidate_batch(
        &self,
        account_id: u64,
        payments: &Vec<(HubAssetKey, i128)>,
    ) -> Vec<(HubAssetKey, i128)> {
        let offers = merge(&self.env, payments);
        require_one_market_per_token(&self.env, &offers);
        let estimate =
            self.controller()
                .get_liquidation_estimate(&account_id, &offers, &SeizeMode::Transfer);
        let mut planned = Vec::new(&self.env);
        for (market, offer) in offers.iter() {
            let refund: i128 = estimate
                .refunds
                .iter()
                .filter(|refund| refund.asset == market.asset)
                .map(|refund| refund.amount)
                .sum();
            let paid = offer - refund;
            if paid > 0 {
                planned.push_back((market, paid));
            }
        }
        self.authorize_pulls(&planned);
        self.controller().liquidate(
            &self.env.current_contract_address(),
            &account_id,
            &planned,
            &SeizeMode::Transfer,
        );
        planned
    }

    // ----- Account views ------------------------------------------------------

    /// Whether the account exists.
    pub fn account_exists(&self, account_id: u64) -> bool {
        self.controller().account_exists(&account_id)
    }

    /// The owner of the account: the owner of its position NFT.
    pub fn owner_of(&self, account_id: u64) -> Address {
        self.position_nft().owner_of(&nft_id(&self.env, account_id))
    }

    /// Whether the current contract owns the account.
    pub fn owns(&self, account_id: u64) -> bool {
        u32::try_from(account_id).is_ok_and(|token_id| {
            matches!(
                self.position_nft().try_owner_of(&token_id),
                Ok(Ok(owner)) if owner == self.env.current_contract_address()
            )
        })
    }

    /// The number of accounts `owner` holds.
    pub fn account_count(&self, owner: &Address) -> u32 {
        self.position_nft().balance(owner)
    }

    /// Up to `limit` account ids of `owner`, starting at index `start`.
    pub fn accounts_of(&self, owner: &Address, start: u32, limit: u32) -> Vec<u64> {
        let nft = self.position_nft();
        let end = nft.balance(owner).min(start.saturating_add(limit));
        let mut ids = Vec::new(&self.env);
        for index in start..end {
            ids.push_back(u64::from(nft.get_owner_token_id(owner, &index)));
        }
        ids
    }

    /// The spoke the account was opened in.
    pub fn account_spoke(&self, account_id: u64) -> u32 {
        self.controller()
            .get_account_attributes(&account_id)
            .spoke_id
    }

    /// Every supplied and borrowed amount of the account.
    pub fn position(&self, account_id: u64) -> Position {
        let controller = self.controller();
        let (supplies, borrows) = controller.get_account_positions(&account_id);
        let mut position = Position {
            collateral: Vec::new(&self.env),
            debt: Vec::new(&self.env),
        };
        for market in supplies.keys() {
            let amount = controller.get_collateral_amount(&account_id, &market);
            position.collateral.push_back((market, amount));
        }
        for market in borrows.keys() {
            let amount = controller.get_borrow_amount(&account_id, &market);
            position.debt.push_back((market, amount));
        }
        position
    }

    /// The account's supply of `market`, interest included, rounded half-up.
    pub fn collateral(&self, account_id: u64, market: &HubAssetKey) -> i128 {
        self.controller().get_collateral_amount(&account_id, market)
    }

    /// The account's debt in `market`, interest included.
    pub fn debt(&self, account_id: u64, market: &HubAssetKey) -> i128 {
        self.controller().get_borrow_amount(&account_id, market)
    }

    /// The health factor, WAD; `i128::MAX` without debt. Below 1.0 (`WAD`) the
    /// account is liquidatable.
    pub fn health_factor(&self, account_id: u64) -> i128 {
        self.controller().get_health_factor(&account_id)
    }

    /// Whether the account can be liquidated now.
    pub fn is_liquidatable(&self, account_id: u64) -> bool {
        self.controller().is_liquidatable(&account_id)
    }

    /// Total collateral value, USD WAD.
    pub fn collateral_usd(&self, account_id: u64) -> i128 {
        self.controller().get_total_collateral_usd(&account_id)
    }

    /// Total debt value, USD WAD.
    pub fn debt_usd(&self, account_id: u64) -> i128 {
        self.controller().get_total_borrow_usd(&account_id)
    }

    /// How much more the account can borrow, USD WAD: its LTV-weighted
    /// collateral minus its debt, and 0 when the debt is higher.
    pub fn borrowable_usd(&self, account_id: u64) -> i128 {
        let controller = self.controller();
        let limit = controller.get_ltv_collateral_usd(&account_id);
        let debt = controller.get_total_borrow_usd(&account_id);
        limit.saturating_sub(debt).max(0)
    }

    /// What a liquidation that repays `amount` of `market` would pay out.
    pub fn liquidation_estimate(
        &self,
        account_id: u64,
        market: &HubAssetKey,
        amount: i128,
    ) -> LiquidationEstimate {
        self.controller().get_liquidation_estimate(
            &account_id,
            &vec![&self.env, (market.clone(), amount)],
            &SeizeMode::Transfer,
        )
    }

    // ----- Markets ------------------------------------------------------------

    /// The market key of `asset` in hub `hub_id`.
    pub fn market(&self, hub_id: u32, asset: &Address) -> HubAssetKey {
        HubAssetKey {
            asset: asset.clone(),
            hub_id,
        }
    }

    /// Whether `market` is listed.
    pub fn is_listed(&self, market: &HubAssetKey) -> bool {
        matches!(self.controller().try_get_market_index(market), Ok(Ok(_)))
    }

    /// The first hub in `hub_ids` that lists `asset`.
    pub fn find_market(&self, asset: &Address, hub_ids: &[u32]) -> Option<HubAssetKey> {
        hub_ids
            .iter()
            .map(|hub_id| self.market(*hub_id, asset))
            .find(|market| self.is_listed(market))
    }

    /// The spoke's configuration, or `None` if it does not exist.
    pub fn spoke(&self, spoke_id: u32) -> Option<SpokeConfig> {
        self.controller()
            .try_get_spoke(&spoke_id)
            .ok()
            .and_then(Result::ok)
    }

    /// The market's configuration in the spoke, or `None` if the spoke does
    /// not list it.
    pub fn spoke_asset(&self, spoke_id: u32, market: &HubAssetKey) -> Option<SpokeAssetConfig> {
        self.controller()
            .try_get_spoke_asset(&spoke_id, market)
            .ok()
            .and_then(Result::ok)
    }

    /// Whether a new supply of `market` in `spoke_id` passes the listing
    /// checks: the spoke is active, and the market is listed as collateral and
    /// not paused or frozen. Caps and the account's state are checked by the
    /// controller when you supply.
    pub fn can_supply(&self, spoke_id: u32, market: &HubAssetKey) -> bool {
        self.spoke(spoke_id)
            .is_some_and(|spoke| !spoke.is_deprecated)
            && self
                .spoke_asset(spoke_id, market)
                .is_some_and(|asset| asset.is_collateralizable && !asset.paused && !asset.frozen)
    }

    /// Whether a new borrow of `market` in `spoke_id` passes the listing
    /// checks: the market is listed as borrowable and not paused or frozen.
    pub fn can_borrow(&self, spoke_id: u32, market: &HubAssetKey) -> bool {
        self.spoke_asset(spoke_id, market)
            .is_some_and(|asset| asset.is_borrowable && !asset.paused && !asset.frozen)
    }

    /// The annual supply rate, RAY.
    pub fn supply_rate(&self, market: &HubAssetKey) -> i128 {
        self.pool().get_deposit_rate(&pool_key(market))
    }

    /// The annual borrow rate, RAY.
    pub fn borrow_rate(&self, market: &HubAssetKey) -> i128 {
        self.pool().get_borrow_rate(&pool_key(market))
    }

    /// The market's utilization, RAY.
    pub fn utilization(&self, market: &HubAssetKey) -> i128 {
        self.pool().get_utilisation(&pool_key(market))
    }

    /// Cash the pool holds for the market, token base units.
    pub fn liquidity(&self, market: &HubAssetKey) -> i128 {
        self.pool().get_reserves(&pool_key(market))
    }

    /// The supply index, RAY. One supply share is worth `supply_index / RAY`
    /// token units; it only grows while the market is solvent.
    pub fn supply_index(&self, market: &HubAssetKey) -> i128 {
        self.controller().get_market_index(market).supply_index
    }

    // ----- Prices -------------------------------------------------------------

    /// The USD price of `asset`, WAD. Panics if the price is not usable.
    pub fn price(&self, asset: &Address) -> i128 {
        self.price_feed(asset).price_wad
    }

    /// The USD price of `asset`, WAD, or `None` if it is not usable now
    /// (stale, deviating, or not configured).
    pub fn try_price(&self, asset: &Address) -> Option<i128> {
        let status = self.quote(asset);
        status.valid.then_some(status.final_wad)
    }

    /// The full price status of `asset`: validity, staleness, deviation, both
    /// source prices and the error code. It never panics on a bad price.
    pub fn quote(&self, asset: &Address) -> price_aggregator::PriceStatus {
        let key = price_aggregator::PriceKey::Token(asset.clone());
        let quotes = self
            .price_aggregator()
            .quotes(&vec![&self.env, key.clone()]);
        quotes
            .get(key)
            .unwrap_or_else(|| panic!("no quote for the asset"))
    }

    /// The USD value of `amount` base units of `asset`, WAD, rounded down.
    /// Panics if the price is not usable.
    pub fn value_usd(&self, asset: &Address, amount: i128) -> i128 {
        let feed = self.price_feed(asset);
        let scale = I256::from_i128(&self.env, 10).pow(feed.asset_decimals);
        I256::from_i128(&self.env, amount)
            .mul(&I256::from_i128(&self.env, feed.price_wad))
            .div(&scale)
            .to_i128()
            .unwrap_or_else(|| panic!("USD value overflows i128"))
    }

    fn price_feed(&self, asset: &Address) -> price_aggregator::PriceFeedRaw {
        let key = price_aggregator::PriceKey::Token(asset.clone());
        let feeds = self
            .price_aggregator()
            .prices(&vec![&self.env, key.clone()]);
        feeds
            .get(key)
            .unwrap_or_else(|| panic!("no price for the asset"))
    }
}

impl XoxnoLending {
    /// Authorizes one `transfer(current contract, pool, amount)` per entry
    /// inside the next call.
    fn authorize_pulls(&self, pulls: &Vec<(HubAssetKey, i128)>) {
        let mut transfers = Vec::new(&self.env);
        for (market, amount) in pulls.iter() {
            transfers.push_back((market.asset, amount));
        }
        authorize_transfers_as_current(
            &self.env,
            &self.env.current_contract_address(),
            &self.addresses.pool,
            &transfers,
        );
    }
}

/// `assets` with each market once and its amounts summed, in first-appearance
/// order: the order and amounts the controller uses.
fn merge(env: &Env, assets: &Vec<(HubAssetKey, i128)>) -> Vec<(HubAssetKey, i128)> {
    let mut merged: Vec<(HubAssetKey, i128)> = Vec::new(env);
    for (market, amount) in assets.iter() {
        match (0..merged.len()).find(|&index| merged.get_unchecked(index).0 == market) {
            Some(index) => {
                let sum = merged
                    .get_unchecked(index)
                    .1
                    .checked_add(amount)
                    .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));
                merged.set(index, (market, sum));
            }
            None => merged.push_back((market, amount)),
        }
    }
    merged
}

fn require_one_market_per_token(env: &Env, payments: &Vec<(HubAssetKey, i128)>) {
    for i in 0..payments.len() {
        for j in (i + 1)..payments.len() {
            if payments.get_unchecked(i).0.asset == payments.get_unchecked(j).0.asset {
                panic_with_error!(env, GenericError::InvalidPayments);
            }
        }
    }
}

fn pool_key(market: &HubAssetKey) -> pool::HubAssetKey {
    pool::HubAssetKey {
        asset: market.asset.clone(),
        hub_id: market.hub_id,
    }
}

impl XoxnoLending {
    /// Whether the account's position NFT no longer exists: `owner_of` fails
    /// with `NonExistentToken`. Any other failure is not a burn.
    fn nft_burned(&self, account_id: u64) -> bool {
        let missing = Error::from_contract_error(NonFungibleTokenError::NonExistentToken as u32);
        matches!(
            self.position_nft().try_owner_of(&nft_id(&self.env, account_id)),
            Err(Ok(error)) if error == missing
        )
    }
}

fn nft_id(env: &Env, account_id: u64) -> u32 {
    u32::try_from(account_id)
        .unwrap_or_else(|_| panic_with_error!(env, GenericError::AccountNotFound))
}
