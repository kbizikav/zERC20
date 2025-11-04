use candid::{CandidType, Deserialize};
use ic_cdk::api::{canister_self, time};
use ic_cdk::management_canister::{
    vetkd_derive_key, vetkd_public_key, VetKDCurve, VetKDDeriveKeyArgs, VetKDKeyId,
    VetKDPublicKeyArgs,
};
use ic_cdk_macros::{init, post_upgrade, pre_upgrade, query, update};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::convert::TryFrom;

pub mod authorization;
use authorization::{authorization_message, recover_address_from_signature, Address};

const CONTEXT_BASE: &[u8] = b"icp-stealth-announcement-v1";
type Result<T> = std::result::Result<T, String>;

#[derive(Clone, CandidType, Deserialize)]
pub struct InitArgs {
    pub key_id_name: String,
}

#[derive(Clone, CandidType, Deserialize)]
struct Config {
    key_id_name: String,
}

#[derive(Clone, CandidType, Deserialize)]
struct State {
    config: Config,
    max_nonce_by_address: BTreeMap<Address, u64>,
}

thread_local! {
    static STATE: RefCell<Option<State>> = RefCell::new(None);
}

#[derive(CandidType, Deserialize)]
pub struct EncryptedViewKeyRequest {
    pub address: Vec<u8>,
    pub transport_public_key: Vec<u8>,
    pub expiry_ns: u64,
    pub nonce: u64,
    pub signature: Vec<u8>,
}

#[init]
fn init(args: InitArgs) {
    let key_id_name = args.key_id_name;
    let config = Config { key_id_name };

    STATE.with(|state| {
        *state.borrow_mut() = Some(State {
            config,
            max_nonce_by_address: BTreeMap::new(),
        })
    });
}

#[pre_upgrade]
fn pre_upgrade() {
    let state = STATE.with(|state| state.borrow().clone());
    ic_cdk::storage::stable_save((state,)).expect("failed to persist state");
}

#[post_upgrade]
fn post_upgrade() {
    let (maybe_state,): (Option<State>,) =
        ic_cdk::storage::stable_restore().expect("failed to restore state");
    STATE.with(|state| {
        *state.borrow_mut() = maybe_state;
    });
}

#[update]
async fn get_view_public_key(address: Vec<u8>) -> Result<Vec<u8>> {
    let address = to_address(&address)?;
    let config = with_state(|state| state.config.clone())?;
    let args = VetKDPublicKeyArgs {
        canister_id: None,
        context: context_for_address(&address),
        key_id: VetKDKeyId {
            curve: VetKDCurve::Bls12_381_G2,
            name: config.key_id_name.clone(),
        },
    };

    let reply = vetkd_public_key(&args)
        .await
        .map_err(|err| err.to_string())?;
    Ok(reply.public_key)
}

#[update]
async fn request_encrypted_view_key(request: EncryptedViewKeyRequest) -> Result<Vec<u8>> {
    let address = to_address(&request.address)?;
    let config = with_state(|state| state.config.clone())?;
    let now = time();
    if request.expiry_ns < now {
        return Err("authorization expired".to_string());
    }

    STATE.with(|state| -> Result<()> {
        let mut state = state.borrow_mut();
        let state = state
            .as_mut()
            .ok_or_else(|| "key manager not initialized".to_string())?;
        let entry = state.max_nonce_by_address.entry(address).or_default();
        if request.nonce <= *entry {
            return Err("nonce reuse detected".to_string());
        }
        *entry = request.nonce;
        Ok(())
    })?;

    let message = authorization_message(
        canister_self(),
        &address,
        &request.transport_public_key,
        request.expiry_ns,
        request.nonce,
    );
    let recovered = recover_address_from_signature(&message, &request.signature)
        .map_err(|err| err.to_string())?;
    if recovered != address {
        return Err("signature does not match address".to_string());
    }

    let args = VetKDDeriveKeyArgs {
        context: context_for_address(&address),
        input: Vec::new(),
        key_id: VetKDKeyId {
            curve: VetKDCurve::Bls12_381_G2,
            name: config.key_id_name.clone(),
        },
        transport_public_key: request.transport_public_key.clone(),
    };

    let reply = vetkd_derive_key(&args)
        .await
        .map_err(|err| err.to_string())?;

    Ok(reply.encrypted_key)
}

#[query]
fn get_max_nonce(address: Vec<u8>) -> Result<u64> {
    let address = to_address(&address)?;
    let max_nonce =
        with_state(|state| state.max_nonce_by_address.get(&address).copied())?.unwrap_or(0);
    Ok(max_nonce)
}

fn with_state<F, R>(f: F) -> Result<R>
where
    F: FnOnce(&State) -> R,
{
    STATE.with(|state| {
        state
            .borrow()
            .as_ref()
            .map(f)
            .ok_or_else(|| "key manager not initialized".to_string())
    })
}

fn to_address(bytes: &[u8]) -> Result<Address> {
    <[u8; 20]>::try_from(bytes).map_err(|_| "address must be 20 bytes".to_string())
}

pub fn context_for_address(address: &Address) -> Vec<u8> {
    let mut context = CONTEXT_BASE.to_vec();
    context.extend_from_slice(address);
    context
}

ic_cdk::export_candid!();
