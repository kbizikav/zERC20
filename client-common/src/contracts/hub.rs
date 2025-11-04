use alloy::{
    network::Ethereum,
    primitives::{Address, B256, Bytes, U256},
    providers::{PendingTransactionBuilder, Provider},
    rpc::types::TransactionReceipt,
    sol,
};

use crate::contracts::{
    ContractError, ContractResult,
    utils::{NormalProvider, get_provider_with_signer, send_call_with_legacy, uint256_as_u64},
};

sol!(
    #[sol(rpc, ignore_unlinked)]
    Hub,
    "abi/Hub.json",
);

#[derive(Debug, Clone)]
pub struct HubTokenInfo {
    pub chain_id: u64,
    pub eid: u32,
    pub verifier: Address,
    pub token: Address,
}

impl From<Hub::TokenInfo> for HubTokenInfo {
    fn from(value: Hub::TokenInfo) -> Self {
        Self {
            chain_id: value.chainId,
            eid: value.eid,
            verifier: value.verifier,
            token: value.token,
        }
    }
}

impl From<Hub::tokenInfosReturn> for HubTokenInfo {
    fn from(value: Hub::tokenInfosReturn) -> Self {
        Self {
            chain_id: value.chainId,
            eid: value.eid,
            verifier: value.verifier,
            token: value.token,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AggregationRootUpdatedEvent {
    pub root: U256,
    pub agg_seq: u64,
    pub snapshot: Vec<U256>,
    pub transfer_tree_indices: Vec<u64>,
}

#[derive(Debug, Clone)]
pub struct TransferRootUpdatedEvent {
    pub eid: u32,
    pub index: u64,
    pub new_root: U256,
}

#[derive(Debug, Clone)]
pub struct TokenRegisteredEvent {
    pub eid: u32,
    pub index: u64,
    pub chain_id: u64,
    pub token: Address,
    pub verifier: Address,
}

#[derive(Debug, Clone)]
pub struct TokenUpdatedEvent {
    pub eid: u32,
    pub index: u64,
    pub chain_id: u64,
    pub token: Address,
    pub verifier: Address,
}

#[derive(Clone)]
pub struct HubContract {
    provider: NormalProvider,
    address: Address,
    legacy_tx: bool,
}

impl HubContract {
    pub fn new(provider: NormalProvider, address: Address) -> Self {
        Self {
            provider,
            address,
            legacy_tx: false,
        }
    }

    pub fn address(&self) -> Address {
        self.address
    }

    pub fn provider(&self) -> NormalProvider {
        self.provider.clone()
    }

    fn contract_with_provider(&self) -> Hub::HubInstance<NormalProvider> {
        Hub::new(self.address, self.provider.clone())
    }

    pub const fn legacy_tx(&self) -> bool {
        self.legacy_tx
    }

    pub fn with_legacy_tx(mut self, legacy_tx: bool) -> Self {
        self.legacy_tx = legacy_tx;
        self
    }

    pub async fn agg_seq(&self) -> ContractResult<u64> {
        let seq = self.contract_with_provider().aggSeq().call().await?;
        Ok(seq)
    }

    pub async fn is_up_to_date(&self) -> ContractResult<bool> {
        let up_to_date = self.contract_with_provider().isUpToDate().call().await?;
        Ok(up_to_date)
    }

    pub async fn transfer_root(&self, index: u64) -> ContractResult<U256> {
        let root = self
            .contract_with_provider()
            .transferRoots(U256::from(index))
            .call()
            .await?;
        Ok(root)
    }

    pub async fn transfer_tree_index(&self, index: u64) -> ContractResult<u64> {
        let position = self
            .contract_with_provider()
            .transferTreeIndices(U256::from(index))
            .call()
            .await?;
        Ok(position)
    }

    pub async fn max_leaves(&self) -> ContractResult<U256> {
        let value = self.contract_with_provider().MAX_LEAVES().call().await?;
        Ok(value)
    }

    pub async fn zero_hash(&self, depth: u64) -> ContractResult<U256> {
        let hash = self
            .contract_with_provider()
            .zeroHash(U256::from(depth))
            .call()
            .await?;
        Ok(hash)
    }

    pub async fn eid_position(&self, eid: u32) -> ContractResult<u64> {
        let pos = self
            .contract_with_provider()
            .eidToPosition(eid)
            .call()
            .await?;
        Ok(uint256_as_u64(pos))
    }

    pub async fn token_info(&self, index: u64) -> ContractResult<HubTokenInfo> {
        let res = self
            .contract_with_provider()
            .tokenInfos(U256::from(index))
            .call()
            .await?;
        Ok(HubTokenInfo::from(res))
    }

    pub async fn token_infos(&self) -> ContractResult<Vec<HubTokenInfo>> {
        let res = self.contract_with_provider().getTokenInfos().call().await?;
        Ok(res.into_iter().map(HubTokenInfo::from).collect())
    }

    pub async fn quote_broadcast(
        &self,
        target_eids: Vec<u32>,
        lz_options: Bytes,
    ) -> ContractResult<U256> {
        let fee = self
            .contract_with_provider()
            .quoteBroadcast(target_eids, lz_options)
            .call()
            .await?;
        Ok(fee)
    }

    pub async fn broadcast(
        &self,
        private_key: B256,
        target_eids: Vec<u32>,
        lz_options: Bytes,
        native_fee: U256,
    ) -> ContractResult<PendingTransactionBuilder<Ethereum>> {
        let signer = get_provider_with_signer(&self.provider, private_key);
        let contract = Hub::new(self.address, signer.clone());
        let call = contract
            .broadcast(target_eids, lz_options)
            .value(native_fee)
            .with_cloned_provider();
        send_call_with_legacy(call, &signer, self.legacy_tx).await
    }

    pub async fn register_token(
        &self,
        private_key: B256,
        info: HubTokenInfo,
    ) -> ContractResult<PendingTransactionBuilder<Ethereum>> {
        let signer = get_provider_with_signer(&self.provider, private_key);
        let contract = Hub::new(self.address, signer.clone());
        let token_info = Hub::TokenInfo {
            chainId: info.chain_id,
            eid: info.eid,
            verifier: info.verifier,
            token: info.token,
        };
        let call = contract.registerToken(token_info).with_cloned_provider();
        send_call_with_legacy(call, &signer, self.legacy_tx).await
    }

    pub async fn update_token(
        &self,
        private_key: B256,
        info: HubTokenInfo,
    ) -> ContractResult<PendingTransactionBuilder<Ethereum>> {
        let signer = get_provider_with_signer(&self.provider, private_key);
        let contract = Hub::new(self.address, signer.clone());
        let token_info = Hub::TokenInfo {
            chainId: info.chain_id,
            eid: info.eid,
            verifier: info.verifier,
            token: info.token,
        };
        let call = contract.updateToken(token_info).with_cloned_provider();
        send_call_with_legacy(call, &signer, self.legacy_tx).await
    }

    pub fn parse_aggregation_root_updated(
        &self,
        receipt: &TransactionReceipt,
    ) -> ContractResult<AggregationRootUpdatedEvent> {
        for log in receipt.logs() {
            match log.log_decode_validate::<Hub::AggregationRootUpdated>() {
                Ok(event) => {
                    let inner = event.inner;
                    return Ok(AggregationRootUpdatedEvent {
                        root: inner.root,
                        agg_seq: inner.aggSeq,
                        snapshot: inner.transferRootsSnapshot.clone(),
                        transfer_tree_indices: inner.transferTreeIndicesSnapshot.clone(),
                    });
                }
                Err(_) => continue,
            }
        }
        Err(ContractError::MissingEvent("AggregationRootUpdated"))
    }

    pub fn parse_transfer_root_updated(
        &self,
        receipt: &TransactionReceipt,
    ) -> ContractResult<TransferRootUpdatedEvent> {
        for log in receipt.logs() {
            match log.log_decode_validate::<Hub::TransferRootUpdated>() {
                Ok(event) => {
                    let inner = event.inner;
                    return Ok(TransferRootUpdatedEvent {
                        eid: inner.eid,
                        index: uint256_as_u64(inner.index),
                        new_root: inner.newRoot,
                    });
                }
                Err(_) => continue,
            }
        }
        Err(ContractError::MissingEvent("TransferRootUpdated"))
    }

    pub fn parse_token_registered(
        &self,
        receipt: &TransactionReceipt,
    ) -> ContractResult<TokenRegisteredEvent> {
        for log in receipt.logs() {
            match log.log_decode_validate::<Hub::TokenRegistered>() {
                Ok(event) => {
                    let inner = event.inner;
                    return Ok(TokenRegisteredEvent {
                        eid: inner.eid,
                        index: uint256_as_u64(inner.index),
                        chain_id: inner.chainId,
                        token: inner.token,
                        verifier: inner.verifier,
                    });
                }
                Err(_) => continue,
            }
        }
        Err(ContractError::MissingEvent("TokenRegistered"))
    }

    pub fn parse_token_updated(
        &self,
        receipt: &TransactionReceipt,
    ) -> ContractResult<TokenUpdatedEvent> {
        for log in receipt.logs() {
            match log.log_decode_validate::<Hub::TokenUpdated>() {
                Ok(event) => {
                    let inner = event.inner;
                    return Ok(TokenUpdatedEvent {
                        eid: inner.eid,
                        index: uint256_as_u64(inner.index),
                        chain_id: inner.chainId,
                        token: inner.token,
                        verifier: inner.verifier,
                    });
                }
                Err(_) => continue,
            }
        }
        Err(ContractError::MissingEvent("TokenUpdated"))
    }

    pub async fn aggregation_root_events(
        &self,
        from_block: u64,
        to_block: u64,
    ) -> ContractResult<Vec<AggregationRootUpdatedEvent>> {
        let contract = Hub::new(self.address, self.provider.clone());
        let events = contract
            .event_filter::<Hub::AggregationRootUpdated>()
            .address(self.address)
            .from_block(from_block)
            .to_block(to_block)
            .query()
            .await?;
        Ok(events
            .into_iter()
            .map(|(event, _)| AggregationRootUpdatedEvent {
                root: event.root,
                agg_seq: event.aggSeq,
                snapshot: event.transferRootsSnapshot,
                transfer_tree_indices: event.transferTreeIndicesSnapshot,
            })
            .collect())
    }

    pub async fn latest_block(&self) -> ContractResult<u64> {
        self.provider
            .get_block_number()
            .await
            .map_err(|err| ContractError::transport("get_block_number", err))
    }
}
