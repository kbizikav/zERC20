use std::{sync::OnceLock, time::Duration};

use anyhow::{Context, Result, bail};
use log::{error, warn};
use sqlx::{PgPool, postgres::PgQueryResult};
use tokio::{sync::oneshot, task::JoinHandle, time::sleep};
use uuid::Uuid;

const LEASE_TTL: Duration = Duration::from_secs(30);
const RENEW_FRACTION: u32 = 3; // renew every LEASE_TTL / 3

static SESSION_HOLDER: OnceLock<Uuid> = OnceLock::new();

fn session_holder() -> Uuid {
    *SESSION_HOLDER.get_or_init(Uuid::new_v4)
}

pub struct LeaseGuard {
    pool: PgPool,
    lease_key: i64,
    holder: Uuid,
    stop_tx: Option<oneshot::Sender<()>>,
    renew_task: Option<JoinHandle<()>>,
}

impl LeaseGuard {
    async fn new(pool: PgPool, lease_key: i64, holder: Uuid, ttl: Duration) -> Result<Self> {
        let (stop_tx, stop_rx) = oneshot::channel();
        let renew_task = tokio::spawn(spawn_renewal(pool.clone(), lease_key, holder, ttl, stop_rx));
        Ok(Self {
            pool,
            lease_key,
            holder,
            stop_tx: Some(stop_tx),
            renew_task: Some(renew_task),
        })
    }

    pub async fn release(mut self) -> Result<()> {
        if let Some(stop_tx) = self.stop_tx.take() {
            let _ = stop_tx.send(());
        }
        if let Some(handle) = self.renew_task.take() {
            if let Err(join_err) = handle.await {
                warn!(
                    "lease renewal task for key {} panicked: {join_err:?}",
                    self.lease_key
                );
            }
        }
        let deleted = release_lease(&self.pool, self.lease_key, self.holder).await?;
        if deleted.rows_affected() == 0 {
            bail!("lease was not held for key {}", self.lease_key);
        }
        Ok(())
    }
}

pub async fn try_acquire_lock(pool: &PgPool, key: i64) -> Result<Option<LeaseGuard>> {
    let holder = session_holder();
    let ttl_ms = i64::try_from(LEASE_TTL.as_millis()).expect("lease ttl too large");
    let row = sqlx::query_scalar::<_, bool>(
        r#"
        INSERT INTO leases (lease_key, holder, expires_at, updated_at)
        VALUES ($1, $2, now() + ($3::bigint) * interval '1 millisecond', now())
        ON CONFLICT (lease_key) DO UPDATE
            SET holder = EXCLUDED.holder,
                expires_at = EXCLUDED.expires_at,
                updated_at = now()
            WHERE leases.holder = EXCLUDED.holder
               OR leases.expires_at < now()
        RETURNING TRUE
        "#,
    )
    .bind(key)
    .bind(holder)
    .bind(ttl_ms)
    .fetch_optional(pool)
    .await
    .context("failed to attempt acquiring lease")?;

    match row {
        Some(true) => {
            let guard = LeaseGuard::new(pool.clone(), key, holder, LEASE_TTL).await?;
            Ok(Some(guard))
        }
        Some(false) => Ok(None),
        None => Ok(None),
    }
}

async fn spawn_renewal(
    pool: PgPool,
    lease_key: i64,
    holder: Uuid,
    ttl: Duration,
    mut stop_rx: oneshot::Receiver<()>,
) {
    let renew_every = ttl / RENEW_FRACTION;
    let renew_every = if renew_every.is_zero() {
        Duration::from_millis(100)
    } else {
        renew_every
    };

    loop {
        tokio::select! {
            _ = sleep(renew_every) => {
                if let Err(err) = renew_lease(&pool, lease_key, holder, ttl).await {
                    warn!(
                        "failed to renew lease for key {}: {err:?}; stopping renewal",
                        lease_key
                    );
                    break;
                }
            }
            _ = &mut stop_rx => {
                break;
            }
        }
    }
}

async fn renew_lease(pool: &PgPool, lease_key: i64, holder: Uuid, ttl: Duration) -> Result<()> {
    let ttl_ms = i64::try_from(ttl.as_millis()).expect("lease ttl too large");
    let renewed: Option<bool> = sqlx::query_scalar(
        r#"
        UPDATE leases
           SET expires_at = now() + ($3::bigint) * interval '1 millisecond',
               updated_at = now()
         WHERE lease_key = $1 AND holder = $2
        RETURNING TRUE
        "#,
    )
    .bind(lease_key)
    .bind(holder)
    .bind(ttl_ms)
    .fetch_optional(pool)
    .await
    .context("failed to renew lease")?;

    if renewed.is_none() {
        bail!("lease no longer held for key {lease_key}");
    }

    Ok(())
}

async fn release_lease(pool: &PgPool, lease_key: i64, holder: Uuid) -> Result<PgQueryResult> {
    sqlx::query("DELETE FROM leases WHERE lease_key = $1 AND holder = $2")
        .bind(lease_key)
        .bind(holder)
        .execute(pool)
        .await
        .context("failed to release lease")
}

impl Drop for LeaseGuard {
    fn drop(&mut self) {
        if self.stop_tx.is_some() {
            let pool = self.pool.clone();
            let key = self.lease_key;
            let holder = self.holder;
            let stop_tx = self.stop_tx.take();
            let renew_task = self.renew_task.take();
            tokio::spawn(async move {
                if let Some(stop_tx) = stop_tx {
                    let _ = stop_tx.send(());
                }
                if let Some(handle) = renew_task {
                    let _ = handle.await;
                }
                if let Err(err) = release_lease(&pool, key, holder).await {
                    error!(
                        "failed to release lease for key {} during drop: {err:?}",
                        key
                    );
                }
            });
        }
    }
}
