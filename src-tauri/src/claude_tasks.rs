use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

use crate::claude_home::resolve_default_claude_home;

/// A task from Claude's task system stored in ~/.claude/tasks/<session-id>/<task-id>.json
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeTask {
    pub id: String,
    pub subject: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub active_form: Option<String>,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub blocks: Vec<String>,
    #[serde(default)]
    pub blocked_by: Vec<String>,
}

/// Response containing all tasks for a session
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeTasksResponse {
    pub session_id: String,
    pub tasks: Vec<ClaudeTask>,
}

/// Get the tasks directory path for a given session
fn get_tasks_dir(session_id: &str) -> Option<PathBuf> {
    let claude_home = resolve_default_claude_home()?;
    let tasks_dir = claude_home.join("tasks").join(session_id);
    if tasks_dir.is_dir() {
        Some(tasks_dir)
    } else {
        None
    }
}

/// Read all tasks for a given session (thread) ID
#[tauri::command]
pub async fn get_claude_tasks(session_id: String) -> Result<ClaudeTasksResponse, String> {
    let session_id_clone = session_id.clone();
    
    tokio::task::spawn_blocking(move || {
        let tasks_dir = match get_tasks_dir(&session_id_clone) {
            Some(dir) => dir,
            None => {
                return Ok(ClaudeTasksResponse {
                    session_id: session_id_clone,
                    tasks: Vec::new(),
                });
            }
        };

        let mut tasks: Vec<ClaudeTask> = Vec::new();

        let entries = fs::read_dir(&tasks_dir).map_err(|e| e.to_string())?;

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "json") {
                match fs::read_to_string(&path) {
                    Ok(content) => {
                        match serde_json::from_str::<ClaudeTask>(&content) {
                            Ok(task) => tasks.push(task),
                            Err(e) => {
                                eprintln!("Failed to parse task file {:?}: {}", path, e);
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("Failed to read task file {:?}: {}", path, e);
                    }
                }
            }
        }

        // Sort tasks by ID (numeric sort)
        tasks.sort_by(|a, b| {
            let a_num: i32 = a.id.parse().unwrap_or(0);
            let b_num: i32 = b.id.parse().unwrap_or(0);
            a_num.cmp(&b_num)
        });

        Ok(ClaudeTasksResponse {
            session_id: session_id_clone,
            tasks,
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_task_json() {
        let json = r#"{
            "id": "1",
            "subject": "Test task",
            "description": "A test task description",
            "activeForm": "Testing",
            "status": "pending",
            "blocks": ["2"],
            "blockedBy": []
        }"#;

        let task: ClaudeTask = serde_json::from_str(json).unwrap();
        assert_eq!(task.id, "1");
        assert_eq!(task.subject, "Test task");
        assert_eq!(task.status, "pending");
        assert_eq!(task.blocks, vec!["2"]);
        assert!(task.blocked_by.is_empty());
    }

    #[test]
    fn test_parse_minimal_task_json() {
        let json = r#"{
            "id": "1",
            "subject": "Minimal task"
        }"#;

        let task: ClaudeTask = serde_json::from_str(json).unwrap();
        assert_eq!(task.id, "1");
        assert_eq!(task.subject, "Minimal task");
        assert_eq!(task.status, "");
        assert!(task.blocks.is_empty());
    }
}
