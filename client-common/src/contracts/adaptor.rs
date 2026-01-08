use alloy::{
    network::Ethereum,
    primitives::{Address, B256, Bytes, U256},
    providers::{PendingTransactionBuilder, Provider},
    rpc::types::TransactionReceipt,
    sol,
};

use crate::contracts::{
    ContractError, ContractResult,
    utils::{NormalProvider, get_provider_with_signer, send_call_with_legacy},
};

sol!(
    #[sol(rpc)]
    Adaptor,
    "abi/Adaptor.json",
);

#[derive(Debug, Clone)]
pub struct BridgeRequest {
    pub dst_eid: u32,
    pub to: Address,
    pub min_amount_out: U256,
    pub extra_options: Bytes,
    pub compose_msg: Bytes,
    pub oft_cmd: Bytes,
}

impl From<BridgeRequest> for Adaptor::BridgeRequest {
    fn from(value: BridgeRequest) -> Self {
        Self {
            dstEid: value.dst_eid,
            to: value.to,
            minAmountOut: value.min_amount_out,
            extraOptions: value.extra_options,
            composeMsg: value.compose_msg,
            oftCmd: value.oft_cmd,
        }
    }
}

impl From<Adaptor::BridgeRequest> for BridgeRequest {
    fn from(value: Adaptor::BridgeRequest) -> Self {
        Self {
            dst_eid: value.dstEid,
            to: value.to,
            min_amount_out: value.minAmountOut,
            extra_options: value.extraOptions,
            compose_msg: value.composeMsg,
            oft_cmd: value.oftCmd,
        }
    }
}

#[derive(Debug, Clone)]
pub struct FeeQuote {
    pub token_unwrap_fee: U256,
    pub native_bridge_fee: U256,
    pub token_bridge_fee: U256,
}

impl From<Adaptor::FeeQuote> for FeeQuote {
    fn from(value: Adaptor::FeeQuote) -> Self {
        Self {
            token_unwrap_fee: value.tokenUnwrapFee,
            native_bridge_fee: value.nativeBridgeFee,
            token_bridge_fee: value.tokenBridgeFee,
        }
    }
}

#[derive(Debug, Clone)]
pub struct UnwrapAndBridgeEvent {
    pub caller: Address,
    pub amount_in: U256,
    pub amount_out: U256,
    pub receiver: Address,
    pub dst_eid: u32,
}

#[derive(Debug, Clone)]
pub struct BridgeUnderlyingTokenEvent {
    pub user: Address,
    pub to: Address,
    pub dst_eid: u32,
    pub amount_out: U256,
    pub native_fee_used: U256,
}

#[derive(Debug, Clone)]
pub struct BridgeZerc20Event {
    pub to: Address,
    pub dst_eid: u32,
    pub amount_returned: U256,
}

#[derive(Debug, Clone)]
pub struct BridgeUnderlyingTokenFailedEvent {
    pub user: Address,
    pub to: Address,
    pub dst_eid: u32,
    pub amount: U256,
    pub native_bridge_fee: U256,
    pub min_amount_out: U256,
    pub revert_data: Bytes,
}

#[derive(Debug, Clone)]
pub struct BridgeZerc20FailedEvent {
    pub user: Address,
    pub to: Address,
    pub dst_eid: u32,
    pub amount: U256,
    pub revert_data: Bytes,
}

#[derive(Debug, Clone)]
pub struct DecodeBridgeRequestFailedEvent {
    pub message: Bytes,
    pub revert_data: Bytes,
}

#[derive(Debug, Clone)]
pub struct QuoteFailedEvent {
    pub amount: U256,
    pub request: BridgeRequest,
    pub revert_data: Bytes,
}

#[derive(Debug, Clone)]
pub struct UnwrapFailedEvent {
    pub user: Address,
    pub amount: U256,
    pub min_amount_out: U256,
    pub revert_data: Bytes,
}

#[derive(Clone)]
pub struct AdaptorContract {
    provider: NormalProvider,
    address: Address,
    legacy_tx: bool,
}

impl AdaptorContract {
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

    fn contract_with_provider(&self) -> Adaptor::AdaptorInstance<NormalProvider> {
        Adaptor::new(self.address, self.provider.clone())
    }

