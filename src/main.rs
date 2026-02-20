mod models;

use anyhow::{Context, Result};
use chrono::Local;
use clap::{Parser, Subcommand};

use comfy_table::{Cell, Color, Table};
use cron::Schedule;
use dialoguer::{theme::ColorfulTheme, Confirm, Input, Password, Select};
use directories::ProjectDirs;
use log::{error, info, warn};
use models::{AppConfig, ConnectionDetails, DatabaseConfig, DbType};
use simplelog::{CombinedLogger, Config, LevelFilter, TermLogger, TerminalMode, WriteLogger};
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::str::FromStr;
use std::time::Duration;
use tokio::time::sleep;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Add a new database configuration
    Add,
    /// List all database configurations
    List,
    /// Edit an existing database configuration
    Edit { name: Option<String> },
    /// Delete a database configuration
    Delete { name: Option<String> },
    /// Run backups immediately for all configured databases
    Run,
    /// Run in daemon mode (continuous background backups based on schedule)
    Daemon,
    /// Enable a database configuration
    Start { name: String },
    /// Disable a database configuration
    Stop { name: String },
}

#[tokio::main]
async fn main() -> Result<()> {
    if std::env::var("RUST_LOG").is_err() {
        std::env::set_var("RUST_LOG", "info");
    }

    // Initialize logging
    let config_dir = ProjectDirs::from("com", "db-shield", "db-backup-rs")
        .map(|dirs| dirs.config_dir().to_path_buf())
        .ok_or_else(|| anyhow::anyhow!("Could not determine config directory"))?;

    if !config_dir.exists() {
        fs::create_dir_all(&config_dir)?;
    }

    let log_file = fs::File::create(config_dir.join("backup.log"))?;

    CombinedLogger::init(vec![
        TermLogger::new(
            LevelFilter::Info,
            Config::default(),
            TerminalMode::Mixed,
            simplelog::ColorChoice::Auto,
        ),
        WriteLogger::new(LevelFilter::Info, Config::default(), log_file),
    ])
    .unwrap_or_else(|e| println!("Failed to init logger: {}", e));

    let cli = Cli::parse();

    match cli.command {
        Commands::Add => command_add().await?,
        Commands::List => command_list()?,
        Commands::Edit { name } => command_edit(name).await?,
        Commands::Delete { name } => command_delete(name).await?,
        Commands::Run => command_run().await?,
        Commands::Daemon => command_daemon().await?,
        Commands::Start { name } => command_start(name).await?,
        Commands::Stop { name } => command_stop(name).await?,
    }

    Ok(())
}

fn get_config_path() -> Result<PathBuf> {
    let proj_dirs = ProjectDirs::from("com", "db-shield", "db-backup-rs")
        .context("Could not determine config directory")?;
    let config_dir = proj_dirs.config_dir();
    fs::create_dir_all(config_dir)?;
    Ok(config_dir.join("config.toml"))
}

fn load_config() -> Result<AppConfig> {
    let config_path = get_config_path()?;
    if !config_path.exists() {
        return Ok(AppConfig::default());
    }
    let content = fs::read_to_string(&config_path)?;
    let config: AppConfig = toml::from_str(&content)?;
    Ok(config)
}

fn save_config(config: &AppConfig) -> Result<()> {
    let config_path = get_config_path()?;
    let content = toml::to_string_pretty(config)?;
    fs::write(config_path, content)?;
    Ok(())
}

fn find_db_index(query: &str, databases: &[DatabaseConfig]) -> Result<usize> {
    // Try to parse as ID (1-based index)
    if let Ok(id) = query.parse::<usize>() {
        if id > 0 && id <= databases.len() {
            return Ok(id - 1);
        }
    }

    // Try to find by name
    if let Some(idx) = databases.iter().position(|db| db.name == query) {
        return Ok(idx);
    }

    anyhow::bail!("Database configuration not found: '{}'", query);
}

async fn command_start(query: String) -> Result<()> {
    let mut config = load_config()?;
    let idx = find_db_index(&query, &config.databases)?;

    config.databases[idx].enabled = true;

    save_config(&config)?;
    info!(
        "Enabled backup for database: {}",
        config.databases[idx].name
    );
    Ok(())
}

async fn command_stop(query: String) -> Result<()> {
    let mut config = load_config()?;
    let idx = find_db_index(&query, &config.databases)?;

    config.databases[idx].enabled = false;

    save_config(&config)?;
    info!(
        "Disabled backup for database: {}",
        config.databases[idx].name
    );
    Ok(())
}

