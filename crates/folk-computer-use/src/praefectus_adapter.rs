use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use ed25519_dalek::{Signer, SigningKey};
use folk_core::AccessMode;
use folk_mcp::json_text_result;
use praefectus::{
    Action, ActionRequest, AuthorityGrant, CancellationToken, Capabilities,
    Ed25519AuthorityVerifier, Engine, ExecuteReport, Executor, MouseButton, NativeExecutor,
    PROTOCOL_VERSION, SafetyClass, SignedAuthority, TargetRef, Terminal, VerificationPolicy,
    canonical_authority_bytes, normalized_action_hash,
};
use serde_json::{Value, json, to_value};

use crate::ToolExecError;

static HOST_AUTHORITY: OnceLock<(SigningKey, String)> = OnceLock::new();

pub(crate) fn execute_click(
    mode: AccessMode,
    x: i64,
    y: i64,
    button: &str,
    count: u32,
    background: bool,
) -> Result<Option<Value>, ToolExecError> {
    super::ensure_mutation(mode)?;
    if !click_is_candidate(button, count, background) {
        return Ok(None);
    }
    let button = match button {
        "left" => MouseButton::Left,
        "right" => MouseButton::Right,
        _ => return Ok(None),
    };
    try_execute(
        Action::Click {
            button,
            count,
            allow_coordinate_fallback: false,
        },
        x,
        y,
    )
}

pub(crate) fn execute_move(
    mode: AccessMode,
    x: i64,
    y: i64,
) -> Result<Option<Value>, ToolExecError> {
    super::ensure_mutation(mode)?;
    try_execute(Action::Move, x, y)
}

fn try_execute(action: Action, x: i64, y: i64) -> Result<Option<Value>, ToolExecError> {
    let executor = NativeExecutor::default();
    let Ok(capabilities) = executor.capabilities() else {
        return Ok(None);
    };
    if !supports_action(&capabilities, action_name(&action)) {
        return Ok(None);
    }
    let observation = executor.observe_coordinates()?;
    let display = observation
        .displays
        .iter()
        .find(|display| {
            display.width > 0
                && display.height > 0
                && x >= display.x
                && y >= display.y
                && x < display.x.saturating_add(display.width)
                && y < display.y.saturating_add(display.height)
        })
        .ok_or(ToolExecError::Missing("coordinate on a display"))?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(std::io::Error::other)?
        .as_millis() as i64;
    let (signing_key, session_id) = HOST_AUTHORITY.get_or_init(|| {
        (
            SigningKey::from_bytes(&rand::random()),
            format!("folk-process-{:032x}", rand::random::<u128>()),
        )
    });
    let operation_id = new_operation_id();
    let issuer = "folk-around".to_string();
    let key_id = "process-key".to_string();
    let policy_generation = "folk-full-v1".to_string();
    let subject = "folk-local-host".to_string();
    let (safety, verification) = action_policy(&action);
    let mut request = ActionRequest {
        protocol_version: PROTOCOL_VERSION,
        action_version: PROTOCOL_VERSION,
        target_version: PROTOCOL_VERSION,
        operation_id: operation_id.clone(),
        subject: subject.clone(),
        session_id: session_id.clone(),
        authority: SignedAuthority {
            grant: AuthorityGrant {
                protocol_version: PROTOCOL_VERSION,
                issuer: issuer.clone(),
                key_id: key_id.clone(),
                operation_id,
                subject,
                session_id: session_id.clone(),
                risk: safety,
                expires_at_ms: now + 30_000,
                policy_generation: policy_generation.clone(),
                action_hash: String::new(),
            },
            signature: String::new(),
        },
        action,
        target: TargetRef::Coordinates {
            x,
            y,
            display_id: display.display_id.clone(),
            display_geometry_hash: observation.display_geometry_hash,
            snapshot_id: observation.snapshot_id,
            snapshot_content_hash: observation.snapshot_content_hash,
        },
        deadline_at_ms: now + 30_000,
        verification,
        safety,
    };
    request.authority.grant.action_hash = normalized_action_hash(&request)?;
    request.authority.signature = signing_key
        .sign(&canonical_authority_bytes(&request.authority.grant)?)
        .to_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let verifier = Ed25519AuthorityVerifier::new([(
        issuer,
        key_id,
        policy_generation,
        signing_key.verifying_key(),
    )]);
    let report = Engine::new(executor, ledger_path()?, verifier)
        .execute(&request, &CancellationToken::default())?;
    report_result(report).map(Some)
}

