use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use ed25519_dalek::{Signer, SigningKey};
use folk_core::AccessMode;
use folk_mcp::json_text_result;
use praefectus::semantic::{SemanticObservation, SemanticTargetRef};
use praefectus::{
    Action, ActionRequest, AuthorityGrant, CancellationToken, Capabilities,
    Ed25519AuthorityVerifier, Engine, ExecuteReport, Executor, InteractionMode, NativeExecutor,
    PROTOCOL_VERSION, SafetyClass, SignedAuthority, TargetRef, Terminal, VerificationPolicy,
    canonical_authority_bytes, normalized_action_hash,
};
use serde_json::{Value, json, to_value};
use sha2::{Digest, Sha256};

use crate::ToolExecError;

const MAX_CACHED_OBSERVATIONS: usize = 8;
const ACTION_DEADLINE_MS: i64 = 30_000;

static HOST_AUTHORITY: OnceLock<(SigningKey, String)> = OnceLock::new();
static OBSERVATIONS: OnceLock<Mutex<VecDeque<SemanticObservation>>> = OnceLock::new();

pub(crate) fn supports_semantic_action(action: &str) -> bool {
    NativeExecutor::default()
        .capabilities()
        .is_ok_and(|capabilities| supports_action(&capabilities, action))
}

pub(crate) fn observe_semantic() -> Result<SemanticObservation, ToolExecError> {
    let executor = NativeExecutor::default();
    let cancellation = CancellationToken::default();
    let observation =
        executor.observe_semantic(&cancellation, now_ms()?.saturating_add(ACTION_DEADLINE_MS))?;
    cache_observation(observation.clone())?;
    Ok(observation)
}

pub(crate) fn execute_click(
    mode: AccessMode,
    operation_id: &str,
    observation_id: &str,
    tag: &str,
    interaction_mode: InteractionMode,
) -> Result<Value, ToolExecError> {
    super::ensure_mutation(mode)?;
    let (target, deadline_at_ms) = cached_target(observation_id, tag)?;
    execute_semantic(
        Action::Invoke,
        target,
        operation_id,
        deadline_at_ms,
        interaction_mode,
    )
}

pub(crate) fn execute_set_value(
    mode: AccessMode,
    operation_id: &str,
    observation_id: &str,
    tag: &str,
    value: &str,
    interaction_mode: InteractionMode,
) -> Result<Value, ToolExecError> {
    super::ensure_mutation(mode)?;
    let (target, deadline_at_ms) = cached_target(observation_id, tag)?;
    execute_semantic(
        Action::SetValue {
            value: value.to_string(),
        },
        target,
        operation_id,
        deadline_at_ms,
        interaction_mode,
    )
}

pub(crate) fn execute_move(
    mode: AccessMode,
    _x: i64,
    _y: i64,
) -> Result<Option<Value>, ToolExecError> {
    super::ensure_mutation(mode)?;
    Err(ToolExecError::CoordinatesUnavailable)
}

fn execute_semantic(
    action: Action,
    target: SemanticTargetRef,
    operation_id: &str,
    deadline_at_ms: i64,
    interaction_mode: InteractionMode,
) -> Result<Value, ToolExecError> {
    let executor = NativeExecutor::default();
    let capabilities = executor.capabilities()?;
    if !supports_action(&capabilities, action_name(&action)) {
        return Err(ToolExecError::SemanticUnavailable);
    }
    let (signing_key, session_id) = HOST_AUTHORITY.get_or_init(|| {
        (
            SigningKey::from_bytes(&rand::random()),
            format!("folk-process-{:032x}", rand::random::<u128>()),
        )
    });
    let request = signed_request(
        action,
        target,
        operation_id,
        deadline_at_ms,
        signing_key,
        session_id,
        interaction_mode,
    )?;
    let grant = &request.authority.grant;
    let verifier = Ed25519AuthorityVerifier::new([(
        grant.issuer.clone(),
        grant.key_id.clone(),
        grant.policy_generation.clone(),
        signing_key.verifying_key(),
    )])?;
    let report = Engine::new(executor, ledger_path()?, verifier)
        .execute(&request, &CancellationToken::default())?;
    report_result(report)
}