fn get_schedule_input() -> Result<String> {
    let options = vec![
        "Every Minute (Test)",
        "Hourly",
        "Daily",
        "Weekly",
        "Monthly",
        "Custom (Cron Expression)",
    ];

    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Select Backup Schedule")
        .default(1)
        .items(&options)
        .interact()?;

    match selection {
        0 => Ok("0 * * * * *".to_string()), // Every minute (at 0 seconds)
        1 => Ok("0 0 * * * *".to_string()), // Hourly
        2 => {
            // Daily
            let hour: u32 = Input::with_theme(&ColorfulTheme::default())
                .with_prompt("At what hour (0-23)?")
                .default(0)
                .validate_with(|input: &u32| -> Result<(), &str> {
                    if *input <= 23 {
                        Ok(())
                    } else {
                        Err("Hour must be between 0 and 23")
                    }
                })
                .interact_text()?;
            Ok(format!("0 0 {} * * *", hour))
        }
        3 => {
            // Weekly
            let days = vec![
                "Sunday",
                "Monday",
                "Tuesday",
                "Wednesday",
                "Thursday",
                "Friday",
                "Saturday",
            ];
            let day_idx = Select::with_theme(&ColorfulTheme::default())
                .with_prompt("On which day?")
                .items(&days)
                .default(0)
                .interact()?;

            let hour: u32 = Input::with_theme(&ColorfulTheme::default())
                .with_prompt("At what hour (0-23)?")
                .default(0)
                .validate_with(|input: &u32| -> Result<(), &str> {
                    if *input <= 23 {
                        Ok(())
                    } else {
                        Err("Hour must be between 0 and 23")
                    }
                })
                .interact_text()?;

            // Cron 0-6 is Sun-Sat. day_idx matches this perfectly.
            Ok(format!("0 0 {} * * {}", hour, day_idx))
        }
        4 => {
            // Monthly
            let date: u32 = Input::with_theme(&ColorfulTheme::default())
                .with_prompt("On which date (1-31)?")
                .default(1)
                .validate_with(|input: &u32| -> Result<(), &str> {
                    if *input >= 1 && *input <= 31 {
                        Ok(())
                    } else {
                        Err("Date must be between 1 and 31")
                    }
                })
                .interact_text()?;

            let hour: u32 = Input::with_theme(&ColorfulTheme::default())
                .with_prompt("At what hour (0-23)?")
                .default(0)
                .validate_with(|input: &u32| -> Result<(), &str> {
                    if *input <= 23 {
                        Ok(())
                    } else {
                        Err("Hour must be between 0 and 23")
                    }
                })
                .interact_text()?;

            Ok(format!("0 0 {} {} * *", hour, date))
        }
        5 => {
            // Custom
            let schedule: String = Input::with_theme(&ColorfulTheme::default())
                .with_prompt("Enter Cron Expression")
                .default("0 0 * * * *".into())
                .validate_with(|input: &String| -> Result<(), &str> {
                    if Schedule::from_str(input).is_ok() {
                        Ok(())
                    } else {
                        Err("Invalid cron expression")
                    }
                })
                .interact_text()?;
            Ok(schedule)
        }
        _ => unreachable!(),
    }
}

async fn command_add() -> Result<()> {
    println!("Adding a new database configuration...");

    let db_types = vec![DbType::MariaDB, DbType::PostgreSQL];
    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Select Database Type")
        .default(0)
        .items(&db_types)
        .interact()?;
    let db_type = db_types[selection].clone();

    let name: String = Input::with_theme(&ColorfulTheme::default())
        .with_prompt("Configuration Name (e.g. production-db)")
        .interact_text()?;

    let host: String = Input::with_theme(&ColorfulTheme::default())
        .with_prompt("Host")
        .default("localhost".into())
        .interact_text()?;

    let port: u16 = Input::with_theme(&ColorfulTheme::default())
        .with_prompt("Port")
        .default(match db_type {
            DbType::MariaDB => 3306,
            DbType::PostgreSQL => 5432,
        })
        .interact_text()?;

    let user: String = Input::with_theme(&ColorfulTheme::default())
        .with_prompt("User")
        .interact_text()?;

    let password: Option<String> = Password::with_theme(&ColorfulTheme::default())
        .with_prompt("Password (optional)")
        .allow_empty_password(true)
        .interact()
        .ok()
        .filter(|p| !p.is_empty());

    let database: String = Input::with_theme(&ColorfulTheme::default())
        .with_prompt("Database Name")
        .interact_text()?;

    let output_dir_str: String = Input::with_theme(&ColorfulTheme::default())
        .with_prompt("Output Directory for Backups")
        .default("./backups".into())
        .interact_text()?;
    let output_dir = PathBuf::from(output_dir_str);

    let retention_count: usize = Input::with_theme(&ColorfulTheme::default())
        .with_prompt("Retention Count (number of backups to keep)")
        .default(5)
        .interact_text()?;

    let schedule = get_schedule_input()?;

    let mut config = load_config()?;
    let new_db_config = DatabaseConfig {
        name,
        db_type,
        connection: ConnectionDetails {
            host,
            port,
            user,
            password,
            database,
        },
        output_dir,
        retention_count,
        schedule: Some(schedule),
        enabled: true,
    };

    config.databases.push(new_db_config);
    save_config(&config)?;

    println!("Configuration saved successfully!");
    Ok(())
}

