use std::collections::HashMap;
use std::env;
use std::io::ErrorKind;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use tokio::time::timeout;

use crate::types::WorkspaceEntry;

pub(crate) struct ActiveTurn {
    pub(crate) turn_id: String,
    pub(crate) child: Arc<Mutex<Child>>,
}

pub(crate) struct WorkspaceSession {
    pub(crate) entry: WorkspaceEntry,
    pub(crate) claude_bin: Option<String>,
    pub(crate) active_turns: Mutex<HashMap<String, ActiveTurn>>,
}

impl WorkspaceSession {
    pub(crate) async fn track_turn(
        &self,
        thread_id: String,
        turn_id: String,
        child: Arc<Mutex<Child>>,
    ) {
        let mut active_turns = self.active_turns.lock().await;
        active_turns.insert(
            thread_id,
            ActiveTurn {
                turn_id,
                child,
            },
        );
    }

    pub(crate) async fn clear_turn(&self, thread_id: &str, turn_id: &str) {
        let mut active_turns = self.active_turns.lock().await;
        if let Some(active_turn) = active_turns.get(thread_id) {
            if active_turn.turn_id == turn_id {
                active_turns.remove(thread_id);
            }
        }
    }

    pub(crate) async fn interrupt_turn(
        &self,
        thread_id: &str,
        turn_id: &str,
    ) -> Result<(), String> {
        let mut active_turns = self.active_turns.lock().await;
        let Some(active_turn) = active_turns.remove(thread_id) else {
            return Ok(());
        };
        if active_turn.turn_id != turn_id {
            active_turns.insert(thread_id.to_string(), active_turn);
            return Ok(());
        }
        let mut child = active_turn.child.lock().await;
        match child.kill().await {
            Ok(_) => Ok(()),
            Err(err) if err.kind() == ErrorKind::InvalidInput => Ok(()),
            Err(err) => Err(err.to_string()),
        }
    }
}

pub(crate) fn build_claude_path_env(claude_bin: Option<&str>) -> Option<String> {
    let mut paths: Vec<String> = env::var("PATH")
        .unwrap_or_default()
        .split(':')
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
        .collect();
    let mut extras = vec![
        "/opt/homebrew/bin",
        "/usr/local/bin",
        "/usr/bin",
        "/bin",
        "/usr/sbin",
        "/sbin",
    ]
    .into_iter()
    .map(|value| value.to_string())
    .collect::<Vec<String>>();
    if let Ok(home) = env::var("HOME") {
        extras.push(format!("{home}/.local/bin"));
        extras.push(format!("{home}/.local/share/mise/shims"));
        extras.push(format!("{home}/.cargo/bin"));
        extras.push(format!("{home}/.bun/bin"));
        let nvm_root = Path::new(&home).join(".nvm/versions/node");
        if let Ok(entries) = std::fs::read_dir(nvm_root) {
            for entry in entries.flatten() {
                let bin_path = entry.path().join("bin");
                if bin_path.is_dir() {
                    extras.push(bin_path.to_string_lossy().to_string());
                }
            }
        }
    }
    if let Some(bin_path) = claude_bin.filter(|value| !value.trim().is_empty()) {
        let parent = Path::new(bin_path).parent();
        if let Some(parent) = parent {
            extras.push(parent.to_string_lossy().to_string());
        }
    }
    for extra in extras {
        if !paths.contains(&extra) {
            paths.push(extra);
        }
    }
    if paths.is_empty() {
        None
    } else {
        Some(paths.join(":"))
    }
}

pub(crate) fn build_claude_command_with_bin(claude_bin: Option<String>) -> Command {
    let bin = claude_bin
        .clone()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "claude".into());
    let mut command = Command::new(bin);
    if let Some(path_env) = build_claude_path_env(claude_bin.as_deref()) {
        command.env("PATH", path_env);
    }
    command
}

pub(crate) async fn check_claude_installation(
    claude_bin: Option<String>,
) -> Result<Option<String>, String> {
    let mut command = build_claude_command_with_bin(claude_bin);
    command.arg("--version");
    command.stdout(std::process::Stdio::piped());
    command.stderr(std::process::Stdio::piped());

    let output = match timeout(Duration::from_secs(5), command.output()).await {
        Ok(result) => result.map_err(|e| {
            if e.kind() == ErrorKind::NotFound {
                "Claude Code CLI not found. Install Claude Code and ensure `claude` is on your PATH."
                    .to_string()
            } else {
                e.to_string()
            }
        })?,
        Err(_) => {
            return Err(
                "Timed out while checking Claude Code CLI. Make sure `claude --version` runs in Terminal."
                    .to_string(),
            );
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let detail = if stderr.trim().is_empty() {
            stdout.trim()
        } else {
            stderr.trim()
        };
        if detail.is_empty() {
            return Err(
                "Claude Code CLI failed to start. Try running `claude --version` in Terminal."
                    .to_string(),
            );
        }
        return Err(format!(
            "Claude Code CLI failed to start: {detail}. Try running `claude --version` in Terminal."
        ));
    }

    let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(if version.is_empty() { None } else { Some(version) })
}

pub(crate) async fn spawn_workspace_session(
    entry: WorkspaceEntry,
    default_claude_bin: Option<String>,
) -> Result<Arc<WorkspaceSession>, String> {
    let claude_bin = entry
        .claude_bin
        .clone()
        .filter(|value| !value.trim().is_empty())
        .or(default_claude_bin);
    let _ = check_claude_installation(claude_bin.clone()).await?;

    Ok(Arc::new(WorkspaceSession {
        entry,
        claude_bin,
        active_turns: Mutex::new(HashMap::new()),
    }))
}
