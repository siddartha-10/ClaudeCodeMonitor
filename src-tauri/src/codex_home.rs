use std::env;
use std::path::PathBuf;

use crate::types::WorkspaceEntry;

pub(crate) fn resolve_workspace_claude_home(
    entry: &WorkspaceEntry,
    parent_path: Option<&str>,
) -> Option<PathBuf> {
    if entry.kind.is_worktree() {
        if let Some(parent_path) = parent_path {
            let project_home = PathBuf::from(parent_path).join(".claude");
            if project_home.is_dir() {
                return Some(project_home);
            }
            let legacy_home = PathBuf::from(parent_path).join(".codexmonitor");
            if legacy_home.is_dir() {
                return Some(legacy_home);
            }
        }
    }
    let project_home = PathBuf::from(&entry.path).join(".claude");
    if project_home.is_dir() {
        return Some(project_home);
    }
    let legacy_home = PathBuf::from(&entry.path).join(".codexmonitor");
    if legacy_home.is_dir() {
        return Some(legacy_home);
    }
    None
}

pub(crate) fn resolve_default_claude_home() -> Option<PathBuf> {
    if let Ok(value) = env::var("CLAUDE_HOME") {
        if !value.trim().is_empty() {
            return Some(PathBuf::from(value.trim()));
        }
    }
    if let Ok(value) = env::var("CODEX_HOME") {
        if !value.trim().is_empty() {
            return Some(PathBuf::from(value.trim()));
        }
    }
    resolve_home_dir().map(|home| home.join(".claude"))
}

fn resolve_home_dir() -> Option<PathBuf> {
    if let Ok(value) = env::var("HOME") {
        if !value.trim().is_empty() {
            return Some(PathBuf::from(value));
        }
    }
    if let Ok(value) = env::var("USERPROFILE") {
        if !value.trim().is_empty() {
            return Some(PathBuf::from(value));
        }
    }
    None
}
