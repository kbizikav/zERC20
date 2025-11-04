#![allow(dead_code)]

use std::{net::TcpListener, process::Stdio, time::Duration};

use alloy::{
    network::Ethereum,
    primitives::B256,
    providers::{PendingTransactionBuilder, Provider},
};
use anyhow::{Context, Result, anyhow, bail};
use client_common::contracts::utils::NormalProvider;
use tokio::{process::Command, time::sleep};

pub const DEFAULT_ANVIL_CHAIN_ID: u64 = 1337;
pub const DEFAULT_ANVIL_HOST: &str = "127.0.0.1";

pub async fn is_binary_available(bin: &str) -> bool {
    Command::new(bin)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .is_ok()
}

pub fn find_unused_port() -> Result<u16> {
    let listener = TcpListener::bind((DEFAULT_ANVIL_HOST, 0))
        .context("binding ephemeral port for RPC host")?;
    let port = listener.local_addr().context("querying local addr")?.port();
    drop(listener);
    Ok(port)
}

pub async fn wait_for_anvil(provider: &NormalProvider) -> Result<()> {
    for _ in 0..20 {
        if provider.get_block_number().await.is_ok() {
            return Ok(());
        }
        sleep(Duration::from_millis(150)).await;
    }
    Err(anyhow!("anvil RPC did not become ready in time"))
}

pub async fn await_receipt(pending: PendingTransactionBuilder<Ethereum>) -> Result<()> {
    let receipt = pending
        .get_receipt()
        .await
        .context("failed to fetch transaction receipt")?;

    if receipt.status() {
        Ok(())
    } else {
        bail!("transaction reverted: {:?}", receipt);
    }
}

pub fn parse_private_key(hex_key: &str) -> Result<B256> {
    let raw = hex::decode(hex_key.trim_start_matches("0x"))
        .context("failed to decode private key hex")?;
    if raw.len() != 32 {
        bail!("expected 32-byte private key, got {}", raw.len());
    }
    Ok(B256::from_slice(&raw))
}

pub struct AnvilInstance {
    child: tokio::process::Child,
    rpc_url: String,
}

impl AnvilInstance {
    pub async fn spawn(bin: &str, port: u16, chain_id: u64) -> Result<Self> {
        let rpc_url = format!("http://{DEFAULT_ANVIL_HOST}:{port}");

        let mut cmd = Command::new(bin);
        let child = cmd
            .arg("--host")
            .arg(DEFAULT_ANVIL_HOST)
            .arg("--port")
            .arg(port.to_string())
            .arg("--chain-id")
            .arg(chain_id.to_string())
            .arg("--silent")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .with_context(|| format!("failed to spawn anvil binary at {bin}"))?;

        Ok(Self { child, rpc_url })
    }

    pub fn rpc_url(&self) -> String {
        self.rpc_url.clone()
    }

    pub async fn stop(mut self) -> Result<()> {
        self.child.start_kill().ok();
        let _ = self.child.wait().await;
        Ok(())
    }
}

impl Drop for AnvilInstance {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}
