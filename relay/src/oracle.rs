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
        (1, PriceFeed::BnbUsd) => Some(address!("14e613AC691a42F21B17B217403396B18F671a7f")),
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
