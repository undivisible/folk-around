use std::collections::BTreeMap;
use std::process::Command;
use std::sync::Arc;

use folk_core::AccessMode;
use serde_json::{Value, json};
use thiserror::Error;

pub type ToolResult = Result<Value, ToolError>;
pub type ToolHandler = dyn Fn(Value, AccessMode) -> ToolResult + Send + Sync;

#[derive(Clone)]
pub struct Tool {
    pub name: &'static str,
    pub description: &'static str,
    pub input_schema: Value,
    pub handler: Arc<ToolHandler>,
}

#[derive(Clone)]
pub struct ToolTable {
    mode: AccessMode,
    tools: Vec<Tool>,
}

#[derive(Debug, Error)]
pub enum ToolError {
    #[error("{0}")]
    Message(String),
}

impl ToolTable {
    pub fn new(mode: AccessMode) -> Self {
        Self {
            mode,
            tools: Vec::new(),
        }
    }

    pub fn register<F>(
        &mut self,
        name: &'static str,
        description: &'static str,
        input_schema: Value,
        handler: F,
    ) where
        F: Fn(Value, AccessMode) -> ToolResult + Send + Sync + 'static,
    {
        self.tools.push(Tool {
            name,
            description,
            input_schema,
            handler: Arc::new(handler),
        });
    }

    pub fn list(&self) -> &[Tool] {
        &self.tools
    }

    pub fn retain(&mut self, mut keep: impl FnMut(&Tool) -> bool) {
        self.tools.retain(|tool| keep(tool));
    }

    pub fn call(&self, name: &str, arguments: Value) -> Value {
        for tool in &self.tools {
            if tool.name == name {
                return match (tool.handler)(arguments, self.mode) {
                    Ok(value) => value,
                    Err(err) => err_result(err.to_string()),
                };
            }
        }
        err_result("not found")
    }
}

pub fn text_result(text: impl Into<String>) -> Value {
    json!({
        "content": [
            {
                "type": "text",
                "text": text.into()
            }
        ]
    })
}

pub fn json_text_result(value: &Value) -> Value {
    text_result(value.to_string())
}

pub fn err_result(text: impl Into<String>) -> Value {
    let mut value = text_result(text);
    if let Value::Object(map) = &mut value {
        map.insert("isError".to_string(), Value::Bool(true));
    }
    value
}

pub fn object_schema(
    properties: BTreeMap<&'static str, Value>,
    required: &[&'static str],
) -> Value {
    let mut schema = json!({
        "type": "object",
        "properties": properties,
    });
    if !required.is_empty() {
        schema["required"] = Value::Array(
            required
                .iter()
                .map(|field| Value::String((*field).to_string()))
                .collect(),
        );
    }
    schema
}

pub fn string_property(description: &'static str) -> Value {
    json!({
        "type": "string",
        "description": description
    })
}

pub fn number_property(description: &'static str) -> Value {
    json!({
        "type": "number",
        "description": description
    })
}

pub fn empty_schema() -> Value {
    json!({"type":"object","properties":{}})
}

pub fn handle_message(
    verbose: bool,
    table: &ToolTable,
    msg: Value,
) -> Result<Option<String>, serde_json::Error> {
    let Value::Object(object) = msg else {
        return Ok(None);
    };
    let Some(Value::String(method)) = object.get("method") else {
        return Ok(None);
    };
    let id = object.get("id").cloned();
    let is_notification = id.as_ref().is_none_or(Value::is_null);

    if verbose {
        log_status(&format!("<- {method}"));
    }

    match method.as_str() {
        "initialize" => {
            if is_notification {
                return Ok(None);
            }
            json_response(
                id.unwrap_or(Value::Null),
                json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": {
                        "tools": {
                            "listChanged": false
                        }
                    },
                    "serverInfo": {
                        "name": integration_name("folk-around"),
                        "version": env!("CARGO_PKG_VERSION")
                    }
                }),
            )
            .map(Some)
        }
        "notifications/initialized" => Ok(None),
        "ping" => {
            if is_notification {
                return Ok(None);
            }
            json_response(id.unwrap_or(Value::Null), json!({})).map(Some)
        }
        "tools/list" => {
            if is_notification {
                return Ok(None);
            }
            let tools = table
                .list()
                .iter()
                .map(|tool| {
                    json!({
                        "name": tool.name,
                        "description": tool.description,
                        "inputSchema": tool.input_schema,
                    })
                })
                .collect::<Vec<_>>();
            json_response(id.unwrap_or(Value::Null), json!({ "tools": tools })).map(Some)
        }
        "tools/call" => {
            if is_notification {
                return Ok(None);
            }
            let id = id.unwrap_or(Value::Null);
            let Some(Value::Object(params)) = object.get("params") else {
                return error_response(id, -32602, "Missing params").map(Some);
            };
            let Some(Value::String(name)) = params.get("name") else {
                return error_response(id, -32602, "Missing name").map(Some);
            };
            let arguments = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            if verbose {
                log_status(&tool_log("started"));
            }
            let result = table.call(name, arguments);
            if verbose {
                log_status(&tool_log("finished"));
            }
            json_response(id, result).map(Some)
        }
        _ => {
            if is_notification {
                Ok(None)
            } else {
                error_response(id.unwrap_or(Value::Null), -32601, method).map(Some)
            }
        }
    }
}

fn tool_log(status: &str) -> String {
    format!("tool {status}")
}

fn log_status(message: &str) {
    folk_core::log_status(message);
}

fn json_response(id: Value, result: Value) -> Result<String, serde_json::Error> {
    serde_json::to_string(&json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result
    }))
}

fn error_response(id: Value, code: i32, message: &str) -> Result<String, serde_json::Error> {
    serde_json::to_string(&json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": code,
            "message": message
        }
    }))
}

fn integration_name(base: &str) -> String {
    let raw = std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| {
            Command::new("hostname")
                .output()
                .ok()
                .and_then(|output| String::from_utf8(output.stdout).ok())
                .unwrap_or_default()
        });
    let suffix = raw
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    if suffix.is_empty() {
        base.to_string()
    } else {
        format!("{base}-{suffix}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initialize_should_match_protocol_shape() {
        let table = ToolTable::new(AccessMode::Full);
        let response = handle_message(
            false,
            &table,
            json!({"jsonrpc":"2.0","id":1,"method":"initialize"}),
        )
        .unwrap()
        .unwrap();
        let value: Value = serde_json::from_str(&response).unwrap();
        assert_eq!(value["result"]["protocolVersion"], "2024-11-05");
        assert!(
            value["result"]["serverInfo"]["name"]
                .as_str()
                .unwrap()
                .starts_with("folk-around")
        );
    }

    #[test]
    fn notification_should_not_respond() {
        let table = ToolTable::new(AccessMode::Full);
        let response = handle_message(
            false,
            &table,
            json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        )
        .unwrap();
        assert!(response.is_none());
    }

    #[test]
    fn tool_logs_exclude_arguments_and_results() {
        let message = tool_log("finished");

        assert_eq!(message, "tool finished");
        assert!(!message.contains("private-text"));
    }
}
