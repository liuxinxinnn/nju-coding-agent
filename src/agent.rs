use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::context::ContextManager;
use crate::error::{Error, Result};
use crate::llm::{LanguageModel, Message, Role};
use crate::project::ProjectProfile;
use crate::tool::ToolRegistry;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentPhase {
    Planning,
    Executing,
    Verifying,
    Done,
}

impl AgentPhase {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Planning => "PLAN",
            Self::Executing => "EXEC",
            Self::Verifying => "VERIFY",
            Self::Done => "DONE",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentState {
    pub messages: Vec<Message>,
    pub workspace_revision: u64,
    pub last_verified_revision: Option<u64>,
}

#[derive(Debug, Clone)]
pub enum AgentEvent {
    ProjectDetected {
        kind: String,
        evidence: Vec<String>,
        verification_command: Option<String>,
    },
    PhaseChanged {
        phase: AgentPhase,
    },
    PlanCreated {
        plan: String,
    },
    Thinking {
        step: usize,
        phase: AgentPhase,
    },
    ToolCall {
        step: usize,
        phase: AgentPhase,
        name: String,
        arguments: String,
    },
    ToolResult {
        name: String,
        result: String,
    },
    ContextCompressed {
        covered_messages: usize,
        before_tokens: u64,
        after_tokens: u64,
    },
    WorkspaceChanged {
        revision: u64,
        tool_name: String,
    },
    VerificationFinished {
        revision: u64,
        command: String,
        passed: bool,
    },
    FinishBlocked {
        workspace_revision: u64,
        last_verified_revision: Option<u64>,
    },
}

type EventHandler = Arc<dyn Fn(AgentEvent) + Send + Sync>;

pub struct Agent {
    model: Arc<dyn LanguageModel>,
    tools: ToolRegistry,
    messages: Vec<Message>,
    max_steps: usize,
    context: ContextManager,
    event_handler: Option<EventHandler>,
    phase: AgentPhase,
    workspace_revision: u64,
    last_verified_revision: Option<u64>,
    project: ProjectProfile,
    initial_system_message: Message,
}

impl Agent {
    pub fn new(
        model: Arc<dyn LanguageModel>,
        tools: ToolRegistry,
        workspace: &Path,
        max_steps: usize,
        context_window_tokens: u64,
    ) -> Self {
        let project = ProjectProfile::detect(workspace);
        let initial_system_message = Message::system(system_prompt(workspace, &project));
        Self {
            model,
            tools,
            messages: vec![initial_system_message.clone()],
            max_steps: max_steps.max(2),
            context: ContextManager::new(context_window_tokens),
            event_handler: None,
            phase: AgentPhase::Done,
            workspace_revision: 0,
            last_verified_revision: None,
            project,
            initial_system_message,
        }
    }

    pub fn on_event<F>(&mut self, handler: F)
    where
        F: Fn(AgentEvent) + Send + Sync + 'static,
    {
        self.event_handler = Some(Arc::new(handler));
    }

    pub fn messages(&self) -> &[Message] {
        &self.messages
    }

    pub const fn phase(&self) -> AgentPhase {
        self.phase
    }

    pub const fn workspace_revision(&self) -> u64 {
        self.workspace_revision
    }

    pub const fn last_verified_revision(&self) -> Option<u64> {
        self.last_verified_revision
    }

    pub const fn project_profile(&self) -> &ProjectProfile {
        &self.project
    }

    pub fn export_state(&self) -> AgentState {
        AgentState {
            messages: self.messages.clone(),
            workspace_revision: self.workspace_revision,
            last_verified_revision: self.last_verified_revision,
        }
    }

    pub fn restore_state(&mut self, mut state: AgentState) -> Result<()> {
        if state.messages.is_empty() || state.messages[0].role != Role::System {
            return Err(Error::Agent(
                "saved session must start with a system message".to_owned(),
            ));
        }
        if state
            .last_verified_revision
            .is_some_and(|revision| revision > state.workspace_revision)
        {
            return Err(Error::Agent(
                "saved session has an invalid verified revision".to_owned(),
            ));
        }
        state.messages[0] = self.initial_system_message.clone();
        self.messages = state.messages;
        self.workspace_revision = state.workspace_revision;
        self.last_verified_revision = state.last_verified_revision;
        self.phase = AgentPhase::Done;
        Ok(())
    }

