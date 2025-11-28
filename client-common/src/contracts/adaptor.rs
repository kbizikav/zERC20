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
    pub extra_options: Bytes,
    pub compose_msg: Bytes,
    pub oft_cmd: Bytes,
    pub refund_address: Address,
    pub to: Address,
    pub min_amount_out: U256,
}

impl From<BridgeRequest> for Adaptor::BridgeRequest {
    fn from(value: BridgeRequest) -> Self {
        Self {
            dstEid: value.dst_eid,
            extraOptions: value.extra_options,
            composeMsg: value.compose_msg,
            oftCmd: value.oft_cmd,
            refundAddress: value.refund_address,
            to: value.to,
            minAmountOut: value.min_amount_out,
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
pub struct SendParam {
    pub dst_eid: u32,
    pub to: B256,
    pub amount_ld: U256,
    pub min_amount_ld: U256,
    pub extra_options: Bytes,
    pub compose_msg: Bytes,
    pub oft_cmd: Bytes,
}

impl From<Adaptor::SendParam> for SendParam {
    fn from(value: Adaptor::SendParam) -> Self {
        Self {
            dst_eid: value.dstEid,
            to: value.to,
            amount_ld: value.amountLD,
            min_amount_ld: value.minAmountLD,
            extra_options: value.extraOptions,
            compose_msg: value.composeMsg,
            oft_cmd: value.oftCmd,
        }
    }
}

#[derive(Debug, Clone)]
pub struct MessagingFee {
    pub native_fee: U256,
    pub lz_token_fee: U256,
}

impl From<Adaptor::MessagingFee> for MessagingFee {
    fn from(value: Adaptor::MessagingFee) -> Self {
        Self {
            native_fee: value.nativeFee,
            lz_token_fee: value.lzTokenFee,
        }
    }
}

#[derive(Debug, Clone)]
pub struct StargateSendFailureEvent {
    pub native_fee: U256,
    pub send_param: SendParam,
    pub fee: MessagingFee,
    pub refund_address: Address,
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
pub struct ReturnZerc20Event {
    pub to: Address,
    pub dst_eid: u32,
    pub amount_returned: U256,
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
            .LIQUIDITY_MANAGER()
            .call()
            .await?;
        Ok(addr)
    }

    pub async fn stargate(&self) -> ContractResult<Address> {
        let addr = self.contract_with_provider().STARGATE().call().await?;
        Ok(addr)
    }

    pub async fn underlying_token(&self) -> ContractResult<Address> {
        let addr = self
            .contract_with_provider()
            .UNDERLYING_TOKEN()
            .call()
            .await?;
        Ok(addr)
    }

    pub async fn zerc20(&self) -> ContractResult<Address> {
        let addr = self.contract_with_provider().ZERC20().call().await?;
        Ok(addr)
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

    pub fn parse_return_zerc20(
        &self,
        receipt: &TransactionReceipt,
    ) -> ContractResult<ReturnZerc20Event> {
        for log in receipt.logs() {
            match log.log_decode_validate::<Adaptor::ReturnZerc20>() {
                Ok(event) => {
                    let inner = event.inner;
                    return Ok(ReturnZerc20Event {
                        to: inner.to,
                        dst_eid: inner.dstEid,
                        amount_returned: inner.amountReturned,
                    });
                }
                Err(_) => continue,
            }
        }
        Err(ContractError::MissingEvent("ReturnZerc20"))
    }

    pub fn parse_stargate_send_failure(
        &self,
        receipt: &TransactionReceipt,
    ) -> ContractResult<StargateSendFailureEvent> {
        for log in receipt.logs() {
            match log.log_decode_validate::<Adaptor::StargateSendFailure>() {
                Ok(event) => {
                    let inner = event.inner;
                    return Ok(StargateSendFailureEvent {
                        native_fee: inner.nativeFee,
                        send_param: SendParam::from(inner.sendParam.clone()),
                        fee: MessagingFee::from(inner.fee.clone()),
                        refund_address: inner.refundAddress,
                    });
                }
                Err(_) => continue,
            }
        }
        Err(ContractError::MissingEvent("StargateSendFailure"))
    }

    pub async fn latest_block(&self) -> ContractResult<u64> {
        self.provider
            .get_block_number()
            .await
            .map_err(|err| ContractError::transport("get_block_number", err))
    }
}
