#[allow(dead_code)]
#[path = "../backend/mod.rs"]
mod backend;
#[path = "../codex_home.rs"]
mod codex_home;
#[path = "../codex_config.rs"]
mod codex_config;
#[path = "../storage.rs"]
mod storage;
#[allow(dead_code)]
#[path = "../types.rs"]
mod types;

use chrono::DateTime;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::env;
use std::fs::{self, File};
use std::io::{BufRead, BufReader as StdBufReader};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use ignore::WalkBuilder;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::process::Command;
use tokio::sync::{broadcast, mpsc, Mutex};
use uuid::Uuid;

use backend::claude_cli::{build_claude_command_with_bin, spawn_workspace_session, WorkspaceSession};
use backend::events::{AppServerEvent, EventSink, TerminalOutput};
use storage::{read_settings, read_workspaces, write_settings, write_workspaces};
use types::{
    AppSettings, WorkspaceEntry, WorkspaceInfo, WorkspaceKind, WorkspaceSettings, WorktreeInfo,
};

const DEFAULT_LISTEN_ADDR: &str = "127.0.0.1:4732";

#[derive(Clone)]
struct DaemonEventSink {
    tx: broadcast::Sender<DaemonEvent>,
}

#[derive(Clone)]
enum DaemonEvent {
    AppServer(AppServerEvent),
    #[allow(dead_code)]
    TerminalOutput(TerminalOutput),
}

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
    is_sidechain: Option<bool>,
}

impl EventSink for DaemonEventSink {
    fn emit_app_server_event(&self, event: AppServerEvent) {
        let _ = self.tx.send(DaemonEvent::AppServer(event));
    }

    fn emit_terminal_output(&self, event: TerminalOutput) {
        let _ = self.tx.send(DaemonEvent::TerminalOutput(event));
    }
}

struct DaemonConfig {
    listen: SocketAddr,
    token: Option<String>,
    data_dir: PathBuf,
}

struct DaemonState {
    data_dir: PathBuf,
    workspaces: Mutex<HashMap<String, WorkspaceEntry>>,
    sessions: Mutex<HashMap<String, Arc<WorkspaceSession>>>,
    storage_path: PathBuf,
    settings_path: PathBuf,
    app_settings: Mutex<AppSettings>,
    event_sink: DaemonEventSink,
}

impl DaemonState {
    fn load(config: &DaemonConfig, event_sink: DaemonEventSink) -> Self {
        let storage_path = config.data_dir.join("workspaces.json");
        let settings_path = config.data_dir.join("settings.json");
        let workspaces = read_workspaces(&storage_path).unwrap_or_default();
        let app_settings = read_settings(&settings_path).unwrap_or_default();
        Self {
            data_dir: config.data_dir.clone(),
            workspaces: Mutex::new(workspaces),
            sessions: Mutex::new(HashMap::new()),
            storage_path,
            settings_path,
            app_settings: Mutex::new(app_settings),
            event_sink,
        }
    }

    async fn kill_session(&self, workspace_id: &str) {
        let session = {
            let mut sessions = self.sessions.lock().await;
            sessions.remove(workspace_id)
        };

        let Some(session) = session else {
            return;
        };

        let mut active_turns = session.active_turns.lock().await;
        let children = active_turns
            .drain()
            .map(|(_, active_turn)| active_turn.child)
            .collect::<Vec<_>>();
        drop(active_turns);
        for child in children {
            let mut guard = child.lock().await;
            let _ = guard.kill().await;
        }
    }

    async fn list_workspaces(&self) -> Vec<WorkspaceInfo> {
        let workspaces = self.workspaces.lock().await;
        let sessions = self.sessions.lock().await;
        let mut result = Vec::new();
        for entry in workspaces.values() {
            result.push(WorkspaceInfo {
                id: entry.id.clone(),
                name: entry.name.clone(),
                path: entry.path.clone(),
                connected: sessions.contains_key(&entry.id),
                claude_bin: entry.claude_bin.clone(),
                kind: entry.kind.clone(),
                parent_id: entry.parent_id.clone(),
                worktree: entry.worktree.clone(),
                settings: entry.settings.clone(),
            });
        }
        sort_workspaces(&mut result);
        result
    }

    async fn add_workspace(
        &self,
        path: String,
        claude_bin: Option<String>,
        _client_version: String,
    ) -> Result<WorkspaceInfo, String> {
        let name = PathBuf::from(&path)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("Workspace")
            .to_string();

        let entry = WorkspaceEntry {
            id: Uuid::new_v4().to_string(),
            name: name.clone(),
            path: path.clone(),
            claude_bin,
            kind: WorkspaceKind::Main,
            parent_id: None,
            worktree: None,
            settings: WorkspaceSettings::default(),
        };

        let default_bin = {
            let settings = self.app_settings.lock().await;
            settings.claude_bin.clone()
        };
        let session = spawn_workspace_session(entry.clone(), default_bin).await?;

        let list = {
            let mut workspaces = self.workspaces.lock().await;
            workspaces.insert(entry.id.clone(), entry.clone());
            workspaces.values().cloned().collect::<Vec<_>>()
        };
        write_workspaces(&self.storage_path, &list)?;

        self.sessions.lock().await.insert(entry.id.clone(), session);
        emit_event(&self.event_sink, &entry.id, "claude/connected", json!({}));

        Ok(WorkspaceInfo {
            id: entry.id,
            name: entry.name,
            path: entry.path,
            connected: true,
            claude_bin: entry.claude_bin,
            kind: entry.kind,
            parent_id: entry.parent_id,
            worktree: entry.worktree,
            settings: entry.settings,
        })
    }

    async fn add_worktree(
        &self,
        parent_id: String,
        branch: String,
        _client_version: String,
    ) -> Result<WorkspaceInfo, String> {
        let branch = branch.trim().to_string();
        if branch.trim().is_empty() {
            return Err("Branch name is required.".to_string());
        }

        let parent_entry = {
            let workspaces = self.workspaces.lock().await;
            workspaces
                .get(&parent_id)
                .cloned()
                .ok_or("parent workspace not found")?
        };

        if parent_entry.kind.is_worktree() {
            return Err("Cannot create a worktree from another worktree.".to_string());
        }

        let worktree_root = self.data_dir.join("worktrees").join(&parent_entry.id);
        std::fs::create_dir_all(&worktree_root)
            .map_err(|e| format!("Failed to create worktree directory: {e}"))?;

        let safe_name = sanitize_worktree_name(&branch);
        let worktree_path = unique_worktree_path(&worktree_root, &safe_name)?;
        let worktree_path_string = worktree_path.to_string_lossy().to_string();

        let repo_path = PathBuf::from(&parent_entry.path);
        let branch_exists = git_branch_exists(&repo_path, &branch).await?;
        if branch_exists {
            run_git_command(
                &repo_path,
                &["worktree", "add", &worktree_path_string, &branch],
            )
            .await?;
        } else if let Some(remote_ref) = git_find_remote_tracking_branch(&repo_path, &branch).await? {
            run_git_command(
                &repo_path,
                &["worktree", "add", "-b", &branch, &worktree_path_string, &remote_ref],
            )
            .await?;
        } else {
            run_git_command(
                &repo_path,
                &["worktree", "add", "-b", &branch, &worktree_path_string],
            )
            .await?;
        }

        let entry = WorkspaceEntry {
            id: Uuid::new_v4().to_string(),
            name: branch.to_string(),
            path: worktree_path_string,
            claude_bin: parent_entry.claude_bin.clone(),
            kind: WorkspaceKind::Worktree,
            parent_id: Some(parent_entry.id.clone()),
            worktree: Some(WorktreeInfo {
                branch: branch.to_string(),
            }),
            settings: WorkspaceSettings::default(),
        };

        let default_bin = {
            let settings = self.app_settings.lock().await;
            settings.claude_bin.clone()
        };
        let session = spawn_workspace_session(entry.clone(), default_bin).await?;

        let list = {
            let mut workspaces = self.workspaces.lock().await;
            workspaces.insert(entry.id.clone(), entry.clone());
            workspaces.values().cloned().collect::<Vec<_>>()
        };
        write_workspaces(&self.storage_path, &list)?;

        self.sessions.lock().await.insert(entry.id.clone(), session);
        emit_event(&self.event_sink, &entry.id, "claude/connected", json!({}));

        Ok(WorkspaceInfo {
            id: entry.id,
            name: entry.name,
            path: entry.path,
            connected: true,
            claude_bin: entry.claude_bin,
            kind: entry.kind,
            parent_id: entry.parent_id,
            worktree: entry.worktree,
            settings: entry.settings,
        })
    }

