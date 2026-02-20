use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub enum DbType {
    MariaDB,
    PostgreSQL,
}

impl std::fmt::Display for DbType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ConnectionDetails {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: Option<String>,
    pub database: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DatabaseConfig {
    pub name: String,
    pub db_type: DbType,
    pub connection: ConnectionDetails,
    pub output_dir: PathBuf,
    pub retention_count: usize,
    /// Cron expression for scheduling (e.g., "0 0 * * * *")
    /// If None, it won't be scheduled automatically.
    /// Cron expression for scheduling (e.g., "0 0 * * * *")
    /// If None, it won't be scheduled automatically.
    pub schedule: Option<String>,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_enabled() -> bool {
    true
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct AppConfig {
    pub databases: Vec<DatabaseConfig>,
}
