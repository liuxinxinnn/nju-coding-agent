mod command;
mod files;
mod sandbox;

use std::path::PathBuf;

use crate::error::Result;
use crate::tool::ToolRegistry;

pub use command::{CommandDecision, CommandPolicy};
pub use sandbox::Sandbox;

pub fn default_registry(workspace: PathBuf, auto_approve: bool) -> Result<ToolRegistry> {
    let sandbox = Sandbox::new(workspace)?;
    let mut registry = ToolRegistry::new();
    registry.register(files::ReadFile::new(sandbox.clone()))?;
    registry.register(files::ListFiles::new(sandbox.clone()))?;
    registry.register(files::SearchText::new(sandbox.clone()))?;
    registry.register(files::WriteFile::new(sandbox.clone()))?;
    registry.register(files::ReplaceText::new(sandbox.clone()))?;
    registry.register(command::RunCommand::new(sandbox, auto_approve))?;
    Ok(registry)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::default_registry;

    #[tokio::test]
    async fn default_tools_complete_a_file_edit_workflow() {
        let workspace = tempdir().expect("workspace");
        fs::write(workspace.path().join(".env"), "SECRET=hidden").expect("secret fixture");
        let registry =
            default_registry(workspace.path().to_path_buf(), true).expect("tool registry");

        let written = registry
            .execute(
                "write_file",
                r#"{"path":"src/main.txt","content":"hello bug"}"#,
            )
            .await;
        assert!(written.starts_with("wrote"), "{written}");

        let edited = registry
            .execute(
                "replace_text",
                r#"{"path":"src/main.txt","old_text":"bug","new_text":"world"}"#,
            )
            .await;
        assert!(edited.starts_with("replaced"), "{edited}");

        let read = registry
            .execute("read_file", r#"{"path":"src/main.txt"}"#)
            .await;
        assert!(read.contains("hello world"), "{read}");

        let search = registry
            .execute("search_text", r#"{"query":"world"}"#)
            .await;
        assert!(search.contains("src\\main.txt") || search.contains("src/main.txt"));

        let secret_search = registry
            .execute("search_text", r#"{"query":"SECRET"}"#)
            .await;
        assert_eq!(secret_search, "no matches for 'SECRET'");

        let listing = registry
            .execute("list_files", r#"{"recursive":true}"#)
            .await;
        assert!(!listing.contains(".env"), "{listing}");
    }
}
