use crate::contracts::{
    ContractError, ContractResult,
    utils::{NormalProvider, get_provider_with_signer, send_call_with_legacy, uint256_as_u64},
};
use alloy::network::Ethereum;
use alloy::providers::{PendingTransactionBuilder, Provider};
use alloy::sol_types::SolCall;
use alloy::{
    primitives::{Address, B256, Bytes, U256},
    sol,
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
pub struct TeleportEvent {
    pub to: Address,
    pub value: U256,
    pub eth_block_number: u64,
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
    ) -> anyhow::Result<Self> {
        let signer = get_provider_with_signer(&provider, private_key);
        let implementation = zERC20::deploy(signer.clone()).await?;
        let implementation_address = *implementation.address();

        let init_data = zERC20::initializeCall {
            name_: name_,
            symbol_: symbol_,
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

    pub async fn balance_of(&self, account: Address) -> ContractResult<U256> {
        let contract = zERC20::new(self.address, self.provider.clone());
        let bal = contract.balanceOf(account).call().await?;
        Ok(bal)
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
}
