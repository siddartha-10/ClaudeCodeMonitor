use std::collections::HashMap;
use std::env;
use std::io::ErrorKind;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tokio::io::AsyncWriteExt;
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::Mutex;
use tokio::time::timeout;

use crate::types::WorkspaceEntry;

pub(crate) struct ActiveTurn {
    pub(crate) turn_id: String,
    pub(crate) child: Arc<Mutex<Child>>,
}

/// A persistent session for a single thread.
/// Each thread gets its own CLI process with stdin for bidirectional communication.
pub(crate) struct PersistentSession {
    pub(crate) stdin: ChildStdin,
    pub(crate) child: Child,
    /// Pending turn ID to be used by the reader when starting a new turn
    pub(crate) pending_turn_id: Option<String>,
    /// The permission mode this session was started with (e.g., "dontAsk", "plan")
    /// Used to detect when permission mode changes and session needs restart
    pub(crate) permission_mode: Option<String>,
    /// The model this session was started with (e.g., "claude-sonnet-4-5-20250514")
    /// Used to detect when model changes and session needs restart
    pub(crate) model: Option<String>,
}

pub(crate) struct WorkspaceSession {
    pub(crate) entry: WorkspaceEntry,
    pub(crate) claude_bin: Option<String>,
    pub(crate) active_turns: Mutex<HashMap<String, ActiveTurn>>,
    /// Persistent sessions per thread - allows multiple threads to run in parallel
    pub(crate) persistent_sessions: Mutex<HashMap<String, PersistentSession>>,
    /// Lock to prevent race conditions when initializing persistent sessions
    pub(crate) session_init_lock: Mutex<()>,
}

impl WorkspaceSession {
    /// Track an active turn for a thread.
    /// Used by the daemon binary for per-turn process management.
    #[allow(dead_code)]
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

    /// Clear an active turn after completion.
    /// Used by the daemon binary for per-turn process management.
    #[allow(dead_code)]
    pub(crate) async fn clear_turn(&self, thread_id: &str, turn_id: &str) {
        let mut active_turns = self.active_turns.lock().await;
        if let Some(active_turn) = active_turns.get(thread_id) {
            if active_turn.turn_id == turn_id {
                active_turns.remove(thread_id);
            }
        }
    }

    /// Interrupt a running turn for a specific thread.
    ///
    /// This handles two architectures:
    /// 1. **active_turns** (old per-turn approach): Each turn spawns a new CLI process.
    ///    In this case, we check turn_id to ensure we're killing the correct turn.
    /// 2. **persistent_sessions** (new approach): One CLI process per thread, reused
    ///    across multiple turns. The session is killed and will be respawned on next message.
    ///
    /// For persistent sessions, killing the process is the only way to interrupt since
    /// Claude CLI's stream-json mode has no cancel/abort message type.
    pub(crate) async fn interrupt_turn(
        &self,
        thread_id: &str,
        turn_id: &str,
    ) -> Result<(), String> {
        // First, check active_turns (old per-turn process management)
        {
            let mut active_turns = self.active_turns.lock().await;
            if let Some(active_turn) = active_turns.remove(thread_id) {
                if active_turn.turn_id == turn_id {
                    // Matching turn, kill it
                    let mut child = active_turn.child.lock().await;
                    return match child.kill().await {
                        Ok(_) => Ok(()),
                        Err(err) if err.kind() == ErrorKind::InvalidInput => Ok(()),
                        Err(err) => Err(err.to_string()),
                    };
                } else {
                    // Wrong turn ID, put it back and return
                    active_turns.insert(thread_id.to_string(), active_turn);
                    return Ok(());
                }
            }
            // Thread not in active_turns, continue to check persistent_sessions
        }

        // For persistent sessions, kill the session if it exists.
        // The session will be respawned with --resume on the next message.
        // This is idempotent - returns Ok(()) if no session exists.
        self.kill_persistent_session(thread_id).await
    }

