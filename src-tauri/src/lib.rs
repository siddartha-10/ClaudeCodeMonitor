use tauri::{Manager, RunEvent, WindowEvent};

mod backend;
mod claude;
mod claude_tasks;
mod claude_home;
mod claude_config;
mod file_io;
mod file_ops;
mod file_policy;
mod files;
mod task_manager;
#[cfg(not(target_os = "windows"))]
#[path = "dictation.rs"]
mod dictation;
#[cfg(target_os = "windows")]
#[path = "dictation_stub.rs"]
mod dictation;
mod event_sink;
mod git;
mod git_utils;
mod local_usage;
mod menu;
mod prompts;
mod remote_backend;
mod settings;
mod state;
mod terminal;
mod window;
mod storage;
mod task_watcher;
mod types;
mod utils;
mod workspaces;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(target_os = "linux")]
    {
        // Avoid WebKit compositing issues on some Linux setups (GBM buffer errors).
        if std::env::var_os("WEBKIT_DISABLE_COMPOSITING_MODE").is_none() {
            std::env::set_var("WEBKIT_DISABLE_COMPOSITING_MODE", "1");
        }
    }

    let builder = tauri::Builder::default()
        .enable_macos_default_menu(false)
        .manage(menu::MenuItemRegistry::<tauri::Wry>::default())
        .menu(menu::build_menu)
        .on_menu_event(menu::handle_menu_event)
        .on_window_event(|window, event| {
            if window.label() != "main" {
                return;
            }
            #[cfg(target_os = "macos")]
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .setup(|app| {
            let state = state::AppState::load(&app.handle());
            app.manage(state);
            app.manage(task_watcher::TaskWatcherState::default());
            #[cfg(desktop)]
            {
                app.handle()
                    .plugin(tauri_plugin_updater::Builder::new().build())?;
            }
            Ok(())
        });

    #[cfg(desktop)]
    let builder = builder.plugin(tauri_plugin_window_state::Builder::default().build());

    let app = builder
        .plugin(tauri_plugin_liquid_glass::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_process::init())
        .invoke_handler(tauri::generate_handler![
            settings::get_app_settings,
            settings::update_app_settings,
            menu::menu_set_accelerators,
            claude::claude_doctor,
            workspaces::list_workspaces,
            workspaces::is_workspace_path_dir,
            workspaces::add_workspace,
            workspaces::add_clone,
            workspaces::add_worktree,
            workspaces::remove_workspace,
            workspaces::remove_worktree,
            workspaces::rename_worktree,
            workspaces::rename_worktree_upstream,
            workspaces::apply_worktree_changes,
            workspaces::update_workspace_settings,
            workspaces::update_workspace_claude_bin,
            claude::start_thread,
            claude::send_user_message,
            claude::turn_interrupt,
            claude::start_review,
            claude::respond_to_server_request,
            claude::remember_approval_rule,
            claude::get_commit_message_prompt,
            claude::generate_commit_message,
            claude::generate_run_metadata,
            claude::resume_thread,
            claude::fork_thread_from_message,
            claude::rewind_thread_files,
            claude::list_threads,
            claude::search_thread,
            claude::archive_thread,
            claude::collaboration_mode_list,
            workspaces::connect_workspace,
            git::get_git_status,
            git::list_git_roots,
            git::get_git_diffs,
            git::get_git_log,
            git::get_git_commit_diff,
            git::get_git_remote,
            git::stage_git_file,
            git::stage_git_all,
            git::unstage_git_file,
            git::revert_git_file,
            git::revert_git_all,
            git::commit_git,
            git::push_git,
            git::pull_git,
            git::sync_git,
            git::get_github_issues,
            git::get_github_pull_requests,
            git::get_github_pull_request_diff,
            git::get_github_pull_request_comments,
            workspaces::list_workspace_files,
            workspaces::read_workspace_file,
            workspaces::open_workspace_in,
            git::list_git_branches,
            git::checkout_git_branch,
            git::create_git_branch,
            claude::model_list,
            claude::global_rate_limits,
            claude::skills_list,
            prompts::prompts_list,
            prompts::prompts_create,
            prompts::prompts_update,
            prompts::prompts_delete,
            prompts::prompts_move,
            prompts::prompts_workspace_dir,
            prompts::prompts_global_dir,
            terminal::terminal_open,
            terminal::terminal_write,
            terminal::terminal_resize,
            terminal::terminal_close,
            dictation::dictation_model_status,
            dictation::dictation_download_model,
            dictation::dictation_cancel_download,
            dictation::dictation_remove_model,
            dictation::dictation_start,
            dictation::dictation_request_permission,
            dictation::dictation_stop,
            dictation::dictation_cancel,
            local_usage::local_usage_snapshot,
            claude_tasks::get_claude_tasks,
            task_watcher::task_watcher_start,
            task_watcher::task_watcher_stop,
            task_manager::task_create,
            task_manager::task_read,
            task_manager::task_list_read,
            task_manager::task_update,
            task_manager::task_delete,
            task_manager::task_lists_available,
            files::file_read,
            files::file_write
        ])
        .build(tauri::generate_context!())
        .expect("error while running tauri application");

    app.run(|app_handle, event| {
        if let RunEvent::Reopen { .. } = event {
            if let Some(window) = app_handle.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }
    });
}
