pub mod anvil;

use std::{
    net::TcpListener,
    process::{Command, Stdio},
    time::Duration,
};

use anyhow::{Context, Result};
use rand::{Rng, distributions::Alphanumeric};
use sqlx::{
    Connection, Executor, PgPool,
    postgres::{PgConnection, PgPoolOptions},
};
use tokio::time::sleep;

const POSTGRES_IMAGE: &str = "postgres:16.6";
const POSTGRES_USER: &str = "postgres";
const POSTGRES_PASSWORD: &str = "password";

/// Helper that boots a disposable Postgres instance in Docker for tests.
pub struct TestDatabase {
    pool: PgPool,
    container_name: String,
}

impl TestDatabase {
    /// Start a fresh Postgres container and create a unique database scoped by `prefix`.
    pub async fn create(prefix: &str) -> Result<Self> {
        let container_suffix = random_suffix();
        let container_name = format!("{prefix}_pg_{container_suffix}");
        let port = find_free_port()?;

        stop_container(&container_name)?;
        start_container(&container_name, port)?;
        if let Err(err) = wait_for_postgres(port).await {
            let _ = stop_container(&container_name);
            return Err(err);
        }

        let db_name = format!("{prefix}_db_{container_suffix}");
        if let Err(err) = create_database(port, &db_name).await {
            let _ = stop_container(&container_name);
            return Err(err);
        }

        let database_url =
            format!("postgres://{POSTGRES_USER}:{POSTGRES_PASSWORD}@localhost:{port}/{db_name}");
        let pool = match PgPoolOptions::new()
            .max_connections(5)
            .acquire_timeout(Duration::from_secs(5))
            .connect(&database_url)
            .await
        {
            Ok(pool) => pool,
            Err(err) => {
                let _ = stop_container(&container_name);
                return Err(err).context("failed to connect to test database");
            }
        };

        Ok(Self {
            pool,
            container_name,
        })
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    pub async fn cleanup(self) -> Result<()> {
        self.pool.close().await;
        let container = self.container_name.clone();
        stop_container(&container)?;
        Ok(())
    }
}

impl Drop for TestDatabase {
    fn drop(&mut self) {
        let _ = stop_container(&self.container_name);
    }
}

fn random_suffix() -> String {
    let mut rng = rand::thread_rng();
    (&mut rng)
        .sample_iter(&Alphanumeric)
        .take(10)
        .map(char::from)
        .collect::<String>()
        .to_lowercase()
}

fn find_free_port() -> Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0").context("failed to bind ephemeral port")?;
    let port = listener
        .local_addr()
        .context("failed to query socket addr")?
        .port();
    Ok(port)
}

fn start_container(name: &str, port: u16) -> Result<()> {
    let port_arg = format!("{port}:5432");
    let output = Command::new("docker")
        .args([
            "run",
            "-d",
            "--rm",
            "--name",
            name,
            "-e",
            "POSTGRES_USER=postgres",
            "-e",
            "POSTGRES_PASSWORD=password",
            "-e",
            "POSTGRES_DB=postgres",
            "-p",
            &port_arg,
            POSTGRES_IMAGE,
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .context("failed to execute docker run")?;

    if !output.status.success() {
        return Err(anyhow::anyhow!(
            "failed to start postgres container {name}: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    Ok(())
}

fn stop_container(name: &str) -> Result<()> {
    let status = Command::new("docker")
        .args(["rm", "-f", name])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .context("failed to execute docker rm -f")?;

    if !status.success() {
        // Ignore errors from removing non-existent containers.
        return Ok(());
    }
    Ok(())
}

async fn wait_for_postgres(port: u16) -> Result<()> {
    let admin_url =
        format!("postgres://{POSTGRES_USER}:{POSTGRES_PASSWORD}@localhost:{port}/postgres");

    const MAX_ATTEMPTS: usize = 20;
    for attempt in 0..MAX_ATTEMPTS {
        match PgConnection::connect(&admin_url).await {
            Ok(conn) => {
                conn.close().await.ok();
                return Ok(());
            }
            Err(err) if attempt + 1 == MAX_ATTEMPTS => {
                return Err(anyhow::anyhow!("postgres did not become ready: {err}"));
            }
            Err(_) => sleep(Duration::from_millis(200)).await,
        }
    }
    unreachable!("wait loop should have returned or errored");
}

async fn create_database(port: u16, db_name: &str) -> Result<()> {
    let admin_url =
        format!("postgres://{POSTGRES_USER}:{POSTGRES_PASSWORD}@localhost:{port}/postgres");
    let mut conn = PgConnection::connect(&admin_url)
        .await
        .context("failed to connect to admin database")?;

    let create_sql = format!(r#"CREATE DATABASE "{db_name}""#);
    conn.execute(create_sql.as_str())
        .await
        .context("failed to create test database")?;
    conn.close().await.ok();
    Ok(())
}
