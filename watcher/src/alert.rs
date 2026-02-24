use std::collections::HashMap;
use std::fmt;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use log::{error, info, warn};
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

// -- Platform-neutral embed types --

#[derive(Clone, Serialize)]
pub struct Embed {
    pub title: String,
    pub description: String,
    pub color: u32,
    pub fields: Vec<EmbedField>,
}

#[derive(Clone, Serialize)]
pub struct EmbedField {
    pub name: String,
    pub value: String,
    pub inline: bool,
}

// -- Webhook backends --

#[derive(Debug, Clone)]
pub enum WebhookBackend {
    Discord(String),
    Slack(String),
}

pub struct AlertManager {
    cooldown: Duration,
    last_sent: HashMap<String, Instant>,
    backends: Vec<WebhookBackend>,
    client: reqwest::Client,
}

impl AlertManager {
    pub fn new(backends: Vec<WebhookBackend>, cooldown_seconds: u64) -> Self {
        Self {
            cooldown: Duration::from_secs(cooldown_seconds),
            last_sent: HashMap::new(),
            backends,
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

        // Max 10 embeds per request (Discord limit; Slack has no such limit but we keep consistent)
        let mut send_errors: Vec<String> = Vec::new();
        for chunk in filtered.chunks(10) {
            let embeds: Vec<Embed> = chunk.iter().map(alert_to_embed).collect();
            if let Err(err) = self.send_to_backends(&embeds).await {
                send_errors.push(err.to_string());
            }

            // Record cooldown regardless of partial backend failures so that
            // successfully delivered alerts are not re-sent every cycle.
            for alert in chunk {
                let key = alert.dedup_key();
                self.last_sent.insert(key, now);
            }
        }

        // Clean up expired cooldown entries
        self.last_sent
            .retain(|_, last| now.duration_since(*last) < self.cooldown * 2);

        if send_errors.is_empty() {
            Ok(())
        } else {
            anyhow::bail!("alert send failures: {}", send_errors.join("; "))
        }
    }

    /// Send pre-built embeds directly (no cooldown filtering).
    pub async fn send_embeds(&self, embeds: Vec<Embed>) -> Result<()> {
        if embeds.is_empty() {
            return Ok(());
        }
        for chunk in embeds.chunks(10) {
            self.send_to_backends(chunk).await?;
        }
        Ok(())
    }

    /// Send embeds to all configured backends. Errors are collected and
    /// returned as a single combined error so that one backend failure does
    /// not prevent delivery to the others.
    async fn send_to_backends(&self, embeds: &[Embed]) -> Result<()> {
        let mut errors: Vec<String> = Vec::new();

        for backend in &self.backends {
            let result = match backend {
                WebhookBackend::Discord(url) => self.post_discord(url, embeds).await,
                WebhookBackend::Slack(url) => self.post_slack(url, embeds).await,
            };
            if let Err(err) = result {
                let label = match backend {
                    WebhookBackend::Discord(_) => "Discord",
                    WebhookBackend::Slack(_) => "Slack",
                };
                error!("failed to send to {}: {:?}", label, err);
                errors.push(format!("{}: {}", label, err));
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            anyhow::bail!("webhook send failures: {}", errors.join("; "))
        }
    }

    // -- Discord --

    async fn post_discord(&self, url: &str, embeds: &[Embed]) -> Result<()> {
        let discord_embeds: Vec<DiscordEmbed> = embeds.iter().map(to_discord_embed).collect();
        let payload = DiscordWebhookPayload {
            embeds: discord_embeds,
            username: Some("zERC20 Watcher".to_string()),
        };

        let resp = self
            .client
            .post(url)
            .json(&payload)
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

    // -- Slack --

    async fn post_slack(&self, url: &str, embeds: &[Embed]) -> Result<()> {
        let attachments: Vec<SlackAttachment> = embeds.iter().map(to_slack_attachment).collect();
        let payload = SlackWebhookPayload {
            username: Some("zERC20 Watcher".to_string()),
            attachments,
        };

        let resp = self
            .client
            .post(url)
            .json(&payload)
            .send()
            .await
            .context("failed to POST to Slack webhook")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp
                .text()
                .await
                .unwrap_or_else(|_| "<unreadable>".to_string());
            warn!("Slack webhook returned {}: {}", status, body);
            anyhow::bail!("Slack webhook returned HTTP {}", status);
        }
        Ok(())
    }
}

// -- Discord payload types (internal) --

#[derive(Serialize)]
struct DiscordWebhookPayload {
    embeds: Vec<DiscordEmbed>,
    #[serde(skip_serializing_if = "Option::is_none")]
    username: Option<String>,
}

#[derive(Clone, Serialize)]
struct DiscordEmbed {
    title: String,
    description: String,
    color: u32,
    fields: Vec<DiscordField>,
}

#[derive(Clone, Serialize)]
struct DiscordField {
    name: String,
    value: String,
    inline: bool,
}

fn to_discord_embed(embed: &Embed) -> DiscordEmbed {
    DiscordEmbed {
        title: embed.title.clone(),
        description: embed.description.clone(),
        color: embed.color,
        fields: embed
            .fields
            .iter()
            .map(|f| DiscordField {
                name: f.name.clone(),
                value: f.value.clone(),
                inline: f.inline,
            })
            .collect(),
    }
}

// -- Slack payload types (internal) --

#[derive(Serialize)]
struct SlackWebhookPayload {
    #[serde(skip_serializing_if = "Option::is_none")]
    username: Option<String>,
    attachments: Vec<SlackAttachment>,
}

#[derive(Serialize)]
struct SlackAttachment {
    title: String,
    text: String,
    color: String,
    fields: Vec<SlackField>,
}

#[derive(Serialize)]
struct SlackField {
    title: String,
    value: String,
    short: bool,
}

fn color_to_hex(color: u32) -> String {
    format!("#{:06X}", color)
}

fn to_slack_attachment(embed: &Embed) -> SlackAttachment {
    SlackAttachment {
        title: embed.title.clone(),
        text: embed.description.clone(),
        color: color_to_hex(embed.color),
        fields: embed
            .fields
            .iter()
            .map(|f| SlackField {
                title: f.name.clone(),
                value: f.value.clone(),
                short: f.inline,
            })
            .collect(),
    }
}

// -- Alert → Embed conversion --

fn alert_to_embed(alert: &Alert) -> Embed {
    let mut fields: Vec<EmbedField> = vec![
        EmbedField {
            name: "Severity".to_string(),
            value: alert.severity.to_string(),
            inline: true,
        },
        EmbedField {
            name: "Domain".to_string(),
            value: alert.domain.clone(),
            inline: true,
        },
    ];
    for f in &alert.fields {
        fields.push(EmbedField {
            name: f.name.clone(),
            value: f.value.clone(),
            inline: f.inline,
        });
    }

    Embed {
        title: alert.title.clone(),
        description: alert.description.clone(),
        color: alert.severity.color(),
        fields,
    }
}
