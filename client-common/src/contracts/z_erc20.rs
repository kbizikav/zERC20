// SPDX-License-Identifier: BUSL-1.1

#![allow(clippy::too_many_arguments)]

use crate::contracts::{
    ContractError, ContractResult,
    utils::{NormalProvider, get_provider_with_signer, send_call_with_legacy, uint256_as_u64},
};
use alloy::{
    eips::BlockNumberOrTag,
    network::Ethereum,
    network::primitives::{BlockResponse, HeaderResponse},
    primitives::{Address, B256, Bytes, U256},
    providers::{PendingTransactionBuilder, Provider},
    sol,
    sol_types::SolCall,
};
use api_types::indexer::IndexedEvent;
use serde::{Deserialize, Serialize}; // for get_block_number

sol!(
    #[sol(rpc)]
    zERC20,
    "abi/zERC20.json",
);

sol!(
    #[sol(rpc)]
    ERC1967Proxy,
    "abi/ERC1967Proxy.json",
);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Eip712Domain {
    pub fields: u8,
    pub name: String,
    pub version: String,
    pub chain_id: U256,
    pub verifying_contract: Address,
    pub salt: B256,
    pub extensions: Vec<U256>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TeleportEvent {
    pub to: Address,
    pub value: U256,
    pub eth_block_number: u64,
}

#[derive(Debug, Clone)]
pub struct SendParam {
    pub dst_eid: u32,
    pub to: B256,
    pub amount_ld: U256,
    pub min_amount_ld: U256,
    pub extra_options: Bytes,
    pub compose_msg: Bytes,
    pub oft_cmd: Bytes,
}

#[derive(Debug, Clone)]
pub struct MessagingFee {
    pub native_fee: U256,
    pub lz_token_fee: U256,
}

#[derive(Clone)]
pub struct ZErc20Contract {
    provider: NormalProvider,
    address: Address,
    legacy_tx: bool,
}

impl ZErc20Contract {
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

    pub const fn legacy_tx(&self) -> bool {
        self.legacy_tx
    }

    pub fn with_legacy_tx(mut self, legacy_tx: bool) -> Self {
        self.legacy_tx = legacy_tx;
        self
    }

    pub async fn deploy(
        provider: NormalProvider,
        private_key: B256,
        name_: String,
        symbol_: String,
        owner: Address,
        endpoint: Address,
        decimals: u8,
    ) -> anyhow::Result<Self> {
        let signer = get_provider_with_signer(&provider, private_key);
        let implementation = zERC20::deploy(signer.clone(), endpoint, decimals).await?;
        let implementation_address = *implementation.address();

        let init_data = zERC20::initializeCall {
            name_,
            symbol_,
            initialOwner: owner,
        }
        .abi_encode();

        let proxy =
            ERC1967Proxy::deploy(signer, implementation_address, Bytes::from(init_data)).await?;
        let address = *proxy.address();
        Ok(Self {
            provider,
            address,
            legacy_tx: false,
        })
    }

    pub async fn hash_chain(&self) -> ContractResult<U256> {
        let contract = zERC20::new(self.address, self.provider.clone());
        let hash_chain = contract.hashChain().call().await?;
        Ok(hash_chain)
    }

    pub async fn index(&self) -> ContractResult<u64> {
        let contract = zERC20::new(self.address, self.provider.clone());
        let index = contract.index().call().await?;
        Ok(uint256_as_u64(index))
    }

    pub async fn verifier(&self) -> ContractResult<Address> {
        let contract = zERC20::new(self.address, self.provider.clone());
        let addr = contract.verifier().call().await?;
        Ok(addr)
    }

    pub async fn minter(&self) -> ContractResult<Address> {
        let contract = zERC20::new(self.address, self.provider.clone());
        let addr = contract.minter().call().await?;
        Ok(addr)
    }

    pub async fn domain_separator(&self) -> ContractResult<B256> {
        let contract = zERC20::new(self.address, self.provider.clone());
        let separator = contract.DOMAIN_SEPARATOR().call().await?;
        Ok(separator)
    }

    pub async fn eip712_domain(&self) -> ContractResult<Eip712Domain> {
        let contract = zERC20::new(self.address, self.provider.clone());
        let domain = contract.eip712Domain().call().await?;
        Ok(Eip712Domain::from(domain))
    }

    pub async fn nonces(&self, owner: Address) -> ContractResult<U256> {
        let contract = zERC20::new(self.address, self.provider.clone());
        let nonce = contract.nonces(owner).call().await?;
        Ok(nonce)
    }

    pub async fn mint(
        &self,
        private_key: B256,
        to: Address,
        amount: U256,
    ) -> ContractResult<PendingTransactionBuilder<Ethereum>> {
        let signer = get_provider_with_signer(&self.provider, private_key);
        let contract = zERC20::new(self.address, signer.clone());
        let call = contract.mint(to, amount).with_cloned_provider();
        send_call_with_legacy(call, &signer, self.legacy_tx).await
    }

    pub async fn teleport(
        &self,
        private_key: B256,
        to: Address,
        amount: U256,
    ) -> ContractResult<PendingTransactionBuilder<Ethereum>> {
        let signer = get_provider_with_signer(&self.provider, private_key);
        let contract = zERC20::new(self.address, signer.clone());
        let call = contract.teleport(to, amount).with_cloned_provider();
        send_call_with_legacy(call, &signer, self.legacy_tx).await
    }

    pub async fn set_minter(
        &self,
        private_key: B256,
        new_minter: Address,
    ) -> ContractResult<PendingTransactionBuilder<Ethereum>> {
        let signer = get_provider_with_signer(&self.provider, private_key);
        let contract = zERC20::new(self.address, signer.clone());
        let call = contract.setMinter(new_minter).with_cloned_provider();
        send_call_with_legacy(call, &signer, self.legacy_tx).await
    }
    pub async fn set_verifier(
        &self,
        private_key: B256,
        new_verifier: Address,
    ) -> ContractResult<PendingTransactionBuilder<Ethereum>> {
        let signer = get_provider_with_signer(&self.provider, private_key);
        let contract = zERC20::new(self.address, signer.clone());
        let call = contract.setVerifier(new_verifier).with_cloned_provider();
        send_call_with_legacy(call, &signer, self.legacy_tx).await
    }

    pub async fn burn(
        &self,
        private_key: B256,
        from: Address,
        amount: U256,
    ) -> ContractResult<PendingTransactionBuilder<Ethereum>> {
        let signer = get_provider_with_signer(&self.provider, private_key);
        let contract = zERC20::new(self.address, signer.clone());
        let call = contract.burn(from, amount).with_cloned_provider();
        send_call_with_legacy(call, &signer, self.legacy_tx).await
    }

    pub async fn transfer(
        &self,
        private_key: B256,
        to: Address,
        amount: U256,
    ) -> ContractResult<PendingTransactionBuilder<Ethereum>> {
        let signer = get_provider_with_signer(&self.provider, private_key);
        let contract = zERC20::new(self.address, signer.clone());
        let call = contract.transfer(to, amount).with_cloned_provider();
        send_call_with_legacy(call, &signer, self.legacy_tx).await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn permit(
        &self,
        private_key: B256,
        owner: Address,
        spender: Address,
        value: U256,
        deadline: U256,
        v: u8,
        r: B256,
        s: B256,
    ) -> ContractResult<PendingTransactionBuilder<Ethereum>> {
        let signer = get_provider_with_signer(&self.provider, private_key);
        let contract = zERC20::new(self.address, signer.clone());
        let call = contract
            .permit(owner, spender, value, deadline, v, r, s)
            .with_cloned_provider();
        send_call_with_legacy(call, &signer, self.legacy_tx).await
    }

    pub async fn balance_of(&self, account: Address) -> ContractResult<U256> {
        let contract = zERC20::new(self.address, self.provider.clone());
        let bal = contract.balanceOf(account).call().await?;
        Ok(bal)
    }

    pub async fn quote_send(&self, send_param: SendParam) -> ContractResult<MessagingFee> {
        let contract = zERC20::new(self.address, self.provider.clone());
        let param = zERC20::SendParam::from(send_param);
        let fee = contract.quoteSend(param, false).call().await?;
        Ok(MessagingFee::from(fee))
    }

    pub async fn send(
        &self,
        private_key: B256,
        send_param: SendParam,
        fee: MessagingFee,
        refund_address: Address,
    ) -> ContractResult<PendingTransactionBuilder<Ethereum>> {
        let signer = get_provider_with_signer(&self.provider, private_key);
        let contract = zERC20::new(self.address, signer.clone());
        let param = zERC20::SendParam::from(send_param);
        let fee_param = zERC20::MessagingFee::from(fee);
        let native_fee = fee_param.nativeFee;
        let call = contract
            .send(param, fee_param, refund_address)
            .value(native_fee)
            .with_cloned_provider();
        send_call_with_legacy(call, &signer, self.legacy_tx).await
    }

    pub async fn get_indexed_transfer_events(
        &self,
        from_block: u64,
        to_block: u64,
    ) -> ContractResult<Vec<IndexedEvent>> {
        let contract = zERC20::new(self.address, self.provider.clone());
        let event_with_logs = contract
            .event_filter::<zERC20::IndexedTransfer>()
            .address(self.address)
            .from_block(from_block)
            .to_block(to_block)
            .query()
            .await?;
        let events = event_with_logs
            .into_iter()
            .map(|(e, l)| IndexedEvent {
                event_index: uint256_as_u64(e.index),
                from: e.from,
                to: e.to,
                value: e.value,
                eth_block_number: l.block_number.unwrap_or_default(),
            })
            .collect();
        Ok(events)
    }

    pub async fn get_teleport_events(
        &self,
        from_block: u64,
        to_block: u64,
    ) -> ContractResult<Vec<TeleportEvent>> {
        let contract = zERC20::new(self.address, self.provider.clone());
        let events = contract
            .event_filter::<zERC20::Teleport>()
            .address(self.address)
            .from_block(from_block)
            .to_block(to_block)
            .query()
            .await?;
        Ok(events
            .into_iter()
            .map(|(event, log)| TeleportEvent {
                to: event.to,
                value: event.value,
                eth_block_number: log.block_number.unwrap_or_default(),
            })
            .collect())
    }

    // for event polling
    pub async fn latest_block(&self) -> ContractResult<u64> {
        let n = self
            .provider
            .get_block_number()
            .await
            .map_err(|err| ContractError::transport("get_block_number", err))?;
        Ok(n)
    }

    pub async fn block_hash_by_number(&self, number: u64) -> ContractResult<B256> {
        let block = self
            .provider
            .get_block_by_number(BlockNumberOrTag::Number(number))
            .await
            .map_err(|err| ContractError::transport("get_block_by_number", err))?;
        let Some(block) = block else {
            return Err(ContractError::BlockNotFound(number));
        };
        Ok(block.header().hash())
    }
}

impl From<zERC20::eip712DomainReturn> for Eip712Domain {
    fn from(value: zERC20::eip712DomainReturn) -> Self {
        Self {
            fields: value.fields.as_slice()[0],
            name: value.name,
            version: value.version,
            chain_id: value.chainId,
            verifying_contract: value.verifyingContract,
            salt: value.salt,
            extensions: value.extensions,
        }
    }
}

impl From<SendParam> for zERC20::SendParam {
    fn from(value: SendParam) -> Self {
        Self {
            dstEid: value.dst_eid,
            to: value.to,
            amountLD: value.amount_ld,
            minAmountLD: value.min_amount_ld,
            extraOptions: value.extra_options,
            composeMsg: value.compose_msg,
            oftCmd: value.oft_cmd,
        }
    }
}

impl From<zERC20::MessagingFee> for MessagingFee {
    fn from(value: zERC20::MessagingFee) -> Self {
        Self {
            native_fee: value.nativeFee,
            lz_token_fee: value.lzTokenFee,
        }
    }
}

impl From<MessagingFee> for zERC20::MessagingFee {
    fn from(value: MessagingFee) -> Self {
        Self {
            nativeFee: value.native_fee,
            lzTokenFee: value.lz_token_fee,
        }
    }
}