fn command_list() -> Result<()> {
    let config = load_config()?;
    if config.databases.is_empty() {
        println!("No databases configured.");
        return Ok(());
    }

    let mut table = Table::new();
    table.load_preset(comfy_table::presets::UTF8_FULL);
    table.set_header(vec![
        "ID",
        "Name",
        "Type",
        "Host",
        "Database",
        "Schedule",
        "Retention",
        "Status",
        "Last Backup",
    ]);

    for (i, db) in config.databases.iter().enumerate() {
        let last_backup = get_last_backup(db)
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
            .unwrap_or_else(|| "Never".to_string());

        let status_cell = if db.enabled {
            Cell::new("Enabled").fg(Color::Green)
        } else {
            Cell::new("Disabled").fg(Color::Red)
        };

        table.add_row(vec![
            Cell::new((i + 1).to_string()),
            Cell::new(&db.name),
            Cell::new(db.db_type.to_string()),
            Cell::new(&db.connection.host),
            Cell::new(&db.connection.database),
            Cell::new(db.schedule.clone().unwrap_or_else(|| "None".to_string())),
            Cell::new(db.retention_count.to_string()),
            status_cell,
            Cell::new(last_backup),
        ]);
    }

    println!("{table}");
    Ok(())
}

async fn command_delete(target_name: Option<String>) -> Result<()> {
    let mut config = load_config()?;
    if config.databases.is_empty() {
        println!("No databases configured.");
        return Ok(());
    }

    let selection_idx = if let Some(query) = target_name {
        find_db_index(&query, &config.databases)?
    } else {
        // Explicitly add an "Exit" option
        let mut options: Vec<String> = config
            .databases
            .iter()
            .enumerate()
            .map(|(i, db)| format!("{}. {} ({})", i + 1, db.name, db.db_type))
            .collect();
        options.push("Cancel / Exit".to_string());

        let idx = Select::with_theme(&ColorfulTheme::default())
            .with_prompt("Select configuration to delete")
            .items(&options)
            .default(0)
            .interact()?;

        // Check if the user selected default Cancel / Exit
        if idx == options.len() - 1 {
            println!("Deletion cancelled.");
            return Ok(());
        }
        idx
    };

    if Confirm::with_theme(&ColorfulTheme::default())
        .with_prompt(format!(
            "Are you sure you want to delete '{}'?",
            config.databases[selection_idx].name
        ))
        .interact()?
    {
        config.databases.remove(selection_idx);
        save_config(&config)?;
        println!("Configuration deleted.");
    } else {
        println!("Deletion cancelled.");
    }

    Ok(())
}

