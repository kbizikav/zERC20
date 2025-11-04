use crate::contracts::{
    ContractResult,
    utils::{NormalProvider, get_provider_with_signer, send_call_with_legacy},
};
use alloy::network::Ethereum;
use alloy::primitives::{Address, B256, U256};
use alloy::providers::PendingTransactionBuilder;
use alloy::sol;

sol!(
    #[sol(rpc)]
    Minter,
    "abi/Minter.json",
);

#[derive(Clone)]
pub struct MinterContract {
    provider: NormalProvider,
    address: Address,
    legacy_tx: bool,
}

impl MinterContract {
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

    fn contract_with_provider(&self) -> Minter::MinterInstance<NormalProvider> {
        Minter::new(self.address, self.provider.clone())
    }

    pub const fn legacy_tx(&self) -> bool {
        self.legacy_tx
    }

    pub fn with_legacy_tx(mut self, legacy_tx: bool) -> Self {
        self.legacy_tx = legacy_tx;
        self
    }

    pub async fn zerc20_token(&self) -> ContractResult<Address> {
        let addr = self.contract_with_provider().zerc20Token().call().await?;
        Ok(addr)
    }

    pub async fn token_address(&self) -> ContractResult<Address> {
        let addr = self.contract_with_provider().tokenAddress().call().await?;
        Ok(addr)
    }

    pub async fn deposit_native(
        &self,
        private_key: B256,
        amount: U256,
    ) -> ContractResult<PendingTransactionBuilder<Ethereum>> {
        let signer = get_provider_with_signer(&self.provider, private_key);
        let contract = Minter::new(self.address, signer.clone());
        let call = contract
            .depositNative()
            .value(amount)
            .with_cloned_provider();
        send_call_with_legacy(call, &signer, self.legacy_tx).await
    }

    pub async fn deposit_token(
        &self,
        private_key: B256,
        amount: U256,
    ) -> ContractResult<PendingTransactionBuilder<Ethereum>> {
        let signer = get_provider_with_signer(&self.provider, private_key);
        let contract = Minter::new(self.address, signer.clone());
        let call = contract.depositToken(amount).with_cloned_provider();
        send_call_with_legacy(call, &signer, self.legacy_tx).await
    }

    pub async fn withdraw_native(
        &self,
        private_key: B256,
        amount: U256,
    ) -> ContractResult<PendingTransactionBuilder<Ethereum>> {
        let signer = get_provider_with_signer(&self.provider, private_key);
        let contract = Minter::new(self.address, signer.clone());
        let call = contract.withdrawNative(amount).with_cloned_provider();
        send_call_with_legacy(call, &signer, self.legacy_tx).await
    }

    pub async fn withdraw_token(
        &self,
        private_key: B256,
        amount: U256,
    ) -> ContractResult<PendingTransactionBuilder<Ethereum>> {
        let signer = get_provider_with_signer(&self.provider, private_key);
        let contract = Minter::new(self.address, signer.clone());
        let call = contract.withdrawToken(amount).with_cloned_provider();
        send_call_with_legacy(call, &signer, self.legacy_tx).await
    }
}
