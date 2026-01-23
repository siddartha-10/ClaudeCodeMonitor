use chrono::DateTime;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tauri::{AppHandle, State};
use tokio::io::{AsyncBufReadExt, BufReader as AsyncBufReader};
use tokio::sync::{watch, Mutex};
use tokio::time::{interval, sleep, timeout};
use uuid::Uuid;

pub(crate) use crate::backend::claude_cli::WorkspaceSession;
use crate::backend::claude_cli::{
    build_claude_command_with_bin, build_claude_path_env, check_claude_installation,
    spawn_workspace_session as spawn_workspace_session_inner,
};
use crate::backend::events::{AppServerEvent, EventSink};
use crate::codex_home::{resolve_default_claude_home, resolve_workspace_claude_home};
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
    let archived_ids = archived_threads_path(&state)
        .ok()
        .and_then(|path| read_archived_threads(&path).ok())
        .and_then(|archived| archived.get(&workspace_id).cloned())
        .unwrap_or_default();
    let archived_set = archived_ids
        .into_iter()
        .collect::<std::collections::HashSet<_>>();
    let mut sorted = entries
        .into_iter()
        .filter(|entry| !archived_set.contains(&entry.session_id))
        .collect::<Vec<_>>();
    sorted.sort_by(|a, b| session_sort_key(b).cmp(&session_sort_key(a)));

    let offset = cursor
        .as_ref()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    let limit = limit.unwrap_or(20).clamp(1, 50) as usize;
    let end = (offset + limit).min(sorted.len());
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

    run_claude_turn(
        &workspace_id,
        session,
        app,
        &thread_id,
        prompt,
        model,
        access_mode,
        effort,
    )
    .await
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
    run_claude_turn(
        &workspace_id,
        session,
        app,
        &thread_id,
        prompt,
        None,
        Some("read-only".to_string()),
        None,
    )
    .await
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
pub(crate) async fn account_rate_limits(
    workspace_id: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Value, String> {
    if remote_backend::is_remote_mode(&*state).await {
        return remote_backend::call_remote(
            &*state,
            app,
            "account_rate_limits",
            json!({ "workspaceId": workspace_id }),
        )
        .await;
    }

    Ok(json!({ "rateLimits": {} }))
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
    request_id: u64,
    _result: Value,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<(), String> {
    if remote_backend::is_remote_mode(&*state).await {
        remote_backend::call_remote(
            &*state,
            app,
            "respond_to_server_request",
            json!({ "workspaceId": workspace_id, "requestId": request_id }),
        )
        .await?;
    }
    Ok(())
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

async fn run_claude_turn(
    workspace_id: &str,
    session: Arc<WorkspaceSession>,
    app: AppHandle,
    thread_id: &str,
    prompt: String,
    model: Option<String>,
    access_mode: Option<String>,
    _effort: Option<String>,
) -> Result<Value, String> {
    let turn_id = Uuid::new_v4().to_string();
    let mut item_id = format!("{turn_id}-assistant");
    let event_sink = TauriEventSink::new(app.clone());

    emit_event(
        &event_sink,
        workspace_id,
        "turn/started",
        json!({
            "threadId": thread_id,
            "turn": { "id": turn_id, "threadId": thread_id },
        }),
    );
    emit_event(
        &event_sink,
        workspace_id,
        "item/started",
        json!({
            "threadId": thread_id,
            "item": { "id": item_id, "type": "agentMessage", "text": "" },
        }),
    );

    let mut command = build_claude_command_with_bin(session.claude_bin.clone());
    command.current_dir(&session.entry.path);
    command.arg("-p").arg(prompt);
    command.arg("--output-format").arg("stream-json");
    command.arg("--verbose");
    command.arg("--include-partial-messages");
    command.arg("--max-thinking-tokens").arg("31999");
    command.arg("--add-dir").arg(&session.entry.path);

    if let Some(model) = model {
        if !model.trim().is_empty() {
            command.arg("--model").arg(model);
        }
    }

    let access_mode = access_mode.unwrap_or_else(|| "current".to_string());
    if access_mode == "full-access" {
        command.arg("--permission-mode").arg("bypassPermissions");
    } else if access_mode == "read-only" {
        command.arg("--allowed-tools").arg("Read,Glob,Grep");
    }

    if session_exists(&session.entry, thread_id) {
        command.arg("--resume").arg(thread_id);
    } else {
        command.arg("--session-id").arg(thread_id);
    }

    command.stdout(std::process::Stdio::piped());
    command.stderr(std::process::Stdio::piped());

    let child = command.spawn().map_err(|err| err.to_string())?;
    let child = Arc::new(Mutex::new(child));
    session
        .track_turn(thread_id.to_string(), turn_id.clone(), child.clone())
        .await;

    let (stdout, stderr) = {
        let mut guard = child.lock().await;
        let stdout = guard.stdout.take().ok_or("missing stdout")?;
        let stderr = guard.stderr.take().ok_or("missing stderr")?;
        (stdout, stderr)
    };

    let stderr_handle = tokio::spawn(async move {
        let mut output = String::new();
        let mut reader = AsyncBufReader::new(stderr).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            output.push_str(&line);
            output.push('\n');
        }
        output
    });

    let mut reader = AsyncBufReader::new(stdout).lines();
    let mut full_text = String::new();
    let mut last_text = String::new();
    let mut last_usage: Option<Value> = None;
    let mut last_model_usage: Option<Value> = None;
    let mut last_model: Option<String> = None;
    let mut tool_names: HashMap<String, String> = HashMap::new();
    let mut tool_inputs: HashMap<String, Value> = HashMap::new();
    let mut tool_counter: usize = 0;
    let mut thinking_counter: usize = 0;
    let mut subagent_tool_ids: HashSet<String> = HashSet::new();
    let mut permission_denials_emitted = false;
    while let Ok(Some(line)) = reader.next_line().await {
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(_) => continue,
        };
        // Skip subagent events - they have parent_tool_use_id set
        if value.get("parent_tool_use_id").and_then(|v| v.as_str()).is_some() {
            continue;
        }
        let event_type = value.get("type").and_then(|v| v.as_str()).unwrap_or("");
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
                                        workspace_id,
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
                        let is_subagent_tool = is_subagent_task(&tool_name, &tool_input);
                        if !tool_id.is_empty() {
                            tool_names.insert(tool_id.to_string(), tool_name.clone());
                            tool_inputs.insert(tool_id.to_string(), tool_input.clone());
                            if is_subagent_tool {
                                subagent_tool_ids.insert(tool_id.to_string());
                            }
                        }
                        let item_id = if tool_id.is_empty() {
                            tool_counter += 1;
                            format!("{turn_id}-tool-{tool_counter}")
                        } else {
                            tool_id.to_string()
                        };
                        emit_event(
                            &event_sink,
                            workspace_id,
                            "item/started",
                            json!({
                                "threadId": thread_id,
                                "item": {
                                    "id": item_id,
                                    "type": "commandExecution",
                                    "command": [tool_name],
                                    "status": "running",
                                    "toolInput": tool_input,
                                }
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
                            workspace_id,
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
                    for entry in content {
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
                        let command = tool_names
                            .get(tool_use_id)
                            .cloned()
                            .unwrap_or_else(|| "Tool".to_string());
                        let tool_input = tool_inputs
                            .get(tool_use_id)
                            .cloned()
                            .unwrap_or(Value::Null);
                        output = collapse_subagent_output(output, &command, &tool_input, &value);
                        let item_id = if tool_use_id.is_empty() {
                            tool_counter += 1;
                            format!("{turn_id}-tool-result-{tool_counter}")
                        } else {
                            tool_use_id.to_string()
                        };
                        emit_event(
                            &event_sink,
                            workspace_id,
                            "item/completed",
                            json!({
                                "threadId": thread_id,
                                "item": {
                                    "id": item_id,
                                    "type": "commandExecution",
                                    "command": [command],
                                    "status": "completed",
                                    "aggregatedOutput": output,
                                    "toolInput": tool_input,
                                }
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
            if !permission_denials_emitted {
                let denials = value
                    .get("permission_denials")
                    .or_else(|| value.get("permissionDenials"))
                    .and_then(|item| item.as_array())
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(|entry| {
                                let tool_name = entry
                                    .get("tool_name")
                                    .or_else(|| entry.get("toolName"))
                                    .and_then(|item| item.as_str())?
                                    .trim()
                                    .to_string();
                                if tool_name.is_empty() {
                                    return None;
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
                                Some(json!({
                                    "toolName": tool_name,
                                    "toolUseId": tool_use_id,
                                    "toolInput": tool_input,
                                }))
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                if !denials.is_empty() {
                    permission_denials_emitted = true;
                    emit_event(
                        &event_sink,
                        workspace_id,
                        "turn/permissionDenied",
                        json!({
                            "threadId": thread_id,
                            "turnId": turn_id,
                            "permissionDenials": denials,
                        }),
                    );
                }
            }
        }
    }

    let status = {
        let mut guard = child.lock().await;
        guard.wait().await.map_err(|err| err.to_string())?
    };

    session.clear_turn(thread_id, &turn_id).await;

    let stderr_output = stderr_handle
        .await
        .map_err(|err| err.to_string())?;

    if !status.success() {
        emit_event(
            &event_sink,
            workspace_id,
            "error",
            json!({
                "threadId": thread_id,
                "turnId": turn_id,
                "error": { "message": stderr_output.trim() },
                "willRetry": false,
            }),
        );
        return Err(if stderr_output.trim().is_empty() {
            "Claude CLI failed to run".to_string()
        } else {
            stderr_output
        });
    }

    if let Some(usage) = last_usage.and_then(|u| format_token_usage(u, last_model_usage.as_ref())) {
        emit_event(
            &event_sink,
            workspace_id,
            "thread/tokenUsage/updated",
            json!({
                "threadId": thread_id,
                "tokenUsage": usage,
            }),
        );
    }

    emit_event(
        &event_sink,
        workspace_id,
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
        workspace_id,
        "turn/completed",
        json!({
            "threadId": thread_id,
            "turn": { "id": turn_id, "threadId": thread_id },
        }),
    );

    Ok(json!({
        "result": {
            "turn": { "id": turn_id, "threadId": thread_id }
        }
    }))
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

fn emit_event(event_sink: &TauriEventSink, workspace_id: &str, method: &str, params: Value) {
    event_sink.emit_app_server_event(AppServerEvent {
        workspace_id: workspace_id.to_string(),
        message: json!({
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
                let item = json!({
                    "id": id,
                    "type": "commandExecution",
                    "command": [command],
                    "status": "completed",
                    "aggregatedOutput": output,
                    "toolInput": tool_input,
                });
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
                        let item = json!({
                            "id": id,
                            "type": "commandExecution",
                            "command": [tool_name],
                            "status": "running",
                            "toolInput": tool_input,
                        });
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
    let mut entries = resolve_sessions_index_path(entry)
        .and_then(|index_path| fs::read_to_string(index_path).ok())
        .and_then(|data| serde_json::from_str::<Value>(&data).ok())
        .map(|value| parse_sessions_value(&value))
        .unwrap_or_default();

    let scanned = scan_project_sessions(entry);
    if entries.is_empty() {
        return scanned;
    }

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
    entries
        .iter()
        .filter_map(|entry| serde_json::from_value(entry.clone()).ok())
        .collect()
}

fn scan_project_sessions(entry: &WorkspaceEntry) -> Vec<ClaudeSessionEntry> {
    let Some(project_dir) = resolve_project_dir(entry) else {
        return Vec::new();
    };
    let mut entries = Vec::new();
    let dir_entries = match fs::read_dir(project_dir) {
        Ok(dir_entries) => dir_entries,
        Err(_) => return Vec::new(),
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
        Err(_) => return (None, None, None),
    };
    let reader = BufReader::new(file);
    let mut first_prompt: Option<String> = None;
    let mut message_count: i64 = 0;
    let mut git_branch: Option<String> = None;
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
                    "item": {
                        "id": item_id,
                        "type": "commandExecution",
                        "command": [command],
                        "status": "completed",
                        "aggregatedOutput": output,
                        "toolInput": tool_input,
                    }
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
                        "item": {
                            "id": item_id,
                            "type": "commandExecution",
                            "command": [tool_name],
                            "status": "running",
                            "toolInput": tool_input,
                        }
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
