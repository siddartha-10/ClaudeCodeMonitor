use chrono::DateTime;
use reqwest::Client;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tauri::{AppHandle, State};
use tokio::io::{AsyncBufReadExt, BufReader as AsyncBufReader};
#[cfg(target_os = "macos")]
use tokio::process::Command;
use tokio::sync::watch;
use tokio::time::{interval, sleep, timeout};
use uuid::Uuid;



pub(crate) use crate::backend::claude_cli::WorkspaceSession;
use crate::backend::claude_cli::{
    build_claude_command_with_bin, build_claude_path_env, check_claude_installation,
    spawn_workspace_session as spawn_workspace_session_inner,
};
use crate::backend::events::{AppServerEvent, EventSink};
use crate::claude_home::{resolve_default_claude_home, resolve_workspace_claude_home};
use crate::event_sink::TauriEventSink;
use crate::remote_backend;
use crate::state::{AppState, WorkspaceWatcher};
use crate::types::WorkspaceEntry;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClaudeSessionEntry {
    #[serde(rename = "sessionId")]
    session_id: String,
    #[serde(rename = "fileMtime")]
    file_mtime: Option<i64>,
    #[serde(rename = "firstPrompt")]
    first_prompt: Option<String>,
    #[serde(rename = "messageCount")]
    message_count: Option<i64>,
    created: Option<String>,
    modified: Option<String>,
    #[serde(rename = "gitBranch")]
    git_branch: Option<String>,
    #[serde(rename = "projectPath")]
    project_path: Option<String>,
    #[serde(rename = "isSidechain")]
    #[allow(dead_code)]
    is_sidechain: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClaudeCredentials {
    claude_ai_oauth: Option<ClaudeOauth>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClaudeOauth {
    access_token: String,
}

pub(crate) async fn spawn_workspace_session(
    entry: WorkspaceEntry,
    default_claude_bin: Option<String>,
) -> Result<Arc<WorkspaceSession>, String> {
    spawn_workspace_session_inner(entry, default_claude_bin).await
}

pub(crate) async fn ensure_workspace_thread_watcher(
    workspace_id: &str,
    entry: WorkspaceEntry,
    state: &AppState,
    app: AppHandle,
) {
    let mut watchers = state.thread_watchers.lock().await;
    if let Some(existing) = watchers.get(workspace_id) {
        if existing.workspace_path == entry.path {
            return;
        }
        let _ = existing.shutdown.send(true);
    }
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    watchers.insert(
        workspace_id.to_string(),
        WorkspaceWatcher {
            shutdown: shutdown_tx,
            workspace_path: entry.path.clone(),
        },
    );
    let event_sink = TauriEventSink::new(app);
    tokio::spawn(watch_workspace_threads(
        workspace_id.to_string(),
        entry,
        event_sink,
        shutdown_rx,
    ));
}

pub(crate) async fn stop_workspace_thread_watcher(
    workspace_id: &str,
    state: &AppState,
) {
    if let Some(existing) = state.thread_watchers.lock().await.remove(workspace_id) {
        let _ = existing.shutdown.send(true);
    }
}

#[tauri::command]
pub(crate) async fn claude_doctor(
    claude_bin: Option<String>,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let default_bin = {
        let settings = state.app_settings.lock().await;
        settings.claude_bin.clone()
    };
    let resolved = claude_bin
        .clone()
        .filter(|value| !value.trim().is_empty())
        .or(default_bin);
    let path_env = build_claude_path_env(resolved.as_deref());
    let version = check_claude_installation(resolved.clone()).await?;
    Ok(json!({
        "ok": version.is_some(),
        "claudeBin": resolved,
        "version": version,
        "path": path_env,
    }))
}

#[tauri::command]
pub(crate) async fn start_thread(
    workspace_id: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Value, String> {
    if remote_backend::is_remote_mode(&*state).await {
        return remote_backend::call_remote(
            &*state,
            app,
            "start_thread",
            json!({ "workspaceId": workspace_id }),
        )
        .await;
    }

    let sessions = state.sessions.lock().await;
    let session = sessions
        .get(&workspace_id)
        .ok_or("workspace not connected")?;
    let thread_id = Uuid::new_v4().to_string();
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;
    Ok(json!({
        "thread": {
            "id": thread_id,
            "createdAt": timestamp,
            "updatedAt": timestamp,
            "cwd": session.entry.path,
        }
    }))
}

#[tauri::command]
pub(crate) async fn resume_thread(
    workspace_id: String,
    thread_id: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Value, String> {
    if remote_backend::is_remote_mode(&*state).await {
        return remote_backend::call_remote(
            &*state,
            app,
            "resume_thread",
            json!({ "workspaceId": workspace_id, "threadId": thread_id }),
        )
        .await;
    }

    let entry = {
        let sessions = state.sessions.lock().await;
        sessions
            .get(&workspace_id)
            .ok_or("workspace not connected")?
            .entry
            .clone()
    };

    let thread_id_clone = thread_id.clone();
    let thread = tokio::task::spawn_blocking(move || {
        build_thread_from_session(&entry, &thread_id_clone)
    })
    .await
    .map_err(|err| err.to_string())??;

    Ok(json!({ "thread": thread }))
}

#[tauri::command]
pub(crate) async fn fork_thread_from_message(
    workspace_id: String,
    thread_id: String,
    message_id: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Value, String> {
    if remote_backend::is_remote_mode(&*state).await {
        return remote_backend::call_remote(
            &*state,
            app,
            "fork_thread_from_message",
            json!({
                "workspaceId": workspace_id,
                "threadId": thread_id,
                "messageId": message_id
            }),
        )
        .await;
    }

    let entry = {
        let sessions = state.sessions.lock().await;
        sessions
            .get(&workspace_id)
            .ok_or("workspace not connected")?
            .entry
            .clone()
    };
    let thread_id_clone = thread_id.clone();
    let message_id_clone = message_id.clone();
    let new_thread_id = tokio::task::spawn_blocking(move || {
        fork_session_from_message(&entry, &thread_id_clone, &message_id_clone)
    })
    .await
    .map_err(|err| err.to_string())??;

    Ok(json!({ "threadId": new_thread_id }))
}

#[tauri::command]
pub(crate) async fn rewind_thread_files(
    workspace_id: String,
    thread_id: String,
    message_id: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Value, String> {
    if remote_backend::is_remote_mode(&*state).await {
        return remote_backend::call_remote(
            &*state,
            app,
            "rewind_thread_files",
            json!({
                "workspaceId": workspace_id,
                "threadId": thread_id,
                "messageId": message_id
            }),
        )
        .await;
    }

    let session = {
        let sessions = state.sessions.lock().await;
        sessions
            .get(&workspace_id)
            .ok_or("workspace not connected")?
            .clone()
    };

    let default_bin = {
        let settings = state.app_settings.lock().await;
        settings.claude_bin.clone()
    };
    let claude_bin = session
        .claude_bin
        .clone()
        .filter(|value| !value.trim().is_empty())
        .or(default_bin);

    session.kill_persistent_session(&thread_id).await?;

    let mut command = build_claude_command_with_bin(claude_bin);
    command.current_dir(&session.entry.path);
    command.arg("--resume").arg(&thread_id);
    command.arg("--rewind-files").arg(&message_id);
    command.stdout(std::process::Stdio::piped());
    command.stderr(std::process::Stdio::piped());
    command.kill_on_drop(true); // Ensure child is killed if dropped (e.g., on timeout)

    let output = timeout(Duration::from_secs(60), command.output())
        .await
        .map_err(|_| "Claude CLI timed out".to_string())?
        .map_err(|err| err.to_string())?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let detail = if stderr.is_empty() { stdout } else { stderr };
        return Err(if detail.is_empty() {
            "Claude CLI failed to rewind files".to_string()
        } else {
            detail
        });
    }

    Ok(json!({ "ok": true }))
}

#[tauri::command]
pub(crate) async fn list_threads(
    workspace_id: String,
    cursor: Option<String>,
    limit: Option<u32>,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Value, String> {
    if remote_backend::is_remote_mode(&*state).await {
        return remote_backend::call_remote(
            &*state,
            app,
            "list_threads",
            json!({ "workspaceId": workspace_id, "cursor": cursor, "limit": limit }),
        )
        .await;
    }

    let workspace_entry = {
        let sessions = state.sessions.lock().await;
        sessions
            .get(&workspace_id)
            .ok_or("workspace not connected")?
            .entry
            .clone()
    };

    let workspace_path = workspace_entry.path.clone();
    let entries = load_sessions_index(&workspace_entry);
    eprintln!(
        "[debug:sessions] list_threads: loaded {} total entries for workspace '{}'",
        entries.len(),
        workspace_id
    );
    let archived_ids = archived_threads_path(&state)
        .ok()
        .and_then(|path| read_archived_threads(&path).ok())
        .and_then(|archived| archived.get(&workspace_id).cloned())
        .unwrap_or_default();
    let archived_set = archived_ids
        .into_iter()
        .collect::<std::collections::HashSet<_>>();
    if !archived_set.is_empty() {
        eprintln!(
            "[debug:sessions] list_threads: filtering out {} archived threads",
            archived_set.len()
        );
    }
    let total_before_filter = entries.len();
    let mut sorted = entries
        .into_iter()
        .filter(|entry| !archived_set.contains(&entry.session_id))
        .collect::<Vec<_>>();
    let filtered_count = total_before_filter - sorted.len();
    if filtered_count > 0 {
        eprintln!(
            "[debug:sessions] list_threads: {} sessions removed by archive filter, {} remaining",
            filtered_count,
            sorted.len()
        );
    }
    sorted.sort_by(|a, b| session_sort_key(b).cmp(&session_sort_key(a)));

    let offset = cursor
        .as_ref()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    let limit = limit.unwrap_or(20).clamp(1, 50) as usize;
    let end = (offset + limit).min(sorted.len());
    eprintln!(
        "[debug:sessions] list_threads: returning page offset={}, limit={}, total={}, has_more={}",
        offset,
        limit,
        sorted.len(),
        end < sorted.len()
    );
    let next_cursor = if end < sorted.len() {
        Some(end.to_string())
    } else {
        None
    };

    let page_entries = sorted.into_iter().skip(offset).take(limit).collect::<Vec<_>>();
    let mut threads = Vec::new();
    for entry in page_entries {
        let session_id = entry.session_id.clone();
        let created_at = parse_iso_timestamp(entry.created.as_deref())
            .or_else(|| entry.file_mtime)
            .unwrap_or(0);
        let updated_at = parse_iso_timestamp(entry.modified.as_deref())
            .or_else(|| entry.file_mtime)
            .unwrap_or(created_at);
        let cwd = entry
            .project_path
            .clone()
            .unwrap_or_else(|| workspace_path.clone());
        threads.push(json!({
            "id": session_id.clone(),
            "preview": entry.first_prompt.unwrap_or_default(),
            "messageCount": entry.message_count.unwrap_or(0),
            "createdAt": created_at,
            "updatedAt": updated_at,
            "cwd": cwd,
            "gitBranch": entry.git_branch,
        }));
        threads.extend(list_subagent_threads(&workspace_entry, &session_id, &cwd));
    }

    Ok(json!({
        "data": threads,
        "nextCursor": next_cursor,
    }))
}

#[tauri::command]
pub(crate) async fn search_thread(
    workspace_id: String,
    query: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Value, String> {
    if remote_backend::is_remote_mode(&*state).await {
        return remote_backend::call_remote(
            &*state,
            app,
            "search_thread",
            json!({ "workspaceId": workspace_id, "query": query }),
        )
        .await;
    }

    let workspace_entry = {
        let sessions = state.sessions.lock().await;
        sessions
            .get(&workspace_id)
            .ok_or("workspace not connected")?
            .entry
            .clone()
    };

    let workspace_path = workspace_entry.path.clone();
    let entries = load_sessions_index(&workspace_entry);
    let query_lower = query.to_lowercase();

    // Filter out archived threads (same as list_threads)
    let archived_ids = archived_threads_path(&state)
        .ok()
        .and_then(|path| read_archived_threads(&path).ok())
        .and_then(|archived| archived.get(&workspace_id).cloned())
        .unwrap_or_default();
    let archived_set: std::collections::HashSet<_> = archived_ids.into_iter().collect();

    let matching: Vec<_> = entries
        .into_iter()
        .filter(|entry| !archived_set.contains(&entry.session_id))
        .filter(|entry| entry.session_id.to_lowercase().contains(&query_lower))
        .collect();

    eprintln!(
        "[debug:sessions] search_thread: query='{}' matched {} sessions (excluded {} archived)",
        query,
        matching.len(),
        archived_set.len()
    );

    let mut threads = Vec::new();
    for entry in matching {
        let session_id = entry.session_id.clone();
        let created_at = parse_iso_timestamp(entry.created.as_deref())
            .or_else(|| entry.file_mtime)
            .unwrap_or(0);
        let updated_at = parse_iso_timestamp(entry.modified.as_deref())
            .or_else(|| entry.file_mtime)
            .unwrap_or(created_at);
        let cwd = entry
            .project_path
            .clone()
            .unwrap_or_else(|| workspace_path.clone());
        threads.push(json!({
            "id": session_id,
            "preview": entry.first_prompt.unwrap_or_default(),
            "messageCount": entry.message_count.unwrap_or(0),
            "createdAt": created_at,
            "updatedAt": updated_at,
            "cwd": cwd,
            "gitBranch": entry.git_branch,
        }));
    }

    Ok(json!({
        "data": threads,
    }))
}

#[tauri::command]
pub(crate) async fn archive_thread(
    workspace_id: String,
    thread_id: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Value, String> {
    if remote_backend::is_remote_mode(&*state).await {
        return remote_backend::call_remote(
            &*state,
            app,
            "archive_thread",
            json!({ "workspaceId": workspace_id, "threadId": thread_id }),
        )
        .await;
    }

    let path = archived_threads_path(&state)?;
    let mut archived = read_archived_threads(&path)?;
    let entry = archived.entry(workspace_id).or_default();
    if !entry.contains(&thread_id) {
        entry.push(thread_id);
        write_archived_threads(&path, &archived)?;
    }
    Ok(json!({ "ok": true }))
}

#[tauri::command]
pub(crate) async fn send_user_message(
    workspace_id: String,
    thread_id: String,
    text: String,
    model: Option<String>,
    effort: Option<String>,
    access_mode: Option<String>,
    images: Option<Vec<String>>,
    _collaboration_mode: Option<Value>,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Value, String> {
    if remote_backend::is_remote_mode(&*state).await {
        return remote_backend::call_remote(
            &*state,
            app,
            "send_user_message",
            json!({
                "workspaceId": workspace_id,
                "threadId": thread_id,
                "text": text,
                "model": model,
                "effort": effort,
                "accessMode": access_mode,
                "images": images,
            }),
        )
        .await;
    }

    let sessions = state.sessions.lock().await;
    let session = sessions
        .get(&workspace_id)
        .ok_or("workspace not connected")?
        .clone();
    drop(sessions);

    ensure_workspace_thread_watcher(&workspace_id, session.entry.clone(), &state, app.clone()).await;

    let prompt = build_prompt_with_images(text, images);
    if prompt.trim().is_empty() {
        return Err("empty user message".to_string());
    }

    let event_sink = TauriEventSink::new(app.clone());

    // Ensure persistent session exists and get turn_id
    let turn_id = ensure_persistent_session(
        &workspace_id,
        &session,
        &thread_id,
        model.as_deref(),
        access_mode.as_deref(),
        None, // max_thinking_tokens - use default
        event_sink,
    ).await?;

    // Set the pending turn ID so the reader knows which turn_id to use
    session.set_pending_turn_id(&thread_id, turn_id.clone()).await;

    // Send the user message via stdin
    session.send_message(&thread_id, &prompt).await?;

    Ok(json!({
        "result": {
            "turn": { "id": turn_id, "threadId": thread_id }
        }
    }))
}

#[tauri::command]
pub(crate) async fn collaboration_mode_list(
    workspace_id: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Value, String> {
    if remote_backend::is_remote_mode(&*state).await {
        return remote_backend::call_remote(
            &*state,
            app,
            "collaboration_mode_list",
            json!({ "workspaceId": workspace_id }),
        )
        .await;
    }
    Ok(json!({ "data": [] }))
}

#[tauri::command]
pub(crate) async fn turn_interrupt(
    workspace_id: String,
    thread_id: String,
    turn_id: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Value, String> {
    if remote_backend::is_remote_mode(&*state).await {
        return remote_backend::call_remote(
            &*state,
            app,
            "turn_interrupt",
            json!({ "workspaceId": workspace_id, "threadId": thread_id, "turnId": turn_id }),
        )
        .await;
    }

    let sessions = state.sessions.lock().await;
    let session = sessions
        .get(&workspace_id)
        .ok_or("workspace not connected")?;
    session.interrupt_turn(&thread_id, &turn_id).await?;
    Ok(json!({ "ok": true }))
}

#[tauri::command]
pub(crate) async fn start_review(
    workspace_id: String,
    thread_id: String,
    target: Value,
    delivery: Option<String>,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Value, String> {
    if remote_backend::is_remote_mode(&*state).await {
        return remote_backend::call_remote(
            &*state,
            app,
            "start_review",
            json!({
                "workspaceId": workspace_id,
                "threadId": thread_id,
                "target": target,
                "delivery": delivery,
            }),
        )
        .await;
    }

    let sessions = state.sessions.lock().await;
    let session = sessions
        .get(&workspace_id)
        .ok_or("workspace not connected")?
        .clone();
    drop(sessions);

    let prompt = build_review_prompt(&workspace_id, &target, &state).await?;
    let event_sink = TauriEventSink::new(app.clone());

    // Ensure persistent session exists and get turn_id
    let turn_id = ensure_persistent_session(
        &workspace_id,
        &session,
        &thread_id,
        None,
        None, // access_mode - use default
        None, // max_thinking_tokens - use default
        event_sink,
    ).await?;

    // Set the pending turn ID so the reader knows which turn_id to use
    session.set_pending_turn_id(&thread_id, turn_id.clone()).await;

    // Send the review prompt via stdin
    session.send_message(&thread_id, &prompt).await?;

    Ok(json!({
        "result": {
            "turn": { "id": turn_id, "threadId": thread_id }
        }
    }))
}

#[tauri::command]
pub(crate) async fn model_list(
    workspace_id: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Value, String> {
    if remote_backend::is_remote_mode(&*state).await {
        return remote_backend::call_remote(
            &*state,
            app,
            "model_list",
            json!({ "workspaceId": workspace_id }),
        )
        .await;
    }

    let data = vec![
        json!({
            "id": "claude-opus-4-5-20251101",
            "model": "claude-opus-4-5-20251101",
            "displayName": "Claude Opus 4.5",
            "description": "Highest quality reasoning model.",
            "supportedReasoningEfforts": [],
            "defaultReasoningEffort": "",
            "isDefault": true,
        }),
        json!({
            "id": "claude-sonnet-4-5-20250929",
            "model": "claude-sonnet-4-5-20250929",
            "displayName": "Claude Sonnet 4.5",
            "description": "Fast, balanced model.",
            "supportedReasoningEfforts": [],
            "defaultReasoningEffort": "",
            "isDefault": false,
        }),
    ];

    Ok(json!({ "data": data }))
}

#[tauri::command]
pub(crate) async fn global_rate_limits() -> Result<Value, String> {
    let token = match read_oauth_token().await {
        Some(t) => t,
        None => return Ok(json!({ "rateLimits": null })),
    };
    let usage: Value = Client::new()
        .get("https://api.anthropic.com/api/oauth/usage")
        .header("Authorization", format!("Bearer {token}"))
        .header("anthropic-beta", "oauth-2025-04-20")
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    let window = |key: &str| -> Option<Value> {
        let w = usage.get(key)?;
        let pct = w.get("utilization")?.as_f64()?;
        let resets = w.get("resets_at").and_then(|v| v.as_str()).and_then(|s| {
            DateTime::parse_from_rfc3339(s).ok().map(|t| t.timestamp_millis())
        });
        Some(json!({ "usedPercent": pct, "resetsAt": resets }))
    };
    Ok(json!({
        "rateLimits": {
            "primary": window("five_hour"),
            "secondary": window("seven_day"),
            "sonnet": window("seven_day_sonnet"),
        }
    }))
}

#[cfg(target_os = "macos")]
async fn read_oauth_token() -> Option<String> {
    // Don't filter by account - $USER may be empty in Tauri context
    let output = Command::new("security")
        .args(["find-generic-password", "-s", "Claude Code-credentials", "-w"])
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return read_oauth_token_from_file().await;
    }
    let raw = String::from_utf8_lossy(&output.stdout);
    if let Ok(creds) = serde_json::from_str::<ClaudeCredentials>(raw.trim()) {
        if let Some(oauth) = creds.claude_ai_oauth {
            return Some(oauth.access_token);
        }
    }
    read_oauth_token_from_file().await
}

#[cfg(not(target_os = "macos"))]
async fn read_oauth_token() -> Option<String> {
    read_oauth_token_from_file().await
}

async fn read_oauth_token_from_file() -> Option<String> {
    let path = resolve_default_claude_home()?.join(".credentials.json");
    let raw = fs::read_to_string(&path).ok()?;
    let creds: ClaudeCredentials = serde_json::from_str(&raw).ok()?;
    creds.claude_ai_oauth.map(|oauth| oauth.access_token)
}

#[tauri::command]
pub(crate) async fn skills_list(
    workspace_id: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Value, String> {
    if remote_backend::is_remote_mode(&*state).await {
        return remote_backend::call_remote(
            &*state,
            app,
            "skills_list",
            json!({ "workspaceId": workspace_id }),
        )
        .await;
    }

    Ok(json!({ "data": [] }))
}

#[tauri::command]
pub(crate) async fn respond_to_server_request(
    workspace_id: String,
    thread_id: String,
    tool_use_id: String,
    result: Value,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<(), String> {
    if remote_backend::is_remote_mode(&*state).await {
        remote_backend::call_remote(
            &*state,
            app,
            "respond_to_server_request",
            json!({ "workspaceId": workspace_id, "threadId": thread_id, "toolUseId": tool_use_id, "result": result }),
        )
        .await?;
        return Ok(());
    }

    let sessions = state.sessions.lock().await;
    let session = sessions
        .get(&workspace_id)
        .ok_or("workspace not connected")?;

    session.send_response(&thread_id, tool_use_id, result).await
}

/// Gets the diff content for commit message generation
#[tauri::command]
pub(crate) async fn get_commit_message_prompt(
    workspace_id: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let diff = crate::git::get_workspace_diff(&workspace_id, &state).await?;

    if diff.trim().is_empty() {
        return Err("No changes to generate commit message for".to_string());
    }

    let prompt = format!(
        "Generate a concise git commit message for the following changes. \
Follow conventional commit format (e.g., feat:, fix:, refactor:, docs:, etc.). \
Focus on the 'why' rather than the 'what'. Keep the summary line under 72 characters. \
Only output the commit message, nothing else.\n\n\
Changes:\n{diff}"
    );

    Ok(prompt)
}

#[tauri::command]
pub(crate) async fn remember_approval_rule(
    workspace_id: String,
    rule: String,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let rule = rule.trim();
    if rule.is_empty() {
        return Err("empty rule".to_string());
    }

    let (entry, parent_path) = {
        let workspaces = state.workspaces.lock().await;
        let entry = workspaces
            .get(&workspace_id)
            .ok_or("workspace not found")?
            .clone();
        let parent_path = entry
            .parent_id
            .as_ref()
            .and_then(|parent_id| workspaces.get(parent_id))
            .map(|parent| parent.path.clone());
        (entry, parent_path)
    };

    let settings_path = resolve_permissions_path(&entry, parent_path.as_deref())?;
    let mut settings = read_settings_json(&settings_path)?;
    let permissions = settings
        .entry("permissions")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or("Unable to update permissions".to_string())?;
    let allow = permissions
        .entry("allow")
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .ok_or("Unable to update permissions".to_string())?;
    if !allow.iter().any(|item| item.as_str() == Some(rule)) {
        allow.push(Value::String(rule.to_string()));
    }
    write_settings_json(&settings_path, &settings)?;

    Ok(json!({
        "ok": true,
        "rulesPath": settings_path,
    }))
}

/// Generates a commit message in the background without showing in the main chat
#[tauri::command]
pub(crate) async fn generate_commit_message(
    workspace_id: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let diff = crate::git::get_workspace_diff(&workspace_id, &state).await?;

    if diff.trim().is_empty() {
        return Err("No changes to generate commit message for".to_string());
    }

    let prompt = format!(
        "Generate a concise git commit message for the following changes. \
Follow conventional commit format (e.g., feat:, fix:, refactor:, docs:, etc.). \
Focus on the 'why' rather than the 'what'. Keep the summary line under 72 characters. \
Only output the commit message, nothing else.\n\n\
Changes:\n{diff}"
    );

    let entry = {
        let sessions = state.sessions.lock().await;
        sessions
            .get(&workspace_id)
            .ok_or("workspace not connected")?
            .entry
            .clone()
    };

    let default_bin = {
        let settings = state.app_settings.lock().await;
        settings.claude_bin.clone()
    };

    let response = run_claude_prompt_once(
        &entry.path,
        default_bin,
        prompt,
        Some("dontAsk".to_string()),
        Some("haiku".to_string()),
    )
    .await?;

    Ok(response)
}

#[tauri::command]
pub async fn generate_run_metadata(
    workspace_id: String,
    prompt: String,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let entry = {
        let sessions = state.sessions.lock().await;
        sessions
            .get(&workspace_id)
            .ok_or("workspace not connected")?
            .entry
            .clone()
    };

    let default_bin = {
        let settings = state.app_settings.lock().await;
        settings.claude_bin.clone()
    };

    let system_prompt = format!(
        "Generate metadata for a coding task based on the user's prompt. \
Return ONLY valid JSON with no additional text, in this exact format:\n\
{{\"title\": \"Title Case 3-7 Words\", \"worktreeName\": \"prefix/kebab-case-name\"}}\n\n\
Rules for title:\n\
- 3-7 words in Title Case\n\
- Describe the task concisely\n\n\
Rules for worktreeName:\n\
- Use one of these prefixes: feat/, fix/, chore/, test/, docs/, refactor/, perf/, build/, ci/, style/\n\
- Use kebab-case after the prefix\n\
- Keep it short and descriptive\n\n\
User's task description:\n{prompt}"
    );

    let response = run_claude_prompt_once(
        &entry.path,
        default_bin,
        system_prompt,
        Some("dontAsk".to_string()),
        Some("haiku".to_string()),
    )
    .await?;

    // Try to parse the response as JSON and return it
    let trimmed = response.trim();
    // Handle case where response might have markdown code blocks
    let json_str = if trimmed.starts_with("```") {
        trimmed
            .trim_start_matches("```json")
            .trim_start_matches("```")
            .trim_end_matches("```")
            .trim()
    } else {
        trimmed
    };

    match serde_json::from_str::<Value>(json_str) {
        Ok(parsed) => Ok(parsed),
        Err(_) => {
            // Return a default structure if parsing fails
            Ok(json!({
                "title": null,
                "worktreeName": null
            }))
        }
    }
}

fn build_prompt_with_images(text: String, images: Option<Vec<String>>) -> String {
    let mut prompt = text.trim().to_string();
    if let Some(images) = images {
        let mut image_lines = Vec::new();
        for image in images {
            let trimmed = image.trim();
            if trimmed.is_empty() {
                continue;
            }
            image_lines.push(format!("[image] {trimmed}"));
        }
        if !image_lines.is_empty() {
            if !prompt.is_empty() {
                prompt.push_str("\n\n");
            }
            prompt.push_str("Attached images:\n");
            prompt.push_str(&image_lines.join("\n"));
        }
    }
    prompt
}

async fn run_claude_prompt_once(
    cwd: &str,
    claude_bin: Option<String>,
    prompt: String,
    permission_mode: Option<String>,
    model: Option<String>,
) -> Result<String, String> {
    let mut command = build_claude_command_with_bin(claude_bin);
    command.current_dir(cwd);
    command.arg("-p").arg(prompt);
    command.arg("--output-format").arg("stream-json");
    command.arg("--verbose");
    command.arg("--no-session-persistence");
    if let Some(mode) = permission_mode {
        command.arg("--permission-mode").arg(mode);
    }
    if let Some(m) = model {
        command.arg("--model").arg(m);
    }
    command.stdout(std::process::Stdio::piped());
    command.stderr(std::process::Stdio::piped());

    let output = timeout(Duration::from_secs(60), command.output())
        .await
        .map_err(|_| "Claude CLI timed out".to_string())?
        .map_err(|err| err.to_string())?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            "Claude CLI failed to run".to_string()
        } else {
            stderr
        });
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut message = String::new();
    for line in stdout.lines() {
        if let Ok(value) = serde_json::from_str::<Value>(line) {
            if value.get("type").and_then(|v| v.as_str()) == Some("assistant") {
                if let Some(text) = value
                    .get("message")
                    .and_then(|m| m.get("content"))
                    .and_then(|content| content.as_array())
                    .map(|content| extract_text_from_content(content))
                {
                    if !text.is_empty() {
                        message = text;
                    }
                }
            }
        }
    }
    Ok(message.trim().to_string())
}

/// Container for the stdout and stderr readers from a spawned persistent Claude CLI session.
pub(crate) struct PersistentSessionReaders {
    pub stdout: AsyncBufReader<tokio::process::ChildStdout>,
    pub stderr: AsyncBufReader<tokio::process::ChildStderr>,
}

/// Spawns a persistent Claude CLI session with bidirectional streaming.
///
/// This function spawns Claude CLI with streaming JSON input/output format,
/// allowing for continuous interaction without spawning new processes for each turn.
///
/// # Arguments
/// * `session` - The workspace session containing entry and claude_bin information
/// * `thread_id` - The thread/session ID for session persistence
/// * `model` - Optional model to use
/// * `access_mode` - Optional permission mode (e.g., "dontAsk", "askEdits", etc.)
/// * `max_thinking_tokens` - Optional max thinking tokens for extended thinking
///
/// # Returns
/// Readers for both stdout and stderr (the child process is stored in the session for cleanup)
pub(crate) async fn spawn_persistent_claude_session(
    session: &Arc<WorkspaceSession>,
    thread_id: &str,
    model: Option<&str>,
    access_mode: Option<&str>,
    max_thinking_tokens: Option<u32>,
) -> Result<PersistentSessionReaders, String> {
    let mut command = build_claude_command_with_bin(session.claude_bin.clone());
    command.current_dir(&session.entry.path);

    // Set up streaming JSON input/output format
    command.arg("--print");
    command.arg("--input-format").arg("stream-json");
    command.arg("--output-format").arg("stream-json");
    command.arg("--include-partial-messages");
    command.arg("--verbose");

    // Set model if specified
    if let Some(model) = model {
        if !model.trim().is_empty() {
            command.arg("--model").arg(model);
        }
    }

    // Set permission mode if specified
    // Map UI access modes to valid Claude CLI permission modes:
    // - "read-only" → "plan" (requires plan approval, safest)
    // - "current" → skip (use CLI default)
    // - "full-access" → "bypassPermissions" (bypass all permission checks)
    // Also accept direct CLI modes: acceptEdits, bypassPermissions, default, delegate, dontAsk, plan
    if let Some(mode) = access_mode {
        let mode_trimmed = mode.trim();
        let mapped_mode = match mode_trimmed {
            "read-only" => Some("plan"),
            "full-access" => Some("bypassPermissions"),
            "current" => None, // Use CLI default
            // Direct CLI modes pass through
            "acceptEdits" | "bypassPermissions" | "default" | "delegate" | "dontAsk" | "plan" => Some(mode_trimmed),
            _ => None, // Unknown modes are ignored
        };
        if let Some(cli_mode) = mapped_mode {
            command.arg("--permission-mode").arg(cli_mode);
        }
    }

    // Set max thinking tokens (default to 31999, Claude's default)
    let thinking_tokens = max_thinking_tokens.unwrap_or(31999);
    command.arg("--max-thinking-tokens").arg(thinking_tokens.to_string());

    // Use --resume if session exists, otherwise --session-id
    if session_exists(&session.entry, thread_id) {
        command.arg("--resume").arg(thread_id);
    } else {
        command.arg("--session-id").arg(thread_id);
    }

    // Configure stdio for bidirectional communication
    command.stdin(std::process::Stdio::piped());
    command.stdout(std::process::Stdio::piped());
    command.stderr(std::process::Stdio::piped());

    // Spawn the process
    let mut child = command.spawn().map_err(|err| {
        format!("Failed to spawn Claude CLI: {}", err)
    })?;

    // Take stdin for bidirectional communication
    let stdin = child.stdin.take().ok_or("Failed to capture stdin")?;

    // Take stdout for reading responses
    let stdout = child.stdout.take().ok_or("Failed to capture stdout")?;
    let stdout_reader = AsyncBufReader::new(stdout);

    // Take stderr for reading error messages
    let stderr = child.stderr.take().ok_or("Failed to capture stderr")?;
    let stderr_reader = AsyncBufReader::new(stderr);

    // Store the persistent session for this thread (stdin + child + permission_mode + model)
    // Convert access_mode to the CLI permission mode for storage
    let stored_permission_mode = access_mode.map(|mode| {
        match mode {
            "read-only" => "plan".to_string(),
            "full-access" => "bypassPermissions".to_string(),
            "current" => "default".to_string(),
            other => other.to_string(),
        }
    });
    // Store the model for detecting changes
    let stored_model = model.map(|m| m.to_string());
    session.set_persistent_session(thread_id.to_string(), stdin, child, stored_permission_mode, stored_model).await;

    Ok(PersistentSessionReaders {
        stdout: stdout_reader,
        stderr: stderr_reader,
    })
}

/// Ensures a persistent session exists for the given workspace and thread.
/// If no session exists for this thread, spawns one and starts the background stdout reader.
///
/// Each thread gets its own persistent session, allowing multiple threads to run in parallel.
/// Uses locking to prevent race conditions where multiple concurrent callers could
/// spawn duplicate sessions for the same thread.
///
/// Returns the turn_id for the current turn.
async fn ensure_persistent_session(
    workspace_id: &str,
    session: &Arc<WorkspaceSession>,
    thread_id: &str,
    model: Option<&str>,
    access_mode: Option<&str>,
    max_thinking_tokens: Option<u32>,
    event_sink: TauriEventSink,
) -> Result<String, String> {
    // Acquire the session initialization lock to prevent race conditions
    let _init_guard = session.session_init_lock.lock().await;

    // Convert requested access_mode to CLI permission mode for comparison
    let requested_permission_mode = access_mode.map(|mode| {
        match mode {
            "read-only" => "plan".to_string(),
            "full-access" => "bypassPermissions".to_string(),
            "current" => "default".to_string(),
            other => other.to_string(),
        }
    });

    // Convert requested model for comparison (normalize empty strings to None)
    let requested_model = model
        .filter(|m| !m.trim().is_empty())
        .map(|m| m.to_string());

    // Check if a persistent session already exists for THIS thread
    if session.has_persistent_session(thread_id).await {
        // Check if permission mode changed - if so, we need to restart the session
        let current_permission_mode = session.get_persistent_session_permission_mode(thread_id).await;
        let current_model = session.get_persistent_session_model(thread_id).await;

        // Only restart if the requested mode is different from the current mode
        // (treating None as equivalent to "default" for comparison)
        let current_mode = current_permission_mode.as_deref().unwrap_or("default");
        let requested_mode = requested_permission_mode.as_deref().unwrap_or("default");

        let permission_mode_changed = current_mode != requested_mode;
        let model_changed = current_model != requested_model;

        if permission_mode_changed {
            // Permission mode changed - kill the old session and spawn a new one
            // This follows Claude CLI behavior: permission mode is per-process,
            // so changing it requires starting a new process with --resume
            eprintln!(
                "[ensure_persistent_session] Permission mode changed from '{}' to '{}' for thread {}, restarting session",
                current_mode, requested_mode, thread_id
            );
            session.kill_persistent_session(thread_id).await?;
        } else if model_changed {
            // Model changed - kill the old session and spawn a new one
            // This follows Claude CLI behavior: model is per-process,
            // so changing it requires starting a new process with --resume --model
            eprintln!(
                "[ensure_persistent_session] Model changed from '{:?}' to '{:?}' for thread {}, restarting session",
                current_model, requested_model, thread_id
            );
            session.kill_persistent_session(thread_id).await?;
        } else {
            // Session exists with same permission mode and model, just return a new turn_id
            return Ok(Uuid::new_v4().to_string());
        }
    }

    let turn_id = Uuid::new_v4().to_string();

    // Spawn a new persistent session for this thread
    let readers = spawn_persistent_claude_session(session, thread_id, model, access_mode, max_thinking_tokens).await?;

    // Spawn background task to read stdout and emit events
    let workspace_id_owned = workspace_id.to_string();
    let thread_id_owned = thread_id.to_string();
    let turn_id_clone = turn_id.clone();
    let event_sink_clone = event_sink.clone();
    let session_clone = Arc::clone(session);
    tokio::spawn(async move {
        read_persistent_stdout(
            readers.stdout,
            workspace_id_owned,
            thread_id_owned,
            turn_id_clone,
            session_clone,
            event_sink_clone,
        ).await;
    });

    // Spawn background task to read stderr and emit error events
    let workspace_id_for_stderr = workspace_id.to_string();
    let thread_id_for_stderr = thread_id.to_string();
    let session_for_stderr = Arc::clone(session);
    tokio::spawn(async move {
        read_persistent_stderr(
            readers.stderr,
            workspace_id_for_stderr,
            thread_id_for_stderr,
            session_for_stderr,
            event_sink,
        ).await;
    });

    Ok(turn_id)
}

/// Background task that reads stdout from the persistent Claude CLI session
/// and emits events to the frontend.
async fn read_persistent_stdout(
    mut reader: AsyncBufReader<tokio::process::ChildStdout>,
    workspace_id: String,
    thread_id: String,
    initial_turn_id: String,
    session: Arc<WorkspaceSession>,
    event_sink: TauriEventSink,
) {
    let mut current_turn_id = initial_turn_id;
    let mut item_id = format!("{current_turn_id}-assistant");
    let mut full_text = String::new();
    let mut last_text = String::new();
    let mut last_usage: Option<Value> = None;
    let mut last_model_usage: Option<Value> = None;
    let mut last_model: Option<String> = None;
    let mut tool_names: HashMap<String, String> = HashMap::new();
    let mut tool_inputs: HashMap<String, Value> = HashMap::new();
    let mut tool_counter: usize = 0;
    let mut thinking_counter: usize = 0;
    let mut request_id_counter: u64 = 0;
    let mut permission_denial_ids: HashSet<String> = HashSet::new();
    let mut turn_active = false;

    let mut line = String::new();

    loop {
        line.clear();
        match reader.read_line(&mut line).await {
            Ok(0) => {
                // EOF - process ended
                if turn_active {
                    emit_event(
                        &event_sink,
                        &workspace_id,
                        "turn/completed",
                        json!({
                            "threadId": thread_id,
                            "turn": { "id": current_turn_id, "threadId": thread_id },
                        }),
                    );
                }
                break;
            }
            Ok(_) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }

                let value: Value = match serde_json::from_str(trimmed) {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                // Skip subagent events - they have parent_tool_use_id set
                if value.get("parent_tool_use_id").and_then(|v| v.as_str()).is_some() {
                    continue;
                }

                let event_type = value.get("type").and_then(|v| v.as_str()).unwrap_or("");
                let subtype = value.get("subtype").and_then(|v| v.as_str()).unwrap_or("");

                // Handle system init event
                if event_type == "system" {
                    if subtype == "init" {
                        // Extract session info from init event
                        let session_id = value
                            .get("session_id")
                            .and_then(|v| v.as_str())
                            .unwrap_or(&thread_id);
                        let model = value
                            .get("model")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                        let tools = value
                            .get("tools")
                            .and_then(|v| v.as_array())
                            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect::<Vec<_>>());

                        // Emit session initialized event
                        emit_event(
                            &event_sink,
                            &workspace_id,
                            "session/initialized",
                            json!({
                                "threadId": thread_id,
                                "sessionId": session_id,
                                "model": model,
                                "tools": tools,
                            }),
                        );
                        continue;
                    }
                }

                // Start a new turn if we receive an assistant message and no turn is active
                if event_type == "assistant" && !turn_active {
                    turn_active = true;
                    // Use pending_turn_id from session if available, otherwise generate new one
                    // This ensures turn_id returned by send_user_message matches emitted events
                    current_turn_id = session
                        .take_pending_turn_id(&thread_id)
                        .await
                        .unwrap_or_else(|| Uuid::new_v4().to_string());
                    item_id = format!("{current_turn_id}-assistant");
                    full_text.clear();
                    last_text.clear();
                    last_usage = None;
                    last_model_usage = None;
                    last_model = None;
                    tool_names.clear();
                    tool_inputs.clear();
                    tool_counter = 0;
                    thinking_counter = 0;
                    permission_denial_ids.clear();

                    emit_event(
                        &event_sink,
                        &workspace_id,
                        "turn/started",
                        json!({
                            "threadId": thread_id,
                            "turn": { "id": current_turn_id, "threadId": thread_id },
                        }),
                    );
                    emit_event(
                        &event_sink,
                        &workspace_id,
                        "item/started",
                        json!({
                            "threadId": thread_id,
                            "item": { "id": item_id, "type": "agentMessage", "text": "" },
                        }),
                    );
                }

                if event_type == "assistant" {
                    if let Some(uuid) = value.get("uuid").and_then(|v| v.as_str()) {
                        if !uuid.is_empty() {
                            item_id = uuid.to_string();
                        }
                    }
                    if let Some(message) = value.get("message") {
                        if let Some(model) = message.get("model").and_then(|v| v.as_str()) {
                            if !model.trim().is_empty() {
                                last_model = Some(model.to_string());
                            }
                        }
                        if let Some(content) = message.get("content").and_then(|v| v.as_array()) {
                            for entry in content {
                                let entry_type = entry.get("type").and_then(|v| v.as_str()).unwrap_or("");
                                if entry_type == "thinking" {
                                    if let Some(thinking) = entry.get("thinking").and_then(|v| v.as_str()) {
                                        let trimmed = thinking.trim();
                                        if !trimmed.is_empty() {
                                            thinking_counter += 1;
                                            let thinking_id = format!("{item_id}-thinking-{thinking_counter}");
                                            emit_event(
                                                &event_sink,
                                                &workspace_id,
                                                "item/started",
                                                json!({
                                                    "threadId": thread_id,
                                                    "item": {
                                                        "id": thinking_id,
                                                        "type": "reasoning",
                                                        "summary": "",
                                                        "content": trimmed,
                                                    }
                                                }),
                                            );
                                        }
                                    }
                                    continue;
                                }
                                if entry_type != "tool_use" {
                                    continue;
                                }
                                let tool_id = entry
                                    .get("id")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("");
                                let tool_name = entry
                                    .get("name")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("Tool")
                                    .to_string();
                                let tool_input = entry.get("input").cloned().unwrap_or(Value::Null);
                                if !tool_id.is_empty() {
                                    tool_names.insert(tool_id.to_string(), tool_name.clone());
                                    tool_inputs.insert(tool_id.to_string(), tool_input.clone());
                                }
                                let item_id_tool = if tool_id.is_empty() {
                                    tool_counter += 1;
                                    format!("{current_turn_id}-tool-{tool_counter}")
                                } else {
                                    tool_id.to_string()
                                };
                                // Check for AskUserQuestion - emit request_user_input event
                                if tool_name == "AskUserQuestion" {
                                    request_id_counter += 1;
                                    // Extract questions array from tool_input
                                    // Claude's AskUserQuestion tool has structure:
                                    // { "questions": [{ "question": "...", "header": "...", "options": [{ "label": "...", "description": "..." }] }] }
                                    let questions_raw = tool_input.get("questions").and_then(|v| v.as_array());
                                    let questions = if let Some(q_array) = questions_raw {
                                        // Map each question, adding the tool_id as the id for each
                                        q_array.iter().enumerate().map(|(idx, q)| {
                                            let id = if idx == 0 {
                                                tool_id.to_string()
                                            } else {
                                                format!("{}-{}", tool_id, idx)
                                            };
                                            json!({
                                                "id": id,
                                                "header": q.get("header").and_then(|v| v.as_str()).unwrap_or("Claude needs your input"),
                                                "question": q.get("question").and_then(|v| v.as_str()).unwrap_or(""),
                                                "options": q.get("options").cloned().unwrap_or(Value::Null),
                                            })
                                        }).collect::<Vec<_>>()
                                    } else {
                                        // Fallback for legacy single question format
                                        let question_text = tool_input
                                            .get("question")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("")
                                            .to_string();
                                        vec![json!({
                                            "id": tool_id,
                                            "header": "Claude needs your input",
                                            "question": question_text,
                                        })]
                                    };
                                    emit_event_with_id(
                                        &event_sink,
                                        &workspace_id,
                                        "item/tool/requestUserInput",
                                        request_id_counter,
                                        json!({
                                            "threadId": thread_id,
                                            "turnId": current_turn_id,
                                            "itemId": item_id_tool,
                                            "toolUseId": tool_id,
                                            "questions": questions,
                                        }),
                                    );
                                }

                                emit_event(
                                    &event_sink,
                                    &workspace_id,
                                    "item/started",
                                    json!({
                                        "threadId": thread_id,
                                        "item": build_tool_item(
                                            &item_id_tool,
                                            &tool_name,
                                            &tool_input,
                                            "running",
                                            None,
                                            None,
                                        ),
                                    }),
                                );
                            }
                        }
                        let text = extract_text_from_message(message);
                        if !text.is_empty() {
                            full_text = text.clone();
                            let delta = if full_text.starts_with(&last_text) {
                                full_text[last_text.len()..].to_string()
                            } else {
                                full_text.clone()
                            };
                            if !delta.is_empty() {
                                emit_event(
                                    &event_sink,
                                    &workspace_id,
                                    "item/agentMessage/delta",
                                    json!({
                                        "threadId": thread_id,
                                        "itemId": item_id,
                                        "delta": delta,
                                    }),
                                );
                                last_text = full_text.clone();
                            }
                        }
                        if let Some(usage) = message.get("usage") {
                            last_usage = Some(usage.clone());
                        }
                    }
                } else if event_type == "user" {
                    if let Some(message) = value.get("message") {
                        if let Some(content) = message.get("content").and_then(|v| v.as_array()) {
                            for (index, entry) in content.iter().enumerate() {
                                if entry.get("type").and_then(|v| v.as_str()) != Some("tool_result") {
                                    continue;
                                }
                                let tool_use_id = entry
                                    .get("tool_use_id")
                                    .or_else(|| entry.get("toolUseId"))
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("");
                                let content_value = entry.get("content").cloned().unwrap_or(Value::Null);
                                let mut output = tool_result_output(&content_value);
                                let is_error = entry
                                    .get("is_error")
                                    .and_then(|v| v.as_bool())
                                    .unwrap_or(false);
                                if output.trim().is_empty() {
                                    if let Some(fallback) = value
                                        .get("toolUseResult")
                                        .or_else(|| value.get("tool_use_result"))
                                    {
                                        output = fallback
                                            .get("content")
                                            .map(tool_result_output)
                                            .unwrap_or_else(|| tool_result_output(fallback));
                                    }
                                }
                                let output_lower = output.to_lowercase();
                                let is_permission_denial = is_error
                                    && (output_lower.contains("requested permissions")
                                        || output_lower.contains("haven't granted"));
                                let result_value = tool_result_value(&content_value, &value);
                                let command = tool_names
                                    .get(tool_use_id)
                                    .cloned()
                                    .unwrap_or_else(|| "Tool".to_string());
                                let tool_input = tool_inputs
                                    .get(tool_use_id)
                                    .cloned()
                                    .unwrap_or(Value::Null);
                                if is_permission_denial {
                                    let denial_id = if tool_use_id.is_empty() {
                                        format!("{thread_id}-{command}-{index}")
                                    } else {
                                        tool_use_id.to_string()
                                    };
                                    if permission_denial_ids.insert(denial_id) {
                                        emit_event(
                                            &event_sink,
                                            &workspace_id,
                                            "turn/permissionDenied",
                                            json!({
                                                "threadId": thread_id,
                                                "turnId": current_turn_id,
                                            "permissionDenials": [json!({
                                                "toolName": command,
                                                "toolUseId": tool_use_id,
                                                "toolInput": tool_input.clone(),
                                            })],
                                            }),
                                        );
                                    }
                                }
                                output = collapse_subagent_output(output, &command, &tool_input, &value);
                                let item_id_result = if tool_use_id.is_empty() {
                                    tool_counter += 1;
                                    format!("{current_turn_id}-tool-result-{tool_counter}")
                                } else {
                                    tool_use_id.to_string()
                                };
                                emit_event(
                                    &event_sink,
                                    &workspace_id,
                                    "item/completed",
                                    json!({
                                        "threadId": thread_id,
                                        "item": build_tool_item(
                                            &item_id_result,
                                            &command,
                                            &tool_input,
                                            "completed",
                                            Some(output.as_str()),
                                            Some(&result_value),
                                        ),
                                    }),
                                );
                            }
                        }
                    }
                } else if event_type == "result" {
                    if let Some(usage) = value.get("usage") {
                        last_usage = Some(usage.clone());
                    }
                    if let Some(model_usage) = value.get("modelUsage") {
                        last_model_usage = Some(model_usage.clone());
                    }
                    let mut denials: Vec<Value> = Vec::new();
                    if let Some(items) = value
                        .get("permission_denials")
                        .or_else(|| value.get("permissionDenials"))
                        .and_then(|item| item.as_array())
                    {
                        for (index, entry) in items.iter().enumerate() {
                            let tool_name = entry
                                .get("tool_name")
                                .or_else(|| entry.get("toolName"))
                                .and_then(|item| item.as_str())
                                .unwrap_or("")
                                .trim()
                                .to_string();
                            if tool_name.is_empty() {
                                continue;
                            }
                            let tool_use_id = entry
                                .get("tool_use_id")
                                .or_else(|| entry.get("toolUseId"))
                                .and_then(|item| item.as_str())
                                .unwrap_or("")
                                .to_string();
                            let tool_input = entry
                                .get("tool_input")
                                .or_else(|| entry.get("toolInput"))
                                .cloned()
                                .unwrap_or(Value::Null);
                            let denial_id = if tool_use_id.is_empty() {
                                format!("{thread_id}-{tool_name}-{index}")
                            } else {
                                tool_use_id.clone()
                            };
                            if permission_denial_ids.insert(denial_id) {
                                denials.push(json!({
                                    "toolName": tool_name,
                                    "toolUseId": tool_use_id,
                                    "toolInput": tool_input,
                                }));
                            }
                        }
                    }
                    if !denials.is_empty() {
                        emit_event(
                            &event_sink,
                            &workspace_id,
                            "turn/permissionDenied",
                            json!({
                                "threadId": thread_id,
                                "turnId": current_turn_id,
                                "permissionDenials": denials,
                            }),
                        );
                    }

                    // Result event signals end of turn
                    if turn_active {
                        if let Some(usage) = last_usage.take().and_then(|u| format_token_usage(u, last_model_usage.as_ref())) {
                            emit_event(
                                &event_sink,
                                &workspace_id,
                                "thread/tokenUsage/updated",
                                json!({
                                    "threadId": thread_id,
                                    "tokenUsage": usage,
                                }),
                            );
                        }

                        emit_event(
                            &event_sink,
                            &workspace_id,
                            "item/completed",
                            json!({
                                "threadId": thread_id,
                                "item": {
                                    "id": item_id,
                                    "type": "agentMessage",
                                    "text": full_text,
                                    "model": last_model,
                                },
                            }),
                        );
                        emit_event(
                            &event_sink,
                            &workspace_id,
                            "turn/completed",
                            json!({
                                "threadId": thread_id,
                                "turn": { "id": current_turn_id, "threadId": thread_id },
                            }),
                        );

                        turn_active = false;
                    }
                }
            }
            Err(_) => {
                // Error reading - process likely ended
                if turn_active {
                    emit_event(
                        &event_sink,
                        &workspace_id,
                        "turn/completed",
                        json!({
                            "threadId": thread_id,
                            "turn": { "id": current_turn_id, "threadId": thread_id },
                        }),
                    );
                }
                break;
            }
        }
    }
}

/// Background task that reads stderr from the persistent Claude CLI session
/// and emits error events to the frontend.
async fn read_persistent_stderr(
    mut reader: AsyncBufReader<tokio::process::ChildStderr>,
    workspace_id: String,
    thread_id: String,
    session: Arc<WorkspaceSession>,
    event_sink: TauriEventSink,
) {
    let mut line = String::new();

    loop {
        line.clear();
        match reader.read_line(&mut line).await {
            Ok(0) => {
                // EOF - process ended, cleanup the session for this thread
                let _ = session.kill_persistent_session(&thread_id).await;
                break;
            }
            Ok(_) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }

                // Emit stderr message to frontend
                emit_event(
                    &event_sink,
                    &workspace_id,
                    "claude/stderr",
                    json!({ "message": trimmed, "threadId": thread_id }),
                );
            }
            Err(_) => {
                // Error reading - process likely ended, cleanup
                let _ = session.kill_persistent_session(&thread_id).await;
                break;
            }
        }
    }
}

fn emit_event(event_sink: &TauriEventSink, workspace_id: &str, method: &str, params: Value) {
    event_sink.emit_app_server_event(AppServerEvent {
        workspace_id: workspace_id.to_string(),
        message: json!({
            "method": method,
            "params": params,
        }),
    });
}

fn emit_event_with_id(event_sink: &TauriEventSink, workspace_id: &str, method: &str, id: u64, params: Value) {
    event_sink.emit_app_server_event(AppServerEvent {
        workspace_id: workspace_id.to_string(),
        message: json!({
            "id": id,
            "method": method,
            "params": params,
        }),
    });
}

fn build_thread_from_session(entry: &WorkspaceEntry, thread_id: &str) -> Result<Value, String> {
    let session_path = if let Some((parent_id, agent_id)) =
        parse_subagent_thread_id(thread_id)
    {
        resolve_subagent_path(entry, &parent_id, &agent_id)
    } else {
        resolve_session_path(entry, thread_id)
    }
    .ok_or_else(|| "Session file not found".to_string())?;
    let file = File::open(&session_path).map_err(|err| err.to_string())?;
    let reader = BufReader::new(file);
    let mut items: Vec<Value> = Vec::new();
    let mut tool_names: HashMap<String, String> = HashMap::new();
    let mut tool_inputs: HashMap<String, Value> = HashMap::new();
    let mut tool_item_indices: HashMap<String, usize> = HashMap::new();
    let mut subagent_tool_ids: HashSet<String> = HashSet::new();
    let mut preview: Option<String> = None;
    let mut created_at: Option<i64> = None;
    let mut updated_at: Option<i64> = None;

    for line in reader.lines() {
        let line = match line {
            Ok(line) => line,
            Err(_) => continue,
        };
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let event_type = value.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if event_type != "user" && event_type != "assistant" {
            continue;
        }
        let timestamp = value
            .get("timestamp")
            .and_then(value_to_millis)
            .unwrap_or(0);
        if created_at.is_none() {
            created_at = Some(timestamp);
        }
        updated_at = Some(timestamp);

        let message = value.get("message");
        let content = message.map(normalize_message_content).unwrap_or_default();

        if event_type == "user" {
            if has_user_message_content(&content) {
                if preview.is_none() {
                    let text = extract_text_from_content(&content);
                    if !text.is_empty() {
                        preview = Some(text);
                    }
                }
                items.push(json!({
                    "id": value.get("uuid").and_then(|v| v.as_str()).unwrap_or(thread_id),
                    "type": "userMessage",
                    "content": content.clone(),
                }));
            }
            for entry in content.iter() {
                if entry.get("type").and_then(|v| v.as_str()) != Some("tool_result") {
                    continue;
                }
                let tool_use_id = entry
                    .get("tool_use_id")
                    .or_else(|| entry.get("toolUseId"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let content_value = entry.get("content").cloned().unwrap_or(Value::Null);
                let mut output = tool_result_output(&content_value);
                if output.trim().is_empty() {
                    if let Some(fallback) = value
                        .get("toolUseResult")
                        .or_else(|| value.get("tool_use_result"))
                    {
                        output = fallback
                            .get("content")
                            .map(tool_result_output)
                            .unwrap_or_else(|| tool_result_output(fallback));
                    }
                }
                let result_value = tool_result_value(&content_value, &value);
                let command = tool_names
                    .get(tool_use_id)
                    .cloned()
                    .unwrap_or_else(|| "Tool".to_string());
                let tool_input = tool_inputs
                    .get(tool_use_id)
                    .cloned()
                    .unwrap_or(Value::Null);
                // Only skip nested subagent tool results (those with agentId)
                // Task tool results should be shown - they don't have agentId
                if extract_subagent_id(&value).is_some() {
                    continue;
                }
                output = collapse_subagent_output(output, &command, &tool_input, &value);
                let id = if tool_use_id.is_empty() {
                    format!("{thread_id}-tool-result-{}", items.len())
                } else {
                    tool_use_id.to_string()
                };
                let item_id = id.clone();
                let item = build_tool_item(
                    &id,
                    &command,
                    &tool_input,
                    "completed",
                    Some(output.as_str()),
                    Some(&result_value),
                );
                if let Some(index) = tool_item_indices.get(&item_id) {
                    items[*index] = item;
                } else {
                    tool_item_indices.insert(item_id, items.len());
                    items.push(item);
                }
            }
        } else if event_type == "assistant" {
            let mut text = String::new();
            let mut thinking_index = 0;
            for entry in content.iter() {
                match entry.get("type").and_then(|v| v.as_str()) {
                    Some("text") => {
                        if let Some(piece) = entry.get("text").and_then(|v| v.as_str()) {
                            text.push_str(piece);
                        }
                    }
                    Some("thinking") => {
                        if let Some(thinking) =
                            entry.get("thinking").and_then(|v| v.as_str())
                        {
                            let trimmed = thinking.trim();
                            if !trimmed.is_empty() {
                                let message_id = value
                                    .get("uuid")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or(thread_id);
                                let id = format!("{message_id}-thinking-{thinking_index}");
                                thinking_index += 1;
                                items.push(json!({
                                    "id": id,
                                    "type": "reasoning",
                                    "summary": "",
                                    "content": trimmed,
                                }));
                            }
                        }
                    }
                    Some("tool_use") => {
                        let tool_id = entry
                            .get("id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        let tool_name = entry
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("Tool")
                            .to_string();
                        let tool_input = entry.get("input").cloned().unwrap_or(Value::Null);
                        let is_subagent_tool = is_subagent_task(&tool_name, &tool_input);
                        if !tool_id.is_empty() {
                            tool_names.insert(tool_id.to_string(), tool_name.clone());
                            tool_inputs.insert(tool_id.to_string(), tool_input.clone());
                            if is_subagent_tool {
                                subagent_tool_ids.insert(tool_id.to_string());
                            }
                        }
                        // Don't skip Task tools - we want to show them
                        // (subagent_tool_ids tracking above is still needed for collapsing output)
                    let id = if tool_id.is_empty() {
                        format!("{thread_id}-tool-{}", items.len())
                    } else {
                        tool_id.to_string()
                    };
                    let item_id = id.clone();
                    let item = build_tool_item(
                        &id,
                        &tool_name,
                        &tool_input,
                        "running",
                        None,
                        None,
                    );
                    if let Some(index) = tool_item_indices.get(&item_id) {
                        items[*index] = item;
                    } else {
                        tool_item_indices.insert(item_id, items.len());
                        items.push(item);
                        }
                    }
                    _ => {}
                }
            }
            if !text.trim().is_empty() {
                let model = message
                    .and_then(|message| message.get("model"))
                    .and_then(|value| value.as_str());
                items.push(json!({
                    "id": value.get("uuid").and_then(|v| v.as_str()).unwrap_or(thread_id),
                    "type": "agentMessage",
                    "text": text.trim(),
                    "model": model,
                }));
            }
        }
    }

    let metadata = load_sessions_index(entry)
        .into_iter()
        .find(|entry| entry.session_id == thread_id);
    let created_at = metadata
        .as_ref()
        .and_then(|entry| parse_iso_timestamp(entry.created.as_deref()))
        .or(created_at)
        .unwrap_or(0);
    let updated_at = metadata
        .as_ref()
        .and_then(|entry| parse_iso_timestamp(entry.modified.as_deref()))
        .or(updated_at)
        .unwrap_or(created_at);
    let preview = metadata
        .and_then(|entry| entry.first_prompt)
        .or(preview)
        .unwrap_or_default();

    Ok(json!({
        "id": thread_id,
        "preview": preview,
        "createdAt": created_at,
        "updatedAt": updated_at,
        "cwd": entry.path,
        "turns": [
            {
                "id": thread_id,
                "items": items,
            }
        ],
    }))
}

fn load_sessions_index(entry: &WorkspaceEntry) -> Vec<ClaudeSessionEntry> {
    let index_path = resolve_sessions_index_path(entry);
    let mut entries = match &index_path {
        Some(path) => {
            eprintln!("[debug:sessions] Loading sessions index from {:?}", path);
            match fs::read_to_string(path) {
                Ok(data) => match serde_json::from_str::<Value>(&data) {
                    Ok(value) => {
                        let parsed = parse_sessions_value(&value);
                        eprintln!(
                            "[debug:sessions] Parsed {} entries from sessions index",
                            parsed.len()
                        );
                        parsed
                    }
                    Err(err) => {
                        eprintln!(
                            "[debug:sessions] Failed to parse sessions index JSON at {:?}: {}",
                            path, err
                        );
                        Vec::new()
                    }
                },
                Err(err) => {
                    eprintln!(
                        "[debug:sessions] Failed to read sessions index at {:?}: {}",
                        path, err
                    );
                    Vec::new()
                }
            }
        }
        None => {
            eprintln!(
                "[debug:sessions] No sessions index found for workspace {:?}, falling back to filesystem scan",
                entry.path
            );
            Vec::new()
        }
    };

    let scanned = scan_project_sessions(entry);
    if entries.is_empty() {
        eprintln!(
            "[debug:sessions] Index was empty, using {} scanned entries only",
            scanned.len()
        );
        return scanned;
    }

    eprintln!(
        "[debug:sessions] Merging {} index entries with {} scanned entries",
        entries.len(),
        scanned.len()
    );
    let mut merged: HashMap<String, ClaudeSessionEntry> = HashMap::new();
    for entry in entries.drain(..) {
        merged.insert(entry.session_id.clone(), entry);
    }
    for scanned_entry in scanned {
        let session_id = scanned_entry.session_id.clone();
        match merged.get_mut(&session_id) {
            Some(existing) => {
                if existing.file_mtime.is_none()
                    || scanned_entry
                        .file_mtime
                        .zip(existing.file_mtime)
                        .map(|(scanned, current)| scanned > current)
                        .unwrap_or(false)
                {
                    existing.file_mtime = scanned_entry.file_mtime;
                }
                if existing.first_prompt.is_none() {
                    existing.first_prompt = scanned_entry.first_prompt;
                }
                if existing.message_count.is_none() {
                    existing.message_count = scanned_entry.message_count;
                }
                if existing.git_branch.is_none() {
                    existing.git_branch = scanned_entry.git_branch;
                }
                if existing.project_path.is_none() {
                    existing.project_path = scanned_entry.project_path;
                }
            }
            None => {
                merged.insert(session_id, scanned_entry);
            }
        }
    }

    eprintln!(
        "[debug:sessions] Merge complete: {} total sessions after merging index + scan",
        merged.len()
    );
    merged.into_values().collect()
}

fn parse_sessions_value(value: &Value) -> Vec<ClaudeSessionEntry> {
    if let Some(entries) = value.get("entries").and_then(|v| v.as_array()) {
        parse_sessions_entries(entries)
    } else if let Some(entries) = value.get("sessions").and_then(|v| v.as_array()) {
        parse_sessions_entries(entries)
    } else if value.is_array() {
        parse_sessions_entries(value.as_array().unwrap_or(&Vec::new()))
    } else {
        Vec::new()
    }
}

fn parse_sessions_entries(entries: &[Value]) -> Vec<ClaudeSessionEntry> {
    let mut parsed = Vec::new();
    let mut skipped = 0;
    for entry in entries {
        match serde_json::from_value::<ClaudeSessionEntry>(entry.clone()) {
            Ok(session_entry) => parsed.push(session_entry),
            Err(err) => {
                skipped += 1;
                let session_id = entry
                    .get("sessionId")
                    .or_else(|| entry.get("session_id"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("<unknown>");
                eprintln!(
                    "[debug:sessions] Failed to deserialize session entry '{}': {} | raw keys: {:?}",
                    session_id,
                    err,
                    entry.as_object().map(|o| o.keys().collect::<Vec<_>>())
                );
            }
        }
    }
    if skipped > 0 {
        eprintln!(
            "[debug:sessions] Skipped {} of {} entries due to deserialization failures",
            skipped,
            entries.len()
        );
    }
    parsed
}

fn scan_project_sessions(entry: &WorkspaceEntry) -> Vec<ClaudeSessionEntry> {
    let Some(project_dir) = resolve_project_dir(entry) else {
        eprintln!(
            "[debug:sessions] Could not resolve project dir for workspace {:?}",
            entry.path
        );
        return Vec::new();
    };
    eprintln!("[debug:sessions] Scanning project sessions in {:?}", project_dir);
    let mut entries = Vec::new();
    let dir_entries = match fs::read_dir(&project_dir) {
        Ok(dir_entries) => dir_entries,
        Err(err) => {
            eprintln!(
                "[debug:sessions] Failed to read project directory {:?}: {}",
                project_dir, err
            );
            return Vec::new();
        }
    };
    for dir_entry in dir_entries.flatten() {
        let path = dir_entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("jsonl") {
            continue;
        }
        let session_id = match path.file_stem().and_then(|stem| stem.to_str()) {
            Some(stem) if !stem.is_empty() => stem.to_string(),
            _ => continue,
        };
        let metadata = dir_entry.metadata().ok();
        let file_mtime = metadata
            .and_then(|meta| meta.modified().ok())
            .and_then(|timestamp| timestamp.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_millis() as i64);
        let (first_prompt, message_count, git_branch) =
            scan_session_metadata(&path);
        entries.push(ClaudeSessionEntry {
            session_id,
            file_mtime,
            first_prompt,
            message_count,
            created: None,
            modified: None,
            git_branch,
            project_path: Some(entry.path.clone()),
            is_sidechain: Some(false),
        });
    }
    eprintln!(
        "[debug:sessions] Filesystem scan found {} .jsonl session files in {:?}",
        entries.len(),
        project_dir
    );
    entries
}

fn list_session_files(entry: &WorkspaceEntry) -> Vec<(String, PathBuf, i64)> {
    let Some(project_dir) = resolve_project_dir(entry) else {
        return Vec::new();
    };
    let dir_entries = match fs::read_dir(project_dir) {
        Ok(dir_entries) => dir_entries,
        Err(_) => return Vec::new(),
    };
    let mut sessions = Vec::new();
    for dir_entry in dir_entries.flatten() {
        let path = dir_entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("jsonl") {
            continue;
        }
        let session_id = match path.file_stem().and_then(|stem| stem.to_str()) {
            Some(stem) if !stem.is_empty() => stem.to_string(),
            _ => continue,
        };
        let file_mtime = dir_entry
            .metadata()
            .ok()
            .and_then(|meta| meta.modified().ok())
            .and_then(|timestamp| timestamp.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_millis() as i64)
            .unwrap_or(0);
        sessions.push((session_id, path, file_mtime));
    }
    sessions
}

fn scan_session_metadata(path: &Path) -> (Option<String>, Option<i64>, Option<String>) {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(err) => {
            eprintln!(
                "[debug:sessions] Failed to open session file {:?}: {}",
                path, err
            );
            return (None, None, None);
        }
    };
    let reader = BufReader::new(file);
    let mut first_prompt: Option<String> = None;
    let mut message_count: i64 = 0;
    let mut git_branch: Option<String> = None;
    let mut line_errors: u32 = 0;
    let mut json_errors: u32 = 0;
    let mut total_lines: u32 = 0;
    for line in reader.lines() {
        total_lines += 1;
        let line = match line {
            Ok(line) => line,
            Err(err) => {
                line_errors += 1;
                if line_errors == 1 {
                    eprintln!(
                        "[debug:sessions] Read error in session file {:?} at line {}: {}",
                        path, total_lines, err
                    );
                }
                continue;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(err) => {
                json_errors += 1;
                if json_errors == 1 {
                    eprintln!(
                        "[debug:sessions] JSON parse error in session file {:?} at line {}: {}",
                        path, total_lines, err
                    );
                }
                continue;
            }
        };
        let event_type = value.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if event_type == "user" || event_type == "assistant" {
            message_count += 1;
        }
        if git_branch.is_none() {
            git_branch = value
                .get("gitBranch")
                .and_then(|branch| branch.as_str())
                .map(|branch| branch.to_string());
        }
        if first_prompt.is_none() && event_type == "user" {
            if let Some(message) = value.get("message") {
                let text = extract_text_from_message(message);
                if !text.is_empty() {
                    first_prompt = Some(text);
                }
            }
        }
    }

    if line_errors > 0 || json_errors > 0 {
        eprintln!(
            "[debug:sessions] Session file {:?}: {} total lines, {} read errors, {} JSON parse errors",
            path, total_lines, line_errors, json_errors
        );
    }

    (
        first_prompt,
        if message_count > 0 {
            Some(message_count)
        } else {
            None
        },
        git_branch,
    )
}

const SUBAGENT_THREAD_MARKER: &str = "::subagent::";

fn subagent_thread_id(parent_id: &str, agent_id: &str) -> String {
    format!("{parent_id}{SUBAGENT_THREAD_MARKER}{agent_id}")
}

fn parse_subagent_thread_id(thread_id: &str) -> Option<(String, String)> {
    let (parent_id, agent_id) = thread_id.split_once(SUBAGENT_THREAD_MARKER)?;
    if parent_id.is_empty() || agent_id.is_empty() {
        return None;
    }
    Some((parent_id.to_string(), agent_id.to_string()))
}

fn resolve_subagent_path(
    entry: &WorkspaceEntry,
    parent_id: &str,
    agent_id: &str,
) -> Option<PathBuf> {
    let project_dir = resolve_project_dir(entry)?;
    let subagent_dir = project_dir.join(parent_id).join("subagents");
    let candidate = subagent_dir.join(format!("{agent_id}.jsonl"));
    if candidate.exists() {
        Some(candidate)
    } else {
        None
    }
}

fn list_subagent_files(
    entry: &WorkspaceEntry,
    parent_id: &str,
) -> Vec<(String, PathBuf, i64)> {
    let mut files = Vec::new();
    let project_dir = match resolve_project_dir(entry) {
        Some(dir) => dir,
        None => return files,
    };
    let subagent_dir = project_dir.join(parent_id).join("subagents");
    let dir_entries = match fs::read_dir(subagent_dir) {
        Ok(entries) => entries,
        Err(_) => return files,
    };
    for dir_entry in dir_entries.flatten() {
        let path = dir_entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("jsonl") {
            continue;
        }
        let agent_id = match path.file_stem().and_then(|stem| stem.to_str()) {
            Some(stem) if !stem.is_empty() => stem.to_string(),
            _ => continue,
        };
        let file_mtime = dir_entry
            .metadata()
            .ok()
            .and_then(|meta| meta.modified().ok())
            .and_then(|timestamp| timestamp.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_millis() as i64)
            .unwrap_or(0);
        files.push((agent_id, path, file_mtime));
    }
    files
}

fn build_subagent_thread(
    parent_id: &str,
    agent_id: &str,
    cwd: &str,
    path: &Path,
    file_mtime: i64,
) -> Value {
    let (first_prompt, message_count, git_branch) = scan_session_metadata(path);
    let preview = first_prompt.unwrap_or_else(|| format!("Subagent {agent_id}"));
    json!({
        "id": subagent_thread_id(parent_id, agent_id),
        "preview": preview,
        "messageCount": message_count.unwrap_or(0),
        "createdAt": file_mtime,
        "updatedAt": file_mtime,
        "cwd": cwd,
        "gitBranch": git_branch,
        "parentId": parent_id,
    })
}

fn list_subagent_threads(entry: &WorkspaceEntry, parent_id: &str, cwd: &str) -> Vec<Value> {
    list_subagent_files(entry, parent_id)
        .into_iter()
        .map(|(agent_id, path, file_mtime)| {
            build_subagent_thread(parent_id, &agent_id, cwd, &path, file_mtime)
        })
        .collect()
}

fn process_subagent_line(
    workspace_id: &str,
    thread_id: &str,
    turn_id: &str,
    value: &Value,
    event_sink: &TauriEventSink,
    tool_names: &mut HashMap<String, String>,
    tool_inputs: &mut HashMap<String, Value>,
    tool_counter: &mut usize,
) {
    let event_type = value.get("type").and_then(|v| v.as_str()).unwrap_or("");
    if event_type != "user" && event_type != "assistant" {
        return;
    }

    let message = value.get("message");
    let content = message.map(normalize_message_content).unwrap_or_default();

    if event_type == "user" {
        if has_user_message_content(&content) {
            let message_id = value
                .get("uuid")
                .and_then(|v| v.as_str())
                .unwrap_or(thread_id);
            emit_event(
                event_sink,
                workspace_id,
                "item/completed",
                json!({
                    "threadId": thread_id,
                    "item": {
                        "id": message_id,
                        "type": "userMessage",
                        "content": content.clone(),
                    }
                }),
            );
        }

        for entry in content.iter() {
            if entry.get("type").and_then(|v| v.as_str()) != Some("tool_result") {
                continue;
            }
            let tool_use_id = entry
                .get("tool_use_id")
                .or_else(|| entry.get("toolUseId"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let content_value = entry.get("content").cloned().unwrap_or(Value::Null);
            let mut output = tool_result_output(&content_value);
            if output.trim().is_empty() {
                if let Some(fallback) = value
                    .get("toolUseResult")
                    .or_else(|| value.get("tool_use_result"))
                {
                    output = fallback
                        .get("content")
                        .map(tool_result_output)
                        .unwrap_or_else(|| tool_result_output(fallback));
                }
            }
            let result_value = tool_result_value(&content_value, value);
            let command = tool_names
                .get(tool_use_id)
                .cloned()
                .unwrap_or_else(|| "Tool".to_string());
            let tool_input = tool_inputs
                .get(tool_use_id)
                .cloned()
                .unwrap_or(Value::Null);
            output = collapse_subagent_output(output, &command, &tool_input, value);
            let item_id = if tool_use_id.is_empty() {
                *tool_counter += 1;
                format!("{turn_id}-tool-result-{}", *tool_counter)
            } else {
                tool_use_id.to_string()
            };
            emit_event(
                event_sink,
                workspace_id,
                "item/completed",
                json!({
                    "threadId": thread_id,
                    "item": build_tool_item(
                        &item_id,
                        &command,
                        &tool_input,
                        "completed",
                        Some(output.as_str()),
                        Some(&result_value),
                    ),
                }),
            );
        }
        return;
    }

    let mut thinking_index = 0;
    for entry in content.iter() {
        match entry.get("type").and_then(|v| v.as_str()) {
            Some("tool_use") => {
                let tool_id = entry
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let tool_name = entry
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Tool")
                    .to_string();
                let tool_input = entry.get("input").cloned().unwrap_or(Value::Null);
                if !tool_id.is_empty() {
                    tool_names.insert(tool_id.to_string(), tool_name.clone());
                    tool_inputs.insert(tool_id.to_string(), tool_input.clone());
                }
                let item_id = if tool_id.is_empty() {
                    *tool_counter += 1;
                    format!("{turn_id}-tool-{}", *tool_counter)
                } else {
                    tool_id.to_string()
                };
                emit_event(
                    event_sink,
                    workspace_id,
                    "item/started",
                    json!({
                        "threadId": thread_id,
                        "item": build_tool_item(
                            &item_id,
                            &tool_name,
                            &tool_input,
                            "running",
                            None,
                            None,
                        ),
                    }),
                );
            }
            Some("thinking") => {
                if let Some(thinking) = entry.get("thinking").and_then(|v| v.as_str()) {
                    let trimmed = thinking.trim();
                    if !trimmed.is_empty() {
                        let message_id = value
                            .get("uuid")
                            .and_then(|v| v.as_str())
                            .unwrap_or(thread_id);
                        let id = format!("{message_id}-thinking-{thinking_index}");
                        thinking_index += 1;
                        emit_event(
                            event_sink,
                            workspace_id,
                            "item/completed",
                            json!({
                                "threadId": thread_id,
                                "item": {
                                    "id": id,
                                    "type": "reasoning",
                                    "summary": "",
                                    "content": trimmed,
                                }
                            }),
                        );
                    }
                }
            }
            _ => {}
        }
    }

    let text = extract_text_from_content(&content).trim().to_string();
    if !text.is_empty() {
        let message_id = value
            .get("uuid")
            .and_then(|v| v.as_str())
            .unwrap_or(thread_id);
        let model = message
            .and_then(|message| message.get("model"))
            .and_then(|value| value.as_str());
        emit_event(
            event_sink,
            workspace_id,
            "item/completed",
            json!({
                "threadId": thread_id,
                "item": {
                    "id": message_id,
                    "type": "agentMessage",
                    "text": text,
                    "model": model,
                }
            }),
        );
    }
}

async fn tail_subagent_thread(
    workspace_id: String,
    thread_id: String,
    path: PathBuf,
    event_sink: TauriEventSink,
    shutdown: watch::Receiver<bool>,
) {
    let turn_id = Uuid::new_v4().to_string();
    emit_event(
        &event_sink,
        &workspace_id,
        "turn/started",
        json!({
            "threadId": thread_id.clone(),
            "turn": { "id": turn_id, "threadId": thread_id.clone() },
        }),
    );

    let file = match tokio::fs::File::open(&path).await {
        Ok(file) => file,
        Err(_) => {
            emit_event(
                &event_sink,
                &workspace_id,
                "turn/completed",
                json!({
                    "threadId": thread_id.clone(),
                    "turn": { "id": turn_id, "threadId": thread_id.clone() },
                }),
            );
            return;
        }
    };
    let mut reader = AsyncBufReader::new(file);
    let mut line = String::new();
    let mut tool_names: HashMap<String, String> = HashMap::new();
    let mut tool_inputs: HashMap<String, Value> = HashMap::new();
    let mut tool_counter: usize = 0;

    loop {
        if *shutdown.borrow() {
            break;
        }
        line.clear();
        match reader.read_line(&mut line).await {
            Ok(0) => {
                sleep(Duration::from_millis(120)).await;
            }
            Ok(_) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                let value: Value = match serde_json::from_str(trimmed) {
                    Ok(value) => value,
                    Err(_) => continue,
                };
                process_subagent_line(
                    &workspace_id,
                    &thread_id,
                    &turn_id,
                    &value,
                    &event_sink,
                    &mut tool_names,
                    &mut tool_inputs,
                    &mut tool_counter,
                );
            }
            Err(_) => break,
        }
    }

    emit_event(
        &event_sink,
        &workspace_id,
        "turn/completed",
        json!({
            "threadId": thread_id.clone(),
            "turn": { "id": turn_id, "threadId": thread_id.clone() },
        }),
    );
}

async fn watch_workspace_threads(
    workspace_id: String,
    entry: WorkspaceEntry,
    event_sink: TauriEventSink,
    shutdown: watch::Receiver<bool>,
) {
    let mut known_sessions: HashSet<String> = HashSet::new();
    let mut known_subagents: HashSet<String> = HashSet::new();
    let mut active_subagents: HashMap<String, tokio::task::JoinHandle<()>> = HashMap::new();
    let cwd = entry.path.clone();

    let initial_sessions = list_session_files(&entry);
    for (session_id, _, _) in &initial_sessions {
        known_sessions.insert(session_id.clone());
    }
    for (session_id, _, _) in &initial_sessions {
        for (agent_id, _, _) in list_subagent_files(&entry, session_id) {
            let thread_id = subagent_thread_id(session_id, &agent_id);
            known_subagents.insert(thread_id);
        }
    }

    let mut ticker = interval(Duration::from_millis(1000));
    loop {
        if *shutdown.borrow() {
            break;
        }
        ticker.tick().await;
        let sessions = list_session_files(&entry);
        for (session_id, path, file_mtime) in &sessions {
            if known_sessions.insert(session_id.clone()) {
                let (first_prompt, message_count, git_branch) = scan_session_metadata(path);
                let thread = json!({
                    "id": session_id,
                    "preview": first_prompt.unwrap_or_default(),
                    "messageCount": message_count.unwrap_or(0),
                    "createdAt": *file_mtime,
                    "updatedAt": *file_mtime,
                    "cwd": cwd.clone(),
                    "gitBranch": git_branch,
                });
                emit_event(
                    &event_sink,
                    &workspace_id,
                    "thread/created",
                    json!({ "thread": thread }),
                );
            }
        }

        for (session_id, _, _) in &sessions {
            for (agent_id, path, file_mtime) in list_subagent_files(&entry, session_id) {
                let thread_id = subagent_thread_id(session_id, &agent_id);
                if known_subagents.insert(thread_id.clone()) {
                    let thread =
                        build_subagent_thread(session_id, &agent_id, &cwd, &path, file_mtime);
                    emit_event(
                        &event_sink,
                        &workspace_id,
                        "thread/created",
                        json!({ "thread": thread }),
                    );

                    let handle = tokio::spawn(tail_subagent_thread(
                        workspace_id.clone(),
                        thread_id.clone(),
                        path,
                        event_sink.clone(),
                        shutdown.clone(),
                    ));
                    active_subagents.insert(thread_id, handle);
                }
            }
        }

        active_subagents.retain(|_, handle| !handle.is_finished());
    }

    for (_, handle) in active_subagents {
        let _ = handle.await;
    }
}

fn session_sort_key(entry: &ClaudeSessionEntry) -> i64 {
    parse_iso_timestamp(entry.modified.as_deref())
        .or(entry.file_mtime)
        .unwrap_or(0)
}

fn parse_iso_timestamp(value: Option<&str>) -> Option<i64> {
    let value = value?;
    DateTime::parse_from_rfc3339(value)
        .map(|dt| dt.timestamp_millis())
        .ok()
}

fn value_to_millis(value: &Value) -> Option<i64> {
    match value {
        Value::String(value) => parse_iso_timestamp(Some(value)),
        Value::Number(value) => value.as_i64().map(|raw| if raw < 1_000_000_000_000 { raw * 1000 } else { raw }),
        _ => None,
    }
}

fn resolve_project_dir(entry: &WorkspaceEntry) -> Option<PathBuf> {
    let projects_root = resolve_default_claude_home()?.join("projects");
    Some(projects_root.join(encode_project_path(&entry.path)))
}

fn resolve_sessions_index_path(entry: &WorkspaceEntry) -> Option<PathBuf> {
    let project_dir = resolve_project_dir(entry)?;
    let index_path = project_dir.join("sessions-index.json");
    if index_path.exists() {
        Some(index_path)
    } else {
        None
    }
}

fn fork_session_from_message(
    entry: &WorkspaceEntry,
    thread_id: &str,
    message_id: &str,
) -> Result<String, String> {
    let session_path = resolve_session_path(entry, thread_id)
        .ok_or_else(|| "Session file not found".to_string())?;
    let project_dir = resolve_project_dir(entry)
        .ok_or_else(|| "Session project directory not found".to_string())?;
    let new_thread_id = Uuid::new_v4().to_string();
    let new_path = project_dir.join(format!("{new_thread_id}.jsonl"));

    let file = File::open(&session_path).map_err(|err| err.to_string())?;
    let reader = BufReader::new(file);
    let output = File::create(&new_path).map_err(|err| err.to_string())?;
    let mut writer = BufWriter::new(output);
    let mut found = false;

    for line in reader.lines() {
        let line = line.map_err(|err| err.to_string())?;
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(mut value) = serde_json::from_str::<Value>(&line) {
            rewrite_session_id(&mut value, &new_thread_id);
            let serialized = serde_json::to_string(&value).map_err(|err| err.to_string())?;
            writer
                .write_all(serialized.as_bytes())
                .and_then(|_| writer.write_all(b"\n"))
                .map_err(|err| err.to_string())?;
            if value
                .get("uuid")
                .and_then(|uuid| uuid.as_str())
                .is_some_and(|uuid| uuid == message_id)
            {
                found = true;
                break;
            }
        } else {
            writer
                .write_all(line.as_bytes())
                .and_then(|_| writer.write_all(b"\n"))
                .map_err(|err| err.to_string())?;
        }
    }

    writer.flush().map_err(|err| err.to_string())?;
    drop(writer); // Close file handle before potential delete (required on Windows)

    if !found {
        let _ = fs::remove_file(&new_path);
        return Err("Message not found in session".to_string());
    }

    Ok(new_thread_id)
}

fn rewrite_session_id(value: &mut Value, new_session_id: &str) {
    let Some(obj) = value.as_object_mut() else {
        return;
    };
    if obj.contains_key("sessionId") {
        obj.insert("sessionId".to_string(), Value::String(new_session_id.to_string()));
    }
    if obj.contains_key("session_id") {
        obj.insert("session_id".to_string(), Value::String(new_session_id.to_string()));
    }
}

fn resolve_session_path(entry: &WorkspaceEntry, thread_id: &str) -> Option<PathBuf> {
    if let Some(index_path) = resolve_sessions_index_path(entry) {
        if let Ok(data) = std::fs::read_to_string(index_path) {
            if let Ok(value) = serde_json::from_str::<Value>(&data) {
                if let Some(entries) = value.get("entries").and_then(|v| v.as_array()) {
                    for entry in entries {
                        let id = entry.get("sessionId").and_then(|v| v.as_str());
                        if id == Some(thread_id) {
                            if let Some(path) = entry.get("fullPath").and_then(|v| v.as_str()) {
                                return Some(PathBuf::from(path));
                            }
                        }
                    }
                }
            }
        }
    }
    let project_dir = resolve_project_dir(entry)?;
    let candidate = project_dir.join(format!("{thread_id}.jsonl"));
    if candidate.exists() {
        Some(candidate)
    } else {
        None
    }
}

fn session_exists(entry: &WorkspaceEntry, thread_id: &str) -> bool {
    resolve_session_path(entry, thread_id).is_some()
}

fn encode_project_path(path: &str) -> String {
    let normalized = path.replace('\\', "/");
    if normalized.starts_with('/') {
        format!("-{}", normalized.trim_start_matches('/').replace('/', "-"))
    } else {
        normalized.replace('/', "-")
    }
}

fn extract_text_from_message(message: &Value) -> String {
    let content = normalize_message_content(message);
    extract_text_from_content(&content)
}

fn normalize_message_content(message: &Value) -> Vec<Value> {
    let Some(content) = message.get("content") else {
        return Vec::new();
    };
    match content {
        Value::Array(values) => values.clone(),
        Value::String(text) => {
            if text.trim().is_empty() {
                Vec::new()
            } else {
                vec![json!({ "type": "text", "text": text })]
            }
        }
        Value::Null => Vec::new(),
        other => {
            let text = other
                .as_str()
                .map(|value| value.to_string())
                .unwrap_or_else(|| other.to_string());
            if text.trim().is_empty() {
                Vec::new()
            } else {
                vec![json!({ "type": "text", "text": text })]
            }
        }
    }
}

fn extract_text_from_content(content: &[Value]) -> String {
    let mut text = String::new();
    for entry in content {
        if entry.get("type").and_then(|v| v.as_str()) != Some("text") {
            continue;
        }
        if let Some(piece) = entry.get("text").and_then(|v| v.as_str()) {
            text.push_str(piece);
        }
    }
    text
}

fn tool_result_output(value: &Value) -> String {
    if let Some(text) = value.as_str() {
        return text.to_string();
    }
    if let Some(array) = value.as_array() {
        let text_entries: Vec<String> = array
            .iter()
            .filter_map(|entry| {
                if entry.get("type").and_then(|v| v.as_str()) == Some("text") {
                    entry.get("text").and_then(|v| v.as_str()).map(|v| v.to_string())
                } else {
                    None
                }
            })
            .filter(|text| !text.is_empty())
            .collect();
        if !text_entries.is_empty() {
            return text_entries.join("\n");
        }
    }
    if value.is_null() {
        return String::new();
    }
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

fn tool_result_value(content_value: &Value, event_value: &Value) -> Value {
    let is_empty = match content_value {
        Value::Null => true,
        Value::String(text) => text.trim().is_empty(),
        Value::Array(items) => items.is_empty(),
        _ => false,
    };
    if !is_empty {
        return content_value.clone();
    }
    if let Some(fallback) = event_value
        .get("toolUseResult")
        .or_else(|| event_value.get("tool_use_result"))
    {
        if let Some(content) = fallback.get("content") {
            return content.clone();
        }
        return fallback.clone();
    }
    Value::Null
}

fn parse_mcp_tool_name(tool_name: &str) -> Option<(String, String)> {
    let trimmed = tool_name.trim();
    if trimmed.is_empty() {
        return None;
    }
    if !trimmed.to_lowercase().starts_with("mcp__") {
        return None;
    }
    let parts: Vec<&str> = trimmed.split("__").collect();
    if parts.len() < 3 {
        return None;
    }
    let server = parts[1].trim();
    let tool = parts[2..].join("__");
    if server.is_empty() || tool.trim().is_empty() {
        return None;
    }
    Some((server.to_string(), tool.trim().to_string()))
}

fn extract_string_field(map: &Map<String, Value>, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(value) = map.get(*key).and_then(|value| value.as_str()) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

fn extract_path_from_value(value: &Value) -> Option<String> {
    if let Some(path) = value.as_str() {
        let trimmed = path.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    if let Some(map) = value.as_object() {
        let keys = [
            "file_path",
            "filePath",
            "path",
            "filename",
            "file",
            "notebook_path",
            "notebookPath",
        ];
        return extract_string_field(map, &keys);
    }
    None
}

fn extract_file_paths(tool_input: &Value) -> Vec<String> {
    let mut paths: Vec<String> = Vec::new();
    let Some(map) = tool_input.as_object() else {
        return paths;
    };
    let keys = [
        "file_path",
        "filePath",
        "path",
        "filename",
        "file",
        "notebook_path",
        "notebookPath",
    ];
    if let Some(path) = extract_string_field(map, &keys) {
        paths.push(path);
    }
    for key in ["files", "paths", "targets", "edits", "changes"] {
        if let Some(Value::Array(items)) = map.get(key) {
            for item in items {
                if let Some(path) = extract_path_from_value(item) {
                    paths.push(path);
                }
            }
        }
    }
    let mut deduped: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for path in paths {
        let trimmed = path.trim();
        if trimmed.is_empty() {
            continue;
        }
        if seen.insert(trimmed.to_string()) {
            deduped.push(trimmed.to_string());
        }
    }
    deduped
}

fn build_tool_item(
    id: &str,
    tool_name: &str,
    tool_input: &Value,
    status: &str,
    output: Option<&str>,
    result_value: Option<&Value>,
) -> Value {
    if let Some((server, tool)) = parse_mcp_tool_name(tool_name) {
        let mut item = json!({
            "id": id,
            "type": "mcpToolCall",
            "server": server,
            "tool": tool,
            "arguments": tool_input.clone(),
            "status": status,
        });
        if let Value::Object(ref mut map) = item {
            if let Some(result) = result_value {
                if !result.is_null() {
                    map.insert("result".to_string(), result.clone());
                } else if let Some(output) = output {
                    map.insert("result".to_string(), Value::String(output.to_string()));
                }
            } else if let Some(output) = output {
                map.insert("result".to_string(), Value::String(output.to_string()));
            }
        }
        return item;
    }

    let normalized = tool_name.trim().to_lowercase();
    if normalized == "websearch" {
        let query = tool_input
            .get("query")
            .and_then(|value| value.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let mut item = json!({
            "id": id,
            "type": "webSearch",
            "query": query,
            "status": status,
        });
        if let Value::Object(ref mut map) = item {
            if let Some(output) = output {
                map.insert("aggregatedOutput".to_string(), Value::String(output.to_string()));
            }
        }
        return item;
    }

    if matches!(
        normalized.as_str(),
        "write" | "edit" | "multiedit" | "notebookedit"
    ) {
        let paths = extract_file_paths(tool_input);
        let kind = if normalized == "write" { "add" } else { "modify" };
        let changes = paths
            .into_iter()
            .map(|path| json!({ "path": path, "kind": kind }))
            .collect::<Vec<_>>();
        let mut item = json!({
            "id": id,
            "type": "fileChange",
            "status": status,
            "changes": changes,
        });
        if let Value::Object(ref mut map) = item {
            if let Some(output) = output {
                map.insert("aggregatedOutput".to_string(), Value::String(output.to_string()));
            }
            if !tool_input.is_null() {
                map.insert("toolInput".to_string(), tool_input.clone());
            }
        }
        return item;
    }

    let mut item = json!({
        "id": id,
        "type": "commandExecution",
        "command": [tool_name],
        "status": status,
        "toolInput": tool_input.clone(),
    });
    if let Value::Object(ref mut map) = item {
        if let Some(output) = output {
            map.insert("aggregatedOutput".to_string(), Value::String(output.to_string()));
        }
    }
    item
}

fn extract_subagent_id(value: &Value) -> Option<String> {
    value
        .get("toolUseResult")
        .or_else(|| value.get("tool_use_result"))
        .and_then(|result| result.get("agentId"))
        .and_then(|id| id.as_str())
        .map(|id| id.to_string())
}

fn should_collapse_subagent_output(command: &str, tool_input: &Value, value: &Value) -> bool {
    if command == "Task" {
        return true;
    }
    tool_input.get("subagent_type").is_some()
        || tool_input.get("subagentType").is_some()
        || extract_subagent_id(value).is_some()
}

fn collapse_subagent_output(
    output: String,
    command: &str,
    tool_input: &Value,
    value: &Value,
) -> String {
    if !should_collapse_subagent_output(command, tool_input, value) {
        return output;
    }
    let agent_label = extract_subagent_id(value)
        .map(|id| format!("Subagent {id}"))
        .unwrap_or_else(|| "Subagent".to_string());
    format!("{agent_label} output is available in its thread.")
}

fn is_subagent_task(command: &str, tool_input: &Value) -> bool {
    command.eq_ignore_ascii_case("task")
        || tool_input.get("subagent_type").is_some()
        || tool_input.get("subagentType").is_some()
}

fn has_user_message_content(content: &[Value]) -> bool {
    content.iter().any(|entry| {
        matches!(
            entry.get("type").and_then(|v| v.as_str()),
            Some("text" | "image" | "localImage" | "skill")
        )
    })
}

fn format_token_usage(raw: Value, model_usage: Option<&Value>) -> Option<Value> {
    let Value::Object(map) = raw else {
        return None;
    };
    let input_tokens = usage_number(&map, &["input_tokens", "inputTokens"]);
    let output_tokens = usage_number(&map, &["output_tokens", "outputTokens"]);
    let cached_read = usage_number(&map, &["cache_read_input_tokens", "cacheReadInputTokens"]);
    let cached_create =
        usage_number(&map, &["cache_creation_input_tokens", "cacheCreationInputTokens"]);
    let cached_input_tokens = cached_read + cached_create;
    let reasoning_output_tokens =
        usage_number(&map, &["reasoning_output_tokens", "reasoningOutputTokens"]);
    let total_tokens = input_tokens + output_tokens + cached_input_tokens;

    // Extract modelContextWindow from modelUsage (first model's contextWindow)
    let model_context_window = model_usage
        .and_then(|mu| mu.as_object())
        .and_then(|obj| obj.values().next())
        .and_then(|model_data| model_data.get("contextWindow"))
        .and_then(|cw| cw.as_i64());

    Some(json!({
        "total": {
            "totalTokens": total_tokens,
            "inputTokens": input_tokens,
            "cachedInputTokens": cached_input_tokens,
            "outputTokens": output_tokens,
            "reasoningOutputTokens": reasoning_output_tokens,
        },
        "last": {
            "totalTokens": total_tokens,
            "inputTokens": input_tokens,
            "cachedInputTokens": cached_input_tokens,
            "outputTokens": output_tokens,
            "reasoningOutputTokens": reasoning_output_tokens,
        },
        "modelContextWindow": model_context_window
    }))
}

fn usage_number(map: &Map<String, Value>, keys: &[&str]) -> i64 {
    for key in keys {
        if let Some(value) = map.get(*key) {
            if let Some(number) = value.as_i64() {
                return number;
            }
            if let Some(text) = value.as_str() {
                if let Ok(number) = text.parse::<i64>() {
                    return number;
                }
            }
        }
    }
    0
}


async fn build_review_prompt(
    workspace_id: &str,
    target: &Value,
    state: &State<'_, AppState>,
) -> Result<String, String> {
    let target_type = target.get("type").and_then(|v| v.as_str()).unwrap_or("");
    if target_type == "custom" {
        let instructions = target
            .get("instructions")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if instructions.trim().is_empty() {
            return Err("Review instructions are empty".to_string());
        }
        return Ok(instructions.to_string());
    }

    let diff = crate::git::get_workspace_diff(workspace_id, state).await?;
    if diff.trim().is_empty() {
        return Err("No changes to review".to_string());
    }

    let label = match target_type {
        "baseBranch" => target
            .get("branch")
            .and_then(|v| v.as_str())
            .map(|branch| format!("Review changes against base branch {branch}.")),
        "commit" => target
            .get("sha")
            .and_then(|v| v.as_str())
            .map(|sha| format!("Review commit {sha}.")),
        _ => None,
    };

    let mut prompt = "Review the following changes and provide concise feedback:\n\n".to_string();
    if let Some(label) = label {
        prompt.push_str(&label);
        prompt.push_str("\n\n");
    }
    prompt.push_str(&diff);
    Ok(prompt)
}

fn resolve_permissions_path(
    entry: &WorkspaceEntry,
    parent_path: Option<&str>,
) -> Result<PathBuf, String> {
    if let Some(project_home) = resolve_workspace_claude_home(entry, parent_path) {
        let path = project_home.join("settings.local.json");
        return Ok(path);
    }
    let fallback = PathBuf::from(&entry.path).join(".claude");
    if std::fs::create_dir_all(&fallback).is_ok() {
        return Ok(fallback.join("settings.local.json"));
    }
    resolve_default_claude_home()
        .map(|home| home.join("settings.json"))
        .ok_or_else(|| "Unable to resolve Claude settings path".to_string())
}

fn read_settings_json(path: &Path) -> Result<Map<String, Value>, String> {
    if !path.exists() {
        return Ok(Map::new());
    }
    let contents = std::fs::read_to_string(path).map_err(|err| err.to_string())?;
    let value: Value = serde_json::from_str(&contents).map_err(|err| err.to_string())?;
    match value {
        Value::Object(map) => Ok(map),
        _ => Ok(Map::new()),
    }
}

fn write_settings_json(path: &Path, settings: &Map<String, Value>) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    let contents = serde_json::to_string_pretty(settings).map_err(|err| err.to_string())?;
    std::fs::write(path, contents).map_err(|err| err.to_string())
}

fn archived_threads_path(state: &State<'_, AppState>) -> Result<PathBuf, String> {
    state
        .settings_path
        .parent()
        .map(|path| path.join("archived_threads.json"))
        .ok_or_else(|| "Unable to resolve app data dir.".to_string())
}

fn read_archived_threads(path: &Path) -> Result<HashMap<String, Vec<String>>, String> {
    if !path.exists() {
        return Ok(HashMap::new());
    }
    let contents = std::fs::read_to_string(path).map_err(|err| err.to_string())?;
    serde_json::from_str(&contents).map_err(|err| err.to_string())
}

fn write_archived_threads(
    path: &Path,
    data: &HashMap<String, Vec<String>>,
) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    let contents = serde_json::to_string_pretty(data).map_err(|err| err.to_string())?;
    std::fs::write(path, contents).map_err(|err| err.to_string())
}
