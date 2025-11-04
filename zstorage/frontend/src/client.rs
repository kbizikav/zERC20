use crate::{error::ClientError, error::ClientResult, types};
use candid::{Decode, Encode, Principal};
use ic_agent::Agent;

pub struct StealthCanisterClient {
    agent: Agent,
    storage_canister_id: Principal,
    key_manager_canister_id: Principal,
}

impl StealthCanisterClient {
    pub fn new(
        agent: Agent,
        storage_canister_id: Principal,
        key_manager_canister_id: Principal,
    ) -> Self {
        Self {
            agent,
            storage_canister_id,
            key_manager_canister_id,
        }
    }

    pub fn agent(&self) -> &Agent {
        &self.agent
    }

    pub fn storage_canister_id(&self) -> Principal {
        self.storage_canister_id
    }

    pub fn key_manager_canister_id(&self) -> Principal {
        self.key_manager_canister_id
    }

    pub async fn get_view_public_key(&self, address: [u8; 20]) -> ClientResult<Vec<u8>> {
        let arg = candid::Encode!(&address.to_vec())?;
        let response = self
            .agent
            .update(&self.key_manager_canister_id, "get_view_public_key")
            .with_arg(arg)
            .call_and_wait()
            .await?;
        let result: std::result::Result<Vec<u8>, String> =
            candid::Decode!(&response, std::result::Result<Vec<u8>, String>)?;
        result.map_err(ClientError::Canister)
    }

    pub async fn request_encrypted_view_key(
        &self,
        request: &types::EncryptedViewKeyRequest,
    ) -> ClientResult<Vec<u8>> {
        let arg = candid::Encode!(request)?;
        let response = self
            .agent
            .update(&self.key_manager_canister_id, "request_encrypted_view_key")
            .with_arg(arg)
            .call_and_wait()
            .await?;
        let result: std::result::Result<Vec<u8>, String> =
            candid::Decode!(&response, std::result::Result<Vec<u8>, String>)?;
        result.map_err(ClientError::Canister)
    }

    pub async fn get_max_nonce(&self, address: [u8; 20]) -> ClientResult<u64> {
        let arg = candid::Encode!(&address.to_vec())?;
        let response = self
            .agent
            .query(&self.key_manager_canister_id, "get_max_nonce")
            .with_arg(arg)
            .call()
            .await?;
        let result: std::result::Result<u64, String> =
            candid::Decode!(&response, std::result::Result<u64, String>)?;
        result.map_err(ClientError::Canister)
    }

    pub async fn submit_announcement(
        &self,
        input: &types::AnnouncementInput,
    ) -> ClientResult<types::Announcement> {
        let arg = candid::Encode!(input)?;
        let response = self
            .agent
            .update(&self.storage_canister_id, "submit_announcement")
            .with_arg(arg)
            .call_and_wait()
            .await?;
        let result: std::result::Result<types::Announcement, String> =
            candid::Decode!(&response, std::result::Result<types::Announcement, String>)?;
        result.map_err(ClientError::Canister)
    }

    pub async fn submit_invoice(&self, input: &types::InvoiceSubmission) -> ClientResult<()> {
        let arg = candid::Encode!(input)?;
        let response = self
            .agent
            .update(&self.storage_canister_id, "submit_invoice")
            .with_arg(arg)
            .call_and_wait()
            .await?;
        let result: std::result::Result<(), String> =
            candid::Decode!(&response, std::result::Result<(), String>)?;
        result.map_err(ClientError::Canister)
    }

    pub async fn list_invoices(&self, address: [u8; 20]) -> ClientResult<Vec<Vec<u8>>> {
        let arg = candid::Encode!(&address.to_vec())?;
        let response = self
            .agent
            .query(&self.storage_canister_id, "list_invoices")
            .with_arg(arg)
            .call()
            .await?;
        let result: std::result::Result<Vec<Vec<u8>>, String> =
            candid::Decode!(&response, std::result::Result<Vec<Vec<u8>>, String>)?;
        result.map_err(ClientError::Canister)
    }

    pub async fn list_announcements(
        &self,
        start_after: Option<u64>,
        limit: Option<u32>,
    ) -> ClientResult<types::AnnouncementPage> {
        let arg = candid::Encode!(&start_after, &limit)?;
        let response = self
            .agent
            .query(&self.storage_canister_id, "list_announcements")
            .with_arg(arg)
            .call()
            .await?;
        let page = candid::Decode!(&response, types::AnnouncementPage)?;
        Ok(page)
    }

    pub async fn get_announcement(&self, id: u64) -> ClientResult<Option<types::Announcement>> {
        let arg = candid::Encode!(&id)?;
        let response = self
            .agent
            .query(&self.storage_canister_id, "get_announcement")
            .with_arg(arg)
            .call()
            .await?;
        let (announcement,) = candid::Decode!(&response, (Option<types::Announcement>,))?;
        Ok(announcement)
    }
}
