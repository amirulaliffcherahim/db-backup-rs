# dbr (Database Backup Runner)

A robust, usable CLI tool for automated MariaDB and PostgreSQL backups.

## ✨ Features
*   **Multi-Database**: Support for MariaDB/MySQL and PostgreSQL.
*   **Smart Scheduling**: Easy presets (Hourly, Daily, Weekly, Monthly) or custom Cron expressions.
*   **Deduplication**: Skips redundant backups if data hasn't changed.
*   **Daemon Mode**: Continuously runs in background based on schedules.
*   **Robustness**: Auto-retry on lock errors (`--skip-lock-tables`).
*   **Logging**: Full history saved to `backup.log`.

## 🚀 Installation

```bash
cargo install --path .
```
Executable is named `dbr`.

## 🛠️ Usage

### Quick Commands

| Command | Description |
| :--- | :--- |
| `dbr add` | Interactive wizard to add a database. |
| `dbr list` | Show all databases, status, and last backup time. |
| `dbr edit [name]` | Edit a config. If name is omitted, shows a menu. |
| `dbr delete [name]` | Delete a config. If name is omitted, shows a menu. |
| `dbr run` | Run backups for all enabled databases immediately. |
| `dbr daemon` | Start the scheduler daemon (Ctrl+C to stop). |
| `dbr start <name>` | Enable a disabled database. |
| `dbr stop <name>` | Disable a database (prevents daemon execution). |

### Log Location
`%APPDATA%\db-shield\db-backup-rs\config\backup.log`

## 📦 Deployment (PM2)

To keep the daemon running forever:

```bash
npm install -g pm2
pm2 start dbr --name "db-backup" -- daemon
pm2 save
pm2 startup
```

## 🐳 Docker
```bash
docker build -t dbr .
docker run -d -v $(pwd)/config:/config -v $(pwd)/backups:/backups dbr daemon
```
