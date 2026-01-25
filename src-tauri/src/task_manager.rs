use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

use crate::claude_home::resolve_default_claude_home;

/// Task status enum representing the lifecycle of a task
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    InProgress,
    Completed,
}

impl Default for TaskStatus {
    fn default() -> Self {
        TaskStatus::Pending
    }
}

/// A task from Claude's task system stored in ~/.claude/tasks/<list-id>/<task-id>.json
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: String,
    pub subject: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_form: Option<String>,
    #[serde(default)]
    pub status: TaskStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    #[serde(default)]
    pub blocks: Vec<String>,
    #[serde(default)]
    pub blocked_by: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

/// Partial update structure for updating task fields
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskUpdate {
    pub subject: Option<String>,
    pub description: Option<String>,
    pub active_form: Option<String>,
    pub status: Option<TaskStatus>,
    pub owner: Option<String>,
    pub add_blocks: Option<Vec<String>>,
    pub add_blocked_by: Option<Vec<String>>,
    pub metadata: Option<serde_json::Value>,
}

/// Response containing all tasks for a list
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskListResponse {
    pub list_id: String,
    pub tasks: Vec<Task>,
}

/// Get the base tasks directory (~/.claude/tasks/)
fn get_tasks_dir() -> Result<PathBuf, String> {
    let claude_home = resolve_default_claude_home()
        .ok_or_else(|| "Could not resolve Claude home directory".to_string())?;
    Ok(claude_home.join("tasks"))
}

/// Get the directory for a specific task list
fn get_task_list_dir(list_id: &str) -> Result<PathBuf, String> {
    let tasks_dir = get_tasks_dir()?;
    Ok(tasks_dir.join(list_id))
}

/// Get the path to a specific task file
fn get_task_file_path(list_id: &str, task_id: &str) -> Result<PathBuf, String> {
    let list_dir = get_task_list_dir(list_id)?;
    Ok(list_dir.join(format!("{}.json", task_id)))
}

/// Get the path to a lock file for a task list
fn get_lock_file_path(list_id: &str) -> Result<PathBuf, String> {
    let list_dir = get_task_list_dir(list_id)?;
    Ok(list_dir.join(".lock"))
}

/// Simple file-based lock for basic concurrency control
struct FileLock {
    path: PathBuf,
}

impl FileLock {
    fn acquire(list_id: &str) -> Result<Self, String> {
        let path = get_lock_file_path(list_id)?;

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("Failed to create lock directory: {}", e))?;
        }

        // Try to create lock file (simple approach - not bulletproof but sufficient for most cases)
        let mut attempts = 0;
        const MAX_ATTEMPTS: u32 = 50;
        const SLEEP_MS: u64 = 100;

        while attempts < MAX_ATTEMPTS {
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(_) => return Ok(FileLock { path }),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    // Check if lock file is stale (older than 30 seconds)
                    if let Ok(metadata) = fs::metadata(&path) {
                        if let Ok(modified) = metadata.modified() {
                            if modified.elapsed().unwrap_or_default().as_secs() > 30 {
                                // Remove stale lock
                                let _ = fs::remove_file(&path);
                            }
                        }
                    }
                    attempts += 1;
                    std::thread::sleep(std::time::Duration::from_millis(SLEEP_MS));
                }
                Err(e) => return Err(format!("Failed to acquire lock: {}", e)),
            }
        }

        Err("Failed to acquire lock after maximum attempts".to_string())
    }
}

impl Drop for FileLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// Find the next available task ID for a list
fn next_task_id(list_id: &str) -> Result<String, String> {
    let list_dir = get_task_list_dir(list_id)?;

    if !list_dir.exists() {
        return Ok("1".to_string());
    }

    let entries = fs::read_dir(&list_dir).map_err(|e| e.to_string())?;

    let mut max_id: u32 = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "json") {
            if let Some(stem) = path.file_stem() {
                if let Some(stem_str) = stem.to_str() {
                    if let Ok(id) = stem_str.parse::<u32>() {
                        max_id = max_id.max(id);
                    }
                }
            }
        }
    }

    Ok((max_id + 1).to_string())
}

/// Create a new task in the specified list
pub fn create_task(
    list_id: &str,
    subject: String,
    description: String,
    active_form: Option<String>,
) -> Result<Task, String> {
    let _lock = FileLock::acquire(list_id)?;

    let list_dir = get_task_list_dir(list_id)?;
    fs::create_dir_all(&list_dir).map_err(|e| format!("Failed to create task list directory: {}", e))?;

    let task_id = next_task_id(list_id)?;

    let task = Task {
        id: task_id.clone(),
        subject,
        description,
        active_form,
        status: TaskStatus::Pending,
        owner: None,
        blocks: Vec::new(),
        blocked_by: Vec::new(),
        metadata: None,
    };

    let task_path = get_task_file_path(list_id, &task_id)?;
    let data = serde_json::to_string_pretty(&task).map_err(|e| e.to_string())?;
    fs::write(&task_path, data).map_err(|e| format!("Failed to write task file: {}", e))?;

    Ok(task)
}

