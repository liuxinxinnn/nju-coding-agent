use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use crate::context::ContextManager;
use crate::error::{Error, Result};
use crate::llm::{LanguageModel, Message};
use crate::tool::ToolRegistry;

const MAX_IDENTICAL_CALLS: usize = 3;

pub struct Agent {
    model: Arc<dyn LanguageModel>,
    tools: ToolRegistry,
    messages: Vec<Message>,
    max_steps: usize,
    context: ContextManager,
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
        }
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
        let mut repeated_calls = BTreeMap::<String, usize>::new();

        for step in 1..=self.max_steps {
            if let Some(event) = self
                .context
                .compact_if_needed(&mut self.messages, &definitions)?
            {
                eprintln!(
                    "[context] compressed {} messages: {} -> {} estimated tokens",
                    event.covered_messages, event.before_tokens, event.after_tokens
                );
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
                let repeats = repeated_calls.entry(signature).or_default();
                *repeats += 1;
                if *repeats >= MAX_IDENTICAL_CALLS {
                    return Err(Error::Agent(format!(
                        "stopped after {MAX_IDENTICAL_CALLS} identical calls to '{}'",
                        call.function.name
                    )));
                }

                eprintln!(
                    "[step {step}] {} {}",
                    call.function.name, call.function.arguments
                );
                let observation = self
                    .tools
                    .execute(&call.function.name, &call.function.arguments)
                    .await;
                eprintln!("[tool] {}", one_line(&observation));
                self.messages.push(Message::tool(call.id, observation));
            }
        }

        Err(Error::Agent(format!(
            "maximum step limit ({}) reached",
            self.max_steps
        )))
    }
}

fn system_prompt(workspace: &Path) -> String {
    format!(
        "You are a careful coding agent working only inside this workspace: {}\n\
         Inspect relevant files before editing. Prefer replace_text for small edits and write_file \
         for new files. Run the project's tests or checks after changes when practical. Tool errors \
         are observations: correct the request instead of pretending it succeeded. Never request \
         secrets or bypass the sandbox and command policy. When the task is complete, respond with \
         a concise summary of changes and verification performed.",
        workspace.display()
    )
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

    #[tokio::test]
    async fn executes_tool_then_returns_text() {
        let tool_call = Message {
            role: Role::Assistant,
            content: None,
            tool_calls: Some(vec![ToolCall {
                id: "call-1".to_owned(),
                kind: "function".to_owned(),
                function: FunctionCall {
                    name: "echo".to_owned(),
                    arguments: r#"{"value":"ok"}"#.to_owned(),
                },
            }]),
            tool_call_id: None,
        };
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
}
