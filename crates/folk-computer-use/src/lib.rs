use std::collections::BTreeMap;
use std::process::{Command, Stdio};

use folk_core::AccessMode;
use folk_mcp::{
    ToolError, ToolTable, err_result, json_text_result, number_property, object_schema,
    string_property, text_result,
};
use serde_json::{Value, json};
use tempfile::Builder;
use thiserror::Error;

const SAFE_COMMANDS: &[&str] = &[
    "ls", "cat", "grep", "find", "head", "tail", "wc", "curl", "echo", "date", "whoami",
    "hostname", "uname", "which", "pwd", "ps", "uptime", "df", "du",
];

#[derive(Debug, Error)]
enum ToolExecError {
    #[error("missing {0}")]
    Missing(&'static str),
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("unsupported platform: {0}")]
    Unsupported(&'static str),
    #[error("action blocked in this mode")]
    Blocked,
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
        if mode != AccessMode::Full && !is_safe_command(command) {
            return Ok(err_result("command blocked in this mode"));
        }
        let output = run_shell(command, cwd)?;
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
    #[cfg(target_os = "macos")]
    {
        flatten(run_command("pbpaste", &[], None, None).map(|output| {
            if output.stdout.is_empty() {
                text_result("(empty)")
            } else {
                text_result(output.stdout)
            }
        }))
    }
    #[cfg(not(target_os = "macos"))]
    {
        err_result("unsupported platform: clipboard read requires macOS")
    }
}

fn clipboard_write(args: Value) -> Value {
    #[cfg(target_os = "macos")]
    {
        let result = (|| {
            let text = str_arg(&args, "text").ok_or(ToolExecError::Missing("text"))?;
            run_command("pbcopy", &[], Some(text), None)?;
            Ok(text_result("copied to clipboard"))
        })();
        flatten(result)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = args;
        err_result("unsupported platform: clipboard write requires macOS")
    }
}

fn screen_capture(args: Value, mode: AccessMode) -> Value {
    let result = (|| {
        ensure_observation(mode)?;
        macos_only("screen capture")?;
        let path = match str_arg(&args, "path") {
            Some(path) => path.to_string(),
            None => Builder::new()
                .prefix("folk_screen_")
                .suffix(".png")
                .tempfile()?
                .into_temp_path()
                .keep()
                .map_err(|err| err.error)?
                .to_string_lossy()
                .to_string(),
        };
        let target = str_arg(&args, "target").unwrap_or("display");
        let mut owned_args = vec!["-x".to_string()];
        if target == "region" {
            let x = int_arg(&args, "x").unwrap_or(0);
            let y = int_arg(&args, "y").unwrap_or(0);
            let width = int_arg(&args, "width").unwrap_or(0);
            let height = int_arg(&args, "height").unwrap_or(0);
            if width > 0 && height > 0 {
                owned_args.push("-R".to_string());
                owned_args.push(format!("{x},{y},{width},{height}"));
            }
        } else if target == "window" {
            owned_args.push("-w".to_string());
        }
        owned_args.push(path.clone());
        let borrowed = owned_args.iter().map(String::as_str).collect::<Vec<_>>();
        run_command("screencapture", &borrowed, None, None)?;
        let metadata = std::fs::metadata(&path)?;
        Ok(json_text_result(&json!({
            "path": path,
            "mimeType": "image/png",
            "bytes": metadata.len(),
            "target": target
        })))
    })();
    flatten(result)
}

fn ui_snapshot(mode: AccessMode) -> Value {
    let result = (|| {
        ensure_observation(mode)?;
        macos_only("ui snapshot")?;
        let output = run_command("osascript", &["-e", snapshot_script()], None, None)?;
        let elements = output
            .stdout
            .lines()
            .filter_map(parse_snapshot_line)
            .collect::<Vec<_>>();
        Ok(json_text_result(&json!({
            "platform": "macos",
            "elements": elements
        })))
    })();
    flatten(result)
}

fn snapshot_script() -> &'static str {
    r#"tell application "System Events"
set out to ""
repeat with p in (application processes whose background only is false)
set appName to name of p
set frontValue to frontmost of p as text
set out to out & "app" & tab & appName & tab & frontValue & "\n"
repeat with w in windows of p
try
set winName to name of w
set posValue to position of w
set sizeValue to size of w
set minimizedValue to false
try
set minimizedValue to value of attribute "AXMinimized" of w
end try
set out to out & "window" & tab & appName & tab & winName & tab & (item 1 of posValue as text) & tab & (item 2 of posValue as text) & tab & (item 1 of sizeValue as text) & tab & (item 2 of sizeValue as text) & tab & (minimizedValue as text) & "\n"
end try
end repeat
end repeat
return out
end tell"#
}

