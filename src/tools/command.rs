use std::collections::HashSet;
use std::io::{self, Write as _};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::process::Command;
use tokio::time::timeout;

use crate::error::{Error, Result};
use crate::tool::Tool;

use super::Sandbox;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandDecision {
    Allow,
    Confirm,
    Block,
}

#[derive(Debug, Default)]
pub struct CommandPolicy;

impl CommandPolicy {
    pub fn evaluate(command: &str) -> CommandDecision {
        let normalized = command.trim().to_ascii_lowercase();
        if normalized.is_empty() || is_blocked(&normalized) {
            return CommandDecision::Block;
        }
        if is_read_only(&normalized) {
            CommandDecision::Allow
        } else {
            CommandDecision::Confirm
        }
    }
}

pub struct RunCommand {
    sandbox: Sandbox,
    auto_approve: bool,
    approval: ApprovalFn,
    approved_commands: Mutex<HashSet<String>>,
}

pub type ApprovalFn = Arc<dyn Fn(&str) -> bool + Send + Sync>;

impl RunCommand {
    pub fn new(sandbox: Sandbox, auto_approve: bool) -> Self {
        Self {
            sandbox,
            auto_approve,
            approval: Arc::new(|command| confirm(command).unwrap_or(false)),
            approved_commands: Mutex::new(HashSet::new()),
        }
    }

    pub fn with_approval(sandbox: Sandbox, auto_approve: bool, approval: ApprovalFn) -> Self {
        Self {
            sandbox,
            auto_approve,
            approval,
            approved_commands: Mutex::new(HashSet::new()),
        }
    }
}

#[async_trait]
impl Tool for RunCommand {
    fn name(&self) -> &'static str {
        "run_command"
    }

    fn description(&self) -> &'static str {
        "Run a shell command in the workspace. Destructive commands are blocked; non-read-only commands require approval."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {"type": "string"},
                "timeout_seconds": {"type": "integer", "minimum": 1, "maximum": 120}
            },
            "required": ["command"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, arguments: Value) -> Result<String> {
        let command = arguments
            .get("command")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Tool("missing command".to_owned()))?;
        ensure_command_paths_inside_workspace(command, self.sandbox.root())?;
        match CommandPolicy::evaluate(command) {
            CommandDecision::Block => {
                return Err(Error::Tool("command blocked by safety policy".to_owned()));
            }
            CommandDecision::Confirm if !self.auto_approve => {
                let already_approved = self
                    .approved_commands
                    .lock()
                    .map_err(|_| Error::Tool("command approval cache is unavailable".to_owned()))?
                    .contains(command);
                if !already_approved {
                    if !(self.approval)(command) {
                        return Err(Error::Tool("user denied command execution".to_owned()));
                    }
                    self.approved_commands
                        .lock()
                        .map_err(|_| {
                            Error::Tool("command approval cache is unavailable".to_owned())
                        })?
                        .insert(command.to_owned());
                }
            }
            CommandDecision::Allow | CommandDecision::Confirm => {}
        }

        let timeout_seconds = arguments
            .get("timeout_seconds")
            .and_then(Value::as_u64)
            .unwrap_or(30)
            .clamp(1, 120);
        let mut process = shell_command(command);
        process
            .current_dir(self.sandbox.root())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let output = timeout(Duration::from_secs(timeout_seconds), process.output())
            .await
            .map_err(|_| Error::Tool(format!("command timed out after {timeout_seconds}s")))?
            .map_err(|error| Error::Tool(format!("failed to start command: {error}")))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        Ok(format!(
            "exit_code: {}\nstdout:\n{}\nstderr:\n{}",
            output
                .status
                .code()
                .map_or_else(|| "unknown".to_owned(), |code| code.to_string()),
            if stdout.is_empty() {
                "(empty)"
            } else {
                &stdout
            },
            if stderr.is_empty() {
                "(empty)"
            } else {
                &stderr
            }
        ))
    }
}

#[cfg(windows)]
fn shell_command(command: &str) -> Command {
    let mut process = Command::new("powershell");
    process.args(["-NoProfile", "-NonInteractive", "-Command", command]);
    process
}

#[cfg(not(windows))]
fn shell_command(command: &str) -> Command {
    let mut process = Command::new("sh");
    process.args(["-c", command]);
    process
}

