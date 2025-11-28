use crate::contracts::{
    ContractResult,
    utils::{NormalProvider, get_provider_with_signer, send_call_with_legacy},
};
use alloy::{
    network::Ethereum,
    primitives::{Address, B256, U256},
    providers::PendingTransactionBuilder,
    sol,
};

sol!(
    #[sol(rpc)]
    ERC20,
    "abi/ERC20.json",
);

#[derive(Clone)]
pub struct Erc20Contract {
    provider: NormalProvider,
    address: Address,
    legacy_tx: bool,
}

impl Erc20Contract {
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

    pub async fn balance_of(&self, account: Address) -> ContractResult<U256> {
        let contract = ERC20::new(self.address, self.provider.clone());
        let bal = contract.balanceOf(account).call().await?;
        Ok(bal)
    }

    pub async fn allowance(&self, owner: Address, spender: Address) -> ContractResult<U256> {
        let contract = ERC20::new(self.address, self.provider.clone());
        let allowance = contract.allowance(owner, spender).call().await?;
        Ok(allowance)
    }

    pub async fn approve(
        &self,
        private_key: B256,
        spender: Address,
        amount: U256,
    ) -> ContractResult<PendingTransactionBuilder<Ethereum>> {
        let signer = get_provider_with_signer(&self.provider, private_key);
        let contract = ERC20::new(self.address, signer.clone());
        let call = contract.approve(spender, amount).with_cloned_provider();
        send_call_with_legacy(call, &signer, self.legacy_tx).await
    }

    pub const fn legacy_tx(&self) -> bool {
        self.legacy_tx
    }

    pub fn with_legacy_tx(mut self, legacy_tx: bool) -> Self {
        self.legacy_tx = legacy_tx;
        self
    }
}
