use alloy::{
    network::Ethereum,
    primitives::{Address, B256, Bytes, U256},
    providers::PendingTransactionBuilder,
    rpc::types::TransactionReceipt,
    sol,
};
use zkp::utils::general_recipient::GeneralRecipient;

use crate::contracts::{
    ContractError, ContractResult,
    utils::{NormalProvider, get_provider_with_signer, send_call_with_legacy},
};

sol!(
    #[sol(rpc)]
    Verifier,
    "abi/Verifier.json",
);

#[derive(Debug, Clone)]
pub struct GlobalRootSavedEvent {
    pub agg_seq: u64,
    pub root: U256,
}

#[derive(Debug, Clone)]
pub struct EmergencyTriggeredEvent {
    pub index: u64,
    pub existing_root: U256,
    pub new_root: U256,
}

#[derive(Debug, Clone)]
pub struct VerifiersSetEvent {
    pub root_decider: Address,
    pub withdraw_global_decider: Address,
    pub withdraw_local_decider: Address,
    pub single_withdraw_global_verifier: Address,
    pub single_withdraw_local_verifier: Address,
}

pub struct VerifierContract {
    provider: NormalProvider,
    address: Address,
    legacy_tx: bool,
}

impl VerifierContract {
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

    fn contract_with_provider(&self) -> Verifier::VerifierInstance<NormalProvider> {
        Verifier::new(self.address, self.provider.clone())
    }

    pub const fn legacy_tx(&self) -> bool {
        self.legacy_tx
    }

    pub fn with_legacy_tx(mut self, legacy_tx: bool) -> Self {
        self.legacy_tx = legacy_tx;
        self
    }

    pub async fn token(&self) -> ContractResult<Address> {
        let addr = self.contract_with_provider().TOKEN().call().await?;
        Ok(addr)
    }

    pub async fn hub_eid(&self) -> ContractResult<u32> {
        let eid = self.contract_with_provider().HUB_EID().call().await?;
        Ok(eid)
    }

    pub async fn latest_reserved_index(&self) -> ContractResult<u64> {
        let index = self
            .contract_with_provider()
            .latestReservedIndex()
            .call()
            .await?;
        Ok(index)
    }

    pub async fn latest_proved_index(&self) -> ContractResult<u64> {
        let index = self
            .contract_with_provider()
            .latestProvedIndex()
            .call()
            .await?;
        Ok(index)
    }

    pub async fn latest_relayed_index(&self) -> ContractResult<u64> {
        let index = self
            .contract_with_provider()
            .latestRelayedIndex()
            .call()
            .await?;
        Ok(index)
    }

    pub async fn is_up_to_date(&self) -> ContractResult<bool> {
        let value = self.contract_with_provider().isUpToDate().call().await?;
        Ok(value)
    }

    pub async fn latest_agg_seq(&self) -> ContractResult<u64> {
        let agg_seq = self.contract_with_provider().latestAggSeq().call().await?;
        Ok(agg_seq)
    }

    pub async fn reserved_hash_chain(&self, index: u64) -> ContractResult<U256> {
        let hash_chain = self
            .contract_with_provider()
            .reservedHashChains(index)
            .call()
            .await?;
        Ok(hash_chain)
    }

    pub async fn proved_transfer_root(&self, index: u64) -> ContractResult<U256> {
        let root = self
            .contract_with_provider()
            .provedTransferRoots(index)
            .call()
            .await?;
        Ok(root)
    }

    pub async fn global_transfer_root(&self, agg_seq: u64) -> ContractResult<U256> {
        let root = self
            .contract_with_provider()
            .globalTransferRoots(agg_seq)
            .call()
            .await?;
        Ok(root)
    }

    pub async fn total_teleported(&self, recipient: U256) -> ContractResult<U256> {
        let total = self
            .contract_with_provider()
            .totalTeleported(recipient)
            .call()
            .await?;
        Ok(total)
    }

    pub async fn paused(&self) -> ContractResult<bool> {
        let paused = self.contract_with_provider().paused().call().await?;
        Ok(paused)
    }

    pub async fn root_decider(&self) -> ContractResult<Address> {
        let addr = self.contract_with_provider().rootDecider().call().await?;
        Ok(addr)
    }

    pub async fn withdraw_global_decider(&self) -> ContractResult<Address> {
        let addr = self
            .contract_with_provider()
            .withdrawGlobalDecider()
            .call()
            .await?;
        Ok(addr)
    }

    pub async fn withdraw_local_decider(&self) -> ContractResult<Address> {
        let addr = self
            .contract_with_provider()
            .withdrawLocalDecider()
            .call()
            .await?;
        Ok(addr)
    }

