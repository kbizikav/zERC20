use crate::contracts::{ContractResult, utils::NormalProvider};
use alloy::{
    primitives::{Address, U256},
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
}

impl Erc20Contract {
    pub fn new(provider: NormalProvider, address: Address) -> Self {
        Self { provider, address }
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
}
