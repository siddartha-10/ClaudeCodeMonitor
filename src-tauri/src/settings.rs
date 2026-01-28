use serde::{Deserialize, Serialize};
use std::fs;
use std::fs::File;
use std::io::Read;
use tauri::{State, Window};

use crate::claude_config;
use crate::claude_home;
use crate::state::AppState;
use crate::storage::write_settings;
use crate::types::AppSettings;
use crate::window;

const GLOBAL_SETTINGS_FILENAME: &str = "settings.json";
const MAX_SETTINGS_SIZE: u64 = 1024 * 1024; // 1 MB

const CLAUDE_MD_FILENAME: &str = "CLAUDE.md";
const MAX_CLAUDE_MD_BYTES: u64 = 100_000;

#[derive(Serialize, Deserialize, Clone)]
pub(crate) struct GlobalClaudeSettingsResponse {
    pub exists: bool,
    pub content: String,
    pub truncated: bool,
}

#[derive(Serialize, Deserialize, Clone)]
pub(crate) struct GlobalClaudeMdResponse {
    pub exists: bool,
    pub content: String,
    pub truncated: bool,
}

#[tauri::command]
pub(crate) async fn get_app_settings(
    state: State<'_, AppState>,
    window: Window,
) -> Result<AppSettings, String> {
    let mut settings = state.app_settings.lock().await.clone();
    if let Ok(Some(collab_enabled)) = claude_config::read_collab_enabled() {
        settings.experimental_collab_enabled = collab_enabled;
    }
    if let Ok(Some(steer_enabled)) = claude_config::read_steer_enabled() {
        settings.experimental_steer_enabled = steer_enabled;
    }
    if let Ok(Some(unified_exec_enabled)) = claude_config::read_unified_exec_enabled() {
        settings.experimental_unified_exec_enabled = unified_exec_enabled;
    }
    let _ = window::apply_window_appearance(&window, settings.theme.as_str());
    Ok(settings)
}

#[tauri::command]
pub(crate) async fn update_app_settings(
    settings: AppSettings,
    state: State<'_, AppState>,
    window: Window,
) -> Result<AppSettings, String> {
    let _ = claude_config::write_collab_enabled(settings.experimental_collab_enabled);
    let _ = claude_config::write_steer_enabled(settings.experimental_steer_enabled);
    let _ = claude_config::write_unified_exec_enabled(settings.experimental_unified_exec_enabled);
    write_settings(&state.settings_path, &settings)?;
    let mut current = state.app_settings.lock().await;
    *current = settings.clone();
    let _ = window::apply_window_appearance(&window, settings.theme.as_str());
    Ok(settings)
}

#[tauri::command]
pub(crate) async fn read_global_claude_settings() -> Result<GlobalClaudeSettingsResponse, String> {
    let claude_home = claude_home::resolve_default_claude_home()
        .ok_or_else(|| "Unable to resolve Claude home directory".to_string())?;

    let settings_path = claude_home.join(GLOBAL_SETTINGS_FILENAME);

    if !settings_path.exists() {
        return Ok(GlobalClaudeSettingsResponse {
            exists: false,
            content: String::new(),
            truncated: false,
        });
    }

    let metadata = fs::metadata(&settings_path)
        .map_err(|e| format!("Failed to read settings metadata: {}", e))?;

    let truncated = metadata.len() > MAX_SETTINGS_SIZE;

    let content = if truncated {
        let bytes = fs::read(&settings_path)
            .map_err(|e| format!("Failed to read settings file: {}", e))?;
        let truncated_bytes = &bytes[..MAX_SETTINGS_SIZE as usize];
        String::from_utf8_lossy(truncated_bytes).to_string()
    } else {
        fs::read_to_string(&settings_path)
            .map_err(|e| format!("Failed to read settings file: {}", e))?
    };

    Ok(GlobalClaudeSettingsResponse {
        exists: true,
        content,
        truncated,
    })
}

#[tauri::command]
pub(crate) async fn write_global_claude_settings(content: String) -> Result<(), String> {
    let claude_home = claude_home::resolve_default_claude_home()
        .ok_or_else(|| "Unable to resolve Claude home directory".to_string())?;

    // Create directory if it doesn't exist
    if !claude_home.exists() {
        fs::create_dir_all(&claude_home)
            .map_err(|e| format!("Failed to create Claude home directory: {}", e))?;
    }

    let settings_path = claude_home.join(GLOBAL_SETTINGS_FILENAME);

    fs::write(&settings_path, content)
        .map_err(|e| format!("Failed to write settings file: {}", e))?;

    Ok(())
}

#[tauri::command]
pub(crate) async fn read_global_claude_md() -> Result<GlobalClaudeMdResponse, String> {
    let claude_home = claude_home::resolve_default_claude_home()
        .ok_or_else(|| "Unable to resolve Claude home directory".to_string())?;

    let claude_md_path = claude_home.join(CLAUDE_MD_FILENAME);

    if !claude_md_path.exists() {
        return Ok(GlobalClaudeMdResponse {
            exists: false,
            content: String::new(),
            truncated: false,
        });
    }

    let file = File::open(&claude_md_path)
        .map_err(|e| format!("Failed to open CLAUDE.md: {}", e))?;
    let mut buffer = Vec::new();
    file.take(MAX_CLAUDE_MD_BYTES + 1)
        .read_to_end(&mut buffer)
        .map_err(|e| format!("Failed to read CLAUDE.md: {}", e))?;

    let truncated = buffer.len() > MAX_CLAUDE_MD_BYTES as usize;
    if truncated {
        buffer.truncate(MAX_CLAUDE_MD_BYTES as usize);
    }

    let content = String::from_utf8(buffer)
        .map_err(|_| "CLAUDE.md is not valid UTF-8".to_string())?;

    Ok(GlobalClaudeMdResponse {
        exists: true,
        content,
        truncated,
    })
}

#[tauri::command]
pub(crate) async fn write_global_claude_md(content: String) -> Result<(), String> {
    let claude_home = claude_home::resolve_default_claude_home()
        .ok_or_else(|| "Unable to resolve Claude home directory".to_string())?;

    // Create directory if it doesn't exist
    if !claude_home.exists() {
        fs::create_dir_all(&claude_home)
            .map_err(|e| format!("Failed to create Claude home directory: {}", e))?;
    }

    let claude_md_path = claude_home.join(CLAUDE_MD_FILENAME);

    fs::write(&claude_md_path, content)
        .map_err(|e| format!("Failed to write CLAUDE.md: {}", e))?;

    Ok(())
}
