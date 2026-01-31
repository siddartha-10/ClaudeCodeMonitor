use std::path::PathBuf;

use crate::file_io::{read_text_file_within, write_text_file_within, TextFileResponse};
use crate::file_policy::FilePolicy;

pub(crate) fn read_with_policy(root: &PathBuf, policy: FilePolicy) -> Result<TextFileResponse, String> {
    read_text_file_within(
        root,
        policy.filename,
        policy.root_may_be_missing,
        policy.root_context,
        policy.filename,
    )
}

pub(crate) fn write_with_policy(
    root: &PathBuf,
    policy: FilePolicy,
    content: &str,
) -> Result<(), String> {
    write_text_file_within(
        root,
        policy.filename,
        content,
        policy.create_root,
        policy.root_context,
        policy.filename,
    )
}

#[cfg(test)]
mod tests {
    use std::fs;

    use uuid::Uuid;

    use crate::file_policy::{policy_for, FileKind, FileScope};

    use super::{read_with_policy, write_with_policy};

    fn temp_dir(prefix: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("claude-monitor-{prefix}-{}", Uuid::new_v4()));
        if dir.exists() {
            let _ = fs::remove_dir_all(&dir);
        }
        dir
    }

    #[test]
    fn workspace_claude_md_round_trip_requires_existing_root() {
        let root = temp_dir("workspace-claude-md");
        fs::create_dir_all(&root).expect("create workspace root");
        let policy = policy_for(FileScope::Workspace, FileKind::ClaudeMd).expect("policy");

        write_with_policy(&root, policy, "workspace claude md").expect("write claude md");
        let response = read_with_policy(&root, policy).expect("read claude md");

        assert!(response.exists);
        assert_eq!(response.content, "workspace claude md");
        assert!(!response.truncated);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn workspace_claude_md_write_fails_when_root_missing() {
        let root = temp_dir("workspace-missing-root");
        let policy = policy_for(FileScope::Workspace, FileKind::ClaudeMd).expect("policy");

        let result = write_with_policy(&root, policy, "should fail");
        assert!(result.is_err());
    }

    #[test]
    fn global_claude_md_write_creates_root() {
        let root = temp_dir("global-claude-md");
        let policy = policy_for(FileScope::Global, FileKind::ClaudeMd).expect("policy");

        let initial = read_with_policy(&root, policy).expect("initial read");
        assert!(!initial.exists);

        write_with_policy(&root, policy, "global claude md").expect("write claude md");
        let response = read_with_policy(&root, policy).expect("read claude md");

        assert!(response.exists);
        assert_eq!(response.content, "global claude md");
        assert!(!response.truncated);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn global_settings_write_creates_root() {
        let root = temp_dir("global-settings");
        let policy = policy_for(FileScope::Global, FileKind::Settings).expect("policy");

        write_with_policy(&root, policy, "{\"theme\": \"dark\"}\n").expect("write settings");
        let response = read_with_policy(&root, policy).expect("read settings");

        assert!(response.exists);
        assert!(response.content.contains("\"theme\""));
        assert!(!response.truncated);

        let _ = fs::remove_dir_all(&root);
    }
}
