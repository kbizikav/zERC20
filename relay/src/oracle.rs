// SPDX-License-Identifier: BUSL-1.1

use std::{
    collections::HashSet,
    sync::Arc,
    time::{Duration, Instant},
};

use alloy::{
    primitives::{Address, U256, address},
    providers::Provider,
    sol,
};
use anyhow::{Context, Result, bail};
use client_common::tokens::{TokenEntry, TokenType};
use tokio::sync::RwLock;

sol! {
    #[sol(rpc)]
    interface AggregatorV3 {
        function latestRoundData() external view returns (
            uint80 roundId,
            int256 answer,
            uint256 startedAt,
            uint256 updatedAt,
            uint80 answeredInRound
        );
        function decimals() external view returns (uint8);
    }
}

/// How long a cached price is considered fresh.
const CACHE_TTL: Duration = Duration::from_secs(300); // 5 min
/// Maximum staleness before falling back to hardcoded prices.
const CACHE_MAX_STALE: Duration = Duration::from_secs(1800); // 30 min
/// Background poll interval.
const POLL_INTERVAL: Duration = Duration::from_secs(30);

/// Price feed identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PriceFeed {
    EthUsd,
    BnbUsd,
}

impl PriceFeed {
    fn label(self) -> &'static str {
        match self {
            Self::EthUsd => "ETH/USD",
            Self::BnbUsd => "BNB/USD",
        }
    }
}

#[derive(Debug, Clone)]
struct CachedPrice {
    /// Price with `feed_decimals` precision (e.g. 8 for Chainlink).
    price: U256,
    feed_decimals: u8,
    updated_at: Instant,
}

/// Ethereum mainnet Chainlink feed address for a given price feed.
/// Prices are chain-agnostic, so we always query mainnet.
fn chainlink_address(feed: PriceFeed) -> Address {
    match feed {
        PriceFeed::EthUsd => address!("5f4eC3Df9cbd43714FE2740f5E3616155c5b8419"),
        PriceFeed::BnbUsd => address!("14e613AC84a31f709eadbdF89C6CC390fDc9540A"),
    }
}

/// Hardcoded conservative fallback prices (8 decimals, Chainlink convention).
/// Set conservatively high so relayer overcharges rather than undercharges.
fn fallback_price(feed: PriceFeed) -> U256 {
    match feed {
        // $4000 — conservative high for ETH
        PriceFeed::EthUsd => U256::from(4000_0000_0000u64),
        // $800 — conservative high for BNB
        PriceFeed::BnbUsd => U256::from(800_0000_0000u64),
    }
}

/// Which feeds are needed for a given token type on a given chain.
fn required_feeds(chain_id: u64, token_type: TokenType) -> Vec<PriceFeed> {
    match token_type {
        // zETH on ETH-gas chains: 1:1, no feed needed
        TokenType::Eth if is_eth_gas_chain(chain_id) => vec![],
        // zBNB on BSC: 1:1, no feed needed
        TokenType::Bnb if chain_id == 56 => vec![],
        // zUSDC on ETH-gas chains: need ETH/USD
        TokenType::Usdc if is_eth_gas_chain(chain_id) => vec![PriceFeed::EthUsd],
        // zBNB on ETH-gas chains: need ETH/USD and BNB/USD
        TokenType::Bnb if is_eth_gas_chain(chain_id) => {
            vec![PriceFeed::EthUsd, PriceFeed::BnbUsd]
        }
        _ => vec![],
    }
}

fn is_eth_gas_chain(chain_id: u64) -> bool {
    matches!(
        chain_id,
        1 | 42161 | 8453 | 10 | 11155111 | 421614 | 11155420 | 84532
    )
}

/// Shared price oracle with background updating and caching.
///
/// All prices are fetched from Ethereum mainnet Chainlink feeds, regardless
/// of which chain the relay is operating on. This allows testnet deployments
/// to use real-world prices.
pub struct PriceOracle {
    cache: Arc<RwLock<Vec<(PriceFeed, CachedPrice)>>>,
}