fn signed_request(
    action: Action,
    target: SemanticTargetRef,
    operation_id: &str,
    deadline_at_ms: i64,
    signing_key: &SigningKey,
    session_id: &str,
    interaction_mode: InteractionMode,
) -> Result<ActionRequest, ToolExecError> {
    let operation_id = operation_id.to_string();
    let issuer = "folk-around".to_string();
    let key_id = "process-key".to_string();
    let policy_generation = "folk-full-v2".to_string();
    let subject = "folk-local-host".to_string();
    let (safety, verification) = action_policy(&action);
    let mut request = ActionRequest {
        protocol_version: PROTOCOL_VERSION,
        action_version: PROTOCOL_VERSION,
        target_version: PROTOCOL_VERSION,
        verification_version: PROTOCOL_VERSION,
        operation_id: operation_id.clone(),
        subject: subject.clone(),
        session_id: session_id.to_string(),
        authority: SignedAuthority {
            grant: AuthorityGrant {
                protocol_version: PROTOCOL_VERSION,
                issuer: issuer.clone(),
                key_id: key_id.clone(),
                operation_id,
                subject,
                session_id: session_id.to_string(),
                risk: safety,
                expires_at_ms: deadline_at_ms,
                policy_generation: policy_generation.clone(),
                action_hash: String::new(),
            },
            signature: String::new(),
        },
        action,
        target: TargetRef::Element { target },
        interaction_mode,
        deadline_at_ms,
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
    Ok(request)
}

fn cache_observation(observation: SemanticObservation) -> Result<(), ToolExecError> {
    observation.validate(now_ms()?)?;
    let cache = OBSERVATIONS.get_or_init(|| Mutex::new(VecDeque::new()));
    let mut observations = cache
        .lock()
        .map_err(|_| ToolExecError::SemanticUnavailable)?;
    let now = now_ms()?;
    observations.retain(|cached| cached.expires_at_ms > now);
    observations.retain(|cached| cached.observation_id != observation.observation_id);
    observations.push_back(observation);
    while observations.len() > MAX_CACHED_OBSERVATIONS {
        observations.pop_front();
    }
    Ok(())
}

fn cached_target(
    observation_id: &str,
    tag: &str,
) -> Result<(SemanticTargetRef, i64), ToolExecError> {
    let cache = OBSERVATIONS.get_or_init(|| Mutex::new(VecDeque::new()));
    let observations = cache
        .lock()
        .map_err(|_| ToolExecError::SemanticUnavailable)?;
    let observation = observations
        .iter()
        .find(|observation| observation.observation_id == observation_id)
        .ok_or(ToolExecError::SemanticUnavailable)?;
    observation.validate(now_ms()?)?;
    Ok((observation.target(tag)?, observation.expires_at_ms))
}

fn action_name(action: &Action) -> &'static str {
    match action {
        Action::Invoke => "invoke",
        Action::SetValue { .. } => "set_value",
        _ => "",
    }
}

fn supports_action(capabilities: &Capabilities, action: &str) -> bool {
    !action.is_empty()
        && capabilities
            .supported_actions
            .iter()
            .any(|supported| supported == action)
}

fn action_policy(action: &Action) -> (SafetyClass, VerificationPolicy) {
    let verification = match action {
        Action::SetValue { value } => VerificationPolicy::TargetValueHash {
            sha256: value_hash(value),
        },
        _ => VerificationPolicy::None,
    };
    (SafetyClass::External, verification)
}

