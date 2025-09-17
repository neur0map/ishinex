use serde_json::json;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader as AsyncBufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use uuid::Uuid;
use std::fs;
use std::path::PathBuf;

/// Global state to track current Codex process
pub struct CodexProcessState {
    pub current_process: std::sync::Arc<Mutex<Option<Child>>>,
}

impl Default for CodexProcessState {
    fn default() -> Self {
        Self { current_process: std::sync::Arc::new(Mutex::new(None)) }
    }
}

fn create_command_with_env(program: &str) -> Command {
    // Reuse the environment logic from claude module
    let _std_cmd = crate::claude_binary::create_command_with_env(program);
    let mut cmd = Command::new(program);
    for (key, value) in std::env::vars() {
        if key == "PATH"
            || key == "HOME"
            || key == "USER"
            || key == "SHELL"
            || key == "LANG"
            || key == "LC_ALL"
            || key.starts_with("LC_")
            || key == "HOMEBREW_PREFIX"
            || key == "HOMEBREW_CELLAR"
        {
            cmd.env(&key, &value);
        }
    }
    cmd
}

async fn spawn_codex_process(
    app: AppHandle,
    mut cmd: Command,
    session_id: String,
    prompt: String,
    model: String,
    project_path: String,
) -> Result<(), String> {
    use tauri::Manager as _;

    cmd.current_dir(&project_path);
    cmd.stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .stdin(std::process::Stdio::piped());

    let mut child = cmd.spawn().map_err(|e| format!("Failed to spawn codex: {}", e))?;

    // Write prompt to stdin as a fallback (if CLI expects interactive input)
    if let Some(mut stdin) = child.stdin.take() {
        let p = prompt.clone();
        tokio::spawn(async move {
            let _ = stdin.write_all(p.as_bytes()).await;
            let _ = stdin.write_all(b"\n").await;
            let _ = stdin.shutdown().await;
        });
    }

    let pid = child.id().unwrap_or_default();

    // Register session in process registry (without child handle)
    {
        let registry = app.state::<crate::process::ProcessRegistryState>();
        let _ = registry.0.register_chat_session(
            session_id.clone(),
            "codex".to_string(),
            pid,
            project_path.clone(),
            prompt.clone(),
            model.clone(),
        );
    }

    // Track current process for cancellation
    {
        let state = app.state::<CodexProcessState>();
        let mut guard = state.current_process.lock().await;
        *guard = Some(child);
    }

    // Emit init message immediately so UI can bind to session-specific channel
    let init_msg = json!({
        "type": "system",
        "subtype": "init",
        "session_id": session_id,
        "model": model,
        "cwd": project_path,
        "provider": "codex"
    });
    let init_line = init_msg.to_string();
    let _ = app.emit("codex-output", &init_line);
    let _ = app.emit(&format!("codex-output:{}", &init_msg["session_id"].as_str().unwrap_or("")), &init_line);

    // Obtain readers
    let state_for_read = app.state::<CodexProcessState>();
    let mut child_for_read = state_for_read.current_process.lock().await;
    let child_ref = child_for_read.as_mut().ok_or_else(|| "No codex process".to_string())?;
    let stdout = child_ref.stdout.take().ok_or_else(|| "Failed to capture codex stdout".to_string())?;
    let stderr = child_ref.stderr.take().ok_or_else(|| "Failed to capture codex stderr".to_string())?;
    let app_handle_stdout = app.clone();
    let app_handle_stderr = app.clone();
    drop(child_for_read);

    // Stream stdout
    let sid_out = session_id.clone();
    let stdout_task = tokio::spawn(async move {
        let reader = AsyncBufReader::new(stdout);
        let mut lines = reader.lines();
        while let Ok(Some(line)) = lines.next_line().await {
            // Normalize: treat each line as assistant text
            let msg = json!({
                "type": "assistant",
                "message": { "content": [{"type": "text", "text": line}] }
            });
            let s = msg.to_string();
            let _ = app_handle_stdout.emit(&format!("codex-output:{}", sid_out), &s);
            let _ = app_handle_stdout.emit("codex-output", &s);
        }
    });

    // Stream stderr
    let sid_err = session_id.clone();
    let stderr_task = tokio::spawn(async move {
        let reader = AsyncBufReader::new(stderr);
        let mut lines = reader.lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let _ = app_handle_stderr.emit(&format!("codex-error:{}", sid_err), &line);
            let _ = app_handle_stderr.emit("codex-error", &line);
        }
    });

    // Wait for process end
    let app_done = app.clone();
    tokio::spawn(async move {
        let _ = stdout_task.await;
        let _ = stderr_task.await;

        // Small delay to flush messages
        tokio::time::sleep(Duration::from_millis(100)).await;
        let _ = app_done.emit(&format!("codex-complete:{}", session_id), true);
        let _ = app_done.emit("codex-complete", true);

        // Clear state
        let state = app_done.state::<CodexProcessState>();
        let mut guard = state.current_process.lock().await;
        *guard = None;
    });

    Ok(())
}