impl PriceOracle {
    /// Create a new oracle.
    ///
    /// `oracle_rpc_url` — explicit RPC for querying Chainlink (Ethereum mainnet).
    /// If `None`, falls back to the first RPC URL from a mainnet-like token entry.
    ///
    /// Discovers which feeds are needed based on token_type and chain_id,
    /// then kicks off a background updater.
    pub fn new(tokens: &[TokenEntry], oracle_rpc_url: Option<&str>) -> Result<Self> {
        let oracle = Self {
            cache: Arc::new(RwLock::new(Vec::new())),
        };

        let needed_feeds = Self::discover_feeds(tokens);
        if needed_feeds.is_empty() {
            return Ok(oracle);
        }

        let rpc_url = Self::resolve_rpc_url(oracle_rpc_url, tokens)?;
        log::info!("Price oracle using RPC: {}", rpc_url);

        let targets: Vec<(PriceFeed, Address)> = needed_feeds
            .into_iter()
            .map(|feed| (feed, chainlink_address(feed)))
            .collect();

        oracle.spawn_updater(targets, rpc_url);
        Ok(oracle)
    }

    /// Create an oracle with pre-seeded prices (for testing).
    #[cfg(test)]
    fn with_prices(prices: Vec<(PriceFeed, U256, u8)>) -> Self {
        let cache: Vec<(PriceFeed, CachedPrice)> = prices
            .into_iter()
            .map(|(feed, price, decimals)| {
                (
                    feed,
                    CachedPrice {
                        price,
                        feed_decimals: decimals,
                        updated_at: Instant::now(),
                    },
                )
            })
            .collect();
        Self {
            cache: Arc::new(RwLock::new(cache)),
        }
    }

    /// Collect the unique set of price feeds needed across all tokens.
    fn discover_feeds(tokens: &[TokenEntry]) -> Vec<PriceFeed> {
        let mut seen = HashSet::new();
        for token in tokens {
            let Some(tt) = token.token_type else {
                continue;
            };
            for feed in required_feeds(token.chain_id, tt) {
                seen.insert(feed);
            }
        }
        seen.into_iter().collect()
    }

    /// Resolve which RPC URL to use for Chainlink queries.
    ///
    /// Priority: explicit `ORACLE_RPC_URL` > chain 1 token RPC > any mainnet
    /// ETH-gas chain RPC (42161, 8453, etc.) > error.
    fn resolve_rpc_url(explicit: Option<&str>, tokens: &[TokenEntry]) -> Result<String> {
        if let Some(url) = explicit {
            return Ok(url.to_string());
        }

        // Prefer Ethereum mainnet (chain 1)
        if let Some(t) = tokens.iter().find(|t| t.chain_id == 1)
            && let Some(url) = t.rpc_urls.first()
        {
            return Ok(url.clone());
        }

        // Fall back to any mainnet ETH-gas chain (Arbitrum, Base, Optimism)
        // These chains also have Chainlink feeds, but we use mainnet addresses,
        // so this fallback only works if the RPC serves Ethereum mainnet.
        // In practice, users should set ORACLE_RPC_URL for testnet-only configs.
        bail!(
            "no ORACLE_RPC_URL set and no Ethereum mainnet (chain 1) token configured; \
             set ORACLE_RPC_URL to an Ethereum mainnet RPC endpoint"
        )
    }

    fn spawn_updater(&self, targets: Vec<(PriceFeed, Address)>, rpc_url: String) {
        let cache = self.cache.clone();
        tokio::spawn(async move {
            let Ok(url) = rpc_url.parse() else {
                log::error!("invalid oracle RPC URL: {}", rpc_url);
                return;
            };
            let provider = alloy::providers::ProviderBuilder::new().connect_http(url);

            loop {
                for (feed, addr) in &targets {
                    match fetch_price(*addr, &provider).await {
                        Ok((price, decimals)) => {
                            let mut c = cache.write().await;
                            // Upsert
                            if let Some(entry) = c.iter_mut().find(|(f, _)| f == feed) {
                                entry.1 = CachedPrice {
                                    price,
                                    feed_decimals: decimals,
                                    updated_at: Instant::now(),
                                };
                            } else {
                                c.push((
                                    *feed,
                                    CachedPrice {
                                        price,
                                        feed_decimals: decimals,
                                        updated_at: Instant::now(),
                                    },
                                ));
                            }
                            log::debug!("updated {}: {}", feed.label(), price);
                        }
                        Err(err) => {
                            log::warn!("failed to fetch {}: {:#}", feed.label(), err);
                        }
                    }
                }
                tokio::time::sleep(POLL_INTERVAL).await;
            }
        });
    }