fn value_hash(value: &str) -> String {
    Sha256::digest(value.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn report_result(report: ExecuteReport) -> Result<Value, ToolExecError> {
    let retry_safe = report_is_retry_safe(&report);
    Ok(json_text_result(&json!({
        "report": to_value(report)?,
        "retry_safe": retry_safe,
    })))
}

fn now_ms() -> Result<i64, ToolExecError> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(std::io::Error::other)?
        .as_millis() as i64)
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
    use praefectus::semantic::{
        Actionability, SemanticBackend, SemanticElement, SemanticProvenance,
    };
    use praefectus::{AckState, ActionAck, Effect, FailureCode, Receipt};

    use super::*;

    fn observation(now: i64) -> SemanticObservation {
        let observation_id = "1".repeat(64);
        SemanticObservation {
            protocol_version: PROTOCOL_VERSION,
            observation_id: observation_id.clone(),
            generation: 1,
            provenance: SemanticProvenance {
                backend: SemanticBackend::Accessibility,
                backend_name: "test".to_string(),
                process_id: 1,
                process_generation: "generation".to_string(),
                window_id: "window".to_string(),
                document_id: None,
                display_geometry_hash: "2".repeat(64),
                host_opt_ins: Vec::new(),
            },
            observed_at_ms: now,
            expires_at_ms: now + 30_000,
            truncated: false,
            elements: vec![SemanticElement {
                tag: "e0".to_string(),
                element_id: "3".repeat(64),
                parent_id: None,
                fingerprint_hash: "4".repeat(64),
                role: "button".to_string(),
                name: Some("Save".to_string()),
                bounds: None,
                actionability: Actionability {
                    visible: true,
                    enabled: true,
                    unambiguous: true,
                    stable: true,
                    receives_events: true,
                    invokable: true,
                    editable: false,
                },
            }],
        }
    }

    #[test]
    fn full_access_check_precedes_semantic_target_lookup() {
        let result = execute_click(
            AccessMode::Limited,
            "op",
            "missing",
            "e0",
            InteractionMode::Interactive,
        );
        assert!(matches!(result, Err(ToolExecError::Blocked)));
    }

    #[test]
    fn cached_semantic_targets_are_observation_and_tag_bound() {
        let now = now_ms().expect("time should resolve");
        let observation = observation(now);
        cache_observation(observation.clone()).expect("observation should cache");
        let (target, deadline_at_ms) =
            cached_target(&observation.observation_id, "e0").expect("target should resolve");

        assert_eq!(target.observation_id, observation.observation_id);
        assert_eq!(deadline_at_ms, observation.expires_at_ms);
        assert!(cached_target("f", "e0").is_err());
        assert!(cached_target(&observation.observation_id, "e1").is_err());
    }

    #[test]
    fn identical_operation_proposals_have_a_stable_canonical_hash() {
        let now = now_ms().expect("time should resolve");
        let observation = observation(now);
        let target = observation.target("e0").expect("target should resolve");
        let signing_key = SigningKey::from_bytes(&[7; 32]);
        let action = Action::Invoke;
        let first = signed_request(
            action.clone(),
            target.clone(),
            "operation:1",
            observation.expires_at_ms,
            &signing_key,
            "session:1",
            InteractionMode::Interactive,
        )
        .expect("request should build");
        let second = signed_request(
            action,
            target,
            "operation:1",
            observation.expires_at_ms,
            &signing_key,
            "session:1",
            InteractionMode::Interactive,
        )
        .expect("request should replay");

        assert_eq!(
            normalized_action_hash(&first).expect("first hash should resolve"),
            normalized_action_hash(&second).expect("second hash should resolve")
        );
        assert_eq!(
            to_value(first.authority).expect("first authority should serialize"),
            to_value(second.authority).expect("second authority should serialize")
        );
    }

    #[test]
    fn semantic_routing_requires_exact_action_support() {
        let capabilities = Capabilities {
            platform: "test".to_string(),
            backend: "test".to_string(),
            session_isolation: praefectus::SessionIsolation::SharedDesktop,
            supported_actions: vec!["invoke".to_string()],
            action_capabilities: vec![praefectus::ActionCapability {
                action: "invoke".to_string(),
                delivery_route: praefectus::DeliveryRoute::TargetAddressed,
                background_support: praefectus::BackgroundSupport::Guarded,
            }],
            permissions: Default::default(),
            display_geometry_hash: "0".repeat(64),
        };
        assert!(supports_action(&capabilities, "invoke"));
        assert!(!supports_action(&capabilities, "set_value"));
        assert!(!supports_action(&capabilities, ""));
    }

    #[test]
    fn invoke_is_external_and_explicitly_unverified() {
        let (safety, verification) = action_policy(&Action::Invoke);

        assert_eq!(safety, SafetyClass::External);
        assert!(matches!(verification, VerificationPolicy::None));
    }

    #[test]
    fn interaction_mode_changes_the_authorized_action_hash() {
        let now = now_ms().expect("time should resolve");
        let observation = observation(now);
        let target = observation.target("e0").expect("target should resolve");
        let signing_key = SigningKey::from_bytes(&[7; 32]);
        let interactive = signed_request(
            Action::Invoke,
            target.clone(),
            "operation:1",
            observation.expires_at_ms,
            &signing_key,
            "session:1",
            InteractionMode::Interactive,
        )
        .expect("interactive request should build");
        let background = signed_request(
            Action::Invoke,
            target,
            "operation:1",
            observation.expires_at_ms,
            &signing_key,
            "session:1",
            InteractionMode::BackgroundOnly,
        )
        .expect("background request should build");

        assert_ne!(
            normalized_action_hash(&interactive).expect("interactive hash should resolve"),
            normalized_action_hash(&background).expect("background hash should resolve")
        );
    }

    #[test]
    fn set_value_is_external_and_verifies_the_typed_value_hash() {
        let (safety, verification) = action_policy(&Action::SetValue {
            value: "value".to_string(),
        });

        assert_eq!(safety, SafetyClass::External);
        assert!(matches!(
            verification,
            VerificationPolicy::TargetValueHash { sha256 }
                if sha256 == "cd42404d52ad55ccfa9aca4adc828aa5800ad9d385a0671fbcbf724118320619"
        ));
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
            action_name: "click".to_string(),
            action_hash: "0".repeat(64),
            started_at_ms: 1,
            finished_at_ms: 2,
            backend: "test".to_string(),
            fallback_chain: Vec::new(),
            delivery_route: praefectus::DeliveryRoute::TargetAddressed,
            session_isolation: praefectus::SessionIsolation::SharedDesktop,
            interaction_mode: InteractionMode::Interactive,
            context_preservation: praefectus::ContextPreservation::NotApplicable,
            effect,
            before: None,
            after: None,
            warnings: Vec::new(),
        }
    }
}
