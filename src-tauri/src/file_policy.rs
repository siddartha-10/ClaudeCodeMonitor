use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FileScope {
    Workspace,
    Global,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FileKind {
    ClaudeMd,
    Settings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FilePolicy {
    pub(crate) filename: &'static str,
    pub(crate) root_context: &'static str,
    pub(crate) root_may_be_missing: bool,
    pub(crate) create_root: bool,
}

const CLAUDE_MD_FILENAME: &str = "CLAUDE.md";
const SETTINGS_FILENAME: &str = "settings.json";

pub(crate) fn policy_for(scope: FileScope, kind: FileKind) -> Result<FilePolicy, String> {
    match (scope, kind) {
        (FileScope::Workspace, FileKind::ClaudeMd) => Ok(FilePolicy {
            filename: CLAUDE_MD_FILENAME,
            root_context: "workspace root",
            root_may_be_missing: false,
            create_root: false,
        }),
        (FileScope::Global, FileKind::ClaudeMd) => Ok(FilePolicy {
            filename: CLAUDE_MD_FILENAME,
            root_context: "CLAUDE_HOME",
            root_may_be_missing: true,
            create_root: true,
        }),
        (FileScope::Global, FileKind::Settings) => Ok(FilePolicy {
            filename: SETTINGS_FILENAME,
            root_context: "CLAUDE_HOME",
            root_may_be_missing: true,
            create_root: true,
        }),
        (FileScope::Workspace, FileKind::Settings) => {
            Err("settings.json is only supported for global scope".to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{policy_for, FileKind, FileScope};

    #[test]
    fn workspace_claude_md_policy_is_strict() {
        let policy = policy_for(FileScope::Workspace, FileKind::ClaudeMd).expect("policy");
        assert_eq!(policy.filename, "CLAUDE.md");
        assert_eq!(policy.root_context, "workspace root");
        assert!(!policy.root_may_be_missing);
        assert!(!policy.create_root);
    }

    #[test]
    fn global_claude_md_policy_creates_root() {
        let policy = policy_for(FileScope::Global, FileKind::ClaudeMd).expect("policy");
        assert_eq!(policy.filename, "CLAUDE.md");
        assert_eq!(policy.root_context, "CLAUDE_HOME");
        assert!(policy.root_may_be_missing);
        assert!(policy.create_root);
    }

    #[test]
    fn global_settings_policy_creates_root() {
        let policy = policy_for(FileScope::Global, FileKind::Settings).expect("policy");
        assert_eq!(policy.filename, "settings.json");
        assert_eq!(policy.root_context, "CLAUDE_HOME");
        assert!(policy.root_may_be_missing);
        assert!(policy.create_root);
    }

    #[test]
    fn workspace_settings_is_rejected() {
        let result = policy_for(FileScope::Workspace, FileKind::Settings);
        assert!(result.is_err());
    }
}