    /// Get a cached price for the given feed.
    ///
    /// Returns (price, feed_decimals). Falls back to hardcoded prices if the
    /// cache is too stale or empty.
    async fn get_price(&self, feed: PriceFeed) -> (U256, u8) {
        let cache = self.cache.read().await;
        if let Some((_, cached)) = cache.iter().find(|(f, _)| *f == feed) {
            let age = cached.updated_at.elapsed();
            if age < CACHE_TTL {
                return (cached.price, cached.feed_decimals);
            }
            if age < CACHE_MAX_STALE {
                log::warn!(
                    "using stale {} price (age: {:.0}s)",
                    feed.label(),
                    age.as_secs_f64()
                );
                return (cached.price, cached.feed_decimals);
            }
        }
        log::warn!("no cached {} price, using fallback", feed.label());
        (fallback_price(feed), 8)
    }

    /// Check whether all required price feeds for a (chain, token_type) pair
    /// have fresh cached data. Returns `false` if any feed is stale or missing.
    pub async fn has_fresh_prices(&self, chain_id: u64, token_type: TokenType) -> bool {
        let feeds = required_feeds(chain_id, token_type);
        let cache = self.cache.read().await;
        for feed in feeds {
            match cache.iter().find(|(f, _)| *f == feed) {
                Some((_, cached)) if cached.updated_at.elapsed() < CACHE_TTL => {}
                _ => return false,
            }
        }
        true
    }

    /// Convert a token amount (smallest unit) back to native wei.
    pub async fn convert_token_to_native(
        &self,
        chain_id: u64,
        token_type: TokenType,
        token_amount: U256,
    ) -> Result<U256> {
        match token_type {
            TokenType::Eth if is_eth_gas_chain(chain_id) => Ok(token_amount),
            TokenType::Bnb if chain_id == 56 => Ok(token_amount),

            TokenType::Usdc if is_eth_gas_chain(chain_id) => {
                let (eth_price, dec) = self.get_price(PriceFeed::EthUsd).await;
                if eth_price.is_zero() {
                    bail!("ETH/USD price is zero");
                }
                let multiplier = U256::from(10u64).pow(U256::from(12 + dec));
                Ok(token_amount * multiplier / eth_price)
            }

            TokenType::Bnb if is_eth_gas_chain(chain_id) => {
                let (eth_price, eth_dec) = self.get_price(PriceFeed::EthUsd).await;
                let (bnb_price, bnb_dec) = self.get_price(PriceFeed::BnbUsd).await;
                if eth_price.is_zero() {
                    bail!("ETH/USD price is zero");
                }
                if eth_dec != bnb_dec {
                    bail!(
                        "feed decimals mismatch: ETH/USD has {}, BNB/USD has {}",
                        eth_dec,
                        bnb_dec
                    );
                }
                Ok(token_amount * bnb_price / eth_price)
            }

            _ => {
                log::warn!(
                    "no reverse conversion rule for {:?} on chain {}; returning raw amount",
                    token_type,
                    chain_id
                );
                Ok(token_amount)
            }
        }
    }

