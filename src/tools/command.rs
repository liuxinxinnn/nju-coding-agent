use std::io::{self, Write as _};
use std::process::Stdio;
use std::sync::Arc;
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
}

pub type ApprovalFn = Arc<dyn Fn(&str) -> bool + Send + Sync>;

impl RunCommand {
    pub fn new(sandbox: Sandbox, auto_approve: bool) -> Self {
        Self {
            sandbox,
            auto_approve,
            approval: Arc::new(|command| confirm(command).unwrap_or(false)),
        }
    }

    pub fn with_approval(sandbox: Sandbox, auto_approve: bool, approval: ApprovalFn) -> Self {
        Self {
            sandbox,
            auto_approve,
            approval,
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
        match CommandPolicy::evaluate(command) {
            CommandDecision::Block => {
                return Err(Error::Tool("command blocked by safety policy".to_owned()));
            }
            CommandDecision::Confirm if !self.auto_approve && !(self.approval)(command) => {
                return Err(Error::Tool("user denied command execution".to_owned()));
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
    use std::sync::atomic::{AtomicBool, Ordering};

    use serde_json::json;
    use tempfile::tempdir;

    use crate::tool::Tool;

    use super::{CommandDecision, CommandPolicy, RunCommand, Sandbox};

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
}
