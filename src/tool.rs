use std::collections::BTreeMap;

use async_trait::async_trait;
use serde_json::Value;

use crate::error::{Error, Result};
use crate::llm::ToolDefinition;

pub const MAX_TOOL_OUTPUT_CHARS: usize = 20_000;

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn parameters(&self) -> Value;
    async fn execute(&self, arguments: Value) -> Result<String>;
}

#[derive(Default)]
pub struct ToolRegistry {
    tools: BTreeMap<String, Box<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register<T: Tool + 'static>(&mut self, tool: T) -> Result<()> {
        let name = tool.name().to_owned();
        if self.tools.insert(name.clone(), Box::new(tool)).is_some() {
            return Err(Error::Tool(format!("duplicate tool name: {name}")));
        }
        Ok(())
    }

    pub fn definitions(&self) -> Vec<ToolDefinition> {
        self.tools
            .values()
            .map(|tool| {
                ToolDefinition::function(
                    tool.name().to_owned(),
                    tool.description().to_owned(),
                    tool.parameters(),
                )
            })
            .collect()
    }

    pub async fn execute(&self, name: &str, raw_arguments: &str) -> String {
        let Some(tool) = self.tools.get(name) else {
            let available = self.tools.keys().cloned().collect::<Vec<_>>().join(", ");
            return format!(
                "ERROR: unknown tool '{name}'. Available tools: {}",
                if available.is_empty() {
                    "(none)"
                } else {
                    &available
                }
            );
        };

        let arguments = match serde_json::from_str::<Value>(raw_arguments) {
            Ok(value) => value,
            Err(error) => return format!("ERROR: invalid tool arguments: {error}"),
        };

        let result = match tool.execute(arguments).await {
            Ok(output) => output,
            Err(error) => format!("ERROR: {error}"),
        };
        truncate_tool_output(&result)
    }
}

fn truncate_tool_output(output: &str) -> String {
    if output.chars().count() <= MAX_TOOL_OUTPUT_CHARS {
        return output.to_owned();
    }
    let mut truncated: String = output.chars().take(MAX_TOOL_OUTPUT_CHARS).collect();
    truncated.push_str("\n[output truncated; narrow the request and try again]");
    truncated
}
