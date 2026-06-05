use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use folk_core::AccessMode;
use folk_mcp::{
    ToolError, ToolTable, err_result, json_text_result, number_property, object_schema,
    string_property, text_result,
};
use rs_peekaboo::automation::Target;
use rs_peekaboo::{Bounds, Direction, ImageMode, Peekaboo, Point};
use serde_json::{Value, json};
use thiserror::Error;

const SAFE_COMMANDS: &[&str] = &[
    "ls", "cat", "grep", "find", "head", "tail", "wc", "curl", "echo", "date", "whoami",
    "hostname", "uname", "which", "pwd", "ps", "uptime", "df", "du",
];
const RESTRICTED_SHELL_CHARS: &[char] = &[
    ';', '&', '|', '<', '>', '(', ')', '{', '}', '[', ']', '$', '`', '\\', '\'', '"', '*', '?',
    '~', '\n', '\r',
];

#[derive(Debug, Error)]
enum ToolExecError {
    #[error("missing {0}")]
    Missing(&'static str),
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("action blocked in this mode")]
    Blocked,
    #[error("{0}")]
    Peekaboo(#[from] rs_peekaboo::PeekabooError),
    #[error("{0}")]
    Json(#[from] serde_json::Error),
}

pub fn register_tools(table: &mut ToolTable) {
    table.register(
        "folk_shell",
        "Run a shell command on the Folk Around host computer, not on the agent provider or remote model server",
        schema(&[("command", string_property("Shell command to execute on the Folk Around host computer")), ("cwd", string_property("Working directory on the Folk Around host computer"))], &["command"]),
        |args, mode| Ok(shell(args, mode)),
    );
    table.register(
        "folk_system_info",
        "Return OS and CPU details for the Folk Around host computer",
        object_schema(BTreeMap::new(), &[]),
        |_, _| {
            Ok(text_result(format!(
                "os: {}\narch: {}",
                std::env::consts::OS,
                std::env::consts::ARCH
            )))
        },
    );
    table.register(
        "folk_list_apps",
        "List running processes on the Folk Around host computer",
        object_schema(BTreeMap::new(), &[]),
        |_, mode| Ok(list_apps(mode)),
    );
    table.register(
        "folk_spawn",
        "Spawn a background process on the Folk Around host computer in full mode",
        schema(
            &[(
                "command",
                string_property("Background command to spawn on the Folk Around host computer"),
            )],
            &["command"],
        ),
        |args, mode| Ok(spawn_command(args, mode)),
    );
    table.register(
        "folk_clipboard_read",
        "Read the Folk Around host computer clipboard",
        object_schema(BTreeMap::new(), &[]),
        |_, _| Ok(clipboard_read()),
    );
    table.register(
        "folk_clipboard_write",
        "Write text to the Folk Around host computer clipboard",
        schema(
            &[(
                "text",
                string_property("Text to copy to the Folk Around host computer clipboard"),
            )],
            &["text"],
        ),
        |args, _| Ok(clipboard_write(args)),
    );
    table.register(
        "folk_screen_capture",
        "Capture the Folk Around host computer screen and return image metadata",
        schema(
            &[
                (
                    "target",
                    string_property("Capture target: display, window, or region"),
                ),
                (
                    "path",
                    string_property("Output path on the Folk Around host computer"),
                ),
                ("x", number_property("Region x coordinate")),
                ("y", number_property("Region y coordinate")),
                ("width", number_property("Region width")),
                ("height", number_property("Region height")),
            ],
            &[],
        ),
        |args, mode| Ok(screen_capture(args, mode)),
    );
    table.register(
        "folk_ui_snapshot",
        "Return structured app and window context for the Folk Around host computer",
        object_schema(BTreeMap::new(), &[]),
        |_, mode| Ok(ui_snapshot(mode)),
    );
    table.register(
        "folk_click",
        "Click coordinates on the Folk Around host computer",
        schema(
            &[
                (
                    "element_id",
                    string_property("Stable element ID from folk_ui_snapshot"),
                ),
                ("x", number_property("Screen x coordinate")),
                ("y", number_property("Screen y coordinate")),
            ],
            &[],
        ),
        |args, mode| Ok(click(args, mode)),
    );
    table.register(
        "folk_type",
        "Type text on the Folk Around host computer",
        schema(&[("text", string_property("Text to type"))], &["text"]),
        |args, mode| Ok(type_text(args, mode)),
    );
    table.register(
        "folk_hotkey",
        "Press a hotkey on the Folk Around host computer",
        schema(
            &[("keys", string_property("Hotkey keys joined by plus signs"))],
            &["keys"],
        ),
        |args, mode| Ok(hotkey(args, mode)),
    );
    table.register(
        "folk_scroll",
        "Scroll on the Folk Around host computer",
        schema(
            &[
                ("dx", number_property("Horizontal scroll amount")),
                ("dy", number_property("Vertical scroll amount")),
            ],
            &[],
        ),
        |args, mode| Ok(scroll(args, mode)),
    );
    table.register(
        "folk_window",
        "List or focus windows on the Folk Around host computer",
        schema(
            &[
                (
                    "action",
                    string_property("Window action: list, focus, move, resize, close, minimize"),
                ),
                ("app", string_property("Application name")),
                ("title", string_property("Window title")),
                ("x", number_property("Window x coordinate")),
                ("y", number_property("Window y coordinate")),
                ("width", number_property("Window width")),
                ("height", number_property("Window height")),
            ],
            &["action"],
        ),
        |args, mode| Ok(window(args, mode)),
    );
    table.register(
        "folk_app",
        "List, launch, activate, or quit apps on the Folk Around host computer",
        schema(
            &[
                (
                    "action",
                    string_property("App action: list, launch, activate, quit"),
                ),
                ("app", string_property("Application name")),
            ],
            &["action"],
        ),
        |args, mode| Ok(app(args, mode)),
    );
    table.register(
        "folk_menu",
        "Inspect or click menu items on the Folk Around host computer",
        schema(
            &[
                ("action", string_property("Menu action: inspect or click")),
                ("app", string_property("Application name")),
                ("menu", string_property("Menu name")),
                ("item", string_property("Menu item name")),
            ],
            &["action"],
        ),
        |args, mode| Ok(menu(args, mode)),
    );
}

fn schema(fields: &[(&'static str, Value)], required: &[&'static str]) -> Value {
    object_schema(fields.iter().cloned().collect(), required)
}

fn shell(args: Value, mode: AccessMode) -> Value {
    let result = (|| {
        let command = str_arg(&args, "command").ok_or(ToolExecError::Missing("command"))?;
        let cwd = str_arg(&args, "cwd");
        let output = if mode == AccessMode::Full {
            run_shell(command, cwd)?
        } else if let Some((program, args)) = restricted_command(command) {
            run_command(program, &args, None, cwd)?
        } else {
            return Ok(err_result("command blocked in this mode"));
        };
        Ok(text_result(format!(
            "stdout:\n{}\n\nstderr:\n{}\n\nexit: {}",
            output.stdout, output.stderr, output.status
        )))
    })();
    flatten(result)
}

fn list_apps(mode: AccessMode) -> Value {
    let limit = if mode == AccessMode::Full { 50 } else { 30 };
    let result = run_command("ps", &["ax", "-o", "pid=,comm="], None, None).map(|output| {
        text_result(
            output
                .stdout
                .lines()
                .take(limit)
                .collect::<Vec<_>>()
                .join("\n"),
        )
    });
    flatten(result)
}

fn spawn_command(args: Value, mode: AccessMode) -> Value {
    let result = (|| {
        if mode != AccessMode::Full {
            return Ok(err_result("full mode only"));
        }
        let command = str_arg(&args, "command").ok_or(ToolExecError::Missing("command"))?;
        Command::new("/bin/sh")
            .arg("-c")
            .arg(command)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        Ok(text_result("spawned"))
    })();
    flatten(result)
}

fn clipboard_read() -> Value {
    flatten(
        Peekaboo::new()
            .clipboard_read()
            .map(|text| {
                if text.is_empty() {
                    text_result("(empty)")
                } else {
                    text_result(text)
                }
            })
            .map_err(ToolExecError::from),
    )
}

fn clipboard_write(args: Value) -> Value {
    let result = (|| {
        let text = str_arg(&args, "text").ok_or(ToolExecError::Missing("text"))?;
        Peekaboo::new().clipboard_write(text)?;
        Ok(text_result("copied to clipboard"))
    })();
    flatten(result)
}

fn screen_capture(args: Value, mode: AccessMode) -> Value {
    let result = (|| {
        ensure_observation(mode)?;
        let target = str_arg(&args, "target").unwrap_or("display");
        let path = str_arg(&args, "path").map(PathBuf::from);
        let capture = if target == "region" {
            let x = int_arg(&args, "x").unwrap_or(0);
            let y = int_arg(&args, "y").unwrap_or(0);
            let width = int_arg(&args, "width").unwrap_or(0);
            let height = int_arg(&args, "height").unwrap_or(0);
            if width > 0 && height > 0 {
                Peekaboo::new().image_region(
                    Bounds {
                        x,
                        y,
                        width,
                        height,
                    },
                    path,
                    true,
                )?
            } else {
                Peekaboo::new().image(ImageMode::Screen, path, true)?
            }
        } else if target == "window" {
            Peekaboo::new().image(ImageMode::Window, path, true)?
        } else {
            Peekaboo::new().image(ImageMode::Screen, path, true)?
        };
        Ok(json_text_result(&json!({
            "path": capture.path,
            "mimeType": capture.mime_type,
            "bytes": capture.bytes,
            "target": target
        })))
    })();
    flatten(result)
}

fn ui_snapshot(mode: AccessMode) -> Value {
    let result = (|| {
        ensure_observation(mode)?;
        let elements = serde_json::to_value(Peekaboo::new().ui_elements(None)?)?;
        Ok(json_text_result(&json!({
            "platform": "macos",
            "elements": elements
        })))
    })();
    flatten(result)
}

fn click(args: Value, mode: AccessMode) -> Value {
    let result = (|| {
        ensure_mutation(mode)?;
        let target = if let Some(element_id) = str_arg(&args, "element_id") {
            Target::Query {
                query: element_id.to_string(),
                snapshot: None,
            }
        } else {
            Target::Point(Point {
                x: int_arg(&args, "x").ok_or(ToolExecError::Missing("x"))?,
                y: int_arg(&args, "y").ok_or(ToolExecError::Missing("y"))?,
            })
        };
        Peekaboo::new().click(target, "left", 1)?;
        Ok(text_result("clicked"))
    })();
    flatten(result)
}

fn type_text(args: Value, mode: AccessMode) -> Value {
    let result = (|| {
        ensure_mutation(mode)?;
        let text = str_arg(&args, "text").ok_or(ToolExecError::Missing("text"))?;
        Peekaboo::new().type_text(text, false, false, None)?;
        Ok(text_result("typed"))
    })();
    flatten(result)
}

fn hotkey(args: Value, mode: AccessMode) -> Value {
    let result = (|| {
        ensure_mutation(mode)?;
        let keys = str_arg(&args, "keys").ok_or(ToolExecError::Missing("keys"))?;
        let parts = keys.split('+').map(str::trim).collect::<Vec<_>>();
        Peekaboo::new().hotkey(&parts)?;
        Ok(text_result("hotkey pressed"))
    })();
    flatten(result)
}

fn scroll(args: Value, mode: AccessMode) -> Value {
    let result = (|| {
        ensure_mutation(mode)?;
        let dx = int_arg(&args, "dx").unwrap_or(0);
        let dy = int_arg(&args, "dy").unwrap_or(0);
        let (direction, amount) = if dx != 0 {
            let direction = if dx < 0 {
                Direction::Left
            } else {
                Direction::Right
            };
            (direction, dx.unsigned_abs().max(1) as u32)
        } else {
            let direction = if dy < 0 {
                Direction::Down
            } else {
                Direction::Up
            };
            (direction, dy.unsigned_abs().max(1) as u32)
        };
        Peekaboo::new().scroll(direction, amount)?;
        Ok(text_result("scrolled"))
    })();
    flatten(result)
}

fn window(args: Value, mode: AccessMode) -> Value {
    let action = str_arg(&args, "action").unwrap_or("");
    if action == "list" {
        return ui_snapshot(mode);
    }
    let result = (|| {
        ensure_safe_focus_or_full(mode, action)?;
        let app = str_arg(&args, "app").ok_or(ToolExecError::Missing("app"))?;
        match action {
            "focus" | "close" | "minimize" => Peekaboo::new().window(action, Some(app), None)?,
            "move" | "resize" => {
                Peekaboo::new().window(action, Some(app), Some(window_bounds(action, &args)?))?
            }
            _ => return Err(ToolExecError::Missing("action")),
        };
        Ok(text_result("window action complete"))
    })();
    flatten(result)
}

fn window_bounds(action: &str, args: &Value) -> Result<Bounds, ToolExecError> {
    match action {
        "move" => Ok(Bounds {
            x: int_arg(args, "x").ok_or(ToolExecError::Missing("x"))?,
            y: int_arg(args, "y").ok_or(ToolExecError::Missing("y"))?,
            width: 0,
            height: 0,
        }),
        "resize" => Ok(Bounds {
            x: 0,
            y: 0,
            width: int_arg(args, "width").ok_or(ToolExecError::Missing("width"))?,
            height: int_arg(args, "height").ok_or(ToolExecError::Missing("height"))?,
        }),
        _ => Err(ToolExecError::Missing("action")),
    }
}

fn app(args: Value, mode: AccessMode) -> Value {
    let action = str_arg(&args, "action").unwrap_or("");
    if action == "list" {
        return list_apps(mode);
    }
    let result = (|| {
        ensure_safe_focus_or_full(mode, action)?;
        let app = str_arg(&args, "app").ok_or(ToolExecError::Missing("app"))?;
        match action {
            "launch" | "activate" => Peekaboo::new().app(action, Some(app))?,
            "quit" => Peekaboo::new().app(action, Some(app))?,
            _ => return Err(ToolExecError::Missing("action")),
        };
        Ok(text_result("app action complete"))
    })();
    flatten(result)
}

fn menu(args: Value, mode: AccessMode) -> Value {
    let result = (|| {
        let action = str_arg(&args, "action").ok_or(ToolExecError::Missing("action"))?;
        let app = str_arg(&args, "app").ok_or(ToolExecError::Missing("app"))?;
        if action == "inspect" {
            ensure_observation(mode)?;
            return Ok(json_text_result(
                &Peekaboo::new().menu("list", app, None, None)?,
            ));
        }
        ensure_mutation(mode)?;
        let menu = str_arg(&args, "menu").ok_or(ToolExecError::Missing("menu"))?;
        let item = str_arg(&args, "item").ok_or(ToolExecError::Missing("item"))?;
        Peekaboo::new().menu("click", app, Some(menu), Some(item))?;
        Ok(text_result("menu action complete"))
    })();
    flatten(result)
}

fn ensure_observation(_mode: AccessMode) -> Result<(), ToolExecError> {
    Ok(())
}

fn ensure_mutation(mode: AccessMode) -> Result<(), ToolExecError> {
    if mode != AccessMode::Full {
        Err(ToolExecError::Blocked)
    } else {
        Ok(())
    }
}

fn ensure_safe_focus_or_full(mode: AccessMode, action: &str) -> Result<(), ToolExecError> {
    if matches!(action, "list" | "focus" | "activate") {
        Ok(())
    } else {
        ensure_mutation(mode)
    }
}

fn flatten(result: Result<Value, ToolExecError>) -> Value {
    match result {
        Ok(value) => value,
        Err(err) => err_result(err.to_string()),
    }
}

fn restricted_command(command: &str) -> Option<(&str, Vec<&str>)> {
    if command
        .chars()
        .any(|ch| RESTRICTED_SHELL_CHARS.contains(&ch))
    {
        return None;
    }
    let mut parts = command.split_whitespace();
    let program = parts.next()?;
    if !SAFE_COMMANDS.contains(&program) {
        return None;
    }
    Some((program, parts.collect()))
}

fn str_arg<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)?.as_str()
}

fn int_arg(args: &Value, key: &str) -> Option<i64> {
    args.get(key)?
        .as_i64()
        .or_else(|| args.get(key)?.as_f64().map(|n| n as i64))
}

#[derive(Debug)]
struct CommandOutput {
    stdout: String,
    stderr: String,
    status: i32,
}

fn run_shell(command: &str, cwd: Option<&str>) -> Result<CommandOutput, ToolExecError> {
    run_command("/bin/sh", &["-c", command], None, cwd)
}

fn run_command(
    program: &str,
    args: &[&str],
    input: Option<&str>,
    cwd: Option<&str>,
) -> Result<CommandOutput, ToolExecError> {
    let mut command = Command::new(program);
    command.args(args);
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    if input.is_some() {
        command.stdin(Stdio::piped());
    }
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command.spawn()?;
    if let Some(input) = input {
        use std::io::Write;
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(input.as_bytes())?;
        }
    }
    let output = child.wait_with_output()?;
    Ok(CommandOutput {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        status: output.status.code().unwrap_or(-1),
    })
}