    async fn remove_workspace(&self, id: String) -> Result<(), String> {
        let (entry, child_worktrees) = {
            let workspaces = self.workspaces.lock().await;
            let entry = workspaces.get(&id).cloned().ok_or("workspace not found")?;
            if entry.kind.is_worktree() {
                return Err("Use remove_worktree for worktree agents.".to_string());
            }
            let children = workspaces
                .values()
                .filter(|workspace| workspace.parent_id.as_deref() == Some(&id))
                .cloned()
                .collect::<Vec<_>>();
            (entry, children)
        };

        let repo_path = PathBuf::from(&entry.path);
        let mut removed_child_ids = Vec::new();
        let mut failures = Vec::new();

        for child in &child_worktrees {
            let child_path = PathBuf::from(&child.path);
            if child_path.exists() {
                if let Err(err) = run_git_command(
                    &repo_path,
                    &["worktree", "remove", "--force", &child.path],
                )
                .await
                {
                    failures.push((child.id.clone(), err));
                    continue;
                }
            }

            self.kill_session(&child.id).await;
            removed_child_ids.push(child.id.clone());
        }

        let _ = run_git_command(&repo_path, &["worktree", "prune", "--expire", "now"]).await;

        let mut ids_to_remove = removed_child_ids;
        if failures.is_empty() {
            self.kill_session(&id).await;
            ids_to_remove.push(id.clone());
        }

        if !ids_to_remove.is_empty() {
            let list = {
                let mut workspaces = self.workspaces.lock().await;
                for workspace_id in ids_to_remove {
                    workspaces.remove(&workspace_id);
                }
                workspaces.values().cloned().collect::<Vec<_>>()
            };
            write_workspaces(&self.storage_path, &list)?;
        }

        if failures.is_empty() {
            return Ok(());
        }

        let mut message =
            "Failed to remove one or more worktrees; parent workspace was not removed.".to_string();
        for (child_id, error) in failures {
            message.push_str(&format!("\n- {child_id}: {error}"));
        }
        Err(message)
    }

    async fn remove_worktree(&self, id: String) -> Result<(), String> {
        let (entry, parent) = {
            let workspaces = self.workspaces.lock().await;
            let entry = workspaces.get(&id).cloned().ok_or("workspace not found")?;
            if !entry.kind.is_worktree() {
                return Err("Not a worktree workspace.".to_string());
            }
            let parent_id = entry.parent_id.clone().ok_or("worktree parent not found")?;
            let parent = workspaces
                .get(&parent_id)
                .cloned()
                .ok_or("worktree parent not found")?;
            (entry, parent)
        };

        let parent_path = PathBuf::from(&parent.path);
        let entry_path = PathBuf::from(&entry.path);
        if entry_path.exists() {
            run_git_command(
                &parent_path,
                &["worktree", "remove", "--force", &entry.path],
            )
            .await?;
        }
        let _ = run_git_command(&parent_path, &["worktree", "prune", "--expire", "now"]).await;

        self.kill_session(&entry.id).await;

        let list = {
            let mut workspaces = self.workspaces.lock().await;
            workspaces.remove(&entry.id);
            workspaces.values().cloned().collect::<Vec<_>>()
        };
        write_workspaces(&self.storage_path, &list)?;

        Ok(())
    }

    async fn rename_worktree(
        &self,
        id: String,
        branch: String,
        _client_version: String,
    ) -> Result<WorkspaceInfo, String> {
        let trimmed = branch.trim();
        if trimmed.is_empty() {
            return Err("Branch name is required.".to_string());
        }

        let (entry, parent) = {
            let workspaces = self.workspaces.lock().await;
            let entry = workspaces.get(&id).cloned().ok_or("workspace not found")?;
            if !entry.kind.is_worktree() {
                return Err("Not a worktree workspace.".to_string());
            }
            let parent_id = entry.parent_id.clone().ok_or("worktree parent not found")?;
            let parent = workspaces
                .get(&parent_id)
                .cloned()
                .ok_or("worktree parent not found")?;
            (entry, parent)
        };

        let old_branch = entry
            .worktree
            .as_ref()
            .map(|worktree| worktree.branch.clone())
            .ok_or("worktree metadata missing")?;
        if old_branch == trimmed {
            return Err("Branch name is unchanged.".to_string());
        }

        let parent_root = PathBuf::from(&parent.path);

        let (final_branch, _was_suffixed) =
            unique_branch_name(&parent_root, trimmed, None).await?;
        if final_branch == old_branch {
            return Err("Branch name is unchanged.".to_string());
        }

        run_git_command(
            &parent_root,
            &["branch", "-m", &old_branch, &final_branch],
        )
        .await?;

        let worktree_root = self.data_dir.join("worktrees").join(&parent.id);
        std::fs::create_dir_all(&worktree_root)
            .map_err(|e| format!("Failed to create worktree directory: {e}"))?;

        let safe_name = sanitize_worktree_name(&final_branch);
        let current_path = PathBuf::from(&entry.path);
        let next_path =
            unique_worktree_path_for_rename(&worktree_root, &safe_name, &current_path)?;
        let next_path_string = next_path.to_string_lossy().to_string();
        if next_path_string != entry.path {
            if let Err(error) = run_git_command(
                &parent_root,
                &["worktree", "move", &entry.path, &next_path_string],
            )
            .await
            {
                let _ = run_git_command(
                    &parent_root,
                    &["branch", "-m", &final_branch, &old_branch],
                )
                .await;
                return Err(error);
            }
        }

        let (entry_snapshot, list) = {
            let mut workspaces = self.workspaces.lock().await;
            let entry = match workspaces.get_mut(&id) {
                Some(entry) => entry,
                None => return Err("workspace not found".to_string()),
            };
            entry.name = final_branch.clone();
            entry.path = next_path_string.clone();
            match entry.worktree.as_mut() {
                Some(worktree) => {
                    worktree.branch = final_branch.clone();
                }
                None => {
                    entry.worktree = Some(WorktreeInfo {
                        branch: final_branch.clone(),
                    });
                }
            }
            let snapshot = entry.clone();
            let list: Vec<_> = workspaces.values().cloned().collect();
            (snapshot, list)
        };
        write_workspaces(&self.storage_path, &list)?;

        let was_connected = self.sessions.lock().await.contains_key(&entry_snapshot.id);
        if was_connected {
            self.kill_session(&entry_snapshot.id).await;
            let default_bin = {
                let settings = self.app_settings.lock().await;
                settings.claude_bin.clone()
            };
            match spawn_workspace_session(entry_snapshot.clone(), default_bin).await {
                Ok(session) => {
                    self.sessions
                        .lock()
                        .await
                        .insert(entry_snapshot.id.clone(), session);
                }
                Err(error) => {
                    eprintln!(
                        "rename_worktree: respawn failed for {} after rename: {error}",
                        entry_snapshot.id
                    );
                }
            }
        }

        let connected = self.sessions.lock().await.contains_key(&entry_snapshot.id);
        Ok(WorkspaceInfo {
            id: entry_snapshot.id,
            name: entry_snapshot.name,
            path: entry_snapshot.path,
            connected,
            claude_bin: entry_snapshot.claude_bin,
            kind: entry_snapshot.kind,
            parent_id: entry_snapshot.parent_id,
            worktree: entry_snapshot.worktree,
            settings: entry_snapshot.settings,
        })
    }

