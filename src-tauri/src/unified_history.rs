use chrono::DateTime;
use serde_json::Value;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

fn home_dir() -> Result<PathBuf, String> {
    dirs::home_dir().ok_or_else(|| "Could not find home directory".to_string())
}

fn ishinex_dir() -> Result<PathBuf, String> {
    let dir = home_dir()?.join(".ishinex");
    if !dir.exists() { fs::create_dir_all(&dir).map_err(|e| e.to_string())?; }
    Ok(dir)
}

fn encode_project_id(path: &str) -> String { path.replace('/', "-") }

fn read_jsonl(path: &Path) -> Vec<Value> {
    let mut items = Vec::new();
    if let Ok(file) = fs::File::open(path) {
        let reader = BufReader::new(file);
        for line in reader.lines().flatten() {
            if let Ok(v) = serde_json::from_str::<Value>(&line) { items.push(v); }
        }
    }
    items
}

fn try_get_ts(v: &Value) -> Option<i64> {
    // Try ISO string timestamp field
    if let Some(ts) = v.get("timestamp").and_then(|x| x.as_str()) { DateTime::parse_from_rfc3339(ts).ok().map(|d| d.timestamp_millis()) }
    else { None }
}

fn gather_claude(project_path: &str) -> Vec<Value> {
    // ~/.claude/projects/<project_id>/*.jsonl
    let mut res = Vec::new();
    if let Some(home) = dirs::home_dir() {
        let project_id = encode_project_id(project_path);
        let dir = home.join(".claude").join("projects").join(project_id);
        if let Ok(entries) = fs::read_dir(dir) {
            for e in entries.flatten() {
                let p = e.path();
                if p.is_file() && p.extension().and_then(|s| s.to_str()) == Some("jsonl") {
                    res.extend(read_jsonl(&p));
                }
            }
        }
    }
    res
}

fn expand_tilde(p: &str) -> PathBuf {
    if let Some(stripped) = p.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(stripped);
        }
    }
    PathBuf::from(p)
}

fn gather_from_candidates(project_path: &str, roots: &[&str]) -> Vec<Value> {
    let mut out = Vec::new();
    let proj = project_path.to_string();
    for root in roots {
        let path = expand_tilde(root);
        if !path.exists() { continue; }
        let walker = walkdir::WalkDir::new(path).max_depth(4);
        for entry in walker.into_iter().flatten() {
            let p = entry.path();
            if p.is_file() && p.extension().and_then(|s| s.to_str()) == Some("jsonl") {
                // Quick probe for project path presence to avoid over-collecting
                let mut matched = false;
                if let Ok(file) = fs::File::open(p) {
                    let reader = BufReader::new(file);
                    for line in reader.lines().flatten().take(10) {
                        if line.contains(&proj) { matched = true; break; }
                    }
                }
                if matched {
                    out.extend(read_jsonl(p));
                }
            }
        }
    }
    out
}

#[derive(serde::Serialize)]
pub struct UnifyResult {
    pub unified_path: String,
    pub total_messages: usize,
    pub sources: Vec<SourceStat>,
}

#[derive(serde::Serialize)]
pub struct SourceStat {
    pub provider: String,
    pub count: usize,
}

#[tauri::command]
pub async fn unify_provider_histories(project_path: String) -> Result<UnifyResult, String> {
    // Gather
    let mut claude = gather_claude(&project_path);
    let codex = gather_from_candidates(&project_path, &[
        "~/.codex", "~/.openai", "~/.config/openai", "~/.config/codex", "~/Library/Application Support/OpenAI",
    ]);
    let gemini = gather_from_candidates(&project_path, &[
        "~/.gemini", "~/.config/gemini", "~/Library/Application Support/Gemini",
    ]);

    let mut all = Vec::new();
    let mut sources = Vec::new();

    if !claude.is_empty() { sources.push(SourceStat { provider: "claude".into(), count: claude.len() }); }
    if !codex.is_empty() { sources.push(SourceStat { provider: "codex".into(), count: codex.len() }); }
    if !gemini.is_empty() { sources.push(SourceStat { provider: "gemini".into(), count: gemini.len() }); }

    all.append(&mut claude);
    all.extend(codex);
    all.extend(gemini);

    // Sort by timestamp if available
    all.sort_by_key(|v| try_get_ts(v).unwrap_or(0));

    // Write to ~/.ishinex/projects/<project_id>/unified/unified.jsonl
    let base = ishinex_dir()?;
    let project_id = encode_project_id(&project_path);
    let target_dir = base.join("projects").join(project_id).join("unified");
    fs::create_dir_all(&target_dir).map_err(|e| e.to_string())?;
    let unified_path = target_dir.join("unified.jsonl");
    let mut file = fs::File::create(&unified_path).map_err(|e| e.to_string())?;
    use std::io::Write;
    for v in &all {
        let line = serde_json::to_string(v).map_err(|e| e.to_string())?;
        writeln!(file, "{}", line).map_err(|e| e.to_string())?;
    }

    Ok(UnifyResult {
        unified_path: unified_path.to_string_lossy().to_string(),
        total_messages: all.len(),
        sources,
    })
}