    pub async fn reserve_hash_chain(
        &self,
        private_key: B256,
    ) -> ContractResult<PendingTransactionBuilder<Ethereum>> {
        let signer = get_provider_with_signer(&self.provider, private_key);
        let contract = Verifier::new(self.address, signer.clone());
        let call = contract.reserveHashChain().with_cloned_provider();
        send_call_with_legacy(call, &signer, self.legacy_tx).await
    }

    pub fn parse_hash_chain_reserved(
        &self,
        receipt: &TransactionReceipt,
    ) -> ContractResult<(u64, U256)> {
        for log in receipt.logs() {
            match log.log_decode_validate::<Verifier::HashChainReserved>() {
                Ok(event) => {
                    let index = event.inner.index;
                    let hash_chain = event.inner.hashChain;
                    return Ok((index, hash_chain));
                }
                Err(_) => continue,
            }
        }
        Err(ContractError::MissingEvent("HashChainReserved"))
    }

    pub async fn prove_transfer_root(
        &self,
        private_key: B256,
        proof: &[u8],
    ) -> ContractResult<PendingTransactionBuilder<Ethereum>> {
        let signer = get_provider_with_signer(&self.provider, private_key);
        let contract = Verifier::new(self.address, signer.clone());
        let call = contract
            .proveTransferRoot(Bytes::copy_from_slice(proof))
            .with_cloned_provider();
        send_call_with_legacy(call, &signer, self.legacy_tx).await
    }

    pub fn parse_transfer_root_proved(
        &self,
        receipt: &TransactionReceipt,
    ) -> ContractResult<(u64, U256)> {
        for log in receipt.logs() {
            match log.log_decode_validate::<Verifier::TransferRootProved>() {
                Ok(event) => {
                    let index = event.inner.index;
                    let root = event.inner.root;
                    return Ok((index, root));
                }
                Err(_) => continue,
            }
        }
        Err(ContractError::MissingEvent("TransferRootProved"))
    }

    pub async fn relay_transfer_root(
        &self,
        private_key: B256,
        native_fee: U256,
        options: &[u8],
    ) -> ContractResult<PendingTransactionBuilder<Ethereum>> {
        let signer = get_provider_with_signer(&self.provider, private_key);
        let contract = Verifier::new(self.address, signer.clone());
        let call = contract
            .relayTransferRoot(Bytes::copy_from_slice(options))
            .value(native_fee)
            .with_cloned_provider();
        send_call_with_legacy(call, &signer, self.legacy_tx).await
    }

    pub fn parse_transfer_root_relayed(
        &self,
        receipt: &TransactionReceipt,
    ) -> ContractResult<(u64, U256, Bytes)> {
        for log in receipt.logs() {
            match log.log_decode_validate::<Verifier::TransferRootRelayed>() {
                Ok(event) => {
                    let index = event.inner.index;
                    let root = event.inner.root;
                    let guid = event.inner.lzMsgId.clone();
                    return Ok((index, root, guid));
                }
                Err(_) => continue,
            }
        }
        Err(ContractError::MissingEvent("TransferRootRelayed"))
    }

    pub async fn teleport(
        &self,
        private_key: B256,
        is_global: bool,
        root_hint: u64,
        gr: GeneralRecipient,
        proof: &[u8],
    ) -> ContractResult<PendingTransactionBuilder<Ethereum>> {
        let signer = get_provider_with_signer(&self.provider, private_key);
        let contract = Verifier::new(self.address, signer.clone());
        let call = contract
            .teleport(
                is_global,
                root_hint,
                gr_to_contract(gr),
                Bytes::copy_from_slice(proof),
            )
            .with_cloned_provider();
        send_call_with_legacy(call, &signer, self.legacy_tx).await
    }

    pub async fn single_teleport(
        &self,
        private_key: B256,
        is_global: bool,
        root_hint: u64,
        gr: GeneralRecipient,
        proof: &[u8],
    ) -> ContractResult<PendingTransactionBuilder<Ethereum>> {
        let signer = get_provider_with_signer(&self.provider, private_key);
        let contract = Verifier::new(self.address, signer.clone());
        let call = contract
            .singleTeleport(
                is_global,
                root_hint,
                gr_to_contract(gr),
                Bytes::copy_from_slice(proof),
            )
            .with_cloned_provider();
        send_call_with_legacy(call, &signer, self.legacy_tx).await
    }