    async fn rename_worktree_upstream(
        &self,
        id: String,
        old_branch: String,
        new_branch: String,
    ) -> Result<(), String> {
        let old_branch = old_branch.trim();
        let new_branch = new_branch.trim();
        if old_branch.is_empty() || new_branch.is_empty() {
            return Err("Branch name is required.".to_string());
        }
        if old_branch == new_branch {
            return Err("Branch name is unchanged.".to_string());
        }

        let (_entry, parent) = {
            let workspaces = self.workspaces.lock().await;
            let entry = workspaces.get(&id).cloned().ok_or("workspace not found")?;
            if !entry.kind.is_worktree() {
                return Err("Not a worktree workspace.".to_string());
            }
            let parent_id = entry.parent_id.clone().ok_or("worktree parent not found")?;
            let parent = workspaces
                .get(&parent_id)
                .cloned()
                .ok_or("worktree parent not found")?;
            (entry, parent)
        };

        let parent_root = PathBuf::from(&parent.path);
        if !git_branch_exists(&parent_root, new_branch).await? {
            return Err("Local branch not found.".to_string());
        }

        let remote_for_old = git_find_remote_for_branch(&parent_root, old_branch).await?;
        let remote_name = match remote_for_old.as_ref() {
            Some(remote) => remote.clone(),
            None => {
                if git_remote_exists(&parent_root, "origin").await? {
                    "origin".to_string()
                } else {
                    return Err("No git remote configured for this worktree.".to_string());
                }
            }
        };

        if git_remote_branch_exists_live(&parent_root, &remote_name, new_branch).await? {
            return Err("Remote branch already exists.".to_string());
        }

        if remote_for_old.is_some() {
            run_git_command(
                &parent_root,
                &[
                    "push",
                    &remote_name,
                    &format!("{new_branch}:{new_branch}"),
                ],
            )
            .await?;
            run_git_command(
                &parent_root,
                &["push", &remote_name, &format!(":{old_branch}")],
            )
            .await?;
        } else {
            run_git_command(&parent_root, &["push", &remote_name, new_branch]).await?;
        }

        run_git_command(
            &parent_root,
            &[
                "branch",
                "--set-upstream-to",
                &format!("{remote_name}/{new_branch}"),
                new_branch,
            ],
        )
        .await?;

        Ok(())
    }

    async fn update_workspace_settings(
        &self,
        id: String,
        settings: WorkspaceSettings,
    ) -> Result<WorkspaceInfo, String> {
        let (entry_snapshot, list) = {
            let mut workspaces = self.workspaces.lock().await;
            let entry_snapshot = match workspaces.get_mut(&id) {
                Some(entry) => {
                    entry.settings = settings.clone();
                    entry.clone()
                }
                None => return Err("workspace not found".to_string()),
            };
            let list: Vec<_> = workspaces.values().cloned().collect();
            (entry_snapshot, list)
        };
        write_workspaces(&self.storage_path, &list)?;

        let connected = self.sessions.lock().await.contains_key(&id);
        Ok(WorkspaceInfo {
            id: entry_snapshot.id,
            name: entry_snapshot.name,
            path: entry_snapshot.path,
            connected,
            claude_bin: entry_snapshot.claude_bin,
            kind: entry_snapshot.kind,
            parent_id: entry_snapshot.parent_id,
            worktree: entry_snapshot.worktree,
            settings: entry_snapshot.settings,
        })
    }

    async fn update_workspace_claude_bin(
        &self,
        id: String,
        claude_bin: Option<String>,
    ) -> Result<WorkspaceInfo, String> {
        let (entry_snapshot, list) = {
            let mut workspaces = self.workspaces.lock().await;
            let entry_snapshot = match workspaces.get_mut(&id) {
                Some(entry) => {
                    entry.claude_bin = claude_bin.clone();
                    entry.clone()
                }
                None => return Err("workspace not found".to_string()),
            };
            let list: Vec<_> = workspaces.values().cloned().collect();
            (entry_snapshot, list)
        };
        write_workspaces(&self.storage_path, &list)?;

        let connected = self.sessions.lock().await.contains_key(&id);
        Ok(WorkspaceInfo {
            id: entry_snapshot.id,
            name: entry_snapshot.name,
            path: entry_snapshot.path,
            connected,
            claude_bin: entry_snapshot.claude_bin,
            kind: entry_snapshot.kind,
            parent_id: entry_snapshot.parent_id,
            worktree: entry_snapshot.worktree,
            settings: entry_snapshot.settings,
        })
    }

    async fn connect_workspace(&self, id: String, _client_version: String) -> Result<(), String> {
        {
            let sessions = self.sessions.lock().await;
            if sessions.contains_key(&id) {
                return Ok(());
            }
        }

        let entry = {
            let workspaces = self.workspaces.lock().await;
            workspaces
                .get(&id)
                .cloned()
                .ok_or("workspace not found")?
        };

        let default_bin = {
            let settings = self.app_settings.lock().await;
            settings.claude_bin.clone()
        };
        let session = spawn_workspace_session(entry.clone(), default_bin).await?;

        self.sessions.lock().await.insert(id, session);
        emit_event(&self.event_sink, &entry.id, "claude/connected", json!({}));
        Ok(())
    }

    async fn update_app_settings(&self, settings: AppSettings) -> Result<AppSettings, String> {
        let _ = codex_config::write_collab_enabled(settings.experimental_collab_enabled);
        let _ = codex_config::write_steer_enabled(settings.experimental_steer_enabled);
        let _ = codex_config::write_unified_exec_enabled(settings.experimental_unified_exec_enabled);
        write_settings(&self.settings_path, &settings)?;
        let mut current = self.app_settings.lock().await;
        *current = settings.clone();
        Ok(settings)
    }

    async fn get_session(&self, workspace_id: &str) -> Result<Arc<WorkspaceSession>, String> {
        let sessions = self.sessions.lock().await;
        sessions
            .get(workspace_id)
            .cloned()
            .ok_or("workspace not connected".to_string())
    }

    async fn list_workspace_files(&self, workspace_id: String) -> Result<Vec<String>, String> {
        let entry = {
            let workspaces = self.workspaces.lock().await;
            workspaces
                .get(&workspace_id)
                .cloned()
                .ok_or("workspace not found")?
        };

        let root = PathBuf::from(entry.path);
        Ok(list_workspace_files_inner(&root, 20000))
    }