fn click_is_candidate(button: &str, count: u32, background: bool) -> bool {
    !background && matches!(button, "left" | "right") && (1..=3).contains(&count)
}

fn action_name(action: &Action) -> &'static str {
    match action {
        Action::Click { .. } => "click",
        Action::Move => "move",
        Action::SetValue { .. } => "set_value",
        _ => "",
    }
}

fn supports_action(capabilities: &Capabilities, action: &str) -> bool {
    capabilities
        .permissions
        .get("coordinate_capture")
        .copied()
        .unwrap_or(false)
        && capabilities
            .supported_actions
            .iter()
            .any(|supported| supported == action)
}

fn action_policy(action: &Action) -> (SafetyClass, VerificationPolicy) {
    match action {
        Action::Move => (SafetyClass::Reversible, VerificationPolicy::None),
        _ => (SafetyClass::External, VerificationPolicy::SnapshotChanged),
    }
}

fn report_result(report: ExecuteReport) -> Result<Value, ToolExecError> {
    let retry_safe = report_is_retry_safe(&report);
    Ok(json_text_result(&json!({
        "report": to_value(report)?,
        "retry_safe": retry_safe,
    })))
}

fn new_operation_id() -> String {
    format!("folk-{:032x}", rand::random::<u128>())
}

fn ledger_path() -> Result<PathBuf, ToolExecError> {
    ledger_path_from(
        std::env::var_os("XDG_STATE_HOME").map(PathBuf::from),
        std::env::var_os("HOME").map(PathBuf::from),
    )
    .ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "HOME and XDG_STATE_HOME are not set",
        )
        .into()
    })
}

fn ledger_path_from(xdg_state_home: Option<PathBuf>, home: Option<PathBuf>) -> Option<PathBuf> {
    let state_home = match xdg_state_home.filter(|path| !path.as_os_str().is_empty()) {
        Some(path) if path.is_absolute() => path,
        Some(_) => return None,
        None => match home.filter(|path| !path.as_os_str().is_empty()) {
            Some(path) if path.is_absolute() => path.join(".local").join("state"),
            _ => return None,
        },
    };
    Some(
        state_home
            .join("folk-around")
            .join("praefectus")
            .join("operations.jsonl"),
    )
}

fn report_is_retry_safe(report: &ExecuteReport) -> bool {
    let mut terminals = report
        .acknowledgements
        .iter()
        .filter_map(|acknowledgement| match &acknowledgement.state {
            praefectus::AckState::Terminal { terminal } => Some(&**terminal),
            _ => None,
        });
    let Some(first) = terminals.next() else {
        return false;
    };
    terminal_proves_no_effect(first) && terminals.all(terminal_proves_no_effect)
}

fn terminal_proves_no_effect(terminal: &Terminal) -> bool {
    matches!(
        terminal,
        Terminal::Rejected { .. } | Terminal::CancelledBeforeEffect | Terminal::ExpiredBeforeEffect
    )
}

#[cfg(test)]
mod tests {
    use praefectus::{AckState, ActionAck, Effect, FailureCode, Receipt};

    use super::*;

    #[test]
    fn full_access_check_precedes_authority_issuance() {
        let result = execute_move(AccessMode::Limited, 0, 0);
        assert!(matches!(result, Err(ToolExecError::Blocked)));
    }

    #[test]
    fn coordinate_click_routing_preserves_existing_semantics() {
        assert!(click_is_candidate("left", 1, false));
        assert!(click_is_candidate("right", 3, false));
        assert!(!click_is_candidate("left", 1, true));
        assert!(!click_is_candidate("middle", 1, false));
        assert!(!click_is_candidate("left", 0, false));
        assert!(!click_is_candidate("left", 4, false));
    }

    #[test]
    fn coordinate_routing_requires_capture_and_exact_action_support() {
        let mut capabilities = Capabilities {
            platform: "test".to_string(),
            backend: "test".to_string(),
            supported_actions: vec!["click".to_string()],
            permissions: [("coordinate_capture".to_string(), true)]
                .into_iter()
                .collect(),
            display_geometry_hash: "0".repeat(64),
        };
        assert!(supports_action(&capabilities, "click"));
        assert!(!supports_action(&capabilities, "move"));
        capabilities
            .permissions
            .insert("coordinate_capture".to_string(), false);
        assert!(!supports_action(&capabilities, "click"));
    }

