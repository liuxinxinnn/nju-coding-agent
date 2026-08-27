use std::collections::VecDeque;
use std::fmt::Write as _;

use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::fs;

use crate::error::{Error, Result};
use crate::tool::Tool;

use super::Sandbox;

const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_WRITE_BYTES: usize = 2 * 1024 * 1024;
const MAX_LIST_ENTRIES: usize = 2_000;
const DEFAULT_READ_LINES: usize = 400;
const DEFAULT_SEARCH_RESULTS: usize = 50;

pub struct ReadFile {
    sandbox: Sandbox,
}

impl ReadFile {
    pub const fn new(sandbox: Sandbox) -> Self {
        Self { sandbox }
    }
}

#[async_trait]
impl Tool for ReadFile {
    fn name(&self) -> &'static str {
        "read_file"
    }

    fn description(&self) -> &'static str {
        "Read a UTF-8 text file with line numbers. Use offset and limit for large files."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "Path relative to the workspace"},
                "offset": {"type": "integer", "minimum": 1},
                "limit": {"type": "integer", "minimum": 1, "maximum": 1000}
            },
            "required": ["path"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, arguments: Value) -> Result<String> {
        let path = required_string(&arguments, "path")?;
        let resolved = self.sandbox.resolve_existing(path)?;
        let metadata = fs::metadata(&resolved).await?;
        if !metadata.is_file() {
            return Err(Error::Tool(format!("not a file: {}", resolved.display())));
        }
        if metadata.len() > MAX_FILE_BYTES {
            return Err(Error::Tool(format!(
                "file is too large: {} bytes (limit {MAX_FILE_BYTES})",
                metadata.len()
            )));
        }

        let bytes = fs::read(&resolved).await?;
        if bytes.iter().take(8_192).any(|byte| *byte == 0) {
            return Err(Error::Tool("binary files are not supported".to_owned()));
        }
        let content = String::from_utf8(bytes)
            .map_err(|_| Error::Tool("file is not valid UTF-8".to_owned()))?;
        let lines: Vec<&str> = content.lines().collect();
        let offset = optional_usize(&arguments, "offset", 1).max(1);
        let limit = optional_usize(&arguments, "limit", DEFAULT_READ_LINES).clamp(1, 1_000);
        let start = offset.saturating_sub(1).min(lines.len());
        let end = start.saturating_add(limit).min(lines.len());
        let mut output = String::new();
        for (index, line) in lines[start..end].iter().enumerate() {
            let _ = writeln!(output, "{:>6}\t{}", start + index + 1, line);
        }
        if start > 0 || end < lines.len() {
            let _ = write!(
                output,
                "[showing lines {}-{end} of {}]",
                start + 1,
                lines.len()
            );
        }
        Ok(output)
    }
}

pub struct ListFiles {
    sandbox: Sandbox,
}

impl ListFiles {
    pub const fn new(sandbox: Sandbox) -> Self {
        Self { sandbox }
    }
}