impl From<ToolExecError> for ToolError {
    fn from(value: ToolExecError) -> Self {
        Self::Message(value.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use folk_mcp::handle_message;

    #[test]
    fn sandbox_shell_should_match_legacy_safe_command_behavior() {
        let mut table = ToolTable::new(AccessMode::Sandbox);
        register_tools(&mut table);
        let response = handle_message(
            false,
            &table,
            json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"folk_shell","arguments":{"command":"echo hi"}}}),
        )
        .unwrap()
        .unwrap();
        assert!(response.contains("hi"));
    }

    #[test]
    fn sandbox_shell_should_block_shell_metacharacters() {
        let mut table = ToolTable::new(AccessMode::Sandbox);
        register_tools(&mut table);
        let response = handle_message(
            false,
            &table,
            json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"folk_shell","arguments":{"command":"echo hi; uname -s"}}}),
        )
        .unwrap()
        .unwrap();
        assert!(response.contains("command blocked in this mode"));
        assert!(!response.contains("Darwin"));
        assert!(!response.contains("Linux"));
    }

    #[test]
    fn limited_shell_should_execute_allowlisted_program_without_shell() {
        let mut table = ToolTable::new(AccessMode::Limited);
        register_tools(&mut table);
        let response = handle_message(
            false,
            &table,
            json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"folk_shell","arguments":{"command":"echo hi"}}}),
        )
        .unwrap()
        .unwrap();
        assert!(response.contains("hi"));
    }

    #[test]
    fn sandbox_mutation_should_be_blocked_for_computer_use() {
        let mut table = ToolTable::new(AccessMode::Sandbox);
        register_tools(&mut table);
        let response = handle_message(
            false,
            &table,
            json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"folk_type","arguments":{"text":"hi"}}}),
        )
        .unwrap()
        .unwrap();
        assert!(response.contains("action blocked in this mode"));
    }

    #[test]
    fn limited_mutation_should_be_blocked_for_computer_use() {
        let mut table = ToolTable::new(AccessMode::Limited);
        register_tools(&mut table);
        let response = handle_message(
            false,
            &table,
            json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"folk_type","arguments":{"text":"hi"}}}),
        )
        .unwrap()
        .unwrap();
        assert!(response.contains("action blocked in this mode"));
    }

    #[test]
    fn limited_launch_should_be_blocked() {
        assert!(ensure_safe_focus_or_full(AccessMode::Limited, "launch").is_err());
        assert!(ensure_safe_focus_or_full(AccessMode::Limited, "activate").is_ok());
    }
}