    pub async fn deactivate_emergency(
        &self,
        private_key: B256,
    ) -> ContractResult<PendingTransactionBuilder<Ethereum>> {
        let signer = get_provider_with_signer(&self.provider, private_key);
        let contract = Verifier::new(self.address, signer.clone());
        let call = contract.deactivateEmergency().with_cloned_provider();
        send_call_with_legacy(call, &signer, self.legacy_tx).await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn set_verifiers(
        &self,
        private_key: B256,
        root_decider: Address,
        withdraw_global_decider: Address,
        withdraw_local_decider: Address,
        single_withdraw_global_verifier: Address,
        single_withdraw_local_verifier: Address,
    ) -> ContractResult<PendingTransactionBuilder<Ethereum>> {
        let signer = get_provider_with_signer(&self.provider, private_key);
        let contract = Verifier::new(self.address, signer.clone());
        let call = contract
            .setVerifiers(
                root_decider,
                withdraw_global_decider,
                withdraw_local_decider,
                single_withdraw_global_verifier,
                single_withdraw_local_verifier,
            )
            .with_cloned_provider();
        send_call_with_legacy(call, &signer, self.legacy_tx).await
    }

    pub fn parse_teleport(&self, receipt: &TransactionReceipt) -> ContractResult<(Address, U256)> {
        for log in receipt.logs() {
            match log.log_decode_validate::<Verifier::Teleport>() {
                Ok(event) => {
                    let to = event.inner.to;
                    let value = event.inner.value;
                    return Ok((to, value));
                }
                Err(_) => continue,
            }
        }
        Err(ContractError::MissingEvent("Teleport"))
    }

    pub async fn quote_relay(&self, options: &[u8]) -> ContractResult<(U256, U256)> {
        let fee = self
            .contract_with_provider()
            .quoteRelay(Bytes::copy_from_slice(options))
            .call()
            .await?;
        Ok((fee.nativeFee, fee.lzTokenFee))
    }

    pub fn parse_global_root_saved(
        &self,
        receipt: &TransactionReceipt,
    ) -> ContractResult<GlobalRootSavedEvent> {
        for log in receipt.logs() {
            match log.log_decode_validate::<Verifier::GlobalRootSaved>() {
                Ok(event) => {
                    let inner = event.inner;
                    return Ok(GlobalRootSavedEvent {
                        agg_seq: inner.aggSeq,
                        root: inner.root,
                    });
                }
                Err(_) => continue,
            }
        }
        Err(ContractError::MissingEvent("GlobalRootSaved"))
    }

    pub fn parse_emergency_triggered(
        &self,
        receipt: &TransactionReceipt,
    ) -> ContractResult<EmergencyTriggeredEvent> {
        for log in receipt.logs() {
            match log.log_decode_validate::<Verifier::EmergencyTriggered>() {
                Ok(event) => {
                    let inner = event.inner;
                    return Ok(EmergencyTriggeredEvent {
                        index: inner.index,
                        existing_root: inner.root1,
                        new_root: inner.root2,
                    });
                }
                Err(_) => continue,
            }
        }
        Err(ContractError::MissingEvent("EmergencyTriggered"))
    }

    pub fn parse_deactivate_emergency(&self, receipt: &TransactionReceipt) -> ContractResult<()> {
        for log in receipt.logs() {
            if log
                .log_decode_validate::<Verifier::DeactivateEmergency>()
                .is_ok()
            {
                return Ok(());
            }
        }
        Err(ContractError::MissingEvent("DeactivateEmergency"))
    }

    pub fn parse_verifiers_set(
        &self,
        receipt: &TransactionReceipt,
    ) -> ContractResult<VerifiersSetEvent> {
        for log in receipt.logs() {
            match log.log_decode_validate::<Verifier::VerifiersSet>() {
                Ok(event) => {
                    let inner = event.inner;
                    return Ok(VerifiersSetEvent {
                        root_decider: inner.rootDecider,
                        withdraw_global_decider: inner.withdrawGlobalDecider,
                        withdraw_local_decider: inner.withdrawLocalDecider,
                        single_withdraw_global_verifier: inner.singleWithdrawGlobalVerifier,
                        single_withdraw_local_verifier: inner.singleWithdrawLocalVerifier,
                    });
                }
                Err(_) => continue,
            }
        }
        Err(ContractError::MissingEvent("VerifiersSet"))
    }
}

fn gr_to_contract(gr: GeneralRecipient) -> GeneralRecipientLib::GeneralRecipient {
    GeneralRecipientLib::GeneralRecipient {
        chainId: gr.chain_id,
        recipient: gr.address,
        tweak: gr.tweak,
    }
}
