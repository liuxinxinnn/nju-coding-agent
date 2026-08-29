mod command;
mod files;
mod sandbox;

use std::path::PathBuf;
use std::sync::Arc;

use crate::error::Result;
use crate::memory::{MarkdownMemoryStore, MemoryTool};
use crate::tool::ToolRegistry;

pub use command::{ApprovalFn, CommandDecision, CommandPolicy};
pub use sandbox::Sandbox;

pub fn default_registry(workspace: PathBuf, auto_approve: bool) -> Result<ToolRegistry> {
    let memory = Arc::new(MarkdownMemoryStore::open_default(&workspace)?);
    registry_with_memory(workspace, auto_approve, memory)
}

fn registry_with_memory(
    workspace: PathBuf,
    auto_approve: bool,
    memory: Arc<MarkdownMemoryStore>,
) -> Result<ToolRegistry> {
    let sandbox = Sandbox::new(workspace)?;
    let mut registry = ToolRegistry::new();
    registry.register(files::ReadFile::new(sandbox.clone()))?;
    registry.register(files::ListFiles::new(sandbox.clone()))?;
    registry.register(files::SearchText::new(sandbox.clone()))?;
    registry.register(files::WriteFile::new(sandbox.clone()))?;
    registry.register(files::ReplaceText::new(sandbox.clone()))?;
    registry.register(MemoryTool::new(memory))?;
    registry.register(command::RunCommand::new(sandbox, auto_approve))?;
    Ok(registry)
}

pub fn default_registry_with_approval(
    workspace: PathBuf,
    auto_approve: bool,
    approval: ApprovalFn,
) -> Result<ToolRegistry> {
    let memory = Arc::new(MarkdownMemoryStore::open_default(&workspace)?);
    let sandbox = Sandbox::new(workspace)?;
    let mut registry = ToolRegistry::new();
    registry.register(files::ReadFile::new(sandbox.clone()))?;
    registry.register(files::ListFiles::new(sandbox.clone()))?;
    registry.register(files::SearchText::new(sandbox.clone()))?;
    registry.register(files::WriteFile::new(sandbox.clone()))?;
    registry.register(files::ReplaceText::new(sandbox.clone()))?;
    registry.register(MemoryTool::new(memory))?;
    registry.register(command::RunCommand::with_approval(
        sandbox,
        auto_approve,
        approval,
    ))?;
    Ok(registry)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use crate::memory::MarkdownMemoryStore;

    use super::registry_with_memory;

    #[tokio::test]
    async fn default_tools_complete_a_file_edit_workflow() {
        let workspace = tempdir().expect("workspace");
        let data = tempdir().expect("data");
        fs::write(workspace.path().join(".env"), "SECRET=hidden").expect("secret fixture");
        let memory = std::sync::Arc::new(
            MarkdownMemoryStore::new(data.path().to_path_buf(), workspace.path()).expect("memory"),
        );
        let registry = registry_with_memory(workspace.path().to_path_buf(), true, memory)
            .expect("tool registry");

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