fn parse_snapshot_line(line: &str) -> Option<Value> {
    let parts = line.split('\t').collect::<Vec<_>>();
    match parts.as_slice() {
        ["app", app, frontmost] => Some(json!({
            "id": format!("app:{app}"),
            "role": "application",
            "label": app,
            "app": app,
            "window": null,
            "bounds": null,
            "state": {
                "frontmost": frontmost.eq_ignore_ascii_case("true")
            }
        })),
        ["window", app, title, x, y, width, height, minimized] => {
            let x = x.parse::<i64>().ok()?;
            let y = y.parse::<i64>().ok()?;
            let width = width.parse::<i64>().ok()?;
            let height = height.parse::<i64>().ok()?;
            Some(json!({
                "id": format!("window:{app}:{title}"),
                "role": "window",
                "label": title,
                "app": app,
                "window": title,
                "bounds": {
                    "x": x,
                    "y": y,
                    "width": width,
                    "height": height
                },
                "state": {
                    "minimized": minimized.eq_ignore_ascii_case("true")
                }
            }))
        }
        _ => None,
    }
}

fn click(args: Value, mode: AccessMode) -> Value {
    let result = (|| {
        ensure_mutation(mode)?;
        macos_only("click")?;
        let (x, y) = if let Some(element_id) = str_arg(&args, "element_id") {
            window_center(element_id)?
        } else {
            (
                int_arg(&args, "x").ok_or(ToolExecError::Missing("x"))?,
                int_arg(&args, "y").ok_or(ToolExecError::Missing("y"))?,
            )
        };
        run_osascript(&format!(
            "tell application \"System Events\" to click at {{{x}, {y}}}"
        ))?;
        Ok(text_result("clicked"))
    })();
    flatten(result)
}

fn type_text(args: Value, mode: AccessMode) -> Value {
    let result = (|| {
        ensure_mutation(mode)?;
        macos_only("type")?;
        let text = str_arg(&args, "text").ok_or(ToolExecError::Missing("text"))?;
        run_osascript(&format!(
            "tell application \"System Events\" to keystroke {}",
            serde_json::to_string(text).map_err(|_| ToolExecError::Unsupported("type"))?
        ))?;
        Ok(text_result("typed"))
    })();
    flatten(result)
}

fn hotkey(args: Value, mode: AccessMode) -> Value {
    let result = (|| {
        ensure_mutation(mode)?;
        macos_only("hotkey")?;
        let keys = str_arg(&args, "keys").ok_or(ToolExecError::Missing("keys"))?;
        let parts = keys.split('+').map(str::trim).collect::<Vec<_>>();
        let Some(key) = parts.last() else {
            return Err(ToolExecError::Missing("keys"));
        };
        let modifiers = parts[..parts.len().saturating_sub(1)]
            .iter()
            .filter_map(|part| match part.to_ascii_lowercase().as_str() {
                "cmd" | "command" => Some("command down"),
                "shift" => Some("shift down"),
                "alt" | "option" => Some("option down"),
                "ctrl" | "control" => Some("control down"),
                _ => None,
            })
            .collect::<Vec<_>>();
        let script = if modifiers.is_empty() {
            format!("tell application \"System Events\" to keystroke \"{key}\"")
        } else {
            format!(
                "tell application \"System Events\" to keystroke \"{key}\" using {{{}}}",
                modifiers.join(", ")
            )
        };
        run_osascript(&script)?;
        Ok(text_result("hotkey pressed"))
    })();
    flatten(result)
}

fn scroll(args: Value, mode: AccessMode) -> Value {
    let result = (|| {
        ensure_mutation(mode)?;
        macos_only("scroll")?;
        let dy = int_arg(&args, "dy").unwrap_or(0);
        let direction = if dy < 0 { "down" } else { "up" };
        let amount = dy.unsigned_abs().max(1);
        run_osascript(&format!(
            "tell application \"System Events\" to scroll {direction} {amount}"
        ))?;
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
        macos_only("window")?;
        let app = str_arg(&args, "app").ok_or(ToolExecError::Missing("app"))?;
        match action {
            "focus" => run_osascript(&format!("tell application \"{app}\" to activate"))?,
            "close" => run_osascript(&format!("tell application \"{app}\" to close front window"))?,
            "minimize" => run_osascript(&format!(
                "tell application \"System Events\" to tell process \"{app}\" to set value of attribute \"AXMinimized\" of front window to true"
            ))?,
            "move" => {
                let x = int_arg(&args, "x").ok_or(ToolExecError::Missing("x"))?;
                let y = int_arg(&args, "y").ok_or(ToolExecError::Missing("y"))?;
                run_osascript(&format!(
                    "tell application \"System Events\" to tell process \"{app}\" to set position of front window to {{{x}, {y}}}"
                ))?
            }
            "resize" => {
                let width = int_arg(&args, "width").ok_or(ToolExecError::Missing("width"))?;
                let height = int_arg(&args, "height").ok_or(ToolExecError::Missing("height"))?;
                run_osascript(&format!(
                    "tell application \"System Events\" to tell process \"{app}\" to set size of front window to {{{width}, {height}}}"
                ))?
            }
            _ => return Err(ToolExecError::Missing("action")),
        };
        Ok(text_result("window action complete"))
    })();
    flatten(result)
}