#[async_trait]
impl Tool for ListFiles {
    fn name(&self) -> &'static str {
        "list_files"
    }

    fn description(&self) -> &'static str {
        "List files and directories inside the workspace, optionally recursively."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "default": "."},
                "recursive": {"type": "boolean", "default": false},
                "max_depth": {"type": "integer", "minimum": 1, "maximum": 8}
            },
            "additionalProperties": false
        })
    }

    async fn execute(&self, arguments: Value) -> Result<String> {
        let raw = optional_string(&arguments, "path").unwrap_or(".");
        let start = self.sandbox.resolve_existing(raw)?;
        if !start.is_dir() {
            return Err(Error::Tool(format!("not a directory: {}", start.display())));
        }
        let recursive = arguments
            .get("recursive")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let max_depth = optional_usize(&arguments, "max_depth", 3).clamp(1, 8);
        let mut queue = VecDeque::from([(start, 0_usize)]);
        let mut output = Vec::new();

        while let Some((dir, depth)) = queue.pop_front() {
            let mut reader = fs::read_dir(&dir).await?;
            while let Some(entry) = reader.next_entry().await? {
                if output.len() >= MAX_LIST_ENTRIES {
                    output.push(format!("[truncated at {MAX_LIST_ENTRIES} entries]"));
                    return Ok(output.join("\n"));
                }
                let path = entry.path();
                if self.sandbox.is_sensitive_path(&path) {
                    continue;
                }
                let file_type = entry.file_type().await?;
                let label = if file_type.is_dir() { "dir " } else { "file" };
                output.push(format!("{label} {}", self.sandbox.display_relative(&path)));
                if recursive
                    && file_type.is_dir()
                    && !file_type.is_symlink()
                    && depth < max_depth
                    && !ignored_directory(&path)
                {
                    queue.push_back((path, depth + 1));
                }
            }
        }

        output.sort();
        Ok(if output.is_empty() {
            "directory is empty".to_owned()
        } else {
            output.join("\n")
        })
    }
}

pub struct SearchText {
    sandbox: Sandbox,
}

impl SearchText {
    pub const fn new(sandbox: Sandbox) -> Self {
        Self { sandbox }
    }
}

#[async_trait]
impl Tool for SearchText {
    fn name(&self) -> &'static str {
        "search_text"
    }

    fn description(&self) -> &'static str {
        "Recursively search UTF-8 files for a literal text string."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {"type": "string"},
                "path": {"type": "string", "default": "."},
                "max_results": {"type": "integer", "minimum": 1, "maximum": 200}
            },
            "required": ["query"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, arguments: Value) -> Result<String> {
        let query = required_string(&arguments, "query")?;
        if query.is_empty() {
            return Err(Error::Tool("query cannot be empty".to_owned()));
        }
        let raw = optional_string(&arguments, "path").unwrap_or(".");
        let start = self.sandbox.resolve_existing(raw)?;
        let max_results =
            optional_usize(&arguments, "max_results", DEFAULT_SEARCH_RESULTS).clamp(1, 200);
        let mut queue = VecDeque::from([start]);
        let mut results = Vec::new();

        while let Some(path) = queue.pop_front() {
            if results.len() >= max_results {
                break;
            }
            if self.sandbox.is_sensitive_path(&path) {
                continue;
            }
            if path.is_dir() {
                if ignored_directory(&path) && path != self.sandbox.root() {
                    continue;
                }
                let mut reader = match fs::read_dir(&path).await {
                    Ok(reader) => reader,
                    Err(_) => continue,
                };
                while let Ok(Some(entry)) = reader.next_entry().await {
                    if let Ok(file_type) = entry.file_type().await
                        && !file_type.is_symlink()
                    {
                        queue.push_back(entry.path());
                    }
                }
                continue;
            }

            let Ok(metadata) = fs::metadata(&path).await else {
                continue;
            };
            if metadata.len() > MAX_FILE_BYTES {
                continue;
            }
            let Ok(bytes) = fs::read(&path).await else {
                continue;
            };
            if bytes.iter().take(8_192).any(|byte| *byte == 0) {
                continue;
            }
            let Ok(content) = String::from_utf8(bytes) else {
                continue;
            };
            for (index, line) in content.lines().enumerate() {
                if line.contains(query) {
                    results.push(format!(
                        "{}:{}: {}",
                        self.sandbox.display_relative(&path),
                        index + 1,
                        line
                    ));
                    if results.len() >= max_results {
                        break;
                    }
                }
            }
        }

        Ok(if results.is_empty() {
            format!("no matches for '{query}'")
        } else {
            results.join("\n")
        })
    }
}

pub struct WriteFile {
    sandbox: Sandbox,
}

impl WriteFile {
    pub const fn new(sandbox: Sandbox) -> Self {
        Self { sandbox }
    }
}

