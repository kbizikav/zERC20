use std::{
    collections::HashMap,
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

/// Chainlink feed address for a specific (chain_id, feed) pair.
fn chainlink_address(chain_id: u64, feed: PriceFeed) -> Option<Address> {
    match (chain_id, feed) {
        // ETH/USD
        (1, PriceFeed::EthUsd) => Some(address!("5f4eC3Df9cbd43714FE2740f5E3616155c5b8419")),
        (42161, PriceFeed::EthUsd) => Some(address!("639Fe6ab55C921f74e7fac1ee960C0B6293ba612")),
        (8453, PriceFeed::EthUsd) => Some(address!("71041dddad3595F9CEd3DcCFBe3D1F4b0a16Bb70")),
        // BNB/USD
        (1, PriceFeed::BnbUsd) => Some(address!("14e613AC84a31f709eadbdF89C6CC390fDc9540A")),
        (42161, PriceFeed::BnbUsd) => Some(address!("6970460aabF80C5BE983C6b74e5D06dEDCA95D4A")),
        (8453, PriceFeed::BnbUsd) => Some(address!("4b7836916781CAAfbb7Bd1E5FDd20ED544B453b1")),
        // BNB/USD on BSC (for completeness, though BNB on BSC is 1:1)
        (56, PriceFeed::BnbUsd) => Some(address!("0567F2323251f0Aab15c8dFb1967E4e8A7D42aeE")),
        _ => None,
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
        // zETH on ETH-gas chains (1, 42161, 8453): 1:1, no feed needed
        TokenType::Eth if is_eth_gas_chain(chain_id) => vec![],
        // zBNB on BSC: 1:1, no feed needed
        TokenType::Bnb if chain_id == 56 => vec![],
        // zUSDC on ETH-gas chains: need ETH/USD
        TokenType::Usdc if is_eth_gas_chain(chain_id) => vec![PriceFeed::EthUsd],
        // zBNB on ETH-gas chains: need ETH/USD and BNB/USD
        TokenType::Bnb if is_eth_gas_chain(chain_id) => {
            vec![PriceFeed::EthUsd, PriceFeed::BnbUsd]
        }
        // zETH on non-ETH chain (future): would need ETH/USD + native/USD
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
pub struct PriceOracle {
    cache: Arc<RwLock<HashMap<(u64, PriceFeed), CachedPrice>>>,
}

/// Feed to query: (chain_id, feed, chainlink_address, rpc_urls).
struct FeedTarget {
    chain_id: u64,
    feed: PriceFeed,
    feed_address: Address,
    rpc_urls: Vec<String>,
}

impl PriceOracle {
    /// Create a new oracle from the loaded token entries.
    ///
    /// Discovers which Chainlink feeds are needed based on token_type and
    /// chain_id, then kicks off a background updater.
    pub fn new(tokens: &[TokenEntry]) -> Result<Self> {
        let oracle = Self {
            cache: Arc::new(RwLock::new(HashMap::new())),
        };

        let targets = Self::discover_targets(tokens)?;
        if !targets.is_empty() {
            oracle.spawn_updater(targets);
        }

        Ok(oracle)
    }

    /// Create an oracle with pre-seeded prices (for testing).
    #[cfg(test)]
    fn with_prices(prices: Vec<(u64, PriceFeed, U256, u8)>) -> Self {
        let mut cache = HashMap::new();
        for (chain_id, feed, price, decimals) in prices {
            cache.insert(
                (chain_id, feed),
                CachedPrice {
                    price,
                    feed_decimals: decimals,
                    updated_at: Instant::now(),
                },
            );
        }
        Self {
            cache: Arc::new(RwLock::new(cache)),
        }
    }

    fn discover_targets(tokens: &[TokenEntry]) -> Result<Vec<FeedTarget>> {
        // Collect unique (chain_id, feed) pairs and keep rpc_urls from the
        // first token entry on each chain.
        let mut seen: HashMap<(u64, PriceFeed), Vec<String>> = HashMap::new();

        for token in tokens {
            let Some(tt) = token.token_type else {
                continue;
            };
            for feed in required_feeds(token.chain_id, tt) {
                seen.entry((token.chain_id, feed))
                    .or_insert_with(|| token.rpc_urls.clone());
            }
        }

        let mut targets = Vec::new();
        for ((chain_id, feed), rpc_urls) in seen {
            let Some(addr) = chainlink_address(chain_id, feed) else {
                log::warn!(
                    "no Chainlink {} feed address for chain {}; will use fallback price",
                    feed.label(),
                    chain_id
                );
                continue;
            };
            targets.push(FeedTarget {
                chain_id,
                feed,
                feed_address: addr,
                rpc_urls,
            });
        }

        Ok(targets)
    }

    fn spawn_updater(&self, targets: Vec<FeedTarget>) {
        let cache = self.cache.clone();
        tokio::spawn(async move {
            // Build providers once.
            let mut providers: Vec<(u64, PriceFeed, _)> = Vec::new();
            for t in &targets {
                let Ok(url) = t.rpc_urls[0].parse() else {
                    log::error!(
                        "invalid RPC URL for {} on chain {}",
                        t.feed.label(),
                        t.chain_id
                    );
                    continue;
                };
                let provider = alloy::providers::ProviderBuilder::new().connect_http(url);
                providers.push((t.chain_id, t.feed, (t.feed_address, provider)));
            }

            loop {
                for (chain_id, feed, (addr, provider)) in &providers {
                    match fetch_price(*addr, provider).await {
                        Ok((price, decimals)) => {
                            let mut c = cache.write().await;
                            c.insert(
                                (*chain_id, *feed),
                                CachedPrice {
                                    price,
                                    feed_decimals: decimals,
                                    updated_at: Instant::now(),
                                },
                            );
                            log::debug!(
                                "updated {} on chain {}: {}",
                                feed.label(),
                                chain_id,
                                price
                            );
                        }
                        Err(err) => {
                            log::warn!(
                                "failed to fetch {} on chain {}: {:#}",
                                feed.label(),
                                chain_id,
                                err
                            );
                        }
                    }
                }
                tokio::time::sleep(POLL_INTERVAL).await;
            }
        });
    }

    /// Get a cached price for the given feed on the given chain.
    ///
    /// Returns (price, feed_decimals). Falls back to hardcoded prices if the
    /// cache is too stale or empty.
    async fn get_price(&self, chain_id: u64, feed: PriceFeed) -> (U256, u8) {
        let cache = self.cache.read().await;
        if let Some(cached) = cache.get(&(chain_id, feed)) {
            let age = cached.updated_at.elapsed();
            if age < CACHE_TTL {
                return (cached.price, cached.feed_decimals);
            }
            if age < CACHE_MAX_STALE {
                log::warn!(
                    "using stale {} price on chain {} (age: {:.0}s)",
                    feed.label(),
                    chain_id,
                    age.as_secs_f64()
                );
                return (cached.price, cached.feed_decimals);
            }
        }
        log::warn!(
            "no cached {} price for chain {}, using fallback",
            feed.label(),
            chain_id
        );
        (fallback_price(feed), 8)
    }

    /// Check whether all required price feeds for a (chain, token_type) pair
    /// have fresh cached data. Returns `false` if any feed is stale or missing
    /// (i.e. the oracle would use a fallback or stale price).
    pub async fn has_fresh_prices(&self, chain_id: u64, token_type: TokenType) -> bool {
        let feeds = required_feeds(chain_id, token_type);
        let cache = self.cache.read().await;
        for feed in feeds {
            match cache.get(&(chain_id, feed)) {
                Some(cached) if cached.updated_at.elapsed() < CACHE_TTL => {}
                _ => return false,
            }
        }
        true
    }

    /// Convert a token amount (smallest unit) back to native wei.
    ///
    /// This is the inverse of `convert_native_to_token` and is used for swap
    /// quotes: given N zTokens, how much native does the user receive?
    pub async fn convert_token_to_native(
        &self,
        chain_id: u64,
        token_type: TokenType,
        token_amount: U256,
    ) -> Result<U256> {
        match token_type {
            // zETH on ETH-gas chain: 1:1
            TokenType::Eth if is_eth_gas_chain(chain_id) => Ok(token_amount),

            // zBNB on BSC: 1:1
            TokenType::Bnb if chain_id == 56 => Ok(token_amount),

            // zUSDC on ETH-gas chain: token_amount * 10^(12+feed_dec) / eth_price
            TokenType::Usdc if is_eth_gas_chain(chain_id) => {
                let (eth_price, dec) = self.get_price(chain_id, PriceFeed::EthUsd).await;
                if eth_price.is_zero() {
                    bail!("ETH/USD price is zero");
                }
                let multiplier = U256::from(10u64).pow(U256::from(12 + dec));
                Ok(token_amount * multiplier / eth_price)
            }

            // zBNB on ETH-gas chain: token_amount * bnb_price / eth_price
            TokenType::Bnb if is_eth_gas_chain(chain_id) => {
                let (eth_price, _) = self.get_price(chain_id, PriceFeed::EthUsd).await;
                let (bnb_price, _) = self.get_price(chain_id, PriceFeed::BnbUsd).await;
                if eth_price.is_zero() {
                    bail!("ETH/USD price is zero");
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
            // zETH on ETH-gas chain: 1:1 (both 18 decimals)
            TokenType::Eth if is_eth_gas_chain(chain_id) => Ok(native_wei),

            // zBNB on BSC: 1:1 (both 18 decimals)
            TokenType::Bnb if chain_id == 56 => Ok(native_wei),

            // zUSDC on ETH-gas chain: native_wei * eth_price / 10^(18+feed_dec-6)
            // = native_wei * eth_price / 10^(12+feed_dec)
            TokenType::Usdc if is_eth_gas_chain(chain_id) => {
                let (eth_price, dec) = self.get_price(chain_id, PriceFeed::EthUsd).await;
                // native_wei (18 dec) * eth_price (feed_dec dec) / 10^(18 + feed_dec - 6)
                let divisor = U256::from(10u64).pow(U256::from(12 + dec));
                Ok(native_wei * eth_price / divisor)
            }

            // zBNB on ETH-gas chain: native_wei * eth_price / bnb_price
            // Both feeds have the same decimals so they cancel out.
            TokenType::Bnb if is_eth_gas_chain(chain_id) => {
                let (eth_price, _) = self.get_price(chain_id, PriceFeed::EthUsd).await;
                let (bnb_price, _) = self.get_price(chain_id, PriceFeed::BnbUsd).await;
                if bnb_price.is_zero() {
                    bail!("BNB/USD price is zero");
                }
                // native_wei * (eth/usd) / (bnb/usd) = native_wei in BNB terms (18 dec)
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
        // ETH = $3000, gas cost = 0.001 ETH → expect ~$3 = 3_000000 USDC
        let oracle = PriceOracle::with_prices(vec![(1, PriceFeed::EthUsd, usd(3000), 8)]);

        let gas_cost = one_eth() / U256::from(1000); // 0.001 ETH
        let result = oracle
            .convert_native_to_token(1, TokenType::Usdc, gas_cost)
            .await
            .unwrap();

        // 0.001 ETH * $3000 = $3.00 = 3_000000 (6 decimals)
        assert_eq!(result, U256::from(3_000_000u64));
    }

    #[tokio::test]
    async fn usdc_on_eth_chain_1_eth() {
        // 1 ETH at $2500 → 2500_000000 USDC
        let oracle = PriceOracle::with_prices(vec![(1, PriceFeed::EthUsd, usd(2500), 8)]);

        let result = oracle
            .convert_native_to_token(1, TokenType::Usdc, one_eth())
            .await
            .unwrap();

        assert_eq!(result, U256::from(2_500_000_000u64)); // 2500 * 10^6
    }

    #[tokio::test]
    async fn bnb_on_eth_chain_converts_correctly() {
        // ETH = $3000, BNB = $600 → 1 ETH gas = 5 BNB
        let oracle = PriceOracle::with_prices(vec![
            (1, PriceFeed::EthUsd, usd(3000), 8),
            (1, PriceFeed::BnbUsd, usd(600), 8),
        ]);

        let result = oracle
            .convert_native_to_token(1, TokenType::Bnb, one_eth())
            .await
            .unwrap();

        // 1 ETH * (3000/600) = 5 BNB (18 decimals)
        assert_eq!(result, U256::from(5u64) * one_eth());
    }

    #[tokio::test]
    async fn bnb_on_eth_chain_fractional() {
        // ETH = $3000, BNB = $500 → 0.01 ETH gas = 0.06 BNB
        let oracle = PriceOracle::with_prices(vec![
            (42161, PriceFeed::EthUsd, usd(3000), 8),
            (42161, PriceFeed::BnbUsd, usd(500), 8),
        ]);

        let gas_cost = one_eth() / U256::from(100); // 0.01 ETH
        let result = oracle
            .convert_native_to_token(42161, TokenType::Bnb, gas_cost)
            .await
            .unwrap();

        // 0.01 * 3000/500 = 0.06 BNB
        let expected = one_eth() * U256::from(6) / U256::from(100);
        assert_eq!(result, expected);
    }

    #[tokio::test]
    async fn fallback_price_used_when_cache_empty() {
        // No prices seeded → should use fallback ($4000 for ETH/USD)
        let oracle = PriceOracle::with_prices(vec![]);

        let gas_cost = one_eth() / U256::from(1000); // 0.001 ETH
        let result = oracle
            .convert_native_to_token(1, TokenType::Usdc, gas_cost)
            .await
            .unwrap();

        // 0.001 ETH * $4000 fallback = $4.00 = 4_000000 USDC
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
        // 1 ETH at $3000 → 3000_000000 USDC → back to 1 ETH
        let oracle = PriceOracle::with_prices(vec![(1, PriceFeed::EthUsd, usd(3000), 8)]);
        let usdc_amount = U256::from(3_000_000_000u64); // $3000 in 6-dec USDC (3000 * 10^6)
        let result = oracle
            .convert_token_to_native(1, TokenType::Usdc, usdc_amount)
            .await
            .unwrap();
        assert_eq!(result, one_eth());
    }

    #[tokio::test]
    async fn token_to_native_bnb_on_eth_roundtrip() {
        // ETH=$3000, BNB=$600 → 5 BNB → back to 1 ETH
        let oracle = PriceOracle::with_prices(vec![
            (1, PriceFeed::EthUsd, usd(3000), 8),
            (1, PriceFeed::BnbUsd, usd(600), 8),
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
        assert_eq!(
            required_feeds(1, TokenType::Bnb),
            vec![PriceFeed::EthUsd, PriceFeed::BnbUsd]
        );
    }
}