    pub const fn legacy_tx(&self) -> bool {
        self.legacy_tx
    }

    pub fn with_legacy_tx(mut self, legacy_tx: bool) -> Self {
        self.legacy_tx = legacy_tx;
        self
    }

    pub async fn liquidity_manager(&self) -> ContractResult<Address> {
        let addr = self
            .contract_with_provider()
            .liquidityManager()
            .call()
            .await?;
        Ok(addr)
    }

    pub async fn stargate(&self) -> ContractResult<Address> {
        let addr = self.contract_with_provider().stargate().call().await?;
        Ok(addr)
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

    pub async fn native_balance_of(&self, user: Address) -> ContractResult<U256> {
        let balance = self
            .contract_with_provider()
            .nativeBalances(user)
            .call()
            .await?;
        Ok(balance)
    }

    pub async fn underlying_balance_of(&self, user: Address) -> ContractResult<U256> {
        let balance = self
            .contract_with_provider()
            .underlyingTokenBalances(user)
            .call()
            .await?;
        Ok(balance)
    }

    pub async fn zerc20_balance_of(&self, user: Address) -> ContractResult<U256> {
        let balance = self
            .contract_with_provider()
            .zerc20Balances(user)
            .call()
            .await?;
        Ok(balance)
    }

    pub async fn quote_fee(
        &self,
        amount: U256,
        request: BridgeRequest,
    ) -> ContractResult<FeeQuote> {
        let quote = self
            .contract_with_provider()
            .quoteFee(amount, request.into())
            .call()
            .await?;
        Ok(FeeQuote::from(quote))
    }

    pub async fn unwrap_and_bridge(
        &self,
        private_key: B256,
        amount: U256,
        request: BridgeRequest,
        native_fee: U256,
    ) -> ContractResult<PendingTransactionBuilder<Ethereum>> {
        let signer = get_provider_with_signer(&self.provider, private_key);
        let contract = Adaptor::new(self.address, signer.clone());
        let call = contract
            .unwrapAndBridge(amount, request.into())
            .value(native_fee)
            .with_cloned_provider();
        send_call_with_legacy(call, &signer, self.legacy_tx).await
    }

    pub async fn withdraw(
        &self,
        private_key: B256,
        token: Address,
        amount: U256,
    ) -> ContractResult<PendingTransactionBuilder<Ethereum>> {
        let signer = get_provider_with_signer(&self.provider, private_key);
        let contract = Adaptor::new(self.address, signer.clone());
        let call = contract.withdraw(token, amount).with_cloned_provider();
        send_call_with_legacy(call, &signer, self.legacy_tx).await
    }

    pub fn parse_unwrap_and_bridge(
        &self,
        receipt: &TransactionReceipt,
    ) -> ContractResult<UnwrapAndBridgeEvent> {
        for log in receipt.logs() {
            match log.log_decode_validate::<Adaptor::UnwrapAndBridge>() {
                Ok(event) => {
                    let inner = event.inner;
                    return Ok(UnwrapAndBridgeEvent {
                        caller: inner.caller,
                        amount_in: inner.amountIn,
                        amount_out: inner.amountOut,
                        receiver: inner.receiver,
                        dst_eid: inner.dstEid,
                    });
                }
                Err(_) => continue,
            }
        }
        Err(ContractError::MissingEvent("UnwrapAndBridge"))
    }

    pub fn parse_bridge_zerc20(
        &self,
        receipt: &TransactionReceipt,
    ) -> ContractResult<BridgeZerc20Event> {
        for log in receipt.logs() {
            match log.log_decode_validate::<Adaptor::BridgeZerc20>() {
                Ok(event) => {
                    let inner = event.inner;
                    return Ok(BridgeZerc20Event {
                        to: inner.to,
                        dst_eid: inner.dstEid,
                        amount_returned: inner.amountReturned,
                    });
                }
                Err(_) => continue,
            }
        }
        Err(ContractError::MissingEvent("BridgeZerc20"))
    }

    pub fn parse_bridge_underlying_token(
        &self,
        receipt: &TransactionReceipt,
    ) -> ContractResult<BridgeUnderlyingTokenEvent> {
        for log in receipt.logs() {
            match log.log_decode_validate::<Adaptor::BridgeUnderlyingToken>() {
                Ok(event) => {
                    let inner = event.inner;
                    return Ok(BridgeUnderlyingTokenEvent {
                        user: inner.user,
                        to: inner.to,
                        dst_eid: inner.dstEid,
                        amount_out: inner.amountOut,
                        native_fee_used: inner.nativeFeeUsed,
                    });
                }
                Err(_) => continue,
            }
        }
        Err(ContractError::MissingEvent("BridgeUnderlyingToken"))
    }

    pub fn parse_bridge_underlying_token_failed(
        &self,
        receipt: &TransactionReceipt,
    ) -> ContractResult<BridgeUnderlyingTokenFailedEvent> {
        for log in receipt.logs() {
            match log.log_decode_validate::<Adaptor::BridgeUnderlyingTokenFailed>() {
                Ok(event) => {
                    let inner = event.inner;
                    return Ok(BridgeUnderlyingTokenFailedEvent {
                        user: inner.user,
                        to: inner.to,
                        dst_eid: inner.dstEid,
                        amount: inner.amount,
                        native_bridge_fee: inner.nativeBridgeFee,
                        min_amount_out: inner.minAmountOut,
                        revert_data: inner.revertData.clone(),
                    });
                }
                Err(_) => continue,
            }
        }
        Err(ContractError::MissingEvent("BridgeUnderlyingTokenFailed"))
    }

    pub fn parse_bridge_zerc20_failed(
        &self,
        receipt: &TransactionReceipt,
    ) -> ContractResult<BridgeZerc20FailedEvent> {
        for log in receipt.logs() {
            match log.log_decode_validate::<Adaptor::BridgeZerc20Failed>() {
                Ok(event) => {
                    let inner = event.inner;
                    return Ok(BridgeZerc20FailedEvent {
                        user: inner.user,
                        to: inner.to,
                        dst_eid: inner.dstEid,
                        amount: inner.amount,
                        revert_data: inner.revertData.clone(),
                    });
                }
                Err(_) => continue,
            }
        }
        Err(ContractError::MissingEvent("BridgeZerc20Failed"))
    }

    pub fn parse_decode_bridge_request_failed(
        &self,
        receipt: &TransactionReceipt,
    ) -> ContractResult<DecodeBridgeRequestFailedEvent> {
        for log in receipt.logs() {
            match log.log_decode_validate::<Adaptor::DecodeBridgeRequestFailed>() {
                Ok(event) => {
                    let inner = event.inner;
                    return Ok(DecodeBridgeRequestFailedEvent {
                        message: inner.message.clone(),
                        revert_data: inner.revertData.clone(),
                    });
                }
                Err(_) => continue,
            }
        }
        Err(ContractError::MissingEvent("DecodeBridgeRequestFailed"))
    }

    pub fn parse_quote_failed(
        &self,
        receipt: &TransactionReceipt,
    ) -> ContractResult<QuoteFailedEvent> {
        for log in receipt.logs() {
            match log.log_decode_validate::<Adaptor::QuoteFailed>() {
                Ok(event) => {
                    let inner = event.inner;
                    return Ok(QuoteFailedEvent {
                        amount: inner.amount,
                        request: BridgeRequest::from(inner.request.clone()),
                        revert_data: inner.revertData.clone(),
                    });
                }
                Err(_) => continue,
            }
        }
        Err(ContractError::MissingEvent("QuoteFailed"))
    }

    pub fn parse_unwrap_failed(
        &self,
        receipt: &TransactionReceipt,
    ) -> ContractResult<UnwrapFailedEvent> {
        for log in receipt.logs() {
            match log.log_decode_validate::<Adaptor::UnwrapFailed>() {
                Ok(event) => {
                    let inner = event.inner;
                    return Ok(UnwrapFailedEvent {
                        user: inner.user,
                        amount: inner.amount,
                        min_amount_out: inner.minAmountOut,
                        revert_data: inner.revertData.clone(),
                    });
                }
                Err(_) => continue,
            }
        }
        Err(ContractError::MissingEvent("UnwrapFailed"))
    }

    pub async fn latest_block(&self) -> ContractResult<u64> {
        self.provider
            .get_block_number()
            .await
            .map_err(|err| ContractError::transport("get_block_number", err))
    }
}