#[tauri::command]
pub async fn execute_codex_chat(
    app: AppHandle,
    project_path: String,
    prompt: String,
    model: String,
) -> Result<(), String> {
    let codex_path = crate::codex_binary::find_codex_binary(&app)?;

    // Prefer codex chat --model <model> --stream <prompt>; also pipe to stdin as fallback
    let mut cmd = create_command_with_env(&codex_path);
    cmd.arg("-m").arg(&model).arg(&prompt);

    let session_id = Uuid::new_v4().to_string();
    spawn_codex_process(app, cmd, session_id, prompt, model, project_path).await
}

#[tauri::command]
pub async fn resume_codex_chat(
    app: AppHandle,
    project_path: String,
    session_id: String,
    prompt: String,
    model: String,
) -> Result<(), String> {
    let codex_path = crate::codex_binary::find_codex_binary(&app)?;
    let mut cmd = create_command_with_env(&codex_path);
    cmd.arg("-m").arg(&model).arg(&prompt);
    spawn_codex_process(app, cmd, session_id, prompt, model, project_path).await
}

#[tauri::command]
pub async fn cancel_codex_execution(app: AppHandle) -> Result<(), String> {
    let state = app.state::<CodexProcessState>();
    let mut guard = state.current_process.lock().await;
    if let Some(child) = guard.as_mut() {
        child.start_kill().map_err(|e| e.to_string())?;
        *guard = None;
    }
    Ok(())
}

#[tauri::command]
pub async fn list_running_codex_sessions(
    registry: tauri::State<'_, crate::process::ProcessRegistryState>,
) -> Result<Vec<crate::process::ProcessInfo>, String> {
    registry.0.get_running_chat_sessions(Some("codex"))
}

#[tauri::command]
pub async fn get_codex_binary_path(app: AppHandle) -> Result<String, String> {
    crate::codex_binary::find_codex_binary(&app)
}

#[tauri::command]
pub async fn check_codex_version(app: AppHandle) -> Result<Option<String>, String> {
    let path = crate::codex_binary::find_codex_binary(&app)?;
    Ok(crate::codex_binary::get_codex_version(&path))
}

#[tauri::command]
pub async fn set_codex_binary_path(app: AppHandle, path: String) -> Result<(), String> {
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&app_data_dir).map_err(|e| e.to_string())?;
    let db_path = app_data_dir.join("agents.db");
    let conn = rusqlite::Connection::open(&db_path).map_err(|e| e.to_string())?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS app_settings (key TEXT PRIMARY KEY, value TEXT)",
        [],
    )
    .map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO app_settings(key, value) VALUES('codex_binary_path', ?1)
         ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        rusqlite::params![path],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn login_codex(app: AppHandle) -> Result<(), String> {
    let path = crate::codex_binary::find_codex_binary(&app)?;
    let mut cmd = create_command_with_env(&path);
    cmd.arg("login");
    cmd.stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let _ = cmd.spawn().map_err(|e| e.to_string())?;
    Ok(())
}

#[derive(serde::Serialize)]
pub struct LoginStatus {
    pub logged_in: bool,
    pub user: Option<String>,
    pub error: Option<String>,
}

#[tauri::command]
pub async fn check_codex_login(app: AppHandle) -> Result<LoginStatus, String> {
    let path = crate::codex_binary::find_codex_binary(&app)?;
    // Try `codex whoami` first
    let mut cmd = create_command_with_env(&path);
    cmd.arg("whoami");
    match cmd.output().await {
        Ok(out) if out.status.success() => {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            let user = if !s.is_empty() { Some(s) } else { None };
            return Ok(LoginStatus { logged_in: true, user, error: None });
        }
        _ => {}
    }

    // Fallback: try a lightweight call that would fail if not logged in
    let mut cmd2 = create_command_with_env(&path);
    cmd2.arg("models").arg("list").arg("--limit").arg("1");
    match cmd2.output().await {
        Ok(out) if out.status.success() => Ok(LoginStatus { logged_in: true, user: None, error: None }),
        Ok(out) => Ok(LoginStatus { logged_in: false, user: None, error: Some(String::from_utf8_lossy(&out.stderr).to_string()) }),
        Err(e) => Ok(LoginStatus { logged_in: false, user: None, error: Some(e.to_string()) }),
    }
}