    /// Send a response to the Claude CLI server for a specific thread.
    /// This is used for responding to server requests like AskUserQuestion.
    ///
    /// The response format uses tool_result for AskUserQuestion responses:
    /// ```json
    /// {
    ///   "type": "user",
    ///   "message": {
    ///     "role": "user",
    ///     "content": [{
    ///       "type": "tool_result",
    ///       "tool_use_id": "toolu_XXXXX",
    ///       "content": <result_value>
    ///     }]
    ///   }
    /// }
    /// ```
    pub(crate) async fn send_response(
        &self,
        thread_id: &str,
        tool_use_id: String,
        result: Value,
    ) -> Result<(), String> {
        let mut sessions = self.persistent_sessions.lock().await;
        let session = sessions
            .get_mut(thread_id)
            .ok_or_else(|| format!("No persistent session for thread {}", thread_id))?;

        // Build the tool_result message for AskUserQuestion responses
        let response = serde_json::json!({
            "type": "user",
            "message": {
                "role": "user",
                "content": [{
                    "type": "tool_result",
                    "tool_use_id": tool_use_id,
                    "content": result
                }]
            }
        });

        let mut line = serde_json::to_string(&response).map_err(|e| e.to_string())?;
        line.push('\n');

        session.stdin
            .write_all(line.as_bytes())
            .await
            .map_err(|e| e.to_string())
    }

    /// Send a user message to the Claude CLI server for a specific thread.
    /// This is used for sending new messages in a persistent session.
    ///
    /// The message format for stream-json input:
    /// ```json
    /// {"type":"user","message":{"role":"user","content":"Your message here"}}
    /// ```
    pub(crate) async fn send_message(&self, thread_id: &str, message: &str) -> Result<(), String> {
        let mut sessions = self.persistent_sessions.lock().await;
        let session = sessions
            .get_mut(thread_id)
            .ok_or_else(|| format!("No persistent session for thread {}", thread_id))?;

        let msg = serde_json::json!({
            "type": "user",
            "message": {
                "role": "user",
                "content": message
            }
        });

        let mut line = serde_json::to_string(&msg).map_err(|e| e.to_string())?;
        line.push('\n');

        session.stdin
            .write_all(line.as_bytes())
            .await
            .map_err(|e| e.to_string())
    }

    /// Check if a persistent session exists for a specific thread.
    pub(crate) async fn has_persistent_session(&self, thread_id: &str) -> bool {
        self.persistent_sessions.lock().await.contains_key(thread_id)
    }

    /// Store a new persistent session for a thread.
    pub(crate) async fn set_persistent_session(
        &self,
        thread_id: String,
        stdin: ChildStdin,
        child: Child,
        permission_mode: Option<String>,
        model: Option<String>,
    ) {
        let mut sessions = self.persistent_sessions.lock().await;
        sessions.insert(thread_id, PersistentSession {
            stdin,
            child,
            pending_turn_id: None,
            permission_mode,
            model,
        });
    }

    /// Get the permission mode for a thread's persistent session.
    /// Returns None if no session exists or if the session has no permission mode set.
    pub(crate) async fn get_persistent_session_permission_mode(&self, thread_id: &str) -> Option<String> {
        let sessions = self.persistent_sessions.lock().await;
        sessions.get(thread_id).and_then(|s| s.permission_mode.clone())
    }

    /// Get the model for a thread's persistent session.
    /// Returns None if no session exists or if the session has no model set.
    pub(crate) async fn get_persistent_session_model(&self, thread_id: &str) -> Option<String> {
        let sessions = self.persistent_sessions.lock().await;
        sessions.get(thread_id).and_then(|s| s.model.clone())
    }

    /// Set the pending turn ID for a thread's persistent session.
    pub(crate) async fn set_pending_turn_id(&self, thread_id: &str, turn_id: String) {
        let mut sessions = self.persistent_sessions.lock().await;
        if let Some(session) = sessions.get_mut(thread_id) {
            session.pending_turn_id = Some(turn_id);
        }
    }

    /// Take (consume) the pending turn ID for a thread's persistent session.
    pub(crate) async fn take_pending_turn_id(&self, thread_id: &str) -> Option<String> {
        let mut sessions = self.persistent_sessions.lock().await;
        sessions.get_mut(thread_id).and_then(|s| s.pending_turn_id.take())
    }

