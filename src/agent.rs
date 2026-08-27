use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use crate::context::ContextManager;
use crate::error::{Error, Result};
use crate::llm::{LanguageModel, Message};
use crate::tool::ToolRegistry;

#[derive(Debug, Clone)]
pub enum AgentEvent {
    Thinking {
        step: usize,
    },
    ToolCall {
        step: usize,
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
}

type EventHandler = Arc<dyn Fn(AgentEvent) + Send + Sync>;

pub struct Agent {
    model: Arc<dyn LanguageModel>,
    tools: ToolRegistry,
    messages: Vec<Message>,
    max_steps: usize,
    context: ContextManager,
    event_handler: Option<EventHandler>,
}

impl Agent {
    pub fn new(
        model: Arc<dyn LanguageModel>,
        tools: ToolRegistry,
        workspace: &Path,
        max_steps: usize,
        context_window_tokens: u64,
    ) -> Self {
        Self {
            model,
            tools,
            messages: vec![Message::system(system_prompt(workspace))],
            max_steps: max_steps.max(1),
            context: ContextManager::new(context_window_tokens),
            event_handler: None,
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

    pub async fn run_turn(&mut self, task: &str) -> Result<String> {
        if task.trim().is_empty() {
            return Err(Error::Agent("task cannot be empty".to_owned()));
        }
        self.messages.push(Message::user(task));
        let definitions = self.tools.definitions();
        let mut call_cache = BTreeMap::<String, (usize, String)>::new();

        for step in 1..=self.max_steps {
            if step == self.max_steps.saturating_sub(1) {
                self.messages.push(Message::system(
                    "You are near the step limit. If the requested result has already been verified, do not call another tool; return the final concise summary now.",
                ));
            }
            self.emit(AgentEvent::Thinking { step });
            if let Some(event) = self
                .context
                .compact_if_needed(&mut self.messages, &definitions)?
            {
                self.emit(AgentEvent::ContextCompressed {
                    covered_messages: event.covered_messages,
                    before_tokens: event.before_tokens,
                    after_tokens: event.after_tokens,
                });
            }
            let response = self.model.complete(&self.messages, &definitions).await?;
            let tool_calls = response.tool_calls.clone().unwrap_or_default();
            let final_text = response.content.clone().unwrap_or_default();
            self.messages.push(response);

            if tool_calls.is_empty() {
                if final_text.trim().is_empty() {
                    return Err(Error::Agent(format!(
                        "model returned neither text nor tool calls at step {step}"
                    )));
                }
                return Ok(final_text);
            }

            for call in tool_calls {
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

                self.emit(AgentEvent::ToolCall {
                    step,
                    name: call.function.name.clone(),
                    arguments: call.function.arguments.clone(),
                });
                let observation = self
                    .tools
                    .execute(&call.function.name, &call.function.arguments)
                    .await;
                self.emit(AgentEvent::ToolResult {
                    name: call.function.name.clone(),
                    result: observation.clone(),
                });
                if invalidates_cached_observations(&call.function.name, &observation) {
                    call_cache.clear();
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

    fn emit(&self, event: AgentEvent) {
        if let Some(handler) = &self.event_handler {
            handler(event);
            return;
        }
        match event {
            AgentEvent::Thinking { step } => println!("[step {step}] thinking"),
            AgentEvent::ToolCall {
                step,
                name,
                arguments,
            } => println!("[step {step}] {name} {arguments}"),
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
        }
    }
}

fn invalidates_cached_observations(tool_name: &str, observation: &str) -> bool {
    matches!(tool_name, "write_file" | "replace_text") && !observation.starts_with("ERROR:")
}

fn system_prompt(workspace: &Path) -> String {
    format!(
        "You are a careful coding agent working only inside this workspace: {}\n\
         Every requested file must remain inside that workspace. If the user asks for a path outside it, explain that they must restart the agent with that path as --workspace; never bypass the file sandbox through shell commands. \
         Inspect relevant files before editing. Always use write_file to create files and replace_text for small edits; never use run_command, shell redirection, or PowerShell file commands to create or edit files. Use run_command only to build, test, or run the resulting project, and do not repeat a command after it has already returned a successful result. Tool errors \
         are observations: correct the request instead of pretending it succeeded. Never request \
         secrets or bypass the sandbox and command policy. When the task is complete, respond with \
         a concise summary of changes and verification performed.",
        human_path(workspace)
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
    use crate::tool::{Tool, ToolRegistry};

    use super::Agent;

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
            Ok("ok".to_owned())
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
            responses: Mutex::new(VecDeque::from([tool_call, Message::assistant("done")])),
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
            })
            .expect("register command");
        registry
            .register(NamedCountingTool {
                name: "replace_text",
                executions: Arc::clone(&edit_executions),
            })
            .expect("register edit");
        let workspace = tempdir().expect("workspace");
        let mut agent = Agent::new(model, registry, workspace.path(), 6, 128_000);

        let result = agent.run_turn("fix and verify").await.expect("agent run");

        assert_eq!(result, "done");
        assert_eq!(edit_executions.load(Ordering::SeqCst), 1);
        assert_eq!(command_executions.load(Ordering::SeqCst), 2);
    }
}