fn read_db_value(app: &AppHandle, key: &str) -> Option<String> {
    if let Ok(app_data_dir) = app.path().app_data_dir() {
        let db_path = app_data_dir.join("agents.db");
        if db_path.exists() {
            if let Ok(conn) = rusqlite::Connection::open(&db_path) {
                if let Ok(val) = conn.query_row(
                    "SELECT value FROM app_settings WHERE key = ?1",
                    rusqlite::params![key],
                    |row| row.get::<_, String>(0),
                ) { return Some(val); }
            }
        }
    }
    None
}

fn write_db_value(app: &AppHandle, key: &str, value: &str) -> Result<(), String> {
    let app_data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    fs::create_dir_all(&app_data_dir).map_err(|e| e.to_string())?;
    let db_path = app_data_dir.join("agents.db");
    let conn = rusqlite::Connection::open(&db_path).map_err(|e| e.to_string())?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS app_settings (key TEXT PRIMARY KEY, value TEXT)",
        [],
    ).map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO app_settings(key, value) VALUES(?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        rusqlite::params![key, value],
    ).map_err(|e| e.to_string())?;
    Ok(())
}

fn search_codex_config_for_default_model() -> Option<String> {
    let candidates = vec![
        "~/.config/openai",
        "~/.config/codex",
        "~/.openai",
        "~/Library/Application Support/OpenAI",
    ];
    for root in candidates {
        let path = expand_tilde(root);
        if !path.exists() { continue; }
        let walker = walkdir::WalkDir::new(path).max_depth(2);
        for entry in walker.into_iter().flatten() {
            let p = entry.path();
            if p.is_file() {
                if let Ok(data) = fs::read_to_string(p) {
                    // naive search for default model key tokens
                    // supports toml/yaml/json by regex-like scans
                    for key in ["default_model", "model", "chat_model"] {
                        if let Some(val) = extract_model_value(&data, key) {
                            return Some(val);
                        }
                    }
                }
            }
        }
    }
    None
}

fn extract_model_value(content: &str, key: &str) -> Option<String> {
    // very permissive: key: value patterns (json/yaml/toml)
    let patterns = vec![
        format!("\"{}\"\s*[:=]\s*\"([^\"]+)\"", key),
        format!("{}\s*[:=]\s*\"([^\"]+)\"", key),
        format!("{}\s*[:=]\s*([A-Za-z0-9._-]+)", key),
    ];
    for pat in patterns {
        if let Ok(re) = regex::Regex::new(&pat) {
            if let Some(c) = re.captures(content) {
                if let Some(m) = c.get(1) { return Some(m.as_str().to_string()); }
            }
        }
    }
    None
}

fn expand_tilde(p: &str) -> PathBuf {
    if let Some(stripped) = p.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() { return home.join(stripped); }
    }
    PathBuf::from(p)
}

#[tauri::command]
pub async fn get_codex_default_model(app: AppHandle) -> Result<Option<String>, String> {
    if let Some(v) = read_db_value(&app, "codex_default_model") { return Ok(Some(v)); }
    Ok(search_codex_config_for_default_model())
}

#[tauri::command]
pub async fn set_codex_default_model(app: AppHandle, model: String) -> Result<(), String> {
    write_db_value(&app, "codex_default_model", &model)
}

#[tauri::command]
pub async fn list_codex_models(app: AppHandle) -> Result<Vec<String>, String> {
    let path = crate::codex_binary::find_codex_binary(&app)?;
    // Try JSON listing first
    let mut cmd = create_command_with_env(&path);
    cmd.arg("models").arg("list").arg("--json");
    match cmd.output().await {
        Ok(out) if out.status.success() => {
            if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&out.stdout) {
                if let Some(arr) = v.as_array() {
                    let mut list = Vec::new();
                    for item in arr { if let Some(s) = item.as_str() { list.push(s.to_string()); } }
                    if !list.is_empty() { return Ok(list); }
                }
            }
        }
        _ => {}
    }
    // Fallback: plaintext lines
    let mut cmd2 = create_command_with_env(&path);
    cmd2.arg("models").arg("list");
    match cmd2.output().await {
        Ok(out) if out.status.success() => {
            let s = String::from_utf8_lossy(&out.stdout);
            let list: Vec<String> = s.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();
            Ok(list)
        }
        Ok(out) => Err(String::from_utf8_lossy(&out.stderr).to_string()),
        Err(e) => Err(e.to_string()),
    }
}