async fn command_edit(target_name: Option<String>) -> Result<()> {
    // Check if there are any configurations to edit
    let mut config = load_config()?;
    if config.databases.is_empty() {
        println!("No databases configured.");
        return Ok(());
    }

    let selection_idx = if let Some(query) = target_name {
        find_db_index(&query, &config.databases)?
    } else {
        // Select which configuration to edit
        let mut options: Vec<String> = config
            .databases
            .iter()
            .enumerate()
            .map(|(i, db)| format!("{}. {} ({})", i + 1, db.name, db.db_type))
            .collect();
        options.push("Cancel / Exit".to_string());

        let idx = Select::with_theme(&ColorfulTheme::default())
            .with_prompt("Select configuration to edit")
            .items(&options)
            .default(0)
            .interact()?;

        if idx == options.len() - 1 {
            println!("Exiting edit mode.");
            return Ok(());
        }
        idx
    };

    let db = &mut config.databases[selection_idx];

    // Select which field to edit
    let fields = vec![
        "Name",
        "Host",
        "Port",
        "User",
        "Password",
        "Database",
        "Output Directory",
        "Retention Count",
        "Schedule",
        "Exit Edit Mode",
    ];

    loop {
        let field_selection = Select::with_theme(&ColorfulTheme::default())
            .with_prompt(format!("Editing '{}'. Select field to change", db.name))
            .items(&fields)
            .default(fields.len() - 1) // Default to Exit
            .interact()?;

        match field_selection {
            0 => {
                // Name
                db.name = Input::with_theme(&ColorfulTheme::default())
                    .with_prompt("Name")
                    .default(db.name.clone())
                    .interact_text()?;
            }
            1 => {
                // Host
                db.connection.host = Input::with_theme(&ColorfulTheme::default())
                    .with_prompt("Host")
                    .default(db.connection.host.clone())
                    .interact_text()?;
            }
            2 => {
                // Port
                db.connection.port = Input::with_theme(&ColorfulTheme::default())
                    .with_prompt("Port")
                    .default(db.connection.port)
                    .interact_text()?;
            }
            3 => {
                // User
                db.connection.user = Input::with_theme(&ColorfulTheme::default())
                    .with_prompt("User")
                    .default(db.connection.user.clone())
                    .interact_text()?;
            }
            4 => {
                // Password
                let new_pass = Password::with_theme(&ColorfulTheme::default())
                    .with_prompt("Password (leave empty to keep unchanged, type 'clear' to remove)")
                    .allow_empty_password(true)
                    .interact()?;

                if new_pass == "clear" {
                    db.connection.password = None;
                } else if !new_pass.is_empty() {
                    db.connection.password = Some(new_pass);
                }
            }
            5 => {
                // Database
                db.connection.database = Input::with_theme(&ColorfulTheme::default())
                    .with_prompt("Database")
                    .default(db.connection.database.clone())
                    .interact_text()?;
            }
            6 => {
                // Output Dir
                let path_str = Input::with_theme(&ColorfulTheme::default())
                    .with_prompt("Output Directory")
                    .default(db.output_dir.to_string_lossy().to_string())
                    .interact_text()?;
                db.output_dir = PathBuf::from(path_str);
            }
            7 => {
                // Retention
                db.retention_count = Input::with_theme(&ColorfulTheme::default())
                    .with_prompt("Retention Count")
                    .default(db.retention_count)
                    .interact_text()?;
            }
            8 => {
                // Schedule
                println!(
                    "Current Schedule: {}",
                    db.schedule.clone().unwrap_or_else(|| "None".to_string())
                );
                let new_schedule = get_schedule_input()?;
                db.schedule = Some(new_schedule);
            }
            9 => break, // Exit
            _ => unreachable!(),
        }
    }

    save_config(&config)?;
    println!("Configuration updated successfully!");
    Ok(())
}

async fn command_run() -> Result<()> {
    let config = load_config()?;
    if config.databases.is_empty() {
        warn!("No databases configured. Run `add` command first.");
        return Ok(());
    }

    for db in config.databases {
        if let Err(e) = perform_backup(&db).await {
            error!("Failed to backup {}: {}", db.name, e);
        }
    }
    Ok(())
}

async fn command_daemon() -> Result<()> {
    info!("Starting daemon mode...");
    let mut last_run_times: std::collections::HashMap<String, chrono::DateTime<Local>> =
        std::collections::HashMap::new();

    loop {
        sleep(Duration::from_secs(10)).await;
        let now = Local::now();

        let config = match load_config() {
            Ok(c) => c,
            Err(e) => {
                error!("Config error: {}", e);
                continue;
            }
        };

        for db in config.databases {
            if !db.enabled {
                continue;
            }
            if let Some(schedule_str) = &db.schedule {
                if let Ok(schedule) = Schedule::from_str(schedule_str) {
                    let search_start = now - chrono::Duration::seconds(61);
                    if let Some(due_time) = schedule.after(&search_start).next() {
                        if due_time <= now {
                            let last_run = last_run_times.get(&db.name);
                            if let Some(last) = last_run {
                                if *last >= due_time {
                                    continue;
                                }
                            }

                            info!("Executing scheduled backup for {}", db.name);
                            if let Err(e) = perform_backup(&db).await {
                                error!("Backup failed: {}", e);
                            }

                            last_run_times.insert(db.name.clone(), due_time);
                        }
                    }
                }
            }
        }
    }
}