fn window_center(element_id: &str) -> Result<(i64, i64), ToolExecError> {
    let Some(rest) = element_id.strip_prefix("window:") else {
        return Err(ToolExecError::Missing("x"));
    };
    let Some((app, title)) = rest.split_once(':') else {
        return Err(ToolExecError::Missing("x"));
    };
    let script = format!(
        r#"tell application "System Events"
tell process "{app}"
repeat with w in windows
if name of w is "{title}" then
set posValue to position of w
set sizeValue to size of w
return ((item 1 of posValue) + ((item 1 of sizeValue) div 2) as text) & "," & ((item 2 of posValue) + ((item 2 of sizeValue) div 2) as text)
end if
end repeat
end tell
end tell"#
    );
    let output = run_osascript(&script)?;
    let Some((x, y)) = output.stdout.trim().split_once(',') else {
        return Err(ToolExecError::Missing("x"));
    };
    let x = x
        .trim()
        .parse::<i64>()
        .map_err(|_| ToolExecError::Missing("x"))?;
    let y = y
        .trim()
        .parse::<i64>()
        .map_err(|_| ToolExecError::Missing("y"))?;
    Ok((x, y))
}

fn app(args: Value, mode: AccessMode) -> Value {
    let action = str_arg(&args, "action").unwrap_or("");
    if action == "list" {
        return list_apps(mode);
    }
    let result = (|| {
        ensure_safe_focus_or_full(mode, action)?;
        macos_only("app")?;
        let app = str_arg(&args, "app").ok_or(ToolExecError::Missing("app"))?;
        match action {
            "launch" | "activate" => run_command("open", &["-a", app], None, None)?,
            "quit" => {
                ensure_mutation(mode)?;
                run_osascript(&format!("tell application \"{app}\" to quit"))?
            }
            _ => return Err(ToolExecError::Missing("action")),
        };
        Ok(text_result("app action complete"))
    })();
    flatten(result)
}

fn menu(args: Value, mode: AccessMode) -> Value {
    let result = (|| {
        macos_only("menu")?;
        let action = str_arg(&args, "action").ok_or(ToolExecError::Missing("action"))?;
        let app = str_arg(&args, "app").ok_or(ToolExecError::Missing("app"))?;
        if action == "inspect" {
            ensure_observation(mode)?;
            let script = format!(
                "tell application \"System Events\" to tell process \"{app}\" to get name of menu bar items of menu bar 1"
            );
            let output = run_osascript(&script)?;
            return Ok(text_result(output.stdout));
        }
        ensure_mutation(mode)?;
        let menu = str_arg(&args, "menu").ok_or(ToolExecError::Missing("menu"))?;
        let item = str_arg(&args, "item").ok_or(ToolExecError::Missing("item"))?;
        let script = format!(
            "tell application \"System Events\" to tell process \"{app}\" to click menu item \"{item}\" of menu \"{menu}\" of menu bar item \"{menu}\" of menu bar 1"
        );
        run_osascript(&script)?;
        Ok(text_result("menu action complete"))
    })();
    flatten(result)
}

fn run_osascript(script: &str) -> Result<CommandOutput, ToolExecError> {
    run_command("osascript", &["-e", script], None, None)
}

fn ensure_observation(_mode: AccessMode) -> Result<(), ToolExecError> {
    Ok(())
}

fn ensure_mutation(mode: AccessMode) -> Result<(), ToolExecError> {
    if mode == AccessMode::Sandbox {
        Err(ToolExecError::Blocked)
    } else {
        Ok(())
    }
}

fn ensure_safe_focus_or_full(mode: AccessMode, action: &str) -> Result<(), ToolExecError> {
    if matches!(action, "list" | "focus" | "activate" | "launch") {
        Ok(())
    } else {
        ensure_mutation(mode)
    }
}

fn macos_only(action: &'static str) -> Result<(), ToolExecError> {
    if cfg!(target_os = "macos") {
        Ok(())
    } else {
        Err(ToolExecError::Unsupported(action))
    }
}

fn flatten(result: Result<Value, ToolExecError>) -> Value {
    match result {
        Ok(value) => value,
        Err(err) => err_result(err.to_string()),
    }
}

fn is_safe_command(command: &str) -> bool {
    let first = command.split_whitespace().next().unwrap_or("");
    SAFE_COMMANDS.contains(&first)
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
}