    pub fn reset_state(&mut self) {
        self.messages = vec![self.initial_system_message.clone()];
        self.workspace_revision = 0;
        self.last_verified_revision = None;
        self.phase = AgentPhase::Done;
    }

    pub async fn run_turn(&mut self, task: &str) -> Result<String> {
        if task.trim().is_empty() {
            return Err(Error::Agent("task cannot be empty".to_owned()));
        }
        self.messages.push(Message::user(task));
        let definitions = self.tools.definitions();
        let planning_definitions = planning_tool_definitions(&definitions);
        let later_phase_tools = definitions
            .iter()
            .filter(|definition| !is_planning_tool(&definition.function.name))
            .map(|definition| definition.function.name.as_str())
            .collect::<Vec<_>>();
        let mut call_cache = BTreeMap::<String, (usize, String)>::new();
        self.emit(AgentEvent::ProjectDetected {
            kind: self.project.kind.label().to_owned(),
            evidence: self.project.evidence.clone(),
            verification_command: self.project.verification_command.clone(),
        });
        self.transition_to(AgentPhase::Planning);
        self.messages.push(Message::system(planning_prompt(
            &self.project,
            &later_phase_tools,
        )));

        for step in 1..=self.max_steps {
            if self.phase != AgentPhase::Planning && step == self.max_steps.saturating_sub(1) {
                self.messages.push(Message::system(if self.can_finish() {
                    "You are near the step limit. If the requested result is complete, return the final concise summary now."
                } else {
                    "You are near the step limit, but the current workspace revision is unverified. Run one appropriate test, build, lint, or program verification command now; do not claim completion before it passes."
                }));
            }
            self.emit(AgentEvent::Thinking {
                step,
                phase: self.phase,
            });
            let active_definitions = if self.phase == AgentPhase::Planning {
                &planning_definitions
            } else {
                &definitions
            };
            if let Some(event) = self
                .context
                .compact_if_needed(&mut self.messages, active_definitions)?
            {
                self.emit(AgentEvent::ContextCompressed {
                    covered_messages: event.covered_messages,
                    before_tokens: event.before_tokens,
                    after_tokens: event.after_tokens,
                });
            }
            let response = self
                .model
                .complete(&self.messages, active_definitions)
                .await?;
            let tool_calls = response.tool_calls.clone().unwrap_or_default();
            let final_text = response.content.clone().unwrap_or_default();
            self.messages.push(response);

            if tool_calls.is_empty() {
                if final_text.trim().is_empty() {
                    return Err(Error::Agent(format!(
                        "model returned neither text nor tool calls at step {step}"
                    )));
                }
                if self.phase == AgentPhase::Planning {
                    self.emit(AgentEvent::PlanCreated {
                        plan: final_text.clone(),
                    });
                    self.transition_to(AgentPhase::Executing);
                    self.messages
                        .push(Message::system(execution_prompt(&self.project)));
                    continue;
                }
                if self.can_finish() {
                    self.transition_to(AgentPhase::Done);
                    return Ok(final_text);
                }

                self.transition_to(AgentPhase::Verifying);
                self.emit(AgentEvent::FinishBlocked {
                    workspace_revision: self.workspace_revision,
                    last_verified_revision: self.last_verified_revision,
                });
                self.messages.push(Message::system(finish_blocked_prompt(
                    self.workspace_revision,
                    self.last_verified_revision,
                    &self.project,
                )));
                continue;
            }

            for call in tool_calls {
                if self.phase == AgentPhase::Planning && !is_planning_tool(&call.function.name) {
                    let observation = format!(
                        "ERROR: tool '{}' is not allowed during PLAN. Inspect with read_file, list_files, or search_text, then return a concise plan without tool calls.",
                        call.function.name
                    );
                    self.emit(AgentEvent::ToolCall {
                        step,
                        phase: self.phase,
                        name: call.function.name.clone(),
                        arguments: call.function.arguments.clone(),
                    });
                    self.emit(AgentEvent::ToolResult {
                        name: call.function.name,
                        result: observation.clone(),
                    });
                    self.messages.push(Message::tool(call.id, observation));
                    continue;
                }

                let signature = format!("{}:{}", call.function.name, call.function.arguments);
                if let Some((repeat_count, previous_result)) = call_cache.get_mut(&signature) {
                    *repeat_count += 1;
                    let observation = format!(
                        "Duplicate tool call skipped to avoid repeating side effects. The identical call already succeeded or failed with this result:\n\n{previous_result}\n\nUse that result and continue; do not call the same tool with the same arguments again."
                    );
                    self.emit(AgentEvent::ToolResult {
                        name: call.function.name,
                        result: format!("duplicate #{} skipped", *repeat_count),
                    });
                    self.messages.push(Message::tool(call.id, observation));
                    continue;
                }

                let verification_command =
                    command_argument(&call.function.name, &call.function.arguments)
                        .filter(|command| is_project_verification_command(command, &self.project));
                if verification_command.is_some() {
                    self.transition_to(AgentPhase::Verifying);
                } else if self.phase != AgentPhase::Planning
                    && (self.phase != AgentPhase::Verifying
                        || is_workspace_mutation(&call.function.name))
                {
                    self.transition_to(AgentPhase::Executing);
                }

                self.emit(AgentEvent::ToolCall {
                    step,
                    phase: self.phase,
                    name: call.function.name.clone(),
                    arguments: call.function.arguments.clone(),
                });
                let raw_observation = self
                    .tools
                    .execute(&call.function.name, &call.function.arguments)
                    .await;
                self.emit(AgentEvent::ToolResult {
                    name: call.function.name.clone(),
                    result: raw_observation.clone(),
                });

                let mut observation = raw_observation.clone();
                if successful_workspace_mutation(&call.function.name, &raw_observation) {
                    call_cache.clear();
                    self.workspace_revision = self.workspace_revision.saturating_add(1);
                    self.emit(AgentEvent::WorkspaceChanged {
                        revision: self.workspace_revision,
                        tool_name: call.function.name.clone(),
                    });
                    append_runtime_note(
                        &mut observation,
                        &format!(
                            "Workspace revision is now {}. DONE is blocked until this revision passes a real verification command.",
                            self.workspace_revision
                        ),
                    );
                }

                if let Some(command) = verification_command {
                    let passed = command_succeeded(&raw_observation);
                    if passed {
                        self.last_verified_revision = Some(self.workspace_revision);
                    } else {
                        if self.last_verified_revision == Some(self.workspace_revision) {
                            self.last_verified_revision = None;
                        }
                        self.transition_to(AgentPhase::Executing);
                    }
                    self.emit(AgentEvent::VerificationFinished {
                        revision: self.workspace_revision,
                        command: command.clone(),
                        passed,
                    });
                    append_runtime_note(
                        &mut observation,
                        if passed {
                            "Verification passed for the current workspace revision. DONE is now allowed if the task is complete."
                        } else {
                            "Verification failed. Return to EXECUTE, fix the cause, then verify the new current revision again."
                        },
                    );
                }

                call_cache.insert(signature, (0, observation.clone()));
                self.messages.push(Message::tool(call.id, observation));
            }
        }

        Err(Error::Agent(format!(
            "maximum step limit ({}) reached",
            self.max_steps
        )))
    }

