use std::process::{Command, Stdio};
#[cfg(unix)]
use std::{
    fs::{File, Permissions},
    io::Read,
};

use folk_core::AccessMode;
use folk_mcp::{
    ToolError, ToolTable, empty_schema, err_result, json_text_result, number_property,
    object_schema, string_property, text_result,
};
use rs_peekaboo::automation::Target;
use rs_peekaboo::{Bounds, Direction, ImageCapture, ImageMode, Peekaboo, PeekabooConfig, Point};
use serde_json::{Value, json};
#[cfg(unix)]
use sha2::{Digest, Sha256};
use thiserror::Error;

mod praefectus_adapter;

const SAFE_COMMANDS: &[&str] = &[
    "ls", "cat", "grep", "find", "head", "tail", "wc", "curl", "echo", "date", "whoami",
    "hostname", "uname", "which", "pwd", "ps", "uptime", "df", "du",
];
const RESTRICTED_SHELL_CHARS: &[char] = &[
    ';', '&', '|', '<', '>', '(', ')', '{', '}', '[', ']', '$', '`', '\\', '\'', '"', '*', '?',
    '~', '\n', '\r',
];
#[cfg(unix)]
const MAX_SCREENSHOT_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Error)]
enum ToolExecError {
    #[error("missing {0}")]
    Missing(&'static str),
    #[error("invalid tool arguments")]
    InvalidArguments,
    #[error("host I/O request failed")]
    Io(#[from] std::io::Error),
    #[error("action blocked in this mode")]
    Blocked,
    #[error("computer-use backend request failed")]
    Peekaboo(#[from] rs_peekaboo::PeekabooError),
    #[error("{0}")]
    Json(#[from] serde_json::Error),
    #[error("computer-use protocol request failed")]
    Praefectus(#[from] praefectus::ProtocolError),
    #[error("semantic computer-use target is unavailable")]
    Semantic(#[from] praefectus::semantic::SemanticError),
    #[error("semantic computer-use backend is unavailable")]
    SemanticUnavailable,
    #[error("raw coordinate actions are unavailable")]
    CoordinatesUnavailable,
    #[error("safe screenshot reference is unavailable")]
    ScreenshotReferenceUnavailable,
}

fn peekaboo() -> Peekaboo {
    Peekaboo::with_config(PeekabooConfig {
        background: true,
        ..PeekabooConfig::default()
    })
}

pub fn register_tools(table: &mut ToolTable) {
    register_tools_with_semantic_capabilities(
        table,
        praefectus_adapter::supports_semantic_action("invoke"),
        praefectus_adapter::supports_semantic_action("set_value"),
    );
}

fn register_tools_with_semantic_capabilities(
    table: &mut ToolTable,
    semantic_click: bool,
    semantic_set_value: bool,
) {
    table.register(
        "folk_shell",
        "Run a shell command on the Folk Around host computer, not on the agent provider or remote model server",
        schema(
            &[
                (
                    "command",
                    string_property("Shell command to execute on the Folk Around host computer"),
                ),
                (
                    "cwd",
                    string_property("Working directory on the Folk Around host computer"),
                ),
            ],
            &["command"],
        ),
        |args, mode| Ok(shell(args, mode)),
    );
    table.register(
        "folk_system_info",
        "Return OS and CPU details for the Folk Around host computer",
        empty_schema(),
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
        empty_schema(),
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
        "folk_image",
        "Capture an image from the Folk Around host computer",
        schema(
            &[
                ("mode", string_property("Capture mode: screen or window")),
                ("retina", bool_property("Capture at retina scale")),
                (
                    "app",
                    string_property("Optional app name for window-scoped capture"),
                ),
            ],
            &[],
        ),
        |args, mode| Ok(image(args, mode)),
    );
    table.register(
        "folk_see",
        "Capture an image and Praefectus semantic observation for the Folk Around host computer",
        schema(
            &[
                ("mode", string_property("Capture mode: screen or window")),
                ("retina", bool_property("Capture at retina scale")),
            ],
            &[],
        ),
        |args, mode| Ok(see(args, mode)),
    );
    table.register(
        "folk_list_screens",
        "List display information for the Folk Around host computer",
        empty_schema(),
        |_, mode| Ok(list_screens(mode)),
    );
    table.register(
        "folk_clipboard_read",
        "Read the Folk Around host computer clipboard",
        empty_schema(),
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
        |args, mode| Ok(clipboard_write(args, mode)),
    );
    table.register(
        "folk_permissions",
        "Probe or grant screen recording, accessibility, and clipboard access on the Folk Around host computer",
        schema(
            &[(
                "action",
                string_property("Optional action: grant"),
            )],
            &[],
        ),
        |args, mode| Ok(permissions(args, mode)),
    );
    table.register(
        "folk_doctor",
        "Health report for computer-use readiness on the Folk Around host computer",
        empty_schema(),
        |_, _| Ok(doctor()),
    );
    table.register(
        "folk_screen_capture",
        "Capture the Folk Around host computer screen and return image metadata",
        schema(
            &[(
                "target",
                string_property("Capture target: display or window"),
            )],
            &[],
        ),
        |args, mode| Ok(screen_capture(args, mode)),
    );
    table.register(
        "folk_ui_snapshot",
        "Return a Praefectus semantic observation with opaque element tags for the Folk Around host computer",
        empty_schema(),
        |_, mode| Ok(ui_snapshot(mode)),
    );
    table.register(
        "folk_click",
        "Invoke a Praefectus semantic element on the Folk Around host computer",
        strict_schema(
            &[
                (
                    "operation_id",
                    constrained_string_property(
                        "Stable operation ID for durable at-most-once dispatch",
                        1,
                        256,
                        "^[A-Za-z0-9_:-]+$",
                    ),
                ),
                (
                    "observation_id",
                    constrained_string_property(
                        "Observation ID from folk_ui_snapshot",
                        64,
                        64,
                        "^[0-9a-f]{64}$",
                    ),
                ),
                (
                    "tag",
                    constrained_string_property(
                        "Opaque element tag from folk_ui_snapshot",
                        2,
                        5,
                        "^e[0-9]{1,4}$",
                    ),
                ),
                ("interaction_mode", interaction_mode_property()),
            ],
            &["operation_id", "observation_id", "tag", "interaction_mode"],
        ),
        |args, mode| Ok(click(args, mode)),
    );
    table.register(
        "folk_press",
        "Press a named key on the Folk Around host computer",
        schema(
            &[
                (
                    "key",
                    string_property("Key name such as return, tab, space, or arrows"),
                ),
                ("count", number_property("Repeat count")),
                (
                    "delay_ms",
                    number_property("Delay between repeats in milliseconds"),
                ),
            ],
            &["key"],
        ),
        |args, mode| Ok(press(args, mode)),
    );
    table.register(
        "folk_type",
        "Type text on the Folk Around host computer",
        schema(
            &[
                ("text", string_property("Text to type")),
                ("clear", bool_property("Clear the current field first")),
                ("return", bool_property("Press return after typing")),
                (
                    "delay_ms",
                    number_property("Delay between characters in milliseconds"),
                ),
            ],
            &["text"],
        ),
        |args, mode| Ok(type_text(args, mode)),
    );
    table.register(
        "folk_paste",
        "Paste text into the active UI on the Folk Around host computer",
        schema(&[("text", string_property("Text to paste"))], &["text"]),
        |args, mode| Ok(paste(args, mode)),
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
        "folk_swipe",
        "Unavailable until Praefectus supports artifact-bound coordinate endpoints",
        schema(
            &[
                ("from", string_property("Start coordinate as x,y")),
                ("to", string_property("End coordinate as x,y")),
                (
                    "duration_ms",
                    number_property("Swipe duration in milliseconds"),
                ),
            ],
            &["from", "to"],
        ),
        |args, mode| Ok(swipe(args, mode)),
    );
    table.register(
        "folk_drag",
        "Unavailable until Praefectus supports artifact-bound coordinate endpoints",
        schema(
            &[
                ("from", string_property("Start coordinate as x,y")),
                ("to", string_property("End coordinate as x,y")),
                (
                    "duration_ms",
                    number_property("Drag duration in milliseconds"),
                ),
            ],
            &["from", "to"],
        ),
        |args, mode| Ok(drag(args, mode)),
    );
    table.register(
        "folk_move",
        "Move the pointer to a resolved UI element on the Folk Around host computer; raw coordinates require Praefectus artifact provenance",
        schema(
            &[
                (
                    "element_id",
                    string_property("Stable element ID from folk_ui_snapshot"),
                ),
                (
                    "snapshot",
                    string_property("Optional snapshot id from folk_ui_snapshot"),
                ),
                ("x", number_property("Screen x coordinate")),
                ("y", number_property("Screen y coordinate")),
            ],
            &[],
        ),
        |args, mode| Ok(move_pointer(args, mode)),
    );
    table.register(
        "folk_set_value",
        "Set the value of an editable Praefectus semantic element on the Folk Around host computer",
        strict_schema(
            &[
                (
                    "operation_id",
                    constrained_string_property(
                        "Stable operation ID for durable at-most-once dispatch",
                        1,
                        256,
                        "^[A-Za-z0-9_:-]+$",
                    ),
                ),
                (
                    "observation_id",
                    constrained_string_property(
                        "Observation ID from folk_ui_snapshot",
                        64,
                        64,
                        "^[0-9a-f]{64}$",
                    ),
                ),
                (
                    "tag",
                    constrained_string_property(
                        "Opaque element tag from folk_ui_snapshot",
                        2,
                        5,
                        "^e[0-9]{1,4}$",
                    ),
                ),
                (
                    "value",
                    constrained_string_property("Value to set", 0, 16 * 1024, "^[\\s\\S]*$"),
                ),
                ("interaction_mode", interaction_mode_property()),
            ],
            &[
                "operation_id",
                "observation_id",
                "tag",
                "value",
                "interaction_mode",
            ],
        ),
        |args, mode| Ok(set_value(args, mode)),
    );
    table.register(
        "folk_perform_action",
        "Perform an accessibility action on a resolved UI element on the Folk Around host computer",
        schema(
            &[
                (
                    "on",
                    string_property("Stable element ID or label from folk_ui_snapshot"),
                ),
                ("action", string_property("Accessibility action to perform")),
                (
                    "snapshot",
                    string_property("Optional snapshot id from folk_ui_snapshot"),
                ),
            ],
            &["on", "action"],
        ),
        |args, mode| Ok(perform_action(args, mode)),
    );
    table.register(
        "folk_window",
        "List, focus, move, resize, minimize, close, or set bounds of windows on the Folk Around host computer",
        schema(
            &[
                (
                    "action",
                    string_property("Window action: list, focus, close, minimize, move, resize, set-bounds"),
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
        "List, launch, activate, switch, hide, unhide, or quit apps on the Folk Around host computer",
        schema(
            &[
                (
                    "action",
                    string_property("App action: list, launch, activate, switch, hide, unhide, quit"),
                ),
                ("app", string_property("Application name")),
            ],
            &["action"],
        ),
        |args, mode| Ok(app(args, mode)),
    );
    table.register(
        "folk_open",
        "Open a path or URL on the Folk Around host computer",
        schema(
            &[
                ("target", string_property("Path or URL to open")),
                ("app", string_property("Optional app to open with")),
                (
                    "no_focus",
                    bool_property("Open without focusing the target app"),
                ),
            ],
            &["target"],
        ),
        |args, mode| Ok(open_target(args, mode)),
    );
    table.register(
        "folk_menu",
        "Inspect or click menu items on the Folk Around host computer",
        schema(
            &[
                (
                    "action",
                    string_property("Menu action: list, list-all, inspect, or click"),
                ),
                ("app", string_property("Application name")),
                ("menu", string_property("Menu name")),
                ("item", string_property("Menu item name")),
            ],
            &["action"],
        ),
        |args, mode| Ok(menu(args, mode)),
    );
    table.register(
        "folk_sleep",
        "Sleep for a number of seconds on the Folk Around host computer",
        schema(
            &[("seconds", number_property("Sleep duration in seconds"))],
            &["seconds"],
        ),
        |args, _| Ok(sleep(args)),
    );
    table.register(
        "folk_clean",
        "Remove cached snapshots on the Folk Around host computer",
        schema(
            &[
                (
                    "all_snapshots",
                    bool_property("Remove all cached snapshots"),
                ),
                ("snapshot", string_property("Remove a specific snapshot id")),
            ],
            &[],
        ),
        |args, mode| Ok(clean(args, mode)),
    );

    let semantic_observation = semantic_click || semantic_set_value;
    table.retain(|tool| match tool.name {
        "folk_ui_snapshot" | "folk_see" => semantic_observation,
        "folk_click" => semantic_click,
        "folk_set_value" => semantic_set_value,
        "folk_press"
        | "folk_type"
        | "folk_paste"
        | "folk_hotkey"
        | "folk_scroll"
        | "folk_swipe"
        | "folk_drag"
        | "folk_move"
        | "folk_perform_action"
        | "folk_window"
        | "folk_app"
        | "folk_open"
        | "folk_menu"
        | "folk_sleep"
        | "folk_clean"
        | "folk_doctor" => false,
        _ => true,
    });
}

fn schema(fields: &[(&'static str, Value)], required: &[&'static str]) -> Value {
    object_schema(fields.iter().cloned().collect(), required)
}

fn strict_schema(fields: &[(&'static str, Value)], required: &[&'static str]) -> Value {
    let mut value = schema(fields, required);
    value["additionalProperties"] = Value::Bool(false);
    value
}

fn constrained_string_property(
    description: &'static str,
    min_length: usize,
    max_length: usize,
    pattern: &'static str,
) -> Value {
    json!({
        "type": "string",
        "description": description,
        "minLength": min_length,
        "maxLength": max_length,
        "pattern": pattern,
    })
}

fn interaction_mode_property() -> Value {
    json!({
        "type": "string",
        "description": "Delivery mode: interactive or background_only",
        "enum": ["interactive", "background_only"],
    })
}

fn bool_property(description: &'static str) -> Value {
    json!({
        "type": "boolean",
        "description": description
    })
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
        peekaboo()
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

fn clipboard_write(args: Value, mode: AccessMode) -> Value {
    let result = (|| {
        ensure_mutation(mode)?;
        let text = str_arg(&args, "text").ok_or(ToolExecError::Missing("text"))?;
        peekaboo().clipboard_write(text)?;
        Ok(text_result("copied to clipboard"))
    })();
    flatten(result)
}

fn doctor() -> Value {
    flatten(
        peekaboo()
            .doctor()
            .map(|value| json_text_result(&value))
            .map_err(ToolExecError::from),
    )
}

fn screen_capture(args: Value, _mode: AccessMode) -> Value {
    let result = (|| {
        // ponytail: ensure_observation removed - was a no-op. Add back when a mode restricts observation.

        ensure_screenshot_reference_supported()?;
        let target = str_arg(&args, "target").unwrap_or("display");
        if !matches!(target, "display" | "window") {
            return Err(ToolExecError::CoordinatesUnavailable);
        }
        let artifact = folk_core::create_artifact_file()
            .map_err(|_| ToolExecError::ScreenshotReferenceUnavailable)?;
        let path = artifact.path;
        let capture = if target == "window" {
            peekaboo().image(ImageMode::Window, Some(path.clone()), true)?
        } else {
            peekaboo().image(ImageMode::Screen, Some(path.clone()), true)?
        };
        let metadata = json!({
            "target": target,
        });
        image_result(metadata, &capture, &artifact.locator)
    })();
    flatten(result)
}

fn image(args: Value, _mode: AccessMode) -> Value {
    let result = (|| {
        ensure_screenshot_reference_supported()?;
        let artifact = folk_core::create_artifact_file()
            .map_err(|_| ToolExecError::ScreenshotReferenceUnavailable)?;
        let path = artifact.path;
        let retina = args.get("retina").and_then(Value::as_bool).unwrap_or(true);
        let capture = if let Some(app) = str_arg(&args, "app") {
            peekaboo().image_app(app, Some(path.clone()), retina)?
        } else {
            let capture_mode = ImageMode::parse_or_err(str_arg(&args, "mode").unwrap_or("screen"))?;
            peekaboo().image(capture_mode, Some(path.clone()), retina)?
        };
        let metadata = json!({
            "mode": capture.mode,
        });
        image_result(metadata, &capture, &artifact.locator)
    })();
    flatten(result)
}

fn see(args: Value, _mode: AccessMode) -> Value {
    let result = (|| {
        ensure_screenshot_reference_supported()?;
        let capture_mode = ImageMode::parse_or_err(str_arg(&args, "mode").unwrap_or("screen"))?;
        let artifact = folk_core::create_artifact_file()
            .map_err(|_| ToolExecError::ScreenshotReferenceUnavailable)?;
        let path = artifact.path;
        let retina = args.get("retina").and_then(Value::as_bool).unwrap_or(true);
        let observation = praefectus_adapter::observe_semantic()?;
        let capture = peekaboo().image(capture_mode, Some(path), retina)?;
        let metadata = json!({
            "observation": observation,
            "mode": capture.mode,
        });
        image_result(metadata, &capture, &artifact.locator)
    })();
    flatten(result)
}

fn list_screens(_mode: AccessMode) -> Value {
    let result = (|| Ok(json_text_result(&peekaboo().list_screens()?)))();
    flatten(result)
}

fn permissions(args: Value, mode: AccessMode) -> Value {
    let result: Result<Value, ToolExecError> = (|| {
        if str_arg(&args, "action") == Some("grant") {
            ensure_mutation(mode)?;
            Ok(peekaboo().grant_permissions()?)
        } else {
            Ok(peekaboo().permissions())
        }
    })();
    match result {
        Ok(value) => json_text_result(&value),
        Err(err) => err_result(err.to_string()),
    }
}

fn ui_snapshot(_mode: AccessMode) -> Value {
    let result = (|| {
        Ok(json_text_result(&serde_json::to_value(
            praefectus_adapter::observe_semantic()?,
        )?))
    })();
    flatten(result)
}

fn click(args: Value, mode: AccessMode) -> Value {
    let result = (|| {
        ensure_mutation(mode)?;
        ensure_semantic_args(&args, false)?;
        let operation_id =
            str_arg(&args, "operation_id").ok_or(ToolExecError::Missing("operation_id"))?;
        let observation_id =
            str_arg(&args, "observation_id").ok_or(ToolExecError::Missing("observation_id"))?;
        let tag = str_arg(&args, "tag").ok_or(ToolExecError::Missing("tag"))?;
        let interaction_mode = interaction_mode_arg(&args)?;
        praefectus_adapter::execute_click(mode, operation_id, observation_id, tag, interaction_mode)
    })();
    flatten(result)
}

fn type_text(args: Value, mode: AccessMode) -> Value {
    let result = (|| {
        ensure_mutation(mode)?;
        let text = str_arg(&args, "text").ok_or(ToolExecError::Missing("text"))?;
        let clear = args.get("clear").and_then(Value::as_bool).unwrap_or(false);
        let press_return = args.get("return").and_then(Value::as_bool).unwrap_or(false);
        let delay_ms = args.get("delay_ms").and_then(Value::as_u64);
        peekaboo().type_text(text, clear, press_return, delay_ms, str_arg(&args, "app"))?;
        Ok(text_result("typed"))
    })();
    flatten(result)
}

fn press(args: Value, mode: AccessMode) -> Value {
    let result = (|| {
        ensure_mutation(mode)?;
        let key = str_arg(&args, "key").ok_or(ToolExecError::Missing("key"))?;
        let count = int_arg(&args, "count").unwrap_or(1).max(1) as u32;
        let delay_ms = args.get("delay_ms").and_then(Value::as_u64);
        peekaboo().press(key, count, delay_ms)?;
        Ok(text_result("pressed"))
    })();
    flatten(result)
}

fn paste(args: Value, mode: AccessMode) -> Value {
    let result = (|| {
        ensure_mutation(mode)?;
        let text = str_arg(&args, "text").ok_or(ToolExecError::Missing("text"))?;
        peekaboo().paste(text)?;
        Ok(text_result("pasted"))
    })();
    flatten(result)
}

fn hotkey(args: Value, mode: AccessMode) -> Value {
    let result = (|| {
        ensure_mutation(mode)?;
        let keys = str_arg(&args, "keys").ok_or(ToolExecError::Missing("keys"))?;
        let parts = keys.split('+').map(str::trim).collect::<Vec<_>>();
        peekaboo().hotkey(&parts)?;
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
        peekaboo().scroll(direction, amount)?;
        Ok(text_result("scrolled"))
    })();
    flatten(result)
}

fn swipe(args: Value, mode: AccessMode) -> Value {
    let result = (|| {
        ensure_mutation(mode)?;
        let _ = args;
        Err(ToolExecError::CoordinatesUnavailable)
    })();
    flatten(result)
}

fn drag(args: Value, mode: AccessMode) -> Value {
    let result = (|| {
        ensure_mutation(mode)?;
        let _ = args;
        Err(ToolExecError::CoordinatesUnavailable)
    })();
    flatten(result)
}

fn move_pointer(args: Value, mode: AccessMode) -> Value {
    move_pointer_with_adapter(args, mode, praefectus_adapter::execute_move)
}

fn move_pointer_with_adapter(
    args: Value,
    mode: AccessMode,
    execute_coordinate: impl FnOnce(AccessMode, i64, i64) -> Result<Option<Value>, ToolExecError>,
) -> Value {
    let result = (|| {
        ensure_mutation(mode)?;
        let target = if let Some(element_id) = str_arg(&args, "element_id") {
            Target::Query {
                query: element_id.to_string(),
                snapshot: str_arg(&args, "snapshot").map(str::to_string),
            }
        } else {
            if let Some(response) = execute_coordinate(
                mode,
                int_arg(&args, "x").ok_or(ToolExecError::Missing("x"))?,
                int_arg(&args, "y").ok_or(ToolExecError::Missing("y"))?,
            )? {
                return Ok(response);
            }
            Target::Point(Point {
                x: int_arg(&args, "x").ok_or(ToolExecError::Missing("x"))?,
                y: int_arg(&args, "y").ok_or(ToolExecError::Missing("y"))?,
            })
        };
        peekaboo().move_cursor(target)?;
        Ok(text_result("moved"))
    })();
    flatten(result)
}

fn set_value(args: Value, mode: AccessMode) -> Value {
    let result = (|| {
        ensure_mutation(mode)?;
        ensure_semantic_args(&args, true)?;
        let operation_id =
            str_arg(&args, "operation_id").ok_or(ToolExecError::Missing("operation_id"))?;
        let observation_id =
            str_arg(&args, "observation_id").ok_or(ToolExecError::Missing("observation_id"))?;
        let tag = str_arg(&args, "tag").ok_or(ToolExecError::Missing("tag"))?;
        let value = str_arg(&args, "value").ok_or(ToolExecError::Missing("value"))?;
        let interaction_mode = interaction_mode_arg(&args)?;
        praefectus_adapter::execute_set_value(
            mode,
            operation_id,
            observation_id,
            tag,
            value,
            interaction_mode,
        )
    })();
    flatten(result)
}

fn perform_action(args: Value, mode: AccessMode) -> Value {
    let result = (|| {
        ensure_mutation(mode)?;
        let on = str_arg(&args, "on").ok_or(ToolExecError::Missing("on"))?;
        let snapshot = str_arg(&args, "snapshot").map(str::to_string);
        let action = str_arg(&args, "action").ok_or(ToolExecError::Missing("action"))?;
        peekaboo().perform_action(
            Target::Query {
                query: on.to_string(),
                snapshot,
            },
            action,
        )?;
        Ok(text_result("action performed"))
    })();
    flatten(result)
}

fn open_target(args: Value, mode: AccessMode) -> Value {
    let result = (|| {
        ensure_mutation(mode)?;
        let target = str_arg(&args, "target").ok_or(ToolExecError::Missing("target"))?;
        let app = str_arg(&args, "app");
        let no_focus = args
            .get("no_focus")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        peekaboo().open(target, app, no_focus)?;
        Ok(text_result("opened"))
    })();
    flatten(result)
}

fn sleep(args: Value) -> Value {
    let seconds = args.get("seconds").and_then(Value::as_f64).unwrap_or(0.0);
    let millis = (seconds.max(0.0) * 1000.0) as u64;
    std::thread::sleep(std::time::Duration::from_millis(millis));
    json_text_result(&json!({ "slept_ms": millis }))
}

fn clean(args: Value, mode: AccessMode) -> Value {
    let result = (|| {
        ensure_mutation(mode)?;
        let all = args
            .get("all_snapshots")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let snapshot = str_arg(&args, "snapshot");
        let removed = rs_peekaboo::cache::clean_snapshots(all, snapshot)?;
        Ok(json_text_result(&json!({ "removed": removed })))
    })();
    flatten(result)
}

fn window(args: Value, mode: AccessMode) -> Value {
    let action = str_arg(&args, "action").unwrap_or("");
    if action == "list" {
        return flatten(
            peekaboo()
                .window("list", None, None, None)
                .map(|value| json_text_result(&value))
                .map_err(ToolExecError::from),
        );
    }
    let result = (|| {
        if matches!(action, "focus" | "close" | "minimize") {
            ensure_safe_focus_or_full(mode, action)?;
        } else {
            ensure_mutation(mode)?;
        }
        let app = str_arg(&args, "app").ok_or(ToolExecError::Missing("app"))?;
        match action {
            "focus" | "close" | "minimize" => peekaboo().window(action, Some(app), None, None)?,
            "move" | "resize" | "set-bounds" => {
                peekaboo().window(action, Some(app), None, Some(window_bounds(action, &args)?))?
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
        "set-bounds" => Ok(Bounds {
            x: int_arg(args, "x").ok_or(ToolExecError::Missing("x"))?,
            y: int_arg(args, "y").ok_or(ToolExecError::Missing("y"))?,
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
        if matches!(action, "list" | "activate" | "focus" | "switch") {
            ensure_safe_focus_or_full(mode, action)?;
        } else {
            ensure_mutation(mode)?;
        }
        let app = str_arg(&args, "app").ok_or(ToolExecError::Missing("app"))?;
        match action {
            "launch" | "activate" | "switch" | "hide" | "unhide" | "quit" => {
                peekaboo().app(action, Some(app))?
            }
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
        if matches!(action, "list" | "list-all" | "inspect") {
            // ponytail: ensure_observation removed - was a no-op.

            let action_name = if action == "inspect" { "list" } else { action };
            return Ok(json_text_result(&peekaboo().menu(
                action_name,
                app,
                None,
                None,
            )?));
        }
        ensure_mutation(mode)?;
        let menu = str_arg(&args, "menu").ok_or(ToolExecError::Missing("menu"))?;
        let item = str_arg(&args, "item").ok_or(ToolExecError::Missing("item"))?;
        peekaboo().menu("click", app, Some(menu), Some(item))?;
        Ok(text_result("menu action complete"))
    })();
    flatten(result)
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

fn image_result(
    value: Value,
    capture: &ImageCapture,
    locator: &str,
) -> Result<Value, ToolExecError> {
    if capture.ephemeral {
        let _ = std::fs::remove_file(&capture.path);
        return Err(ToolExecError::ScreenshotReferenceUnavailable);
    }
    let artifact = screenshot_reference(capture);
    let mut value = value;
    value["artifact"] = artifact?;
    value["artifact"]["locator"] = json!(locator);
    let text = serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string());
    Ok(json!({
        "content": [{ "type": "text", "text": text }],
        "structuredContent": value
    }))
}

fn ensure_screenshot_reference_supported() -> Result<(), ToolExecError> {
    if cfg!(unix) {
        Ok(())
    } else {
        Err(ToolExecError::ScreenshotReferenceUnavailable)
    }
}

#[cfg(unix)]
fn screenshot_reference(capture: &ImageCapture) -> Result<Value, ToolExecError> {
    use std::os::unix::fs::PermissionsExt;

    let mut file = File::open(&capture.path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > MAX_SCREENSHOT_BYTES {
        return Err(ToolExecError::ScreenshotReferenceUnavailable);
    }
    file.set_permissions(Permissions::from_mode(0o600))?;
    if file.metadata()?.permissions().mode() & 0o077 != 0 {
        return Err(ToolExecError::ScreenshotReferenceUnavailable);
    }

    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 8192];
    let mut read = 0_u64;
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        read = read
            .checked_add(count as u64)
            .ok_or(ToolExecError::ScreenshotReferenceUnavailable)?;
        if read > MAX_SCREENSHOT_BYTES {
            return Err(ToolExecError::ScreenshotReferenceUnavailable);
        }
        hasher.update(&buffer[..count]);
    }

    Ok(json!({
        "contentHash": format!("sha256:{:x}", hasher.finalize()),
        "mimeType": capture.mime_type,
    }))
}

#[cfg(not(unix))]
fn screenshot_reference(_capture: &ImageCapture) -> Result<Value, ToolExecError> {
    Err(ToolExecError::ScreenshotReferenceUnavailable)
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

fn ensure_exact_args(args: &Value, allowed: &[&str]) -> Result<(), ToolExecError> {
    let arguments = args.as_object().ok_or(ToolExecError::InvalidArguments)?;
    if arguments.keys().all(|key| allowed.contains(&key.as_str())) {
        Ok(())
    } else {
        Err(ToolExecError::InvalidArguments)
    }
}

fn ensure_semantic_args(args: &Value, set_value: bool) -> Result<(), ToolExecError> {
    let allowed = if set_value {
        &[
            "operation_id",
            "observation_id",
            "tag",
            "value",
            "interaction_mode",
        ][..]
    } else {
        &["operation_id", "observation_id", "tag", "interaction_mode"][..]
    };
    ensure_exact_args(args, allowed)?;
    let operation_id = str_arg(args, "operation_id").ok_or(ToolExecError::InvalidArguments)?;
    let observation_id = str_arg(args, "observation_id").ok_or(ToolExecError::InvalidArguments)?;
    let tag = str_arg(args, "tag").ok_or(ToolExecError::InvalidArguments)?;
    let valid_operation_id = !operation_id.is_empty()
        && operation_id.len() <= 256
        && operation_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b':'));
    let valid_observation_id = observation_id.len() == 64
        && observation_id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
    let valid_tag = tag.strip_prefix('e').is_some_and(|index| {
        !index.is_empty() && index.len() <= 4 && index.bytes().all(|byte| byte.is_ascii_digit())
    });
    let valid_value =
        !set_value || str_arg(args, "value").is_some_and(|value| value.len() <= 16 * 1024);
    let valid_interaction_mode = str_arg(args, "interaction_mode")
        .is_some_and(|mode| matches!(mode, "interactive" | "background_only"));
    if valid_operation_id
        && valid_observation_id
        && valid_tag
        && valid_value
        && valid_interaction_mode
    {
        Ok(())
    } else {
        Err(ToolExecError::InvalidArguments)
    }
}

fn interaction_mode_arg(args: &Value) -> Result<praefectus::InteractionMode, ToolExecError> {
    match str_arg(args, "interaction_mode") {
        Some("interactive") => Ok(praefectus::InteractionMode::Interactive),
        Some("background_only") => Ok(praefectus::InteractionMode::BackgroundOnly),
        _ => Err(ToolExecError::InvalidArguments),
    }
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
    fn unsupported_ui_effects_are_not_advertised_in_sandbox() {
        let mut table = ToolTable::new(AccessMode::Sandbox);
        register_tools(&mut table);
        assert!(table.list().iter().all(|tool| tool.name != "folk_type"));
    }

    #[test]
    fn unsupported_ui_effects_are_not_advertised_in_limited() {
        let mut table = ToolTable::new(AccessMode::Limited);
        register_tools(&mut table);
        assert!(table.list().iter().all(|tool| tool.name != "folk_type"));
    }

    #[test]
    fn advertised_semantic_tools_never_expose_coordinates() {
        let mut table = ToolTable::new(AccessMode::Full);
        register_tools_with_semantic_capabilities(&mut table, true, true);
        let tools = table
            .list()
            .iter()
            .filter(|tool| matches!(tool.name, "folk_click" | "folk_set_value"))
            .collect::<Vec<_>>();
        assert_eq!(tools.len(), 2);
        for tool in tools {
            assert_eq!(tool.input_schema["additionalProperties"], false);
            let properties = tool.input_schema["properties"].as_object().unwrap();
            assert!(properties.contains_key("operation_id"));
            assert!(properties.contains_key("observation_id"));
            assert!(properties.contains_key("tag"));
            assert_eq!(
                properties["interaction_mode"]["enum"],
                json!(["interactive", "background_only"])
            );
            assert_eq!(properties["operation_id"]["maxLength"], 256);
            assert_eq!(properties["observation_id"]["pattern"], "^[0-9a-f]{64}$");
            assert_eq!(properties["tag"]["pattern"], "^e[0-9]{1,4}$");
            assert!(!properties.contains_key("x"));
            assert!(!properties.contains_key("y"));
            assert!(!properties.contains_key("selector"));
        }
    }

    #[test]
    fn semantic_effects_reject_unknown_arguments_before_target_resolution() {
        let mut table = ToolTable::new(AccessMode::Full);
        register_tools_with_semantic_capabilities(&mut table, true, true);
        let response = handle_message(
            false,
            &table,
            json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"folk_click","arguments":{"operation_id":"op","observation_id":"missing","tag":"e0","x":1}}}),
        )
        .unwrap()
        .unwrap();

        assert!(response.contains("invalid tool arguments"));
        assert!(!response.contains("semantic computer-use target is unavailable"));
    }

    #[test]
    fn semantic_effect_argument_bounds_match_the_protocol() {
        let observation_id = "a".repeat(64);
        assert!(
            ensure_semantic_args(
                &json!({"operation_id":"op:1","observation_id":observation_id,"tag":"e4095","interaction_mode":"interactive"}),
                false,
            )
            .is_ok()
        );
        for invalid in [
            json!({"operation_id":"bad/id","observation_id":"a".repeat(64),"tag":"e0","interaction_mode":"interactive"}),
            json!({"operation_id":"op","observation_id":"A".repeat(64),"tag":"e0","interaction_mode":"interactive"}),
            json!({"operation_id":"op","observation_id":"a".repeat(64),"tag":"e12345","interaction_mode":"interactive"}),
            json!({"operation_id":"op","observation_id":"a".repeat(64),"tag":"e0","interaction_mode":"unknown"}),
            json!({"operation_id":"op","observation_id":"a".repeat(64),"tag":"e0","interaction_mode":"interactive","session_isolation":"host_isolated"}),
            json!({"operation_id":"op","observation_id":"a".repeat(64),"tag":"e0","value":"x".repeat(16 * 1024 + 1),"interaction_mode":"interactive"}),
        ] {
            assert!(ensure_semantic_args(&invalid, invalid.get("value").is_some()).is_err());
        }
    }

    #[test]
    fn host_mode_blocks_semantic_effects_before_argument_resolution() {
        for mode in [AccessMode::Limited, AccessMode::Sandbox] {
            let mut table = ToolTable::new(mode);
            register_tools_with_semantic_capabilities(&mut table, true, true);
            for name in ["folk_click", "folk_set_value"] {
                let response = handle_message(
                    false,
                    &table,
                    json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":name,"arguments":{}}}),
                )
                .unwrap()
                .unwrap();
                assert!(response.contains("action blocked in this mode"));
                assert!(!response.contains("missing required argument"));
            }
        }
    }

    #[test]
    fn praefectus_errors_should_not_expose_private_details() {
        let error = ToolExecError::Praefectus(praefectus::ProtocolError::Io(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "private state path: /Users/private-user/.local/state",
        )));

        assert_eq!(error.to_string(), "computer-use protocol request failed");
    }

    #[test]
    fn backend_errors_should_not_expose_private_details() {
        let error = ToolExecError::Peekaboo(rs_peekaboo::PeekabooError::System(
            "private state path: /Users/private-user/.local/state".to_string(),
        ));

        assert_eq!(error.to_string(), "computer-use backend request failed");
    }

    #[test]
    fn limited_launch_should_be_blocked() {
        assert!(ensure_safe_focus_or_full(AccessMode::Limited, "launch").is_err());
        assert!(ensure_safe_focus_or_full(AccessMode::Limited, "activate").is_ok());
    }

    #[test]
    fn restricted_clipboard_write_should_be_blocked_before_effect() {
        let mut table = ToolTable::new(AccessMode::Limited);
        register_tools(&mut table);
        let response = table.call("folk_clipboard_write", json!({ "text": "private" }));

        assert!(response.to_string().contains("action blocked in this mode"));
    }

    #[test]
    fn restricted_permission_grant_should_be_blocked_before_effect() {
        let mut table = ToolTable::new(AccessMode::Sandbox);
        register_tools(&mut table);
        let response = table.call("folk_permissions", json!({ "action": "grant" }));

        assert!(response.to_string().contains("action blocked in this mode"));
    }

    #[test]
    fn restricted_permission_inspection_should_remain_read_only() {
        let response = permissions(json!({}), AccessMode::Limited);

        assert_ne!(response.get("isError").and_then(Value::as_bool), Some(true));
    }

    #[cfg(unix)]
    #[test]
    fn image_result_should_return_only_a_restricted_content_hash() {
        use std::os::unix::fs::PermissionsExt;

        let path = std::env::temp_dir().join(format!(
            "folk-around-image-result-{}.png",
            std::process::id()
        ));
        std::fs::write(&path, [1_u8, 2, 3, 4]).unwrap();

        let capture = ImageCapture {
            path: path.clone(),
            mode: ImageMode::Screen,
            bytes: 4,
            mime_type: "image/png".to_string(),
            ephemeral: false,
        };
        let response = image_result(
            json!({ "mode": "screen" }),
            &capture,
            "folk-artifact:test.png",
        )
        .unwrap();
        let content = response["content"].as_array().unwrap();

        assert_eq!(
            response["structuredContent"]["artifact"]["contentHash"],
            "sha256:9f64a747e1b97f131fabb6b447296c9b6f0201e79fb3c5356e6c77e89b6a806a"
        );
        assert_eq!(
            response["structuredContent"]["artifact"]["locator"],
            "folk-artifact:test.png"
        );
        assert_eq!(content.len(), 1);
        assert_eq!(content[0]["type"], "text");
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o077,
            0
        );
        assert!(
            !response
                .to_string()
                .contains(path.to_string_lossy().as_ref())
        );
        assert!(!response.to_string().contains("AQIDBA=="));
        assert!(!response.to_string().contains("\"path\""));
        assert!(!response.to_string().contains("\"bytes\""));

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn image_result_should_delete_ephemeral_capture_and_fail_closed() {
        let path = std::env::temp_dir().join(format!(
            "folk-around-ephemeral-image-result-{}.png",
            std::process::id()
        ));
        std::fs::write(&path, [1_u8, 2, 3, 4]).unwrap();

        let capture = ImageCapture {
            path: path.clone(),
            mode: ImageMode::Screen,
            bytes: 4,
            mime_type: "image/png".to_string(),
            ephemeral: true,
        };
        let result = image_result(
            json!({ "mode": "screen" }),
            &capture,
            "folk-artifact:test.png",
        );

        assert!(matches!(
            result,
            Err(ToolExecError::ScreenshotReferenceUnavailable)
        ));
        assert!(!path.exists());
    }

    #[test]
    fn tool_registration_reports_only_hardened_runtime_capabilities() {
        let mut table = ToolTable::new(AccessMode::Full);
        register_tools_with_semantic_capabilities(&mut table, false, false);
        let names = table
            .list()
            .iter()
            .map(|tool| tool.name)
            .collect::<Vec<_>>();
        for expected in [
            "folk_shell",
            "folk_system_info",
            "folk_list_apps",
            "folk_spawn",
            "folk_image",
            "folk_list_screens",
            "folk_clipboard_read",
            "folk_clipboard_write",
            "folk_permissions",
            "folk_screen_capture",
        ] {
            assert!(names.contains(&expected));
        }
        for unavailable in [
            "folk_doctor",
            "folk_press",
            "folk_type",
            "folk_paste",
            "folk_hotkey",
            "folk_scroll",
            "folk_swipe",
            "folk_drag",
            "folk_move",
            "folk_perform_action",
            "folk_window",
            "folk_app",
            "folk_open",
            "folk_menu",
            "folk_sleep",
            "folk_clean",
        ] {
            assert!(!names.contains(&unavailable));
        }
        for tool in table
            .list()
            .iter()
            .filter(|tool| matches!(tool.name, "folk_image" | "folk_see" | "folk_screen_capture"))
        {
            assert!(
                tool.input_schema
                    .get("properties")
                    .and_then(|value| value.get("path"))
                    .is_none()
            );
        }
    }

    #[test]
    fn runtime_semantic_capabilities_control_exact_tool_advertisement() {
        let mut table = ToolTable::new(AccessMode::Full);
        register_tools(&mut table);
        let names = table
            .list()
            .iter()
            .map(|tool| tool.name)
            .collect::<Vec<_>>();

        assert_eq!(
            names.contains(&"folk_click"),
            praefectus_adapter::supports_semantic_action("invoke")
        );
        assert_eq!(
            names.contains(&"folk_set_value"),
            praefectus_adapter::supports_semantic_action("set_value")
        );
        assert_eq!(
            names.contains(&"folk_ui_snapshot"),
            praefectus_adapter::supports_semantic_action("invoke")
                || praefectus_adapter::supports_semantic_action("set_value")
        );
    }

    #[test]
    fn semantic_capability_filter_does_not_advertise_a_different_effect() {
        for (click, set_value) in [(true, false), (false, true)] {
            let mut table = ToolTable::new(AccessMode::Full);
            register_tools_with_semantic_capabilities(&mut table, click, set_value);
            let names = table
                .list()
                .iter()
                .map(|tool| tool.name)
                .collect::<Vec<_>>();

            assert_eq!(names.contains(&"folk_click"), click);
            assert_eq!(names.contains(&"folk_set_value"), set_value);
            assert!(names.contains(&"folk_ui_snapshot"));
        }
    }
}