/// Read a single task from a list
pub fn read_task(list_id: &str, task_id: &str) -> Result<Task, String> {
    let task_path = get_task_file_path(list_id, task_id)?;

    if !task_path.exists() {
        return Err(format!("Task {} not found in list {}", task_id, list_id));
    }

    let content = fs::read_to_string(&task_path)
        .map_err(|e| format!("Failed to read task file: {}", e))?;

    serde_json::from_str(&content)
        .map_err(|e| format!("Failed to parse task file: {}", e))
}

/// Read all tasks in a list
pub fn read_task_list(list_id: &str) -> Result<Vec<Task>, String> {
    let list_dir = get_task_list_dir(list_id)?;

    if !list_dir.exists() {
        return Ok(Vec::new());
    }

    let entries = fs::read_dir(&list_dir).map_err(|e| e.to_string())?;

    let mut tasks: Vec<Task> = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "json") {
            match fs::read_to_string(&path) {
                Ok(content) => {
                    match serde_json::from_str::<Task>(&content) {
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

    Ok(tasks)
}

/// Update an existing task with partial updates
pub fn update_task(list_id: &str, task_id: &str, updates: TaskUpdate) -> Result<Task, String> {
    let _lock = FileLock::acquire(list_id)?;

    let mut task = read_task(list_id, task_id)?;

    // Apply updates
    if let Some(subject) = updates.subject {
        task.subject = subject;
    }
    if let Some(description) = updates.description {
        task.description = description;
    }
    if let Some(active_form) = updates.active_form {
        task.active_form = Some(active_form);
    }
    if let Some(status) = updates.status {
        task.status = status;
    }
    if let Some(owner) = updates.owner {
        task.owner = Some(owner);
    }
    if let Some(add_blocks) = updates.add_blocks {
        for block_id in add_blocks {
            if !task.blocks.contains(&block_id) {
                task.blocks.push(block_id);
            }
        }
    }
    if let Some(add_blocked_by) = updates.add_blocked_by {
        for blocked_by_id in add_blocked_by {
            if !task.blocked_by.contains(&blocked_by_id) {
                task.blocked_by.push(blocked_by_id);
            }
        }
    }
    if let Some(metadata) = updates.metadata {
        // Merge metadata - if existing metadata exists, merge the new keys
        match (&mut task.metadata, metadata) {
            (Some(existing), new_metadata) => {
                if let (Some(existing_obj), Some(new_obj)) = (existing.as_object_mut(), new_metadata.as_object()) {
                    for (key, value) in new_obj {
                        if value.is_null() {
                            existing_obj.remove(key);
                        } else {
                            existing_obj.insert(key.clone(), value.clone());
                        }
                    }
                }
            }
            (None, new_metadata) => {
                // Filter out null values for new metadata
                if let Some(obj) = new_metadata.as_object() {
                    let filtered: serde_json::Map<String, serde_json::Value> = obj
                        .iter()
                        .filter(|(_, v)| !v.is_null())
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect();
                    if !filtered.is_empty() {
                        task.metadata = Some(serde_json::Value::Object(filtered));
                    }
                }
            }
        }
    }

    // Write updated task back
    let task_path = get_task_file_path(list_id, task_id)?;
    let data = serde_json::to_string_pretty(&task).map_err(|e| e.to_string())?;
    fs::write(&task_path, data).map_err(|e| format!("Failed to write task file: {}", e))?;

    Ok(task)
}

/// Delete a task from a list
pub fn delete_task(list_id: &str, task_id: &str) -> Result<(), String> {
    let _lock = FileLock::acquire(list_id)?;

    let task_path = get_task_file_path(list_id, task_id)?;

    if !task_path.exists() {
        return Err(format!("Task {} not found in list {}", task_id, list_id));
    }

    fs::remove_file(&task_path).map_err(|e| format!("Failed to delete task file: {}", e))
}

/// List all available task lists
pub fn list_all_task_lists() -> Result<Vec<String>, String> {
    let tasks_dir = get_tasks_dir()?;

    if !tasks_dir.exists() {
        return Ok(Vec::new());
    }

    let entries = fs::read_dir(&tasks_dir).map_err(|e| e.to_string())?;

    let mut list_ids: Vec<String> = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(name) = path.file_name() {
                if let Some(name_str) = name.to_str() {
                    // Skip hidden directories
                    if !name_str.starts_with('.') {
                        list_ids.push(name_str.to_string());
                    }
                }
            }
        }
    }

    list_ids.sort();
    Ok(list_ids)
}

// ============================================================================
// Tauri Commands
// ============================================================================

/// Create a new task in the specified list
#[tauri::command]
pub async fn task_create(
    list_id: String,
    subject: String,
    description: String,
    active_form: Option<String>,
) -> Result<Task, String> {
    tokio::task::spawn_blocking(move || {
        create_task(&list_id, subject, description, active_form)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Read a single task from a list
#[tauri::command]
pub async fn task_read(list_id: String, task_id: String) -> Result<Task, String> {
    tokio::task::spawn_blocking(move || {
        read_task(&list_id, &task_id)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Read all tasks in a list
#[tauri::command]
pub async fn task_list_read(list_id: String) -> Result<TaskListResponse, String> {
    let list_id_clone = list_id.clone();
    tokio::task::spawn_blocking(move || {
        let tasks = read_task_list(&list_id_clone)?;
        Ok(TaskListResponse {
            list_id: list_id_clone,
            tasks,
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Update an existing task with partial updates
#[tauri::command]
pub async fn task_update(
    list_id: String,
    task_id: String,
    updates: TaskUpdate,
) -> Result<Task, String> {
    tokio::task::spawn_blocking(move || {
        update_task(&list_id, &task_id, updates)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Delete a task from a list
#[tauri::command]
pub async fn task_delete(list_id: String, task_id: String) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        delete_task(&list_id, &task_id)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// List all available task lists
#[tauri::command]
pub async fn task_lists_available() -> Result<Vec<String>, String> {
    tokio::task::spawn_blocking(list_all_task_lists)
        .await
        .map_err(|e| e.to_string())?
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_task_status_serialization() {
        let pending = TaskStatus::Pending;
        let json = serde_json::to_string(&pending).unwrap();
        assert_eq!(json, "\"pending\"");

        let in_progress = TaskStatus::InProgress;
        let json = serde_json::to_string(&in_progress).unwrap();
        assert_eq!(json, "\"in_progress\"");

        let completed = TaskStatus::Completed;
        let json = serde_json::to_string(&completed).unwrap();
        assert_eq!(json, "\"completed\"");
    }

    #[test]
    fn test_task_status_deserialization() {
        let pending: TaskStatus = serde_json::from_str("\"pending\"").unwrap();
        assert_eq!(pending, TaskStatus::Pending);

        let in_progress: TaskStatus = serde_json::from_str("\"in_progress\"").unwrap();
        assert_eq!(in_progress, TaskStatus::InProgress);

        let completed: TaskStatus = serde_json::from_str("\"completed\"").unwrap();
        assert_eq!(completed, TaskStatus::Completed);
    }

    #[test]
    fn test_task_serialization() {
        let task = Task {
            id: "1".to_string(),
            subject: "Test task".to_string(),
            description: "A test description".to_string(),
            active_form: Some("Testing".to_string()),
            status: TaskStatus::Pending,
            owner: None,
            blocks: vec!["2".to_string()],
            blocked_by: Vec::new(),
            metadata: None,
        };

        let json = serde_json::to_string_pretty(&task).unwrap();
        assert!(json.contains("\"id\": \"1\""));
        assert!(json.contains("\"subject\": \"Test task\""));
        assert!(json.contains("\"status\": \"pending\""));
        assert!(json.contains("\"activeForm\": \"Testing\""));
        assert!(!json.contains("\"owner\"")); // Should be skipped
        assert!(!json.contains("\"metadata\"")); // Should be skipped
    }

    #[test]
    fn test_task_deserialization() {
        let json = r#"{
            "id": "1",
            "subject": "Test task",
            "description": "A test task description",
            "activeForm": "Testing",
            "status": "in_progress",
            "owner": "agent-1",
            "blocks": ["2", "3"],
            "blockedBy": ["0"],
            "metadata": {"key": "value"}
        }"#;

        let task: Task = serde_json::from_str(json).unwrap();
        assert_eq!(task.id, "1");
        assert_eq!(task.subject, "Test task");
        assert_eq!(task.description, "A test task description");
        assert_eq!(task.active_form, Some("Testing".to_string()));
        assert_eq!(task.status, TaskStatus::InProgress);
        assert_eq!(task.owner, Some("agent-1".to_string()));
        assert_eq!(task.blocks, vec!["2", "3"]);
        assert_eq!(task.blocked_by, vec!["0"]);
        assert!(task.metadata.is_some());
    }

    #[test]
    fn test_task_deserialization_minimal() {
        let json = r#"{
            "id": "1",
            "subject": "Minimal task",
            "description": ""
        }"#;

        let task: Task = serde_json::from_str(json).unwrap();
        assert_eq!(task.id, "1");
        assert_eq!(task.subject, "Minimal task");
        assert_eq!(task.status, TaskStatus::Pending);
        assert!(task.blocks.is_empty());
        assert!(task.blocked_by.is_empty());
        assert!(task.owner.is_none());
    }

    #[test]
    fn test_task_update_deserialization() {
        let json = r#"{
            "status": "completed",
            "addBlocks": ["5"]
        }"#;

        let update: TaskUpdate = serde_json::from_str(json).unwrap();
        assert_eq!(update.status, Some(TaskStatus::Completed));
        assert_eq!(update.add_blocks, Some(vec!["5".to_string()]));
        assert!(update.subject.is_none());
        assert!(update.description.is_none());
    }
}
