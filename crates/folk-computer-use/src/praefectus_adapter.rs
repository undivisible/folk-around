use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use ed25519_dalek::{Signer, SigningKey};
use folk_core::AccessMode;
use folk_mcp::json_text_result;
use praefectus::{
    Action, ActionRequest, AuthorityGrant, CancellationToken, Ed25519AuthorityVerifier, Engine,
    ExecuteReport, MouseButton, NativeExecutor, PROTOCOL_VERSION, SafetyClass, SignedAuthority,
    TargetRef, Terminal, VerificationPolicy, canonical_authority_bytes, normalized_action_hash,
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
) -> Result<Value, ToolExecError> {
    super::ensure_mutation(mode)?;
    let button = match button {
        "left" => MouseButton::Left,
        "right" => MouseButton::Right,
        "middle" => MouseButton::Middle,
        _ => return Err(ToolExecError::Missing("valid button")),
    };
    execute(
        Action::Click {
            button,
            count,
            allow_coordinate_fallback: false,
        },
        x,
        y,
    )
}

pub(crate) fn execute_move(mode: AccessMode, x: i64, y: i64) -> Result<Value, ToolExecError> {
    super::ensure_mutation(mode)?;
    execute(Action::Move, x, y)
}

fn execute(action: Action, x: i64, y: i64) -> Result<Value, ToolExecError> {
    let executor = NativeExecutor::default();
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
                risk: SafetyClass::Reversible,
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
        verification: VerificationPolicy::SnapshotChanged,
        safety: SafetyClass::Reversible,
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
    let state_home = xdg_state_home
        .filter(|path| !path.as_os_str().is_empty())
        .or_else(|| {
            home.filter(|path| !path.as_os_str().is_empty())
                .map(|path| path.join(".local").join("state"))
        })?;
    Some(
        state_home
            .join("folk-around")
            .join("praefectus")
            .join("operations.jsonl"),
    )
}

fn report_is_retry_safe(report: &ExecuteReport) -> bool {
    !report.acknowledgements.iter().any(|acknowledgement| {
        matches!(
            &acknowledgement.state,
            praefectus::AckState::Terminal { terminal }
                if matches!(&**terminal, Terminal::OutcomeUnknown { .. })
        )
    })
}

#[cfg(test)]
mod tests {
    use praefectus::{AckState, ActionAck, Effect, Receipt};

    use super::*;

    #[test]
    fn full_access_check_precedes_authority_issuance() {
        let result = execute_move(AccessMode::Limited, 0, 0);
        assert!(matches!(result, Err(ToolExecError::Blocked)));
    }

    #[test]
    fn folk_ledger_is_outside_the_working_directory() {
        let path = ledger_path_from(Some(PathBuf::from("/state")), None)
            .expect("XDG state path should resolve");
        assert_eq!(
            path,
            PathBuf::from("/state/folk-around/praefectus/operations.jsonl")
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
    fn outcome_unknown_is_never_retry_safe() {
        let report = ExecuteReport {
            acknowledgements: vec![ActionAck {
                protocol_version: PROTOCOL_VERSION,
                operation_id: "folk-operation".to_string(),
                sequence: 2,
                action_hash: "0".repeat(64),
                replayed: false,
                state: AckState::Terminal {
                    terminal: Box::new(Terminal::OutcomeUnknown {
                        receipt: Receipt {
                            protocol_version: PROTOCOL_VERSION,
                            action_name: "move".to_string(),
                            action_hash: "0".repeat(64),
                            started_at_ms: 1,
                            finished_at_ms: 2,
                            backend: "test".to_string(),
                            fallback_chain: Vec::new(),
                            effect: Effect::Unknown,
                            before: None,
                            after: None,
                            warnings: Vec::new(),
                        },
                        message: "delivery could not be determined".to_string(),
                    }),
                },
            }],
        };
        assert!(!report_is_retry_safe(&report));
    }
}