async fn perform_backup(db: &DatabaseConfig) -> Result<()> {
    info!("Backing up database: {}", db.name);

    if !db.output_dir.exists() {
        fs::create_dir_all(&db.output_dir)?;
    }

    let timestamp = Local::now().format("%Y%m%d_%H%M%S");
    let filename = format!("{}_{}.sql", db.name, timestamp);
    let output_path = db.output_dir.join(&filename);

    match db.db_type {
        DbType::MariaDB => {
            // Deduplication Check: Find earlier backup
            let last_backup = get_last_backup(db);

            // First attempt: Standard backup
            if let Err(e) = run_mysqldump(db, &output_path, false).await {
                warn!(
                    "Standard backup failed for {}. Retrying with --skip-lock-tables. Error: {}",
                    db.name, e
                );

                if let Err(retry_err) = run_mysqldump(db, &output_path, true).await {
                    error!("Retry with --skip-lock-tables also failed for {}", db.name);
                    fs::remove_file(&output_path).ok(); // Cleanup incomplete file
                    return Err(retry_err);
                } else {
                    info!("Backup succeeded with --skip-lock-tables for {}", db.name);
                }
            }

            // Check for deduplication
            if let Some(last_path) = last_backup {
                if let Ok(true) = files_are_identical(&output_path, &last_path) {
                    info!("Backup skipped (Identical to previous): {}", db.name);
                    fs::remove_file(&output_path).ok();
                    return Ok(());
                }
            }
        }
        DbType::PostgreSQL => {
            let mut c = Command::new("pg_dump");
            c.env("PGHOST", &db.connection.host)
                .env("PGPORT", db.connection.port.to_string())
                .env("PGUSER", &db.connection.user)
                .env("PGDATABASE", &db.connection.database);
            if let Some(pass) = &db.connection.password {
                c.env("PGPASSWORD", pass);
            }

            let output_file = fs::File::create(&output_path)?;
            c.stdout(output_file);

            let status = c.status().context("Failed to execute pg_dump")?;
            if !status.success() {
                fs::remove_file(&output_path).ok();
                anyhow::bail!("pg_dump failed with status: {}", status);
            }
        }
    }

    info!("Backup created at: {:?}", output_path);

    rotate_backups(db)?;

    Ok(())
}

async fn run_mysqldump(
    db: &DatabaseConfig,
    output_path: &std::path::Path,
    skip_lock: bool,
) -> Result<()> {
    let mut c = Command::new("mysqldump");
    c.arg(format!("-h{}", db.connection.host))
        .arg(format!("-P{}", db.connection.port))
        .arg(format!("-u{}", db.connection.user));

    if let Some(pass) = &db.connection.password {
        c.env("MYSQL_PWD", pass);
    }

    // Add robustness flags
    c.arg("--column-statistics=0");
    c.arg("--skip-dump-date");

    if skip_lock {
        c.arg("--skip-lock-tables");
        c.arg("--single-transaction");
        c.arg("--quick");
    }

    c.arg(&db.connection.database);

    let output_file = fs::File::create(output_path)?;
    c.stdout(output_file);
    c.stderr(std::process::Stdio::piped());

    let output = c.output().context("Failed to execute mysqldump")?;

    if !output.status.success() {
        let err_msg = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("mysqldump failed: {}", err_msg.trim());
    }

    Ok(())
}

fn get_last_backup(db: &DatabaseConfig) -> Option<PathBuf> {
    let mut backups: Vec<PathBuf> = fs::read_dir(&db.output_dir)
        .ok()?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                name.starts_with(&format!("{}_", db.name)) && name.ends_with(".sql")
            } else {
                false
            }
        })
        .collect();

    backups.sort();
    backups.pop()
}

fn files_are_identical(p1: &std::path::Path, p2: &std::path::Path) -> Result<bool> {
    let f1 = fs::read(p1)?;
    let f2 = fs::read(p2)?;
    Ok(f1 == f2)
}

fn rotate_backups(db: &DatabaseConfig) -> Result<()> {
    let mut backups: Vec<PathBuf> = fs::read_dir(&db.output_dir)?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && path.extension().map_or(false, |ext| ext == "sql"))
        .filter(|path| {
            path.file_name()
                .map_or(false, |name| name.to_string_lossy().starts_with(&db.name))
        })
        .collect();

    backups.sort_by_key(|path| {
        path.metadata()
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
    });

    if backups.len() > db.retention_count {
        let to_remove = backups.len() - db.retention_count;
        for path in backups.iter().take(to_remove) {
            info!("Rotating backup: Removing {:?}", path);
            fs::remove_file(path)?;
        }
    }

    Ok(())
}