    /// Convert a native gas cost (in wei) to the token's smallest unit.
    pub async fn convert_native_to_token(
        &self,
        chain_id: u64,
        token_type: TokenType,
        native_wei: U256,
    ) -> Result<U256> {
        match token_type {
            TokenType::Eth if is_eth_gas_chain(chain_id) => Ok(native_wei),
            TokenType::Bnb if chain_id == 56 => Ok(native_wei),

            TokenType::Usdc if is_eth_gas_chain(chain_id) => {
                let (eth_price, dec) = self.get_price(PriceFeed::EthUsd).await;
                let divisor = U256::from(10u64).pow(U256::from(12 + dec));
                Ok(native_wei * eth_price / divisor)
            }

            TokenType::Bnb if is_eth_gas_chain(chain_id) => {
                let (eth_price, eth_dec) = self.get_price(PriceFeed::EthUsd).await;
                let (bnb_price, bnb_dec) = self.get_price(PriceFeed::BnbUsd).await;
                if bnb_price.is_zero() {
                    bail!("BNB/USD price is zero");
                }
                if eth_dec != bnb_dec {
                    bail!(
                        "feed decimals mismatch: ETH/USD has {}, BNB/USD has {}",
                        eth_dec,
                        bnb_dec
                    );
                }
                Ok(native_wei * eth_price / bnb_price)
            }

            _ => {
                log::warn!(
                    "no conversion rule for {:?} on chain {}; returning raw gas cost",
                    token_type,
                    chain_id
                );
                Ok(native_wei)
            }
        }
    }
}