    fn can_finish(&self) -> bool {
        self.workspace_revision == 0 || self.last_verified_revision == Some(self.workspace_revision)
    }

    fn transition_to(&mut self, phase: AgentPhase) {
        if self.phase != phase {
            self.phase = phase;
            self.emit(AgentEvent::PhaseChanged { phase });
        }
    }

    fn emit(&self, event: AgentEvent) {
        if let Some(handler) = &self.event_handler {
            handler(event);
            return;
        }
        match event {
            AgentEvent::ProjectDetected {
                kind,
                evidence,
                verification_command,
            } => println!(
                "[detect] {kind} · {} · verify: {}",
                if evidence.is_empty() {
                    "no manifest".to_owned()
                } else {
                    evidence.join(", ")
                },
                verification_command.unwrap_or_else(|| "model-selected".to_owned())
            ),
            AgentEvent::PhaseChanged { phase } => println!("[phase:{}]", phase.label()),
            AgentEvent::PlanCreated { plan } => println!("[plan]\n{plan}"),
            AgentEvent::Thinking { step, phase } => {
                println!("[{} step {step}] thinking", phase.label())
            }
            AgentEvent::ToolCall {
                step,
                phase,
                name,
                arguments,
            } => println!("[{} step {step}] {name} {arguments}", phase.label()),
            AgentEvent::ToolResult { name, result } => {
                println!("[tool:{name}] {}", one_line(&result));
            }
            AgentEvent::ContextCompressed {
                covered_messages,
                before_tokens,
                after_tokens,
            } => println!(
                "[context] compressed {covered_messages} messages: {before_tokens} -> {after_tokens} estimated tokens"
            ),
            AgentEvent::WorkspaceChanged {
                revision,
                tool_name,
            } => println!("[revision] {revision} after {tool_name}"),
            AgentEvent::VerificationFinished {
                revision,
                command,
                passed,
            } => println!(
                "[verify:{}] revision {revision} · {command}",
                if passed { "PASS" } else { "FAIL" }
            ),
            AgentEvent::FinishBlocked {
                workspace_revision,
                last_verified_revision,
            } => println!(
                "[verify:BLOCKED] workspace revision {workspace_revision}, last verified {}",
                last_verified_revision.map_or_else(|| "none".to_owned(), |value| value.to_string())
            ),
        }
    }
}

fn successful_workspace_mutation(tool_name: &str, observation: &str) -> bool {
    matches!(tool_name, "write_file" | "replace_text") && !observation.starts_with("ERROR:")
}

fn is_workspace_mutation(tool_name: &str) -> bool {
    matches!(tool_name, "write_file" | "replace_text")
}

fn planning_tool_definitions(
    definitions: &[crate::llm::ToolDefinition],
) -> Vec<crate::llm::ToolDefinition> {
    definitions
        .iter()
        .filter(|definition| is_planning_tool(&definition.function.name))
        .cloned()
        .collect()
}

fn is_planning_tool(tool_name: &str) -> bool {
    matches!(tool_name, "read_file" | "list_files" | "search_text")
}

fn command_argument(tool_name: &str, arguments: &str) -> Option<String> {
    if tool_name != "run_command" {
        return None;
    }
    serde_json::from_str::<serde_json::Value>(arguments)
        .ok()?
        .get("command")?
        .as_str()
        .map(str::to_owned)
}

fn is_verification_command(command: &str) -> bool {
    let normalized = command.trim().to_ascii_lowercase();
    const PREFIXES: &[&str] = &[
        "cargo test",
        "cargo check",
        "cargo clippy",
        "cargo build",
        "cargo run",
        "python -m unittest",
        "python -m pytest",
        "python -m compileall",
        "python -m py_compile",
        "pytest",
        "npm test",
        "npm run test",
        "npm run build",
        "npm run lint",
        "pnpm test",
        "pnpm run test",
        "pnpm run build",
        "pnpm run lint",
        "yarn test",
        "yarn build",
        "yarn lint",
        "go test",
        "go build",
        "go run",
        "dotnet test",
        "dotnet build",
        "mvn test",
        "mvn verify",
        "gradle test",
        "gradlew test",
        ".\\gradlew test",
        "./gradlew test",
    ];
    PREFIXES.iter().any(|prefix| {
        normalized == *prefix
            || normalized
                .strip_prefix(prefix)
                .is_some_and(|rest| rest.starts_with(char::is_whitespace))
    }) || is_python_script_command(&normalized)
}

fn is_project_verification_command(command: &str, project: &ProjectProfile) -> bool {
    is_verification_command(command)
        || project
            .verification_command
            .as_deref()
            .is_some_and(|expected| command.trim().eq_ignore_ascii_case(expected.trim()))
}

fn is_python_script_command(command: &str) -> bool {
    let mut parts = command.split_whitespace();
    matches!(parts.next(), Some("python" | "python3" | "py"))
        && parts
            .next()
            .is_some_and(|argument| argument.trim_matches(['\'', '"']).ends_with(".py"))
}

fn command_succeeded(observation: &str) -> bool {
    observation
        .lines()
        .next()
        .is_some_and(|line| line.trim() == "exit_code: 0")
}

fn append_runtime_note(observation: &mut String, note: &str) {
    observation.push_str("\n\nRUNTIME: ");
    observation.push_str(note);
}

fn planning_prompt(project: &ProjectProfile, later_phase_tools: &[&str]) -> String {
    let later_phase_note = if later_phase_tools.is_empty() {
        "No additional tools are registered for later phases.".to_owned()
    } else {
        format!(
            "After you return the plan, the runtime automatically enters EXECUTE/VERIFY and exposes these additional tools: {}. The current limited tool list is phase-scoped; do not claim those later tools are unavailable and do not try to launch another agent instance.",
            later_phase_tools.join(", ")
        )
    };
    format!(
        "PLAN phase. Understand the task before changing anything. You may inspect the workspace only with read_file, list_files, and search_text. Do not run commands or modify files during PLAN. {later_phase_note} When you have enough evidence, return a concise numbered plan with the intended change and verification strategy, without tool calls. This response is a plan, not the final answer. {}",
        project.prompt_hint()
    )
}

fn execution_prompt(project: &ProjectProfile) -> String {
    format!(
        "The plan is recorded. Enter EXECUTE: inspect as needed, make the smallest correct changes, and use file tools rather than shell commands for edits. After any successful write_file or replace_text, the runtime advances workspace_revision and blocks DONE until a recognized test, build, lint, or program command exits with code 0 for that same revision. {}",
        project.prompt_hint()
    )
}

fn finish_blocked_prompt(
    workspace_revision: u64,
    last_verified_revision: Option<u64>,
    project: &ProjectProfile,
) -> String {
    format!(
        "Runtime invariant blocked DONE: workspace revision {workspace_revision} is not verified (last verified revision: {}). Enter VERIFY now and call run_command with an appropriate real test, build, lint, or program command. A textual claim is not verification; the command must exit with code 0. {}",
        last_verified_revision.map_or_else(|| "none".to_owned(), |value| value.to_string()),
        project.prompt_hint()
    )
}

fn system_prompt(workspace: &Path, project: &ProjectProfile) -> String {
    format!(
        "You are a careful coding agent working only inside this workspace: {}\n\
         Every requested file must remain inside that workspace. If the user asks for a path outside it, explain that they must restart the agent with that path as --workspace; never bypass the file sandbox through shell commands. \
         Follow the runtime-controlled PLAN -> EXECUTE -> VERIFY phases. Inspect relevant files before editing. Always use write_file to create files and replace_text for small edits; never use run_command, shell redirection, or PowerShell file commands to create or edit files. Use run_command only to build, test, lint, or run the resulting project. A command result may be reused while the workspace is unchanged, but the same verification command must run again after a successful file edit. Tool errors \
         are observations: correct the request instead of pretending it succeeded. Never request \
         secrets or bypass the sandbox and command policy. When the task is complete, respond with \
         a concise summary of changes and verification performed. {}",
        human_path(workspace),
        project.prompt_hint()
    )
}

fn human_path(path: &Path) -> String {
    let display = path.display().to_string();
    display.strip_prefix(r"\\?\").unwrap_or(&display).to_owned()
}

fn one_line(value: &str) -> String {
    const LIMIT: usize = 240;
    let flattened = value.lines().next().unwrap_or_default();
    if flattened.chars().count() <= LIMIT {
        flattened.to_owned()
    } else {
        format!("{}...", flattened.chars().take(LIMIT).collect::<String>())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use serde_json::{Value, json};
    use tempfile::tempdir;

    use crate::error::{Error, Result};
    use crate::llm::{FunctionCall, LanguageModel, Message, Role, ToolCall, ToolDefinition};
    use crate::project::ProjectProfile;
    use crate::tool::{Tool, ToolRegistry};

    use super::{Agent, AgentState};

    struct MockModel {
        responses: Mutex<VecDeque<Message>>,
    }

    #[async_trait]
    impl LanguageModel for MockModel {
        async fn complete(
            &self,
            _messages: &[Message],
            _tools: &[ToolDefinition],
        ) -> Result<Message> {
            self.responses
                .lock()
                .map_err(|_| Error::Llm("mock lock poisoned".to_owned()))?
                .pop_front()
                .ok_or_else(|| Error::Llm("mock response exhausted".to_owned()))
        }
    }

    struct EchoTool;

    #[async_trait]
    impl Tool for EchoTool {
        fn name(&self) -> &'static str {
            "echo"
        }

        fn description(&self) -> &'static str {
            "echo a value"
        }

        fn parameters(&self) -> Value {
            json!({"type": "object"})
        }

        async fn execute(&self, arguments: Value) -> Result<String> {
            Ok(arguments.to_string())
        }
    }

    struct CountingEchoTool(Arc<AtomicUsize>);

    #[async_trait]
    impl Tool for CountingEchoTool {
        fn name(&self) -> &'static str {
            "echo"
        }

        fn description(&self) -> &'static str {
            "echo a value"
        }

