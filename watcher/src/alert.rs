use std::collections::HashMap;
use std::fmt;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use log::{info, warn};
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Warning,
    Critical,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Severity::Warning => write!(f, "Warning"),
            Severity::Critical => write!(f, "Critical"),
        }
    }
}

impl Severity {
    fn color(&self) -> u32 {
        match self {
            Severity::Warning => 0xFFD700,  // yellow/gold
            Severity::Critical => 0xFF0000, // red
        }
    }
}

#[derive(Debug, Clone)]
pub struct AlertField {
    pub name: String,
    pub value: String,
    pub inline: bool,
}

#[derive(Debug, Clone)]
pub struct Alert {
    pub severity: Severity,
    pub domain: String,
    pub title: String,
    pub description: String,
    pub fields: Vec<AlertField>,
}

impl Alert {
    fn dedup_key(&self) -> String {
        format!("{}:{}:{}", self.domain, self.severity, self.title)
    }
}

pub struct AlertManager {
    cooldown: Duration,
    last_sent: HashMap<String, Instant>,
    webhook_url: String,
    client: reqwest::Client,
}

impl AlertManager {
    pub fn new(webhook_url: String, cooldown_seconds: u64) -> Self {
        Self {
            cooldown: Duration::from_secs(cooldown_seconds),
            last_sent: HashMap::new(),
            webhook_url,
            client: reqwest::Client::new(),
        }
    }

    pub async fn send_alerts(&mut self, alerts: Vec<Alert>) -> Result<()> {
        let now = Instant::now();
        let filtered: Vec<Alert> = alerts
            .into_iter()
            .filter(|alert| {
                let key = alert.dedup_key();
                match self.last_sent.get(&key) {
                    Some(last) if now.duration_since(*last) < self.cooldown => {
                        info!("suppressed duplicate alert: {}", key);
                        false
                    }
                    _ => true,
                }
            })
            .collect();

        if filtered.is_empty() {
            return Ok(());
        }

        // Discord allows max 10 embeds per request
        for chunk in filtered.chunks(10) {
            let embeds: Vec<DiscordEmbed> = chunk.iter().map(alert_to_embed).collect();
            let payload = DiscordWebhookPayload {
                embeds,
                username: Some("zERC20 Watcher".to_string()),
            };
            self.post_webhook(&payload)
                .await
                .with_context(|| format!("failed to send {} alert(s) to Discord", chunk.len()))?;

            for alert in chunk {
                let key = alert.dedup_key();
                self.last_sent.insert(key, now);
            }
        }

        // Clean up expired cooldown entries
        self.last_sent
            .retain(|_, last| now.duration_since(*last) < self.cooldown * 2);

        Ok(())
    }

    /// Send pre-built embeds directly (no cooldown filtering).
    pub async fn send_embeds(&self, embeds: Vec<DiscordEmbed>) -> Result<()> {
        if embeds.is_empty() {
            return Ok(());
        }
        for chunk in embeds.chunks(10) {
            let payload = DiscordWebhookPayload {
                embeds: chunk.to_vec(),
                username: Some("zERC20 Watcher".to_string()),
            };
            self.post_webhook(&payload)
                .await
                .with_context(|| format!("failed to send {} embed(s) to Discord", chunk.len()))?;
        }
        Ok(())
    }

    async fn post_webhook(&self, payload: &DiscordWebhookPayload) -> Result<()> {
        let resp = self
            .client
            .post(&self.webhook_url)
            .json(payload)
            .send()
            .await
            .context("failed to POST to Discord webhook")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp
                .text()
                .await
                .unwrap_or_else(|_| "<unreadable>".to_string());
            warn!("Discord webhook returned {}: {}", status, body);
            anyhow::bail!("Discord webhook returned HTTP {}", status);
        }
        Ok(())
    }
}

#[derive(Serialize)]
pub struct DiscordWebhookPayload {
    pub embeds: Vec<DiscordEmbed>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
}

#[derive(Clone, Serialize)]
pub struct DiscordEmbed {
    pub title: String,
    pub description: String,
    pub color: u32,
    pub fields: Vec<DiscordField>,
}

#[derive(Clone, Serialize)]
pub struct DiscordField {
    pub name: String,
    pub value: String,
    pub inline: bool,
}

fn alert_to_embed(alert: &Alert) -> DiscordEmbed {
    let mut fields: Vec<DiscordField> = vec![
        DiscordField {
            name: "Severity".to_string(),
            value: alert.severity.to_string(),
            inline: true,
        },
        DiscordField {
            name: "Domain".to_string(),
            value: alert.domain.clone(),
            inline: true,
        },
    ];
    for f in &alert.fields {
        fields.push(DiscordField {
            name: f.name.clone(),
            value: f.value.clone(),
            inline: f.inline,
        });
    }

    DiscordEmbed {
        title: alert.title.clone(),
        description: alert.description.clone(),
        color: alert.severity.color(),
        fields,
    }
}