    #[test]
    fn folk_ledger_is_outside_the_working_directory() {
        let state_root = std::env::temp_dir().join("folk-state");
        let path = ledger_path_from(Some(state_root.clone()), None)
            .expect("XDG state path should resolve");
        assert_eq!(
            path,
            state_root
                .join("folk-around")
                .join("praefectus")
                .join("operations.jsonl")
        );
    }

    #[test]
    fn folk_ledger_rejects_relative_state_roots() {
        assert_eq!(
            (
                ledger_path_from(
                    Some(PathBuf::from("relative-state")),
                    Some(PathBuf::from("/home/folk")),
                ),
                ledger_path_from(None, Some(PathBuf::from("relative-home"))),
            ),
            (None, None)
        );
    }

    #[test]
    fn click_authority_uses_external_risk() {
        let (safety, verification) = action_policy(&Action::Click {
            button: MouseButton::Left,
            count: 1,
            allow_coordinate_fallback: false,
        });

        assert!(
            safety == SafetyClass::External
                && matches!(verification, VerificationPolicy::SnapshotChanged)
        );
    }

    #[test]
    fn move_authority_is_reversible_without_verification() {
        let (safety, verification) = action_policy(&Action::Move);

        assert!(
            safety == SafetyClass::Reversible && matches!(verification, VerificationPolicy::None)
        );
    }

    #[test]
    fn operation_ids_are_random_and_fixed_width() {
        let first = new_operation_id();
        let second = new_operation_id();
        assert_ne!(first, second);
        assert_eq!(first.len(), 37);
        assert!(
            first
                .strip_prefix("folk-")
                .is_some_and(|id| id.bytes().all(|byte| byte.is_ascii_hexdigit()))
        );
    }

    #[test]
    fn no_effect_terminals_serialize_retry_safe_true() {
        for terminal in [
            Terminal::Rejected {
                code: FailureCode::PermissionDenied,
                message: "action rejected".to_string(),
            },
            Terminal::CancelledBeforeEffect,
            Terminal::ExpiredBeforeEffect,
        ] {
            assert!(serialized_retry_safe(terminal));
        }
    }

    #[test]
    fn succeeded_terminal_serializes_retry_safe_false() {
        assert!(!serialized_retry_safe(Terminal::Succeeded {
            receipt: receipt(Effect::Verified),
        }));
    }

    #[test]
    fn outcome_unknown_serializes_retry_safe_false() {
        assert!(!serialized_retry_safe(Terminal::OutcomeUnknown {
            receipt: receipt(Effect::Unknown),
            message: "delivery could not be determined".to_string(),
        }));
    }

    #[test]
    fn failed_or_missing_terminal_is_not_retry_safe() {
        let failed = serialized_retry_safe(Terminal::Failed {
            code: FailureCode::DispatchFailed,
            message: "dispatch failed".to_string(),
        });
        let missing = report_is_retry_safe(&ExecuteReport {
            acknowledgements: vec![acknowledgement(AckState::Accepted)],
        });

        assert_eq!((failed, missing), (false, false));
    }

    fn serialized_retry_safe(terminal: Terminal) -> bool {
        let response = report_result(ExecuteReport {
            acknowledgements: vec![acknowledgement(AckState::Terminal {
                terminal: Box::new(terminal),
            })],
        })
        .expect("report should serialize");
        let payload: Value = serde_json::from_str(
            response["content"][0]["text"]
                .as_str()
                .expect("report text should be present"),
        )
        .expect("report text should be JSON");

        payload["retry_safe"]
            .as_bool()
            .expect("retry_safe should be a boolean")
    }

    fn acknowledgement(state: AckState) -> ActionAck {
        ActionAck {
            protocol_version: PROTOCOL_VERSION,
            operation_id: "folk-operation".to_string(),
            sequence: 2,
            action_hash: "0".repeat(64),
            replayed: false,
            state,
        }
    }

    fn receipt(effect: Effect) -> Receipt {
        Receipt {
            protocol_version: PROTOCOL_VERSION,
            action_name: "move".to_string(),
            action_hash: "0".repeat(64),
            started_at_ms: 1,
            finished_at_ms: 2,
            backend: "test".to_string(),
            fallback_chain: Vec::new(),
            effect,
            before: None,
            after: None,
            warnings: Vec::new(),
        }
    }
}