        fn parameters(&self) -> Value {
            json!({"type": "object"})
        }

        async fn execute(&self, arguments: Value) -> Result<String> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(arguments.to_string())
        }
    }

    struct NamedCountingTool {
        name: &'static str,
        executions: Arc<AtomicUsize>,
        output: &'static str,
    }

    #[async_trait]
    impl Tool for NamedCountingTool {
        fn name(&self) -> &'static str {
            self.name
        }

        fn description(&self) -> &'static str {
            "count executions"
        }

        fn parameters(&self) -> Value {
            json!({"type": "object"})
        }

        async fn execute(&self, _arguments: Value) -> Result<String> {
            self.executions.fetch_add(1, Ordering::SeqCst);
            Ok(self.output.to_owned())
        }
    }

    fn tool_call(id: &str, name: &str, arguments: &str) -> Message {
        Message {
            role: Role::Assistant,
            content: None,
            reasoning_content: None,
            tool_calls: Some(vec![ToolCall {
                id: id.to_owned(),
                kind: "function".to_owned(),
                function: FunctionCall {
                    name: name.to_owned(),
                    arguments: arguments.to_owned(),
                },
            }]),
            tool_call_id: None,
        }
    }

    fn echo_call(id: &str) -> Message {
        tool_call(id, "echo", r#"{"value":"ok"}"#)
    }

    #[tokio::test]
    async fn executes_tool_then_returns_text() {
        let tool_call = echo_call("call-1");
        let model = Arc::new(MockModel {
            responses: Mutex::new(VecDeque::from([
                Message::assistant("1. Inspect and echo."),
                tool_call,
                Message::assistant("done"),
            ])),
        });
        let mut registry = ToolRegistry::new();
        registry.register(EchoTool).expect("register");
        let workspace = tempdir().expect("workspace");
        let mut agent = Agent::new(model, registry, workspace.path(), 5, 128_000);

        let result = agent.run_turn("do it").await.expect("agent run");

        assert_eq!(result, "done");
        assert!(
            agent
                .messages()
                .iter()
                .any(|message| message.role == Role::Tool)
        );
    }

    #[tokio::test]
    async fn identical_tool_call_is_executed_only_once() {
        let model = Arc::new(MockModel {
            responses: Mutex::new(VecDeque::from([
                Message::assistant("1. Inspect and echo."),
                echo_call("call-1"),
                echo_call("call-2"),
                Message::assistant("done"),
            ])),
        });
        let executions = Arc::new(AtomicUsize::new(0));
        let mut registry = ToolRegistry::new();
        registry
            .register(CountingEchoTool(Arc::clone(&executions)))
            .expect("register");
        let workspace = tempdir().expect("workspace");
        let mut agent = Agent::new(model, registry, workspace.path(), 5, 128_000);

        let result = agent.run_turn("do it").await.expect("agent run");

        assert_eq!(result, "done");
        assert_eq!(executions.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn command_can_run_again_after_successful_file_edit() {
        let command_arguments = r#"{"command":"python -m unittest"}"#;
        let model = Arc::new(MockModel {
            responses: Mutex::new(VecDeque::from([
                Message::assistant("1. Reproduce, edit, and verify."),
                tool_call("call-1", "run_command", command_arguments),
                tool_call(
                    "call-2",
                    "replace_text",
                    r#"{"path":"app.py","old_text":"bad","new_text":"good"}"#,
                ),
                tool_call("call-3", "run_command", command_arguments),
                Message::assistant("done"),
            ])),
        });
        let command_executions = Arc::new(AtomicUsize::new(0));
        let edit_executions = Arc::new(AtomicUsize::new(0));
        let mut registry = ToolRegistry::new();
        registry
            .register(NamedCountingTool {
                name: "run_command",
                executions: Arc::clone(&command_executions),
                output: "exit_code: 0\nstdout:\nok\nstderr:\n(empty)",
            })
            .expect("register command");
        registry
            .register(NamedCountingTool {
                name: "replace_text",
                executions: Arc::clone(&edit_executions),
                output: "replaced 1 occurrence",
            })
            .expect("register edit");
        let workspace = tempdir().expect("workspace");
        let mut agent = Agent::new(model, registry, workspace.path(), 6, 128_000);

        let result = agent.run_turn("fix and verify").await.expect("agent run");

        assert_eq!(result, "done");
        assert_eq!(edit_executions.load(Ordering::SeqCst), 1);
        assert_eq!(command_executions.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn planning_phase_rejects_mutating_tools() {
        let model = Arc::new(MockModel {
            responses: Mutex::new(VecDeque::from([
                tool_call(
                    "call-1",
                    "replace_text",
                    r#"{"path":"app.py","old_text":"bad","new_text":"good"}"#,
                ),
                Message::assistant("1. Inspect before editing."),
                Message::assistant("No change needed."),
            ])),
        });
        let edit_executions = Arc::new(AtomicUsize::new(0));
        let mut registry = ToolRegistry::new();
        registry
            .register(NamedCountingTool {
                name: "replace_text",
                executions: Arc::clone(&edit_executions),
                output: "replaced 1 occurrence",
            })
            .expect("register edit");
        let workspace = tempdir().expect("workspace");
        let mut agent = Agent::new(model, registry, workspace.path(), 5, 128_000);

        let result = agent.run_turn("inspect").await.expect("agent run");

        assert_eq!(result, "No change needed.");
        assert_eq!(edit_executions.load(Ordering::SeqCst), 0);
        assert_eq!(agent.workspace_revision(), 0);
        assert_eq!(agent.phase(), super::AgentPhase::Done);
        assert!(agent.messages().iter().any(|message| {
            message
                .content
                .as_deref()
                .is_some_and(|content| content.contains("not allowed during PLAN"))
        }));
    }

    #[tokio::test]
    async fn unverified_edit_blocks_finish_until_command_passes() {
        let model = Arc::new(MockModel {
            responses: Mutex::new(VecDeque::from([
                Message::assistant("1. Edit, then test."),
                tool_call(
                    "call-1",
                    "replace_text",
                    r#"{"path":"app.py","old_text":"bad","new_text":"good"}"#,
                ),
                Message::assistant("premature completion"),
                tool_call(
                    "call-2",
                    "run_command",
                    r#"{"command":"python -m unittest"}"#,
                ),
                Message::assistant("verified completion"),
            ])),
        });
        let edit_executions = Arc::new(AtomicUsize::new(0));
        let command_executions = Arc::new(AtomicUsize::new(0));
        let mut registry = ToolRegistry::new();
        registry
            .register(NamedCountingTool {
                name: "replace_text",
                executions: Arc::clone(&edit_executions),
                output: "replaced 1 occurrence",
            })
            .expect("register edit");
        registry
            .register(NamedCountingTool {
                name: "run_command",
                executions: Arc::clone(&command_executions),
                output: "exit_code: 0\nstdout:\nOK\nstderr:\n(empty)",
            })
            .expect("register command");
        let workspace = tempdir().expect("workspace");
        let mut agent = Agent::new(model, registry, workspace.path(), 7, 128_000);

        let result = agent.run_turn("fix it").await.expect("agent run");

        assert_eq!(result, "verified completion");
        assert_eq!(edit_executions.load(Ordering::SeqCst), 1);
        assert_eq!(command_executions.load(Ordering::SeqCst), 1);
        assert_eq!(agent.workspace_revision(), 1);
        assert_eq!(agent.last_verified_revision(), Some(1));
        assert_eq!(agent.phase(), super::AgentPhase::Done);
        assert!(agent.messages().iter().any(|message| {
            message
                .content
                .as_deref()
                .is_some_and(|content| content.contains("Runtime invariant blocked DONE"))
        }));
    }

    #[tokio::test]
    async fn edit_after_a_passed_check_requires_verifying_the_new_revision() {
        let model = Arc::new(MockModel {
            responses: Mutex::new(VecDeque::from([
                Message::assistant("1. Edit, test, refine, and retest."),
                tool_call(
                    "call-1",
                    "replace_text",
                    r#"{"path":"app.py","old_text":"bad","new_text":"better"}"#,
                ),
                tool_call(
                    "call-2",
                    "run_command",
                    r#"{"command":"python -m unittest"}"#,
                ),
                tool_call(
                    "call-3",
                    "replace_text",
                    r#"{"path":"app.py","old_text":"better","new_text":"good"}"#,
                ),
                Message::assistant("premature completion for revision 2"),
                tool_call(
                    "call-4",
                    "run_command",
                    r#"{"command":"python -m unittest"}"#,
                ),
                Message::assistant("revision 2 verified"),
            ])),
        });
        let edit_executions = Arc::new(AtomicUsize::new(0));
        let command_executions = Arc::new(AtomicUsize::new(0));
        let mut registry = ToolRegistry::new();
        registry
            .register(NamedCountingTool {
                name: "replace_text",
                executions: Arc::clone(&edit_executions),
                output: "replaced 1 occurrence",
            })
            .expect("register edit");
        registry
            .register(NamedCountingTool {
                name: "run_command",
                executions: Arc::clone(&command_executions),
                output: "exit_code: 0\nstdout:\nOK\nstderr:\n(empty)",
            })
            .expect("register command");
        let workspace = tempdir().expect("workspace");
        let mut agent = Agent::new(model, registry, workspace.path(), 9, 128_000);

        let result = agent.run_turn("fix it reliably").await.expect("agent run");

        assert_eq!(result, "revision 2 verified");
        assert_eq!(edit_executions.load(Ordering::SeqCst), 2);
        assert_eq!(command_executions.load(Ordering::SeqCst), 2);
        assert_eq!(agent.workspace_revision(), 2);
        assert_eq!(agent.last_verified_revision(), Some(2));
        assert_eq!(agent.phase(), super::AgentPhase::Done);
    }

    #[test]
    fn recognizes_real_verification_commands_but_not_arbitrary_successes() {
        for command in [
            "cargo test --all",
            "python -m unittest discover -s tests -v",
            "pytest -q",
            "npm run lint",
            "python hello.py",
            ".\\gradlew test",
        ] {
            assert!(super::is_verification_command(command), "{command}");
        }
        for command in ["echo ok", "python --version", "git status", "dir"] {
            assert!(!super::is_verification_command(command), "{command}");
        }
    }

    #[test]
    fn restores_and_resets_persisted_conversation_state() {
        let model = Arc::new(MockModel {
            responses: Mutex::new(VecDeque::new()),
        });
        let workspace = tempdir().expect("workspace");
        let mut agent = Agent::new(model, ToolRegistry::new(), workspace.path(), 5, 128_000);
        let state = AgentState {
            messages: vec![
                Message::system("old system prompt"),
                Message::user("first turn"),
                Message::assistant("first answer"),
            ],
            workspace_revision: 3,
            last_verified_revision: Some(3),
        };

        agent.restore_state(state).expect("restore state");

        assert_eq!(agent.workspace_revision(), 3);
        assert_eq!(agent.last_verified_revision(), Some(3));
        assert_eq!(agent.messages()[1], Message::user("first turn"));
        assert_eq!(agent.messages()[2], Message::assistant("first answer"));
        assert_ne!(
            agent.messages()[0].content.as_deref(),
            Some("old system prompt")
        );

        agent.reset_state();

        assert_eq!(agent.messages().len(), 1);
        assert_eq!(agent.messages()[0].role, Role::System);
        assert_eq!(agent.workspace_revision(), 0);
        assert_eq!(agent.last_verified_revision(), None);
    }

    #[test]
    fn rejects_persisted_verification_ahead_of_workspace_revision() {
        let model = Arc::new(MockModel {
            responses: Mutex::new(VecDeque::new()),
        });
        let workspace = tempdir().expect("workspace");
        let mut agent = Agent::new(model, ToolRegistry::new(), workspace.path(), 5, 128_000);

        let error = agent
            .restore_state(AgentState {
                messages: vec![Message::system("system")],
                workspace_revision: 1,
                last_verified_revision: Some(2),
            })
            .expect_err("invalid revision must be rejected");

        assert!(error.to_string().contains("invalid verified revision"));
        assert_eq!(agent.workspace_revision(), 0);
    }

    #[test]
    fn planning_prompt_explains_that_mutating_tools_arrive_in_later_phases() {
        let workspace = tempdir().expect("workspace");
        let profile = ProjectProfile::detect(workspace.path());

        let prompt =
            super::planning_prompt(&profile, &["write_file", "replace_text", "run_command"]);

        assert!(prompt.contains("automatically enters EXECUTE/VERIFY"));
        assert!(prompt.contains("write_file, replace_text, run_command"));
        assert!(prompt.contains("do not claim those later tools are unavailable"));
    }
}