/// Fetch latest price from a Chainlink AggregatorV3 feed.
async fn fetch_price(feed_address: Address, provider: &impl Provider) -> Result<(U256, u8)> {
    let aggregator = AggregatorV3::new(feed_address, provider);

    let decimals = aggregator
        .decimals()
        .call()
        .await
        .context("failed to call decimals()")?;
    if decimals > 18 {
        bail!("Chainlink returned unsupported feed decimals: {}", decimals);
    }

    let round = aggregator
        .latestRoundData()
        .call()
        .await
        .context("failed to call latestRoundData()")?;

    let answer = round.answer;
    if answer <= alloy::primitives::I256::ZERO {
        bail!("Chainlink returned non-positive price: {}", answer);
    }

    let price = U256::from_be_bytes(answer.into_raw().to_be_bytes::<32>());
    Ok((price, decimals))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: 1 ETH in wei.
    fn one_eth() -> U256 {
        U256::from(10u64).pow(U256::from(18))
    }

    /// Chainlink price with 8 decimals (e.g. $3000 = 3000_00000000).
    fn usd(dollars: u64) -> U256 {
        U256::from(dollars) * U256::from(10u64).pow(U256::from(8))
    }

    #[tokio::test]
    async fn eth_on_eth_chain_is_1_to_1() {
        let oracle = PriceOracle::with_prices(vec![]);
        let result = oracle
            .convert_native_to_token(1, TokenType::Eth, one_eth())
            .await
            .unwrap();
        assert_eq!(result, one_eth(), "zETH on Ethereum should be 1:1");
    }

    #[tokio::test]
    async fn eth_on_testnet_is_1_to_1() {
        let oracle = PriceOracle::with_prices(vec![]);
        let result = oracle
            .convert_native_to_token(421614, TokenType::Eth, one_eth())
            .await
            .unwrap();
        assert_eq!(result, one_eth(), "zETH on Arbitrum Sepolia should be 1:1");
    }

    #[tokio::test]
    async fn bnb_on_bsc_is_1_to_1() {
        let oracle = PriceOracle::with_prices(vec![]);
        let result = oracle
            .convert_native_to_token(56, TokenType::Bnb, one_eth())
            .await
            .unwrap();
        assert_eq!(result, one_eth(), "zBNB on BSC should be 1:1");
    }

    #[tokio::test]
    async fn usdc_on_eth_chain_converts_correctly() {
        let oracle = PriceOracle::with_prices(vec![(PriceFeed::EthUsd, usd(3000), 8)]);

        let gas_cost = one_eth() / U256::from(1000);
        let result = oracle
            .convert_native_to_token(1, TokenType::Usdc, gas_cost)
            .await
            .unwrap();

        assert_eq!(result, U256::from(3_000_000u64));
    }

    #[tokio::test]
    async fn usdc_on_testnet_converts_correctly() {
        // Same price feed works for testnet chains
        let oracle = PriceOracle::with_prices(vec![(PriceFeed::EthUsd, usd(3000), 8)]);

        let gas_cost = one_eth() / U256::from(1000);
        let result = oracle
            .convert_native_to_token(421614, TokenType::Usdc, gas_cost)
            .await
            .unwrap();

        assert_eq!(result, U256::from(3_000_000u64));
    }

    #[tokio::test]
    async fn usdc_on_eth_chain_1_eth() {
        let oracle = PriceOracle::with_prices(vec![(PriceFeed::EthUsd, usd(2500), 8)]);

        let result = oracle
            .convert_native_to_token(1, TokenType::Usdc, one_eth())
            .await
            .unwrap();

        assert_eq!(result, U256::from(2_500_000_000u64));
    }

    #[tokio::test]
    async fn bnb_on_eth_chain_converts_correctly() {
        let oracle = PriceOracle::with_prices(vec![
            (PriceFeed::EthUsd, usd(3000), 8),
            (PriceFeed::BnbUsd, usd(600), 8),
        ]);

        let result = oracle
            .convert_native_to_token(1, TokenType::Bnb, one_eth())
            .await
            .unwrap();

        assert_eq!(result, U256::from(5u64) * one_eth());
    }

    #[tokio::test]
    async fn bnb_on_eth_chain_fractional() {
        let oracle = PriceOracle::with_prices(vec![
            (PriceFeed::EthUsd, usd(3000), 8),
            (PriceFeed::BnbUsd, usd(500), 8),
        ]);

        let gas_cost = one_eth() / U256::from(100);
        let result = oracle
            .convert_native_to_token(42161, TokenType::Bnb, gas_cost)
            .await
            .unwrap();

        let expected = one_eth() * U256::from(6) / U256::from(100);
        assert_eq!(result, expected);
    }

    #[tokio::test]
    async fn fallback_price_used_when_cache_empty() {
        let oracle = PriceOracle::with_prices(vec![]);

        let gas_cost = one_eth() / U256::from(1000);
        let result = oracle
            .convert_native_to_token(1, TokenType::Usdc, gas_cost)
            .await
            .unwrap();

        assert_eq!(result, U256::from(4_000_000u64));
    }

    // ---- convert_token_to_native tests ----

    #[tokio::test]
    async fn token_to_native_eth_1_to_1() {
        let oracle = PriceOracle::with_prices(vec![]);
        let result = oracle
            .convert_token_to_native(1, TokenType::Eth, one_eth())
            .await
            .unwrap();
        assert_eq!(result, one_eth());
    }

    #[tokio::test]
    async fn token_to_native_bnb_on_bsc_1_to_1() {
        let oracle = PriceOracle::with_prices(vec![]);
        let result = oracle
            .convert_token_to_native(56, TokenType::Bnb, one_eth())
            .await
            .unwrap();
        assert_eq!(result, one_eth());
    }

    #[tokio::test]
    async fn token_to_native_usdc_roundtrip() {
        let oracle = PriceOracle::with_prices(vec![(PriceFeed::EthUsd, usd(3000), 8)]);
        let usdc_amount = U256::from(3_000_000_000u64);
        let result = oracle
            .convert_token_to_native(1, TokenType::Usdc, usdc_amount)
            .await
            .unwrap();
        assert_eq!(result, one_eth());
    }

    #[tokio::test]
    async fn token_to_native_bnb_on_eth_roundtrip() {
        let oracle = PriceOracle::with_prices(vec![
            (PriceFeed::EthUsd, usd(3000), 8),
            (PriceFeed::BnbUsd, usd(600), 8),
        ]);
        let bnb_amount = U256::from(5u64) * one_eth();
        let result = oracle
            .convert_token_to_native(1, TokenType::Bnb, bnb_amount)
            .await
            .unwrap();
        assert_eq!(result, one_eth());
    }

    #[tokio::test]
    async fn required_feeds_correctness() {
        assert!(required_feeds(1, TokenType::Eth).is_empty());
        assert!(required_feeds(56, TokenType::Bnb).is_empty());
        assert_eq!(required_feeds(1, TokenType::Usdc), vec![PriceFeed::EthUsd]);
        // Testnet chains also require feeds
        assert_eq!(
            required_feeds(421614, TokenType::Usdc),
            vec![PriceFeed::EthUsd]
        );
        assert_eq!(
            required_feeds(1, TokenType::Bnb),
            vec![PriceFeed::EthUsd, PriceFeed::BnbUsd]
        );
    }
}
