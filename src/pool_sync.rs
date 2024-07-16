//! PoolSync Core Implementation
//!
//! This module contains the core functionality for synchronizing pools across different
//! blockchain networks and protocols. It includes the main `PoolSync` struct and its
//! associated methods for configuring and executing the synchronization process.
//!
use alloy::network::Network;
use alloy::providers::Provider;
use alloy::providers::RootProvider;
use alloy::pubsub::PubSubFrontend;
use alloy::transports::Transport;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Semaphore;

use crate::builder::PoolSyncBuilder;
use crate::cache::{read_cache_file, write_cache_file, PoolCache};
use crate::chain::Chain;
use crate::errors::*;
use crate::pools::*;
use crate::rpc::Rpc;

/// The number of blocks to query in one call to get_logs
const STEP_SIZE: u64 = 10_000;
/// The maximum number of retries for a failed query
const MAX_RETRIES: u32 = 5;

/// The main struct for pool synchronization
pub struct PoolSync {
    /// Map of pool types to their fetcher implementations
    pub fetchers: HashMap<PoolType, Arc<dyn PoolFetcher>>,
    /// The chain to sync on
    pub chain: Chain,
    /// The rate limit of the rpc
    pub rate_limit: usize,
}

impl PoolSync {
    /// Construct a new builder to configure sync parameters
    pub fn builder() -> PoolSyncBuilder {
        PoolSyncBuilder::default()
    }

    /// Synchronizes all added pools for the specified chain
    pub async fn sync_pools<P, T, N>(
        &self,
        provider: Arc<P>,
        ws: Arc<RootProvider<PubSubFrontend, N>>,
    ) -> Result<Vec<Pool>, PoolSyncError>
    where
        P: Provider<T, N> + 'static,
        T: Transport + Clone + 'static,
        N: Network,
    {
        // create the cache files
        std::fs::create_dir_all("cache").unwrap();

        // create all of the caches
        let mut pool_caches: Vec<PoolCache> = self
            .fetchers
            .keys()
            .map(|pool_type| read_cache_file(pool_type, self.chain))
            .collect();

        let end_block = provider.get_block_number().await.unwrap();
        let rate_limiter = Arc::new(Semaphore::new(self.rate_limit));

        // go though each cache, may or may not already by synced up to some point
        for cache in &mut pool_caches {
            let start_block = cache.last_synced_block;
            let fetcher = self.fetchers[&cache.pool_type].clone();

            // fetch all of the pool addresses
            let pools = Rpc::fetch_pool_addrs(
                start_block,
                end_block,
                provider.clone(),
                fetcher,
                self.chain
            ).await;

            // populate all of the pool addresses
            let populated_pools = Rpc::populate_pools(
                pools,
                provider.clone()
            ).await;

            // update the cache
            cache.pools.extend(populated_pools);
            cache.last_synced_block = end_block;
            write_cache_file(cache, self.chain)?;
        }

        // return all the pools
        Ok(pool_caches
            .into_iter()
            .flat_map(|cache| cache.pools)
            .collect())

    }

}