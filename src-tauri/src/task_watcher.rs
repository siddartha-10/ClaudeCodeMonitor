//! File watcher for Claude task lists.
//!
//! Watches the ~/.claude/tasks/<list-id>/ directory for changes and emits
//! Tauri events when task files are created, modified, or deleted.

use notify::RecursiveMode;
use notify_debouncer_mini::new_debouncer;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::{mpsc, Mutex};

use crate::claude_home::resolve_default_claude_home;

/// Holds the shutdown sender for a task watcher
pub struct TaskWatcher {
    /// Send true to this channel to stop the watcher
    shutdown_tx: mpsc::Sender<()>,
}

impl TaskWatcher {
    /// Stop the watcher and clean up resources
    pub async fn stop(self) {
        let _ = self.shutdown_tx.send(()).await;
    }
}

/// State for managing active task watchers
pub struct TaskWatcherState {
    watchers: Mutex<HashMap<String, TaskWatcher>>,
}

impl Default for TaskWatcherState {
    fn default() -> Self {
        Self {
            watchers: Mutex::new(HashMap::new()),
        }
    }
}

/// Get the tasks directory path for a given list ID
fn get_tasks_dir(list_id: &str) -> Option<PathBuf> {
    let claude_home = resolve_default_claude_home()?;
    Some(claude_home.join("tasks").join(list_id))
}

/// Start watching a task list directory for changes.
///
/// Emits "task-list-changed:<list-id>" events when .json files change.
#[tauri::command]
pub async fn task_watcher_start(list_id: String, app_handle: AppHandle) -> Result<(), String> {
    let state = app_handle.state::<TaskWatcherState>();
    let mut watchers = state.watchers.lock().await;

    // Check if already watching this list
    if watchers.contains_key(&list_id) {
        return Ok(()); // Already watching
    }

    let tasks_dir = get_tasks_dir(&list_id).ok_or_else(|| {
        "Could not resolve Claude home directory".to_string()
    })?;

    // Create the directory if it doesn't exist
    if !tasks_dir.exists() {
        std::fs::create_dir_all(&tasks_dir).map_err(|e| {
            format!("Failed to create tasks directory: {}", e)
        })?;
    }

    let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);
    let list_id_clone = list_id.clone();
    let app_handle_clone = app_handle.clone();
    let tasks_dir_clone = tasks_dir.clone();

    // Spawn the watcher task
    tokio::spawn(async move {
        let event_name = format!("task-list-changed:{}", list_id_clone);

        // Create a channel for debounced events
        let (tx, rx) = std::sync::mpsc::channel();

        // Create debouncer with 100ms delay
        let mut debouncer = match new_debouncer(Duration::from_millis(100), tx) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("Failed to create task watcher debouncer: {}", e);
                return;
            }
        };

        // Start watching the directory
        if let Err(e) = debouncer.watcher().watch(&tasks_dir_clone, RecursiveMode::NonRecursive) {
            eprintln!("Failed to watch tasks directory {:?}: {}", tasks_dir_clone, e);
            return;
        }

        println!("Started watching tasks directory: {:?}", tasks_dir_clone);

        // Process events in a loop
        loop {
            tokio::select! {
                _ = shutdown_rx.recv() => {
                    println!("Stopping task watcher for list: {}", list_id_clone);
                    break;
                }
                _ = tokio::time::sleep(Duration::from_millis(50)) => {
                    // Check for events from the debouncer
                    match rx.try_recv() {
                        Ok(Ok(events)) => {
                            // Filter to only .json file changes (ignore .lock files)
                            let has_json_change = events.iter().any(|event| {
                                event.path.extension()
                                    .map(|ext| ext == "json")
                                    .unwrap_or(false)
                            });

                            if has_json_change {
                                println!("Task list changed: {}", list_id_clone);
                                if let Err(e) = app_handle_clone.emit(&event_name, ()) {
                                    eprintln!("Failed to emit task-list-changed event: {}", e);
                                }
                            }
                        }
                        Ok(Err(error)) => {
                            eprintln!("Task watcher error: {:?}", error);
                        }
                        Err(std::sync::mpsc::TryRecvError::Empty) => {
                            // No events, continue
                        }
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                            println!("Task watcher channel disconnected for list: {}", list_id_clone);
                            break;
                        }
                    }
                }
            }
        }
    });

    // Store the watcher
    watchers.insert(
        list_id.clone(),
        TaskWatcher {
            shutdown_tx,
        },
    );

    Ok(())
}

/// Stop watching a task list directory.
#[tauri::command]
pub async fn task_watcher_stop(list_id: String, app_handle: AppHandle) -> Result<(), String> {
    let state = app_handle.state::<TaskWatcherState>();
    let mut watchers = state.watchers.lock().await;

    if let Some(watcher) = watchers.remove(&list_id) {
        watcher.stop().await;
        println!("Stopped task watcher for list: {}", list_id);
    }

    Ok(())
}

/// Stop all active task watchers. Called on app shutdown.
#[allow(dead_code)]
pub async fn stop_all_watchers(app_handle: &AppHandle) {
    let state = app_handle.state::<TaskWatcherState>();
    let mut watchers = state.watchers.lock().await;

    for (list_id, watcher) in watchers.drain() {
        let _ = watcher.shutdown_tx.send(()).await;
        println!("Stopped task watcher for list: {}", list_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_tasks_dir() {
        // This test verifies the path construction logic
        // The actual directory existence depends on the environment
        let list_id = "test-list-123";
        let result = get_tasks_dir(list_id);

        // Should return Some path (assuming HOME is set)
        if let Some(path) = result {
            assert!(path.to_string_lossy().contains("tasks"));
            assert!(path.to_string_lossy().contains(list_id));
        }
    }
}
