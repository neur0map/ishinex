use log::{info, warn};
use std::path::PathBuf;
use std::process::Command;
use tauri::Manager;

/// Find the OpenAI Codex CLI binary path.
/// Checks app DB for a stored path first, then tries `which codex`,
/// finally falls back to `codex` assuming it's in PATH.
pub fn find_codex_binary(app_handle: &tauri::AppHandle) -> Result<String, String> {
    // 1) DB stored path
    if let Ok(app_data_dir) = app_handle.path().app_data_dir() {
        let db_path = app_data_dir.join("agents.db");
        if db_path.exists() {
            if let Ok(conn) = rusqlite::Connection::open(&db_path) {
                if let Ok(stored_path) = conn.query_row(
                    "SELECT value FROM app_settings WHERE key = 'codex_binary_path'",
                    [],
                    |row| row.get::<_, String>(0),
                ) {
                    let pb = PathBuf::from(&stored_path);
                    if pb.exists() && pb.is_file() {
                        info!("Using Codex binary from DB: {}", stored_path);
                        return Ok(stored_path);
                    } else {
                        warn!("Stored codex path does not exist: {}", stored_path);
                    }
                }
            }
        }
    }

    // 2) which codex
    if let Ok(output) = Command::new("which").arg("codex").output() {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path.is_empty() {
                let pb = PathBuf::from(&path);
                if pb.exists() { return Ok(path); }
            }
        }
    }

    // 3) assume in PATH
    Ok("codex".to_string())
}

/// Try get version string using `codex --version` (best-effort)
pub fn get_codex_version(path: &str) -> Option<String> {
    if let Ok(output) = Command::new(path).arg("--version").output() {
        if output.status.success() {
            let s = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !s.is_empty() { return Some(s); }
        }
    }
    None
}