    /// Kill the persistent session for a specific thread and clean up resources.
    pub(crate) async fn kill_persistent_session(&self, thread_id: &str) -> Result<(), String> {
        let mut sessions = self.persistent_sessions.lock().await;
        if let Some(mut session) = sessions.remove(thread_id) {
            // Flush stdin before killing to ensure pending writes are sent
            let _ = session.stdin.flush().await;
            session.child.kill().await.map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// Kill all persistent sessions (used for workspace cleanup).
    pub(crate) async fn kill_all_persistent_sessions(&self) -> Result<(), String> {
        let mut sessions = self.persistent_sessions.lock().await;
        for (_, mut session) in sessions.drain() {
            let _ = session.stdin.flush().await;
            let _ = session.child.kill().await;
        }
        Ok(())
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
        persistent_sessions: Mutex::new(HashMap::new()),
        session_init_lock: Mutex::new(()),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{WorkspaceKind, WorkspaceSettings};
    use std::process::Stdio;
    use uuid::Uuid;

    /// Create a test WorkspaceEntry for testing
    fn create_test_workspace_entry() -> WorkspaceEntry {
        WorkspaceEntry {
            id: Uuid::new_v4().to_string(),
            name: "test-workspace".to_string(),
            path: "/tmp/test-workspace".to_string(),
            claude_bin: None,
            kind: WorkspaceKind::Main,
            parent_id: None,
            worktree: None,
            settings: WorkspaceSettings::default(),
        }
    }

    /// Create a test WorkspaceSession without checking Claude installation
    fn create_test_workspace_session() -> WorkspaceSession {
        WorkspaceSession {
            entry: create_test_workspace_entry(),
            claude_bin: None,
            active_turns: Mutex::new(HashMap::new()),
            persistent_sessions: Mutex::new(HashMap::new()),
            session_init_lock: Mutex::new(()),
        }
    }

    /// Spawn a simple cat process that we can use for testing stdin/stdout
    async fn spawn_test_process() -> (ChildStdin, Child) {
        let mut child = Command::new("cat")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("Failed to spawn cat process for testing");
        let stdin = child.stdin.take().expect("Failed to get stdin");
        (stdin, child)
    }

    // ==========================================================================
    // Tests for has_persistent_session
    // ==========================================================================

    #[tokio::test]
    async fn has_persistent_session_returns_false_for_unknown_thread() {
        let session = create_test_workspace_session();

        // Thread that was never registered should return false
        assert!(!session.has_persistent_session("unknown-thread-id").await);
        assert!(!session.has_persistent_session("thread-1").await);
        assert!(!session.has_persistent_session("thread-2").await);
    }

    #[tokio::test]
    async fn has_persistent_session_returns_true_after_setting_session() {
        let session = create_test_workspace_session();
        let (stdin, child) = spawn_test_process().await;

        // Before setting - should be false
        assert!(!session.has_persistent_session("thread-1").await);

        // Set the session
        session
            .set_persistent_session("thread-1".to_string(), stdin, child, None, None)
            .await;

        // After setting - should be true
        assert!(session.has_persistent_session("thread-1").await);
    }

    #[tokio::test]
    async fn multiple_threads_can_have_independent_sessions() {
        let session = create_test_workspace_session();

        // Spawn processes for three different threads
        let (stdin1, child1) = spawn_test_process().await;
        let (stdin2, child2) = spawn_test_process().await;
        let (stdin3, child3) = spawn_test_process().await;

        // Register all three threads
        session
            .set_persistent_session("thread-alpha".to_string(), stdin1, child1, None, None)
            .await;
        session
            .set_persistent_session("thread-beta".to_string(), stdin2, child2, None, None)
            .await;
        session
            .set_persistent_session("thread-gamma".to_string(), stdin3, child3, None, None)
            .await;

        // All three should exist
        assert!(session.has_persistent_session("thread-alpha").await);
        assert!(session.has_persistent_session("thread-beta").await);
        assert!(session.has_persistent_session("thread-gamma").await);

        // Unknown thread should still be false
        assert!(!session.has_persistent_session("thread-delta").await);
    }

    // ==========================================================================
    // Tests for send_message error handling
    // ==========================================================================

    #[tokio::test]
    async fn send_message_fails_when_no_session_exists() {
        let session = create_test_workspace_session();

        let result = session.send_message("nonexistent-thread", "Hello").await;

        assert!(result.is_err());
        let error = result.unwrap_err();
        assert!(
            error.contains("No persistent session for thread nonexistent-thread"),
            "Expected error about missing session, got: {}",
            error
        );
    }

    #[tokio::test]
    async fn send_message_targets_correct_thread_session() {
        let session = create_test_workspace_session();

        // Set up two thread sessions
        let (stdin1, child1) = spawn_test_process().await;
        let (stdin2, child2) = spawn_test_process().await;

        session
            .set_persistent_session("thread-A".to_string(), stdin1, child1, None, None)
            .await;
        session
            .set_persistent_session("thread-B".to_string(), stdin2, child2, None, None)
            .await;

        // Sending to thread-A should succeed
        let result = session.send_message("thread-A", "Message for A").await;
        assert!(result.is_ok(), "Expected success for thread-A: {:?}", result);

        // Sending to thread-B should also succeed
        let result = session.send_message("thread-B", "Message for B").await;
        assert!(result.is_ok(), "Expected success for thread-B: {:?}", result);

        // Sending to nonexistent thread should fail
        let result = session.send_message("thread-C", "Message for C").await;
        assert!(result.is_err());
    }

    // ==========================================================================
    // Tests for send_response error handling
    // ==========================================================================

    #[tokio::test]
    async fn send_response_fails_when_no_session_exists() {
        let session = create_test_workspace_session();

        let result = session
            .send_response(
                "nonexistent-thread",
                "toolu_123".to_string(),
                serde_json::json!({"decision": "accept"}),
            )
            .await;

        assert!(result.is_err());
        let error = result.unwrap_err();
        assert!(
            error.contains("No persistent session for thread nonexistent-thread"),
            "Expected error about missing session, got: {}",
            error
        );
    }

    #[tokio::test]
    async fn send_response_targets_correct_thread_session() {
        let session = create_test_workspace_session();

        // Set up two thread sessions
        let (stdin1, child1) = spawn_test_process().await;
        let (stdin2, child2) = spawn_test_process().await;

        session
            .set_persistent_session("thread-X".to_string(), stdin1, child1, None, None)
            .await;
        session
            .set_persistent_session("thread-Y".to_string(), stdin2, child2, None, None)
            .await;

        // Sending response to thread-X should succeed
        let result = session
            .send_response(
                "thread-X",
                "toolu_abc".to_string(),
                serde_json::json!({"decision": "accept"}),
            )
            .await;
        assert!(result.is_ok(), "Expected success for thread-X: {:?}", result);

        // Sending response to thread-Y should also succeed
        let result = session
            .send_response(
                "thread-Y",
                "toolu_def".to_string(),
                serde_json::json!({"answers": {"q1": ["Yes"]}}),
            )
            .await;
        assert!(result.is_ok(), "Expected success for thread-Y: {:?}", result);
    }

    // ==========================================================================
    // Tests for pending_turn_id management
    // ==========================================================================

    #[tokio::test]
    async fn pending_turn_id_is_none_by_default() {
        let session = create_test_workspace_session();
        let (stdin, child) = spawn_test_process().await;

        session
            .set_persistent_session("thread-1".to_string(), stdin, child, None, None)
            .await;

        // Should be None initially
        let turn_id = session.take_pending_turn_id("thread-1").await;
        assert!(turn_id.is_none());
    }

    #[tokio::test]
    async fn set_pending_turn_id_stores_value_for_correct_thread() {
        let session = create_test_workspace_session();

        // Set up two threads
        let (stdin1, child1) = spawn_test_process().await;
        let (stdin2, child2) = spawn_test_process().await;

        session
            .set_persistent_session("thread-1".to_string(), stdin1, child1, None, None)
            .await;
        session
            .set_persistent_session("thread-2".to_string(), stdin2, child2, None, None)
            .await;

        // Set pending turn ID for thread-1 only
        session
            .set_pending_turn_id("thread-1", "turn-abc-123".to_string())
            .await;

        // Thread-1 should have the pending turn ID
        let turn_id_1 = session.take_pending_turn_id("thread-1").await;
        assert_eq!(turn_id_1, Some("turn-abc-123".to_string()));

        // Thread-2 should have None
        let turn_id_2 = session.take_pending_turn_id("thread-2").await;
        assert!(turn_id_2.is_none());
    }

    #[tokio::test]
    async fn take_pending_turn_id_consumes_the_value() {
        let session = create_test_workspace_session();
        let (stdin, child) = spawn_test_process().await;

        session
            .set_persistent_session("thread-1".to_string(), stdin, child, None, None)
            .await;
        session
            .set_pending_turn_id("thread-1", "turn-xyz".to_string())
            .await;

        // First take should return the value
        let first_take = session.take_pending_turn_id("thread-1").await;
        assert_eq!(first_take, Some("turn-xyz".to_string()));

        // Second take should return None (consumed)
        let second_take = session.take_pending_turn_id("thread-1").await;
        assert!(second_take.is_none());

        // Third take should also return None
        let third_take = session.take_pending_turn_id("thread-1").await;
        assert!(third_take.is_none());
    }

    #[tokio::test]
    async fn pending_turn_id_can_be_set_multiple_times() {
        let session = create_test_workspace_session();
        let (stdin, child) = spawn_test_process().await;

        session
            .set_persistent_session("thread-1".to_string(), stdin, child, None, None)
            .await;

        // Set first value
        session
            .set_pending_turn_id("thread-1", "turn-first".to_string())
            .await;

        // Overwrite with second value
        session
            .set_pending_turn_id("thread-1", "turn-second".to_string())
            .await;

        // Should get the second value
        let turn_id = session.take_pending_turn_id("thread-1").await;
        assert_eq!(turn_id, Some("turn-second".to_string()));
    }

    #[tokio::test]
    async fn set_pending_turn_id_does_nothing_for_unknown_thread() {
        let session = create_test_workspace_session();

        // Try to set pending turn ID for a thread that doesn't exist
        session
            .set_pending_turn_id("unknown-thread", "turn-123".to_string())
            .await;

        // Should not panic, and take should return None
        let turn_id = session.take_pending_turn_id("unknown-thread").await;
        assert!(turn_id.is_none());
    }

    // ==========================================================================
    // Tests for kill_persistent_session
    // ==========================================================================

    #[tokio::test]
    async fn kill_persistent_session_removes_only_specified_thread() {
        let session = create_test_workspace_session();

        // Set up three thread sessions
        let (stdin1, child1) = spawn_test_process().await;
        let (stdin2, child2) = spawn_test_process().await;
        let (stdin3, child3) = spawn_test_process().await;

        session
            .set_persistent_session("thread-1".to_string(), stdin1, child1, None, None)
            .await;
        session
            .set_persistent_session("thread-2".to_string(), stdin2, child2, None, None)
            .await;
        session
            .set_persistent_session("thread-3".to_string(), stdin3, child3, None, None)
            .await;

        // All three should exist initially
        assert!(session.has_persistent_session("thread-1").await);
        assert!(session.has_persistent_session("thread-2").await);
        assert!(session.has_persistent_session("thread-3").await);

        // Kill only thread-2
        let result = session.kill_persistent_session("thread-2").await;
        assert!(result.is_ok());

        // Thread-2 should be gone, others should remain
        assert!(session.has_persistent_session("thread-1").await);
        assert!(!session.has_persistent_session("thread-2").await);
        assert!(session.has_persistent_session("thread-3").await);
    }

    #[tokio::test]
    async fn kill_persistent_session_is_idempotent() {
        let session = create_test_workspace_session();
        let (stdin, child) = spawn_test_process().await;

        session
            .set_persistent_session("thread-1".to_string(), stdin, child, None, None)
            .await;

        // Kill the session
        let result1 = session.kill_persistent_session("thread-1").await;
        assert!(result1.is_ok());

        // Kill again should succeed (no-op)
        let result2 = session.kill_persistent_session("thread-1").await;
        assert!(result2.is_ok());

        // Session should not exist
        assert!(!session.has_persistent_session("thread-1").await);
    }

    #[tokio::test]
    async fn kill_persistent_session_succeeds_for_unknown_thread() {
        let session = create_test_workspace_session();

        // Killing a nonexistent session should succeed (no-op)
        let result = session.kill_persistent_session("nonexistent").await;
        assert!(result.is_ok());
    }

    // ==========================================================================
    // Tests for kill_all_persistent_sessions
    // ==========================================================================

    #[tokio::test]
    async fn kill_all_persistent_sessions_removes_all_threads() {
        let session = create_test_workspace_session();

        // Set up five thread sessions
        for i in 1..=5 {
            let (stdin, child) = spawn_test_process().await;
            session
                .set_persistent_session(format!("thread-{}", i), stdin, child, None, None)
                .await;
        }

        // All five should exist
        for i in 1..=5 {
            assert!(
                session
                    .has_persistent_session(&format!("thread-{}", i))
                    .await
            );
        }

        // Kill all sessions
        let result = session.kill_all_persistent_sessions().await;
        assert!(result.is_ok());

        // All should be gone
        for i in 1..=5 {
            assert!(
                !session
                    .has_persistent_session(&format!("thread-{}", i))
                    .await
            );
        }
    }

    #[tokio::test]
    async fn kill_all_persistent_sessions_succeeds_when_empty() {
        let session = create_test_workspace_session();

        // Kill all on empty session should succeed
        let result = session.kill_all_persistent_sessions().await;
        assert!(result.is_ok());
    }

    // ==========================================================================
    // Tests for active turns management
    // ==========================================================================

    #[tokio::test]
    async fn track_and_clear_turn_works_correctly() {
        let session = create_test_workspace_session();
        let (stdin, child) = spawn_test_process().await;

        // Need to consume stdin to avoid the child hanging
        drop(stdin);
        let child = Arc::new(Mutex::new(child));

        // Track a turn
        session
            .track_turn(
                "thread-1".to_string(),
                "turn-abc".to_string(),
                child.clone(),
            )
            .await;

        // Verify turn exists
        {
            let active_turns = session.active_turns.lock().await;
            assert!(active_turns.contains_key("thread-1"));
            assert_eq!(active_turns.get("thread-1").unwrap().turn_id, "turn-abc");
        }

        // Clear the turn with matching turn_id
        session.clear_turn("thread-1", "turn-abc").await;

        // Verify turn is removed
        {
            let active_turns = session.active_turns.lock().await;
            assert!(!active_turns.contains_key("thread-1"));
        }
    }

    #[tokio::test]
    async fn clear_turn_does_not_remove_mismatched_turn_id() {
        let session = create_test_workspace_session();
        let (stdin, child) = spawn_test_process().await;

        drop(stdin);
        let child = Arc::new(Mutex::new(child));

        // Track a turn
        session
            .track_turn("thread-1".to_string(), "turn-abc".to_string(), child)
            .await;

        // Try to clear with wrong turn_id
        session.clear_turn("thread-1", "turn-xyz").await;

        // Turn should still exist (wrong turn_id)
        {
            let active_turns = session.active_turns.lock().await;
            assert!(active_turns.contains_key("thread-1"));
        }
    }

    #[tokio::test]
    async fn interrupt_turn_removes_matching_turn() {
        let session = create_test_workspace_session();
        let (stdin, child) = spawn_test_process().await;

        drop(stdin);
        let child = Arc::new(Mutex::new(child));

        // Track a turn
        session
            .track_turn("thread-1".to_string(), "turn-abc".to_string(), child)
            .await;

        // Interrupt the turn
        let result = session.interrupt_turn("thread-1", "turn-abc").await;
        assert!(result.is_ok());

        // Turn should be removed
        {
            let active_turns = session.active_turns.lock().await;
            assert!(!active_turns.contains_key("thread-1"));
        }
    }

    #[tokio::test]
    async fn interrupt_turn_does_not_affect_mismatched_turn_id() {
        let session = create_test_workspace_session();
        let (stdin, child) = spawn_test_process().await;

        drop(stdin);
        let child = Arc::new(Mutex::new(child));

        // Track a turn
        session
            .track_turn("thread-1".to_string(), "turn-abc".to_string(), child)
            .await;

        // Try to interrupt with wrong turn_id
        let result = session.interrupt_turn("thread-1", "turn-xyz").await;
        assert!(result.is_ok());

        // Turn should still exist
        {
            let active_turns = session.active_turns.lock().await;
            assert!(active_turns.contains_key("thread-1"));
        }
    }

    #[tokio::test]
    async fn interrupt_turn_succeeds_for_unknown_thread() {
        let session = create_test_workspace_session();

        // Interrupt on nonexistent thread should succeed
        let result = session.interrupt_turn("nonexistent", "turn-abc").await;
        assert!(result.is_ok());
    }

    // ==========================================================================
    // Tests for interrupt_turn with persistent sessions
    // ==========================================================================

    #[tokio::test]
    async fn interrupt_turn_kills_persistent_session() {
        let session = create_test_workspace_session();
        let (stdin, child) = spawn_test_process().await;

        // Set up a persistent session
        session
            .set_persistent_session("thread-1".to_string(), stdin, child, None, None)
            .await;

        // Verify session exists
        assert!(session.has_persistent_session("thread-1").await);

        // Interrupt the turn (any turn_id works for persistent sessions)
        let result = session.interrupt_turn("thread-1", "any-turn-id").await;
        assert!(result.is_ok());

        // Session should be removed after interrupt
        assert!(!session.has_persistent_session("thread-1").await);
    }

    #[tokio::test]
    async fn interrupt_turn_with_pending_kills_persistent_session() {
        let session = create_test_workspace_session();
        let (stdin, child) = spawn_test_process().await;

        // Set up a persistent session
        session
            .set_persistent_session("thread-1".to_string(), stdin, child, None, None)
            .await;

        // Verify session exists
        assert!(session.has_persistent_session("thread-1").await);

        // Interrupt with "pending" turn_id (what frontend sends when no turn is active)
        let result = session.interrupt_turn("thread-1", "pending").await;
        assert!(result.is_ok());

        // Session should be removed
        assert!(!session.has_persistent_session("thread-1").await);
    }

    #[tokio::test]
    async fn interrupt_turn_prefers_active_turns_over_persistent_sessions() {
        let session = create_test_workspace_session();

        // Set up both an active turn and a persistent session for the same thread
        // (This shouldn't happen in practice, but tests the priority)
        let (stdin1, child1) = spawn_test_process().await;
        let (stdin2, child2) = spawn_test_process().await;

        // Set up persistent session
        session
            .set_persistent_session("thread-1".to_string(), stdin1, child1, None, None)
            .await;

        // Set up active turn (drop stdin to avoid hanging)
        drop(stdin2);
        let child2 = Arc::new(Mutex::new(child2));
        session
            .track_turn("thread-1".to_string(), "turn-abc".to_string(), child2)
            .await;

        // Verify both exist
        assert!(session.has_persistent_session("thread-1").await);
        {
            let active_turns = session.active_turns.lock().await;
            assert!(active_turns.contains_key("thread-1"));
        }

        // Interrupt with matching turn_id - should kill active turn only
        let result = session.interrupt_turn("thread-1", "turn-abc").await;
        assert!(result.is_ok());

        // Active turn should be removed
        {
            let active_turns = session.active_turns.lock().await;
            assert!(!active_turns.contains_key("thread-1"));
        }

        // Persistent session should still exist (we returned early after killing active turn)
        assert!(session.has_persistent_session("thread-1").await);

        // Clean up
        session.kill_persistent_session("thread-1").await.unwrap();
    }

    #[tokio::test]
    async fn interrupt_turn_does_not_kill_persistent_session_when_active_turn_has_wrong_id() {
        let session = create_test_workspace_session();

        // Set up both an active turn and a persistent session
        let (stdin1, child1) = spawn_test_process().await;
        let (stdin2, child2) = spawn_test_process().await;

        // Set up persistent session
        session
            .set_persistent_session("thread-1".to_string(), stdin1, child1, None, None)
            .await;

        // Set up active turn with specific turn_id
        drop(stdin2);
        let child2 = Arc::new(Mutex::new(child2));
        session
            .track_turn("thread-1".to_string(), "turn-abc".to_string(), child2)
            .await;

        // Interrupt with WRONG turn_id - should not kill anything
        let result = session.interrupt_turn("thread-1", "turn-xyz").await;
        assert!(result.is_ok());

        // Both should still exist
        {
            let active_turns = session.active_turns.lock().await;
            assert!(active_turns.contains_key("thread-1"));
        }
        assert!(session.has_persistent_session("thread-1").await);

        // Clean up
        session.kill_all_persistent_sessions().await.unwrap();
    }

    #[tokio::test]
    async fn interrupt_turn_is_idempotent_for_persistent_sessions() {
        let session = create_test_workspace_session();
        let (stdin, child) = spawn_test_process().await;

        // Set up a persistent session
        session
            .set_persistent_session("thread-1".to_string(), stdin, child, None, None)
            .await;

        // First interrupt
        let result1 = session.interrupt_turn("thread-1", "turn-1").await;
        assert!(result1.is_ok());
        assert!(!session.has_persistent_session("thread-1").await);

        // Second interrupt on same thread (session already gone)
        let result2 = session.interrupt_turn("thread-1", "turn-2").await;
        assert!(result2.is_ok());

        // Third interrupt
        let result3 = session.interrupt_turn("thread-1", "turn-3").await;
        assert!(result3.is_ok());
    }

    // ==========================================================================
    // Tests for build_claude_path_env
    // ==========================================================================

    #[test]
    fn build_claude_path_env_includes_standard_paths() {
        let path_env = build_claude_path_env(None);
        assert!(path_env.is_some());

        let path = path_env.unwrap();
        assert!(path.contains("/usr/bin"), "Expected /usr/bin in path: {}", path);
        assert!(path.contains("/bin"), "Expected /bin in path: {}", path);
    }

    #[test]
    fn build_claude_path_env_includes_custom_bin_parent() {
        let path_env = build_claude_path_env(Some("/custom/path/to/claude"));
        assert!(path_env.is_some());

        let path = path_env.unwrap();
        assert!(
            path.contains("/custom/path/to"),
            "Expected /custom/path/to in path: {}",
            path
        );
    }

    #[test]
    fn build_claude_path_env_ignores_empty_bin() {
        let path_env_empty = build_claude_path_env(Some(""));
        let path_env_spaces = build_claude_path_env(Some("   "));
        let path_env_none = build_claude_path_env(None);

        // All three should produce similar results (no custom path added)
        assert!(path_env_empty.is_some());
        assert!(path_env_spaces.is_some());
        assert!(path_env_none.is_some());
    }

    // ==========================================================================
    // Tests for concurrent session access
    // ==========================================================================

    #[tokio::test]
    async fn concurrent_session_operations_are_thread_safe() {
        use std::sync::Arc;
        use tokio::task::JoinSet;

        let session = Arc::new(create_test_workspace_session());
        let mut join_set = JoinSet::new();

        // Spawn 10 concurrent tasks that each create and check their own session
        for i in 0..10 {
            let session_clone = session.clone();
            let thread_id = format!("concurrent-thread-{}", i);

            join_set.spawn(async move {
                let (stdin, child) = spawn_test_process().await;

                // Set session
                session_clone
                    .set_persistent_session(thread_id.clone(), stdin, child, None, None)
                    .await;

                // Verify it exists
                assert!(session_clone.has_persistent_session(&thread_id).await);

                // Set and take pending turn ID
                session_clone
                    .set_pending_turn_id(&thread_id, format!("turn-{}", i))
                    .await;
                let turn_id = session_clone.take_pending_turn_id(&thread_id).await;
                assert_eq!(turn_id, Some(format!("turn-{}", i)));

                thread_id
            });
        }

        // Wait for all tasks to complete
        let mut completed_threads = Vec::new();
        while let Some(result) = join_set.join_next().await {
            completed_threads.push(result.unwrap());
        }

        // All 10 threads should have completed
        assert_eq!(completed_threads.len(), 10);

        // All sessions should still exist
        for i in 0..10 {
            assert!(
                session
                    .has_persistent_session(&format!("concurrent-thread-{}", i))
                    .await
            );
        }

        // Clean up
        session.kill_all_persistent_sessions().await.unwrap();
    }
}