#[async_trait]
impl Tool for WriteFile {
    fn name(&self) -> &'static str {
        "write_file"
    }

    fn description(&self) -> &'static str {
        "Create a UTF-8 text file. Set overwrite=true explicitly to replace an existing file."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string"},
                "content": {"type": "string"},
                "overwrite": {"type": "boolean", "default": false}
            },
            "required": ["path", "content"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, arguments: Value) -> Result<String> {
        let raw = required_string(&arguments, "path")?;
        let content = required_string(&arguments, "content")?;
        if content.len() > MAX_WRITE_BYTES {
            return Err(Error::Tool(format!(
                "content exceeds {MAX_WRITE_BYTES} byte limit"
            )));
        }
        let target = self.sandbox.resolve_writable(raw)?;
        let overwrite = arguments
            .get("overwrite")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if target.exists() && !overwrite {
            return Err(Error::Tool(
                "file exists; use replace_text or set overwrite=true explicitly".to_owned(),
            ));
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).await?;
        }
        fs::write(&target, content).await?;
        Ok(format!(
            "wrote {} bytes to {}",
            content.len(),
            self.sandbox.display_relative(&target)
        ))
    }
}

pub struct ReplaceText {
    sandbox: Sandbox,
}

impl ReplaceText {
    pub const fn new(sandbox: Sandbox) -> Self {
        Self { sandbox }
    }
}

#[async_trait]
impl Tool for ReplaceText {
    fn name(&self) -> &'static str {
        "replace_text"
    }

    fn description(&self) -> &'static str {
        "Replace exact text in a UTF-8 file. By default old_text must occur exactly once."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string"},
                "old_text": {"type": "string"},
                "new_text": {"type": "string"},
                "replace_all": {"type": "boolean", "default": false}
            },
            "required": ["path", "old_text", "new_text"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, arguments: Value) -> Result<String> {
        let raw = required_string(&arguments, "path")?;
        let old = required_string(&arguments, "old_text")?;
        let new = required_string(&arguments, "new_text")?;
        if old.is_empty() {
            return Err(Error::Tool("old_text cannot be empty".to_owned()));
        }
        let target = self.sandbox.resolve_existing(raw)?;
        let content = fs::read_to_string(&target).await?;
        let count = content.matches(old).count();
        if count == 0 {
            return Err(Error::Tool("old_text was not found".to_owned()));
        }
        let replace_all = arguments
            .get("replace_all")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if !replace_all && count != 1 {
            return Err(Error::Tool(format!(
                "old_text matched {count} times; provide more context or set replace_all=true"
            )));
        }
        let updated = if replace_all {
            content.replace(old, new)
        } else {
            content.replacen(old, new, 1)
        };
        fs::write(&target, updated).await?;
        Ok(format!(
            "replaced {} occurrence(s) in {}",
            if replace_all { count } else { 1 },
            self.sandbox.display_relative(&target)
        ))
    }
}

fn required_string<'a>(arguments: &'a Value, key: &str) -> Result<&'a str> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Tool(format!("missing or invalid string parameter '{key}'")))
}

fn optional_string<'a>(arguments: &'a Value, key: &str) -> Option<&'a str> {
    arguments.get(key).and_then(Value::as_str)
}

fn optional_usize(arguments: &Value, key: &str, default: usize) -> usize {
    arguments
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or(default)
}

fn ignored_directory(path: &std::path::Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| matches!(name, ".git" | "target" | "node_modules" | ".idea"))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::json;
    use tempfile::tempdir;

    use crate::tool::Tool;

    use super::{ReplaceText, Sandbox};

    #[tokio::test]
    async fn exact_replace_rejects_ambiguous_match() {
        let root = tempdir().expect("temp dir");
        fs::write(root.path().join("a.txt"), "x x").expect("fixture");
        let tool = ReplaceText::new(Sandbox::new(root.path().to_path_buf()).expect("sandbox"));

        let result = tool
            .execute(json!({"path": "a.txt", "old_text": "x", "new_text": "y"}))
            .await;

        assert!(result.is_err());
    }
}