    async fn start_thread(&self, workspace_id: String) -> Result<Value, String> {
        let session = self.get_session(&workspace_id).await?;
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

    async fn resume_thread(&self, workspace_id: String, thread_id: String) -> Result<Value, String> {
        let entry = {
            let sessions = self.sessions.lock().await;
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

    async fn list_threads(
        &self,
        workspace_id: String,
        cursor: Option<String>,
        limit: Option<u32>,
    ) -> Result<Value, String> {
        let workspace_entry = {
            let sessions = self.sessions.lock().await;
            sessions
                .get(&workspace_id)
                .ok_or("workspace not connected")?
                .entry
                .clone()
        };

        let workspace_path = workspace_entry.path.clone();
        let entries = load_sessions_index(&workspace_entry);
        let archived_ids = read_archived_threads(&self.data_dir.join("archived_threads.json"))
            .ok()
            .and_then(|archived| archived.get(&workspace_id).cloned())
            .unwrap_or_default();
        let archived_set = archived_ids
            .into_iter()
            .collect::<std::collections::HashSet<_>>();
        let mut sorted = entries
            .into_iter()
            .filter(|entry| entry.is_sidechain.unwrap_or(false) == false)
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

    async fn archive_thread(&self, workspace_id: String, thread_id: String) -> Result<Value, String> {
        let path = self.data_dir.join("archived_threads.json");
        let mut archived = read_archived_threads(&path)?;
        let entry = archived.entry(workspace_id).or_default();
        if !entry.contains(&thread_id) {
            entry.push(thread_id);
            write_archived_threads(&path, &archived)?;
        }
        Ok(json!({ "ok": true }))
    }

    async fn send_user_message(
        &self,
        workspace_id: String,
        thread_id: String,
        text: String,
        model: Option<String>,
        effort: Option<String>,
        access_mode: Option<String>,
        images: Option<Vec<String>>,
        _collaboration_mode: Option<Value>,
    ) -> Result<Value, String> {
        let session = self.get_session(&workspace_id).await?;
        let prompt = build_prompt_with_images(text, images);
        if prompt.trim().is_empty() {
            return Err("empty user message".to_string());
        }

        run_claude_turn(
            &self.event_sink,
            &workspace_id,
            session,
            &thread_id,
            prompt,
            model,
            access_mode,
            effort,
        )
        .await
    }

    async fn turn_interrupt(
        &self,
        workspace_id: String,
        thread_id: String,
        turn_id: String,
    ) -> Result<Value, String> {
        let session = self.get_session(&workspace_id).await?;
        session.interrupt_turn(&thread_id, &turn_id).await?;
        Ok(json!({ "ok": true }))
    }

    async fn start_review(
        &self,
        workspace_id: String,
        thread_id: String,
        target: Value,
        delivery: Option<String>,
    ) -> Result<Value, String> {
        let session = self.get_session(&workspace_id).await?;
        let entry = {
            let workspaces = self.workspaces.lock().await;
            workspaces
                .get(&workspace_id)
                .ok_or("workspace not found")?
                .clone()
        };

        let prompt = build_review_prompt(&entry, &target).await?;
        let delivery = delivery.filter(|value| !value.trim().is_empty());
        let prompt = if let Some(delivery) = delivery {
            format!("{prompt}\n\nDelivery preference: {delivery}.")
        } else {
            prompt
        };

        run_claude_turn(
            &self.event_sink,
            &workspace_id,
            session,
            &thread_id,
            prompt,
            None,
            Some("read-only".to_string()),
            None,
        )
        .await
    }

    async fn model_list(&self, workspace_id: String) -> Result<Value, String> {
        let _ = workspace_id;
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

    async fn collaboration_mode_list(&self, workspace_id: String) -> Result<Value, String> {
        let _ = workspace_id;
        Ok(json!({ "data": [] }))
    }

    async fn account_rate_limits(&self, workspace_id: String) -> Result<Value, String> {
        let _ = workspace_id;
        Ok(json!({ "rateLimits": {} }))
    }

    async fn skills_list(&self, workspace_id: String) -> Result<Value, String> {
        let _ = workspace_id;
        Ok(json!({ "data": [] }))
    }

    async fn respond_to_server_request(
        &self,
        workspace_id: String,
        request_id: u64,
        result: Value,
    ) -> Result<Value, String> {
        let _ = (workspace_id, request_id, result);
        Ok(json!({ "ok": true }))
    }

    async fn remember_approval_rule(
        &self,
        workspace_id: String,
        command: Vec<String>,
    ) -> Result<Value, String> {
        let command = command
            .into_iter()
            .map(|item| item.trim().to_string())
            .filter(|item| !item.is_empty())
            .collect::<Vec<_>>();
        if command.is_empty() {
            return Err("empty command".to_string());
        }

        let (entry, parent_path) = {
            let workspaces = self.workspaces.lock().await;
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
        let rule = format_permission_rule(&command);
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
        if !allow.iter().any(|item| item.as_str() == Some(&rule)) {
            allow.push(Value::String(rule));
        }
        write_settings_json(&settings_path, &settings)?;

        Ok(json!({
            "ok": true,
            "rulesPath": settings_path,
        }))
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
    event_sink: &DaemonEventSink,
    workspace_id: &str,
    session: Arc<WorkspaceSession>,
    thread_id: &str,
    prompt: String,
    model: Option<String>,
    access_mode: Option<String>,
    _effort: Option<String>,
) -> Result<Value, String> {
    let turn_id = Uuid::new_v4().to_string();
    let mut item_id = format!("{turn_id}-assistant");

    emit_event(
        event_sink,
        workspace_id,
        "turn/started",
        json!({
            "threadId": thread_id,
            "turn": { "id": turn_id, "threadId": thread_id },
        }),
    );
    emit_event(
        event_sink,
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
        let mut reader = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            output.push_str(&line);
            output.push('\n');
        }
        output
    });

    let mut reader = BufReader::new(stdout).lines();
    let mut full_text = String::new();
    let mut last_text = String::new();
    let mut last_usage: Option<Value> = None;
    let mut tool_names: HashMap<String, String> = HashMap::new();
    let mut tool_inputs: HashMap<String, Value> = HashMap::new();
    let mut tool_counter: usize = 0;
    while let Ok(Some(line)) = reader.next_line().await {
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let event_type = value.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if event_type == "assistant" {
            if let Some(uuid) = value.get("uuid").and_then(|v| v.as_str()) {
                if !uuid.is_empty() {
                    item_id = uuid.to_string();
                }
            }
            if let Some(message) = value.get("message") {
                if let Some(content) = message.get("content").and_then(|v| v.as_array()) {
                    for entry in content {
                        if entry.get("type").and_then(|v| v.as_str()) != Some("tool_use") {
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
                        let item_id = if tool_id.is_empty() {
                            tool_counter += 1;
                            format!("{turn_id}-tool-{tool_counter}")
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
                            event_sink,
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
                        let item_id = if tool_use_id.is_empty() {
                            tool_counter += 1;
                            format!("{turn_id}-tool-result-{tool_counter}")
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
                }
            }
        } else if event_type == "result" {
            if let Some(usage) = value.get("usage") {
                last_usage = Some(usage.clone());
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
            event_sink,
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

    if let Some(usage) = last_usage.and_then(format_token_usage) {
        emit_event(
            event_sink,
            workspace_id,
            "thread/tokenUsage/updated",
            json!({
                "threadId": thread_id,
                "tokenUsage": usage,
            }),
        );
    }

    emit_event(
        event_sink,
        workspace_id,
        "item/completed",
        json!({
            "threadId": thread_id,
            "item": { "id": item_id, "type": "agentMessage", "text": full_text },
        }),
    );
    emit_event(
        event_sink,
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

fn emit_event(event_sink: &DaemonEventSink, workspace_id: &str, method: &str, params: Value) {
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
    let reader = StdBufReader::new(file);
    let mut items: Vec<Value> = Vec::new();
    let mut tool_names: HashMap<String, String> = HashMap::new();
    let mut tool_inputs: HashMap<String, Value> = HashMap::new();
    let mut tool_item_indices: HashMap<String, usize> = HashMap::new();
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
                        if !tool_id.is_empty() {
                            tool_names.insert(tool_id.to_string(), tool_name.clone());
                            tool_inputs.insert(tool_id.to_string(), tool_input.clone());
                        }
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
                items.push(json!({
                    "id": value.get("uuid").and_then(|v| v.as_str()).unwrap_or(thread_id),
                    "type": "agentMessage",
                    "text": text.trim(),
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
    let entries = resolve_sessions_index_path(entry)
        .and_then(|index_path| fs::read_to_string(index_path).ok())
        .and_then(|data| serde_json::from_str::<Value>(&data).ok())
        .map(|value| parse_sessions_value(&value))
        .unwrap_or_default();

    if !entries.is_empty() {
        return entries;
    }

    scan_project_sessions(entry)
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

fn scan_session_metadata(path: &Path) -> (Option<String>, Option<i64>, Option<String>) {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(_) => return (None, None, None),
    };
    let reader = StdBufReader::new(file);
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

fn list_subagent_threads(entry: &WorkspaceEntry, parent_id: &str, cwd: &str) -> Vec<Value> {
    let mut threads = Vec::new();
    let project_dir = match resolve_project_dir(entry) {
        Some(dir) => dir,
        None => return threads,
    };
    let subagent_dir = project_dir.join(parent_id).join("subagents");
    let dir_entries = match fs::read_dir(subagent_dir) {
        Ok(entries) => entries,
        Err(_) => return threads,
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
        let metadata = dir_entry.metadata().ok();
        let file_mtime = metadata
            .and_then(|meta| meta.modified().ok())
            .and_then(|timestamp| timestamp.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_millis() as i64)
            .unwrap_or(0);
        let (first_prompt, message_count, git_branch) = scan_session_metadata(&path);
        let preview = first_prompt.unwrap_or_else(|| format!("Subagent {agent_id}"));
        threads.push(json!({
            "id": subagent_thread_id(parent_id, &agent_id),
            "preview": preview,
            "messageCount": message_count.unwrap_or(0),
            "createdAt": file_mtime,
            "updatedAt": file_mtime,
            "cwd": cwd,
            "gitBranch": git_branch,
            "parentId": parent_id,
        }));
    }
    threads
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
        Value::Number(value) => value
            .as_i64()
            .map(|raw| if raw < 1_000_000_000_000 { raw * 1000 } else { raw }),
        _ => None,
    }
}

fn resolve_project_dir(entry: &WorkspaceEntry) -> Option<PathBuf> {
    let projects_root = codex_home::resolve_default_claude_home()?.join("projects");
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

fn has_user_message_content(content: &[Value]) -> bool {
    content.iter().any(|entry| {
        matches!(
            entry.get("type").and_then(|v| v.as_str()),
            Some("text" | "image" | "localImage" | "skill")
        )
    })
}

fn format_token_usage(raw: Value) -> Option<Value> {
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
        }
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

async fn build_review_prompt(entry: &WorkspaceEntry, target: &Value) -> Result<String, String> {
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

    let repo_root = resolve_git_root(entry).await?;
    let diff = collect_workspace_diff(&repo_root).await?;
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

async fn resolve_git_root(entry: &WorkspaceEntry) -> Result<PathBuf, String> {
    let root = PathBuf::from(&entry.path);
    let output = run_git_command(&root, &["rev-parse", "--show-toplevel"]).await?;
    let trimmed = output.trim();
    if trimmed.is_empty() {
        return Err("Unable to resolve git root".to_string());
    }
    Ok(PathBuf::from(trimmed))
}

async fn collect_workspace_diff(repo_root: &PathBuf) -> Result<String, String> {
    let staged = run_git_command(repo_root, &["diff", "--cached"]).await?;
    if !staged.trim().is_empty() {
        return Ok(staged);
    }
    let workdir = run_git_command(repo_root, &["diff"]).await?;
    Ok(workdir)
}

fn resolve_permissions_path(
    entry: &WorkspaceEntry,
    parent_path: Option<&str>,
) -> Result<PathBuf, String> {
    if let Some(project_home) = codex_home::resolve_workspace_claude_home(entry, parent_path) {
        let path = project_home.join("settings.local.json");
        return Ok(path);
    }
    let fallback = PathBuf::from(&entry.path).join(".claude");
    if std::fs::create_dir_all(&fallback).is_ok() {
        return Ok(fallback.join("settings.local.json"));
    }
    codex_home::resolve_default_claude_home()
        .map(|home| home.join("settings.json"))
        .ok_or_else(|| "Unable to resolve Claude settings path".to_string())
}

fn format_permission_rule(command: &[String]) -> String {
    let joined = command.join(" ");
    format!("Bash({joined}:*)")
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

fn sort_workspaces(workspaces: &mut [WorkspaceInfo]) {
    workspaces.sort_by(|a, b| {
        let a_order = a.settings.sort_order.unwrap_or(u32::MAX);
        let b_order = b.settings.sort_order.unwrap_or(u32::MAX);
        if a_order != b_order {
            return a_order.cmp(&b_order);
        }
        a.name.cmp(&b.name)
    });
}

fn should_skip_dir(name: &str) -> bool {
    matches!(
        name,
        ".git" | "node_modules" | "dist" | "target" | "release-artifacts"
    )
}

fn normalize_git_path(path: &str) -> String {
    path.replace('\\', "/")
}

fn list_workspace_files_inner(root: &PathBuf, max_files: usize) -> Vec<String> {
    let mut results = Vec::new();
    let walker = WalkBuilder::new(root)
        .hidden(false)
        .follow_links(false)
        .require_git(false)
        .filter_entry(|entry| {
            if entry.depth() == 0 {
                return true;
            }
            if entry.file_type().is_some_and(|ft| ft.is_dir()) {
                let name = entry.file_name().to_string_lossy();
                return !should_skip_dir(&name);
            }
            true
        })
        .build();

    for entry in walker {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        if !entry.file_type().is_some_and(|ft| ft.is_file()) {
            continue;
        }
        if let Ok(rel_path) = entry.path().strip_prefix(root) {
            let normalized = normalize_git_path(&rel_path.to_string_lossy());
            if !normalized.is_empty() {
                results.push(normalized);
            }
        }
        if results.len() >= max_files {
            break;
        }
    }

    results.sort();
    results
}

async fn run_git_command(repo_path: &PathBuf, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo_path)
        .output()
        .await
        .map_err(|e| format!("Failed to run git: {e}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let detail = if stderr.trim().is_empty() {
            stdout.trim()
        } else {
            stderr.trim()
        };
        if detail.is_empty() {
            Err("Git command failed.".to_string())
        } else {
            Err(detail.to_string())
        }
    }
}

async fn git_branch_exists(repo_path: &PathBuf, branch: &str) -> Result<bool, String> {
    let status = Command::new("git")
        .args(["show-ref", "--verify", &format!("refs/heads/{branch}")])
        .current_dir(repo_path)
        .status()
        .await
        .map_err(|e| format!("Failed to run git: {e}"))?;
    Ok(status.success())
}

async fn git_remote_exists(repo_path: &PathBuf, remote: &str) -> Result<bool, String> {
    let status = Command::new("git")
        .args(["remote", "get-url", remote])
        .current_dir(repo_path)
        .status()
        .await
        .map_err(|e| format!("Failed to run git: {e}"))?;
    Ok(status.success())
}

async fn git_remote_branch_exists_live(
    repo_path: &PathBuf,
    remote: &str,
    branch: &str,
) -> Result<bool, String> {
    let output = Command::new("git")
        .args([
            "ls-remote",
            "--heads",
            remote,
            &format!("refs/heads/{branch}"),
        ])
        .current_dir(repo_path)
        .output()
        .await
        .map_err(|e| format!("Failed to run git: {e}"))?;
    if output.status.success() {
        Ok(!String::from_utf8_lossy(&output.stdout).trim().is_empty())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let detail = if stderr.trim().is_empty() {
            stdout.trim()
        } else {
            stderr.trim()
        };
        if detail.is_empty() {
            Err("Git command failed.".to_string())
        } else {
            Err(detail.to_string())
        }
    }
}

async fn git_remote_branch_exists(repo_path: &PathBuf, remote: &str, branch: &str) -> Result<bool, String> {
    let status = Command::new("git")
        .args([
            "show-ref",
            "--verify",
            &format!("refs/remotes/{remote}/{branch}"),
        ])
        .current_dir(repo_path)
        .status()
        .await
        .map_err(|e| format!("Failed to run git: {e}"))?;
    Ok(status.success())
}

async fn unique_branch_name(
    repo_path: &PathBuf,
    desired: &str,
    remote: Option<&str>,
) -> Result<(String, bool), String> {
    let mut candidate = desired.to_string();
    if desired.is_empty() {
        return Ok((candidate, false));
    }
    if !git_branch_exists(repo_path, &candidate).await?
        && match remote {
            Some(remote) => !git_remote_branch_exists_live(repo_path, remote, &candidate).await?,
            None => true,
        }
    {
        return Ok((candidate, false));
    }
    for index in 2..1000 {
        candidate = format!("{desired}-{index}");
        let local_exists = git_branch_exists(repo_path, &candidate).await?;
        let remote_exists = match remote {
            Some(remote) => git_remote_branch_exists_live(repo_path, remote, &candidate).await?,
            None => false,
        };
        if !local_exists && !remote_exists {
            return Ok((candidate, true));
        }
    }
    Err("Unable to find an available branch name.".to_string())
}

async fn git_list_remotes(repo_path: &PathBuf) -> Result<Vec<String>, String> {
    let output = run_git_command(repo_path, &["remote"]).await?;
    Ok(output
        .lines()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty())
        .map(|line| line.to_string())
        .collect())
}

async fn git_find_remote_for_branch(
    repo_path: &PathBuf,
    branch: &str,
) -> Result<Option<String>, String> {
    if git_remote_exists(repo_path, "origin").await?
        && git_remote_branch_exists_live(repo_path, "origin", branch).await?
    {
        return Ok(Some("origin".to_string()));
    }

    for remote in git_list_remotes(repo_path).await? {
        if remote == "origin" {
            continue;
        }
        if git_remote_branch_exists_live(repo_path, &remote, branch).await? {
            return Ok(Some(remote));
        }
    }

    Ok(None)
}

async fn git_find_remote_tracking_branch(repo_path: &PathBuf, branch: &str) -> Result<Option<String>, String> {
    if git_remote_branch_exists(repo_path, "origin", branch).await? {
        return Ok(Some(format!("origin/{branch}")));
    }

    for remote in git_list_remotes(repo_path).await? {
        if remote == "origin" {
            continue;
        }
        if git_remote_branch_exists(repo_path, &remote, branch).await? {
            return Ok(Some(format!("{remote}/{branch}")));
        }
    }

    Ok(None)
}

fn sanitize_worktree_name(branch: &str) -> String {
    let mut result = String::new();
    for ch in branch.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
            result.push(ch);
        } else {
            result.push('-');
        }
    }
    let trimmed = result.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "worktree".to_string()
    } else {
        trimmed
    }
}

fn unique_worktree_path(base_dir: &PathBuf, name: &str) -> Result<PathBuf, String> {
    let candidate = base_dir.join(name);
    if !candidate.exists() {
        return Ok(candidate);
    }

    for index in 2..1000 {
        let next = base_dir.join(format!("{name}-{index}"));
        if !next.exists() {
            return Ok(next);
        }
    }

    Err(format!(
        "Failed to find an available worktree path under {}.",
        base_dir.display()
    ))
}

fn unique_worktree_path_for_rename(
    base_dir: &PathBuf,
    name: &str,
    current_path: &PathBuf,
) -> Result<PathBuf, String> {
    let candidate = base_dir.join(name);
    if candidate == *current_path {
        return Ok(candidate);
    }
    if !candidate.exists() {
        return Ok(candidate);
    }
    for index in 2..1000 {
        let next = base_dir.join(format!("{name}-{index}"));
        if next == *current_path || !next.exists() {
            return Ok(next);
        }
    }
    Err(format!(
        "Failed to find an available worktree path under {}.",
        base_dir.display()
    ))
}

fn default_data_dir() -> PathBuf {
    if let Ok(xdg) = env::var("XDG_DATA_HOME") {
        let trimmed = xdg.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed).join("codex-monitor-daemon");
        }
    }
    let home = env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join(".local")
        .join("share")
        .join("codex-monitor-daemon")
}

fn usage() -> String {
    format!(
        "\
USAGE:\n  codex-monitor-daemon [--listen <addr>] [--data-dir <path>] [--token <token> | --insecure-no-auth]\n\n\
OPTIONS:\n  --listen <addr>        Bind address (default: {DEFAULT_LISTEN_ADDR})\n  --data-dir <path>      Data dir holding workspaces.json/settings.json\n  --token <token>        Shared token required by clients\n  --insecure-no-auth      Disable auth (dev only)\n  -h, --help             Show this help\n"
    )
}

fn parse_args() -> Result<DaemonConfig, String> {
    let mut listen = DEFAULT_LISTEN_ADDR
        .parse::<SocketAddr>()
        .map_err(|err| err.to_string())?;
    let mut token = env::var("CODEX_MONITOR_DAEMON_TOKEN")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let mut insecure_no_auth = false;
    let mut data_dir: Option<PathBuf> = None;

    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{}", usage());
                std::process::exit(0);
            }
            "--listen" => {
                let value = args.next().ok_or("--listen requires a value")?;
                listen = value.parse::<SocketAddr>().map_err(|err| err.to_string())?;
            }
            "--token" => {
                let value = args.next().ok_or("--token requires a value")?;
                let trimmed = value.trim();
                if trimmed.is_empty() {
                    return Err("--token requires a non-empty value".to_string());
                }
                token = Some(trimmed.to_string());
            }
            "--data-dir" => {
                let value = args.next().ok_or("--data-dir requires a value")?;
                let trimmed = value.trim();
                if trimmed.is_empty() {
                    return Err("--data-dir requires a non-empty value".to_string());
                }
                data_dir = Some(PathBuf::from(trimmed));
            }
            "--insecure-no-auth" => {
                insecure_no_auth = true;
                token = None;
            }
            _ => return Err(format!("Unknown argument: {arg}")),
        }
    }

    if token.is_none() && !insecure_no_auth {
        return Err(
            "Missing --token (or set CODEX_MONITOR_DAEMON_TOKEN). Use --insecure-no-auth for local dev only."
                .to_string(),
        );
    }

    Ok(DaemonConfig {
        listen,
        token,
        data_dir: data_dir.unwrap_or_else(default_data_dir),
    })
}

fn build_error_response(id: Option<u64>, message: &str) -> Option<String> {
    let id = id?;
    Some(
        serde_json::to_string(&json!({
            "id": id,
            "error": { "message": message }
        }))
        .unwrap_or_else(|_| "{\"id\":0,\"error\":{\"message\":\"serialization failed\"}}".to_string()),
    )
}

fn build_result_response(id: Option<u64>, result: Value) -> Option<String> {
    let id = id?;
    Some(serde_json::to_string(&json!({ "id": id, "result": result })).unwrap_or_else(|_| {
        "{\"id\":0,\"error\":{\"message\":\"serialization failed\"}}".to_string()
    }))
}

fn build_event_notification(event: DaemonEvent) -> Option<String> {
    let payload = match event {
        DaemonEvent::AppServer(payload) => json!({
            "method": "app-server-event",
            "params": payload,
        }),
        DaemonEvent::TerminalOutput(payload) => json!({
            "method": "terminal-output",
            "params": payload,
        }),
    };
    serde_json::to_string(&payload).ok()
}

fn parse_auth_token(params: &Value) -> Option<String> {
    match params {
        Value::String(value) => Some(value.clone()),
        Value::Object(map) => map
            .get("token")
            .and_then(|value| value.as_str())
            .map(|v| v.to_string()),
        _ => None,
    }
}

fn parse_string(value: &Value, key: &str) -> Result<String, String> {
    match value {
        Value::Object(map) => map
            .get(key)
            .and_then(|value| value.as_str())
            .map(|value| value.to_string())
            .ok_or_else(|| format!("missing or invalid `{key}`")),
        _ => Err(format!("missing `{key}`")),
    }
}

fn parse_optional_string(value: &Value, key: &str) -> Option<String> {
    match value {
        Value::Object(map) => map
            .get(key)
            .and_then(|value| value.as_str())
            .map(|v| v.to_string()),
        _ => None,
    }
}

fn parse_optional_u32(value: &Value, key: &str) -> Option<u32> {
    match value {
        Value::Object(map) => map.get(key).and_then(|value| value.as_u64()).and_then(|v| {
            if v > u32::MAX as u64 {
                None
            } else {
                Some(v as u32)
            }
        }),
        _ => None,
    }
}

fn parse_optional_string_array(value: &Value, key: &str) -> Option<Vec<String>> {
    match value {
        Value::Object(map) => map.get(key).and_then(|value| value.as_array()).map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(|value| value.to_string()))
                .collect::<Vec<_>>()
        }),
        _ => None,
    }
}

fn parse_string_array(value: &Value, key: &str) -> Result<Vec<String>, String> {
    parse_optional_string_array(value, key).ok_or_else(|| format!("missing `{key}`"))
}

fn parse_optional_value(value: &Value, key: &str) -> Option<Value> {
    match value {
        Value::Object(map) => map.get(key).cloned(),
        _ => None,
    }
}

async fn handle_rpc_request(
    state: &DaemonState,
    method: &str,
    params: Value,
    client_version: String,
) -> Result<Value, String> {
    match method {
        "ping" => Ok(json!({ "ok": true })),
        "list_workspaces" => {
            let workspaces = state.list_workspaces().await;
            serde_json::to_value(workspaces).map_err(|err| err.to_string())
        }
        "add_workspace" => {
            let path = parse_string(&params, "path")?;
            let claude_bin = parse_optional_string(&params, "claude_bin")
                .or_else(|| parse_optional_string(&params, "codex_bin"));
            let workspace = state.add_workspace(path, claude_bin, client_version).await?;
            serde_json::to_value(workspace).map_err(|err| err.to_string())
        }
        "add_worktree" => {
            let parent_id = parse_string(&params, "parentId")?;
            let branch = parse_string(&params, "branch")?;
            let workspace = state
                .add_worktree(parent_id, branch, client_version)
                .await?;
            serde_json::to_value(workspace).map_err(|err| err.to_string())
        }
        "connect_workspace" => {
            let id = parse_string(&params, "id")?;
            state.connect_workspace(id, client_version).await?;
            Ok(json!({ "ok": true }))
        }
        "remove_workspace" => {
            let id = parse_string(&params, "id")?;
            state.remove_workspace(id).await?;
            Ok(json!({ "ok": true }))
        }
        "remove_worktree" => {
            let id = parse_string(&params, "id")?;
            state.remove_worktree(id).await?;
            Ok(json!({ "ok": true }))
        }
        "rename_worktree" => {
            let id = parse_string(&params, "id")?;
            let branch = parse_string(&params, "branch")?;
            let workspace = state.rename_worktree(id, branch, client_version).await?;
            serde_json::to_value(workspace).map_err(|err| err.to_string())
        }
        "rename_worktree_upstream" => {
            let id = parse_string(&params, "id")?;
            let old_branch = parse_string(&params, "oldBranch")?;
            let new_branch = parse_string(&params, "newBranch")?;
            state
                .rename_worktree_upstream(id, old_branch, new_branch)
                .await?;
            Ok(json!({ "ok": true }))
        }
        "update_workspace_settings" => {
            let id = parse_string(&params, "id")?;
            let settings_value = match params {
                Value::Object(map) => map.get("settings").cloned().unwrap_or(Value::Null),
                _ => Value::Null,
            };
            let settings: WorkspaceSettings =
                serde_json::from_value(settings_value).map_err(|err| err.to_string())?;
            let workspace = state.update_workspace_settings(id, settings).await?;
            serde_json::to_value(workspace).map_err(|err| err.to_string())
        }
        "update_workspace_claude_bin" | "update_workspace_codex_bin" => {
            let id = parse_string(&params, "id")?;
            let claude_bin = parse_optional_string(&params, "claude_bin")
                .or_else(|| parse_optional_string(&params, "codex_bin"));
            let workspace = state.update_workspace_claude_bin(id, claude_bin).await?;
            serde_json::to_value(workspace).map_err(|err| err.to_string())
        }
        "list_workspace_files" => {
            let workspace_id = parse_string(&params, "workspaceId")?;
            let files = state.list_workspace_files(workspace_id).await?;
            serde_json::to_value(files).map_err(|err| err.to_string())
        }
        "get_app_settings" => {
            let mut settings = state.app_settings.lock().await.clone();
            if let Ok(Some(collab_enabled)) = codex_config::read_collab_enabled() {
                settings.experimental_collab_enabled = collab_enabled;
            }
            if let Ok(Some(steer_enabled)) = codex_config::read_steer_enabled() {
                settings.experimental_steer_enabled = steer_enabled;
            }
            if let Ok(Some(unified_exec_enabled)) = codex_config::read_unified_exec_enabled() {
                settings.experimental_unified_exec_enabled = unified_exec_enabled;
            }
            serde_json::to_value(settings).map_err(|err| err.to_string())
        }
        "update_app_settings" => {
            let settings_value = match params {
                Value::Object(map) => map.get("settings").cloned().unwrap_or(Value::Null),
                _ => Value::Null,
            };
            let settings: AppSettings =
                serde_json::from_value(settings_value).map_err(|err| err.to_string())?;
            let updated = state.update_app_settings(settings).await?;
            serde_json::to_value(updated).map_err(|err| err.to_string())
        }
        "start_thread" => {
            let workspace_id = parse_string(&params, "workspaceId")?;
            state.start_thread(workspace_id).await
        }
        "resume_thread" => {
            let workspace_id = parse_string(&params, "workspaceId")?;
            let thread_id = parse_string(&params, "threadId")?;
            state.resume_thread(workspace_id, thread_id).await
        }
        "list_threads" => {
            let workspace_id = parse_string(&params, "workspaceId")?;
            let cursor = parse_optional_string(&params, "cursor");
            let limit = parse_optional_u32(&params, "limit");
            state.list_threads(workspace_id, cursor, limit).await
        }
        "archive_thread" => {
            let workspace_id = parse_string(&params, "workspaceId")?;
            let thread_id = parse_string(&params, "threadId")?;
            state.archive_thread(workspace_id, thread_id).await
        }
        "send_user_message" => {
            let workspace_id = parse_string(&params, "workspaceId")?;
            let thread_id = parse_string(&params, "threadId")?;
            let text = parse_string(&params, "text")?;
            let model = parse_optional_string(&params, "model");
            let effort = parse_optional_string(&params, "effort");
            let access_mode = parse_optional_string(&params, "accessMode");
            let images = parse_optional_string_array(&params, "images");
            let collaboration_mode = parse_optional_value(&params, "collaborationMode");
            state
                .send_user_message(
                    workspace_id,
                    thread_id,
                    text,
                    model,
                    effort,
                    access_mode,
                    images,
                    collaboration_mode,
                )
                .await
        }
        "turn_interrupt" => {
            let workspace_id = parse_string(&params, "workspaceId")?;
            let thread_id = parse_string(&params, "threadId")?;
            let turn_id = parse_string(&params, "turnId")?;
            state.turn_interrupt(workspace_id, thread_id, turn_id).await
        }
        "start_review" => {
            let workspace_id = parse_string(&params, "workspaceId")?;
            let thread_id = parse_string(&params, "threadId")?;
            let target = params
                .as_object()
                .and_then(|map| map.get("target"))
                .cloned()
                .ok_or("missing `target`")?;
            let delivery = parse_optional_string(&params, "delivery");
            state.start_review(workspace_id, thread_id, target, delivery).await
        }
        "model_list" => {
            let workspace_id = parse_string(&params, "workspaceId")?;
            state.model_list(workspace_id).await
        }
        "collaboration_mode_list" => {
            let workspace_id = parse_string(&params, "workspaceId")?;
            state.collaboration_mode_list(workspace_id).await
        }
        "account_rate_limits" => {
            let workspace_id = parse_string(&params, "workspaceId")?;
            state.account_rate_limits(workspace_id).await
        }
        "skills_list" => {
            let workspace_id = parse_string(&params, "workspaceId")?;
            state.skills_list(workspace_id).await
        }
        "respond_to_server_request" => {
            let workspace_id = parse_string(&params, "workspaceId")?;
            let map = params.as_object().ok_or("missing requestId")?;
            let request_id = map
                .get("requestId")
                .and_then(|value| value.as_u64())
                .ok_or("missing requestId")?;
            let result = map.get("result").cloned().ok_or("missing `result`")?;
            state
                .respond_to_server_request(workspace_id, request_id, result)
                .await
        }
        "remember_approval_rule" => {
            let workspace_id = parse_string(&params, "workspaceId")?;
            let command = parse_string_array(&params, "command")?;
            state.remember_approval_rule(workspace_id, command).await
        }
        _ => Err(format!("unknown method: {method}")),
    }
}

async fn forward_events(
    mut rx: broadcast::Receiver<DaemonEvent>,
    out_tx_events: mpsc::UnboundedSender<String>,
) {
    loop {
        let event = match rx.recv().await {
            Ok(event) => event,
            Err(broadcast::error::RecvError::Lagged(_)) => continue,
            Err(broadcast::error::RecvError::Closed) => break,
        };

        let Some(payload) = build_event_notification(event) else {
            continue;
        };

        if out_tx_events.send(payload).is_err() {
            break;
        }
    }
}

async fn handle_client(
    socket: TcpStream,
    config: Arc<DaemonConfig>,
    state: Arc<DaemonState>,
    events: broadcast::Sender<DaemonEvent>,
) {
    let (reader, mut writer) = socket.into_split();
    let mut lines = BufReader::new(reader).lines();

    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<String>();
    let write_task = tokio::spawn(async move {
        while let Some(message) = out_rx.recv().await {
            if writer.write_all(message.as_bytes()).await.is_err() {
                break;
            }
            if writer.write_all(b"\n").await.is_err() {
                break;
            }
        }
    });

    let mut authenticated = config.token.is_none();
    let mut events_task: Option<tokio::task::JoinHandle<()>> = None;

    if authenticated {
        let rx = events.subscribe();
        let out_tx_events = out_tx.clone();
        events_task = Some(tokio::spawn(forward_events(rx, out_tx_events)));
    }

    while let Ok(Some(line)) = lines.next_line().await {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let message: Value = match serde_json::from_str(line) {
            Ok(value) => value,
            Err(_) => continue,
        };

        let id = message.get("id").and_then(|value| value.as_u64());
        let method = message
            .get("method")
            .and_then(|value| value.as_str())
            .unwrap_or("")
            .to_string();
        let params = message.get("params").cloned().unwrap_or(Value::Null);

        if !authenticated {
            if method != "auth" {
                if let Some(response) = build_error_response(id, "unauthorized") {
                    let _ = out_tx.send(response);
                }
                continue;
            }

            let expected = config.token.clone().unwrap_or_default();
            let provided = parse_auth_token(&params).unwrap_or_default();
            if expected != provided {
                if let Some(response) = build_error_response(id, "invalid token") {
                    let _ = out_tx.send(response);
                }
                continue;
            }

            authenticated = true;
            if let Some(response) = build_result_response(id, json!({ "ok": true })) {
                let _ = out_tx.send(response);
            }

            let rx = events.subscribe();
            let out_tx_events = out_tx.clone();
            events_task = Some(tokio::spawn(forward_events(rx, out_tx_events)));

            continue;
        }

        let client_version = format!("daemon-{}", env!("CARGO_PKG_VERSION"));
        let result = handle_rpc_request(&state, &method, params, client_version).await;
        let response = match result {
            Ok(result) => build_result_response(id, result),
            Err(message) => build_error_response(id, &message),
        };
        if let Some(response) = response {
            let _ = out_tx.send(response);
        }
    }

    drop(out_tx);
    if let Some(task) = events_task {
        task.abort();
    }
    write_task.abort();
}

fn main() {
    let config = match parse_args() {
        Ok(config) => config,
        Err(err) => {
            eprintln!("{err}\n\n{}", usage());
            std::process::exit(2);
        }
    };

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("failed to build tokio runtime");

    runtime.block_on(async move {
        let (events_tx, _events_rx) = broadcast::channel::<DaemonEvent>(2048);
        let event_sink = DaemonEventSink {
            tx: events_tx.clone(),
        };
        let state = Arc::new(DaemonState::load(&config, event_sink));
        let config = Arc::new(config);

        let listener = TcpListener::bind(config.listen)
            .await
            .unwrap_or_else(|err| panic!("failed to bind {}: {err}", config.listen));
        eprintln!(
            "codex-monitor-daemon listening on {} (data dir: {})",
            config.listen,
            state
                .storage_path
                .parent()
                .unwrap_or(&state.storage_path)
                .display()
        );

        loop {
            match listener.accept().await {
                Ok((socket, _addr)) => {
                    let config = Arc::clone(&config);
                    let state = Arc::clone(&state);
                    let events = events_tx.clone();
                    tokio::spawn(async move {
                        handle_client(socket, config, state, events).await;
                    });
                }
                Err(_) => continue,
            }
        }
    });
}
