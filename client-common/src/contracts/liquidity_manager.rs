// SPDX-License-Identifier: BUSL-1.1

use alloy::{
    network::Ethereum,
    primitives::{Address, B256, U256},
    providers::{PendingTransactionBuilder, Provider},
    rpc::types::TransactionReceipt,
    sol,
};

use crate::contracts::{
    ContractError, ContractResult,
    utils::{NormalProvider, get_provider_with_signer, send_call_with_legacy},
};

sol!(
    #[sol(rpc, all_derives)]
    LiquidityManager,
    "abi/LiquidityManager.json",
);

// Re-export IncentiveLib::FeeParams for use in set_fee_params
use IncentiveLib::FeeParams;

#[derive(Debug, Clone)]
pub struct WrappedEvent {
    pub caller: Address,
    pub receiver: Address,
    pub amount_out: U256,
    pub reward: U256,
}

#[derive(Debug, Clone)]
pub struct UnwrappedEvent {
    pub caller: Address,
    pub receiver: Address,
    pub amount_out: U256,
    pub fee_amount: U256,
}

#[derive(Clone)]
pub struct LiquidityManagerContract {
    provider: NormalProvider,
    address: Address,
    legacy_tx: bool,
}

impl LiquidityManagerContract {
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

    fn contract_with_provider(&self) -> LiquidityManager::LiquidityManagerInstance<NormalProvider> {
        LiquidityManager::new(self.address, self.provider.clone())
    }

    pub const fn legacy_tx(&self) -> bool {
        self.legacy_tx
    }

    pub fn with_legacy_tx(mut self, legacy_tx: bool) -> Self {
        self.legacy_tx = legacy_tx;
        self
    }

    pub async fn underlying_token(&self) -> ContractResult<Address> {
        let addr = self
            .contract_with_provider()
            .underlyingToken()
            .call()
            .await?;
        Ok(addr)
    }

    pub async fn zerc20(&self) -> ContractResult<Address> {
        let addr = self.contract_with_provider().zerc20().call().await?;
        Ok(addr)
    }

    pub async fn quote_wrap_reward(&self, amount: U256) -> ContractResult<U256> {
        let reward = self
            .contract_with_provider()
            .quoteWrapReward(amount)
            .call()
            .await?;
        Ok(reward)
    }

    pub async fn quote_unwrap_fee(&self, amount: U256) -> ContractResult<U256> {
        let fee = self
            .contract_with_provider()
            .quoteUnwrapFee(amount)
            .call()
            .await?;
        Ok(fee)
    }

    pub async fn wrap(
        &self,
        private_key: B256,
        amount: U256,
        receiver: Address,
    ) -> ContractResult<PendingTransactionBuilder<Ethereum>> {
        self.wrap_with_value(private_key, amount, receiver, U256::ZERO)
            .await
    }

    pub async fn wrap_with_value(
        &self,
        private_key: B256,
        amount: U256,
        receiver: Address,
        value: U256,
    ) -> ContractResult<PendingTransactionBuilder<Ethereum>> {
        let signer = get_provider_with_signer(&self.provider, private_key);
        let contract = LiquidityManager::new(self.address, signer.clone());
        let call = contract
            .wrap(amount, receiver)
            .value(value)
            .with_cloned_provider();
        send_call_with_legacy(call, &signer, self.legacy_tx).await
    }

    pub async fn unwrap(
        &self,
        private_key: B256,
        amount: U256,
        receiver: Address,
    ) -> ContractResult<PendingTransactionBuilder<Ethereum>> {
        let signer = get_provider_with_signer(&self.provider, private_key);
        let contract = LiquidityManager::new(self.address, signer.clone());
        let call = contract.unwrap(amount, receiver).with_cloned_provider();
        send_call_with_legacy(call, &signer, self.legacy_tx).await
    }

    pub fn parse_wrapped(&self, receipt: &TransactionReceipt) -> ContractResult<WrappedEvent> {
        for log in receipt.logs() {
            match log.log_decode_validate::<LiquidityManager::Wrapped>() {
                Ok(event) => {
                    let inner = event.inner;
                    return Ok(WrappedEvent {
                        caller: inner.caller,
                        receiver: inner.receiver,
                        amount_out: inner.amountOut,
                        reward: inner.reward,
                    });
                }
                Err(_) => continue,
            }
        }
        Err(ContractError::MissingEvent("Wrapped"))
    }

    pub fn parse_unwrapped(&self, receipt: &TransactionReceipt) -> ContractResult<UnwrappedEvent> {
        for log in receipt.logs() {
            match log.log_decode_validate::<LiquidityManager::Unwrapped>() {
                Ok(event) => {
                    let inner = event.inner;
                    return Ok(UnwrappedEvent {
                        caller: inner.caller,
                        receiver: inner.receiver,
                        amount_out: inner.amountOut,
                        fee_amount: inner.feeAmount,
                    });
                }
                Err(_) => continue,
            }
        }
        Err(ContractError::MissingEvent("Unwrapped"))
    }

    pub async fn latest_block(&self) -> ContractResult<u64> {
        self.provider
            .get_block_number()
            .await
            .map_err(|err| ContractError::transport("get_block_number", err))
    }

    pub async fn fee_params(&self) -> ContractResult<(U256, U256)> {
        let params = self.contract_with_provider().feeParams().call().await?;
        Ok((params.targetLiquidity, params.k))
    }

    pub async fn fee_surplus(&self) -> ContractResult<U256> {
        let surplus = self.contract_with_provider().feeSurplus().call().await?;
        Ok(surplus)
    }

    pub async fn set_fee_params(
        &self,
        private_key: B256,
        target_liquidity: U256,
        k: U256,
    ) -> ContractResult<PendingTransactionBuilder<Ethereum>> {
        let signer = get_provider_with_signer(&self.provider, private_key);
        let contract = LiquidityManager::new(self.address, signer.clone());
        let params = FeeParams {
            targetLiquidity: target_liquidity,
            k,
        };
        let call = contract.setFeeParams(params).with_cloned_provider();
        send_call_with_legacy(call, &signer, self.legacy_tx).await
    }
}