fn confirm(command: &str) -> Result<bool> {
    eprint!("Approve command `{command}`? [y/N] ");
    io::stderr().flush()?;
    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    Ok(matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

fn ensure_command_paths_inside_workspace(command: &str, workspace: &std::path::Path) -> Result<()> {
    let normalized_command = command.replace('\\', "/");
    if normalized_command
        .split(|character: char| character.is_whitespace() || "'\"`;|&()".contains(character))
        .any(|token| {
            token == ".."
                || token.starts_with("../")
                || token.ends_with("/..")
                || token.contains("/../")
        })
    {
        return Err(Error::Tool(
            "command contains a parent-directory path outside the workspace policy".to_owned(),
        ));
    }

    #[cfg(windows)]
    {
        let workspace = normalize_windows_path(&workspace.display().to_string());
        for candidate in windows_absolute_paths(command) {
            let candidate = normalize_windows_path(candidate);
            let prefix = format!("{workspace}/");
            if candidate != workspace && !candidate.starts_with(&prefix) {
                return Err(Error::Tool(format!(
                    "command path escapes workspace: {candidate}. Restart with that directory as --workspace instead"
                )));
            }
        }
    }

    #[cfg(not(windows))]
    {
        for token in command.split_whitespace() {
            let candidate = token.trim_matches(['\'', '"', ';', '|', '&', '(', ')']);
            if candidate.starts_with('/') && !std::path::Path::new(candidate).starts_with(workspace)
            {
                return Err(Error::Tool(format!(
                    "command path escapes workspace: {candidate}"
                )));
            }
        }
    }
    Ok(())
}

#[cfg(windows)]
fn windows_absolute_paths(command: &str) -> Vec<&str> {
    let bytes = command.as_bytes();
    let mut paths = Vec::new();
    let mut index = 0;
    while index + 2 < bytes.len() {
        if bytes[index].is_ascii_alphabetic()
            && bytes[index + 1] == b':'
            && matches!(bytes[index + 2], b'\\' | b'/')
        {
            let start = index;
            index += 3;
            while index < bytes.len()
                && !bytes[index].is_ascii_whitespace()
                && !matches!(
                    bytes[index],
                    b'\'' | b'"' | b'`' | b';' | b'|' | b'&' | b'(' | b')' | b','
                )
            {
                index += 1;
            }
            if let Some(path) = command.get(start..index) {
                paths.push(path);
            }
            continue;
        }
        index += 1;
    }
    paths
}

#[cfg(windows)]
fn normalize_windows_path(path: &str) -> String {
    path.replace('\\', "/")
        .trim_start_matches("//?/")
        .trim_end_matches('/')
        .to_ascii_lowercase()
}

fn is_read_only(command: &str) -> bool {
    if contains_shell_control(command) {
        return false;
    }
    const PREFIXES: &[&str] = &[
        "git status",
        "git diff",
        "git log",
        "rg",
        "get-childitem",
        "get-content",
        "dir",
        "ls",
        "pwd",
    ];
    PREFIXES
        .iter()
        .any(|prefix| has_token_prefix(command, prefix))
}

fn contains_shell_control(command: &str) -> bool {
    [';', '|', '&', '>', '<', '`']
        .iter()
        .any(|character| command.contains(*character))
        || command.contains("$(")
}

fn has_token_prefix(command: &str, prefix: &str) -> bool {
    command == prefix
        || command
            .strip_prefix(prefix)
            .is_some_and(|rest| rest.starts_with(char::is_whitespace))
}

fn is_blocked(command: &str) -> bool {
    const PATTERNS: &[&str] = &[
        "rm -rf",
        "remove-item -recurse",
        "remove-item -r",
        "del /s",
        "rd /s",
        "rmdir /s",
        "format ",
        "diskpart",
        "mkfs",
        "shutdown",
        "reboot",
        "poweroff",
        "git reset --hard",
        "git clean -fd",
        ":(){:|:&};:",
    ];
    PATTERNS.iter().any(|pattern| command.contains(pattern))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use serde_json::json;
    use tempfile::tempdir;

    use crate::tool::Tool;

    use super::{
        CommandDecision, CommandPolicy, RunCommand, Sandbox, ensure_command_paths_inside_workspace,
    };

    #[test]
    fn blocks_destructive_commands() {
        assert_eq!(CommandPolicy::evaluate("rm -rf ."), CommandDecision::Block);
        assert_eq!(
            CommandPolicy::evaluate("git reset --hard"),
            CommandDecision::Block
        );
    }

    #[test]
    fn requires_confirmation_for_execution() {
        assert_eq!(
            CommandPolicy::evaluate("cargo test"),
            CommandDecision::Confirm
        );
        assert_eq!(
            CommandPolicy::evaluate("git status"),
            CommandDecision::Allow
        );
        assert_eq!(
            CommandPolicy::evaluate("git status; echo injected"),
            CommandDecision::Confirm
        );
    }

    #[tokio::test]
    async fn custom_approval_callback_can_deny_command() {
        let workspace = tempdir().expect("workspace");
        let called = Arc::new(AtomicBool::new(false));
        let callback_state = Arc::clone(&called);
        let tool = RunCommand::with_approval(
            Sandbox::new(workspace.path().to_path_buf()).expect("sandbox"),
            false,
            Arc::new(move |_command| {
                callback_state.store(true, Ordering::SeqCst);
                false
            }),
        );

        let result = tool.execute(json!({"command": "echo hello"})).await;

        assert!(called.load(Ordering::SeqCst));
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn approved_identical_command_is_not_confirmed_twice() {
        let workspace = tempdir().expect("workspace");
        let calls = Arc::new(AtomicUsize::new(0));
        let callback_calls = Arc::clone(&calls);
        let tool = RunCommand::with_approval(
            Sandbox::new(workspace.path().to_path_buf()).expect("sandbox"),
            false,
            Arc::new(move |_command| {
                callback_calls.fetch_add(1, Ordering::SeqCst);
                true
            }),
        );

        let first = tool.execute(json!({"command": "echo hello"})).await;
        let second = tool.execute(json!({"command": "echo hello"})).await;

        assert!(first.is_ok());
        assert!(second.is_ok());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn rejects_parent_directory_paths_in_commands() {
        let workspace = tempdir().expect("workspace");
        let result =
            ensure_command_paths_inside_workspace("python ../outside/hello.py", workspace.path());
        assert!(result.is_err());
    }

    #[cfg(windows)]
    #[test]
    fn rejects_absolute_windows_path_outside_workspace() {
        let workspace = std::path::Path::new(r"D:\NJU-Agent\agent-sandbox");
        let result =
            ensure_command_paths_inside_workspace(r"python D:\NJU-Agent\test\hello.py", workspace);
        assert!(result.is_err());
        assert!(
            ensure_command_paths_inside_workspace(
                r"python D:\NJU-Agent\agent-sandbox\hello.py",
                workspace,
            )
            .is_ok()
        );
    }
}
