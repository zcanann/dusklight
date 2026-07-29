use super::*;
use std::str::FromStr;

pub const NATIVE_TACTIC_FAULT_INJECTION_SCHEMA_V1: &str =
    "dusklight-native-tactic-fault-injection/v1";
pub const NATIVE_TACTIC_FAULT_INJECTION_FILE: &str = "fault-injection.json";
pub const NATIVE_TACTIC_FAULT_EXIT_CODE: i32 = 86;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeTacticFaultPoint {
    BeforeDispatch,
    DuringExecution,
    AfterNativeCompletion,
    AfterRecoveryPointCommit,
    AfterDecisionCommit,
}

impl NativeTacticFaultPoint {
    fn name(self) -> &'static str {
        match self {
            Self::BeforeDispatch => "before_dispatch",
            Self::DuringExecution => "during_execution",
            Self::AfterNativeCompletion => "after_native_completion",
            Self::AfterRecoveryPointCommit => "after_recovery_point_commit",
            Self::AfterDecisionCommit => "after_decision_commit",
        }
    }
}

impl FromStr for NativeTacticFaultPoint {
    type Err = NativeTacticRouteRunError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "before-dispatch" => Ok(Self::BeforeDispatch),
            "during-execution" => Ok(Self::DuringExecution),
            "after-native-completion" => Ok(Self::AfterNativeCompletion),
            "after-recovery-point-commit" => Ok(Self::AfterRecoveryPointCommit),
            "after-decision-commit" => Ok(Self::AfterDecisionCommit),
            _ => Err(route_message(format!(
                "unknown native tactic fault point {value:?}; expected before-dispatch, during-execution, after-native-completion, after-recovery-point-commit, or after-decision-commit"
            ))),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticFaultInjectionMarker {
    pub schema: String,
    pub execution_plan_sha256: Digest,
    pub seed: u64,
    pub decision_index: u64,
    pub point: NativeTacticFaultPoint,
}

impl NativeTacticFaultInjectionMarker {
    pub fn validate(&self) -> Result<(), NativeTacticRouteRunError> {
        if self.schema != NATIVE_TACTIC_FAULT_INJECTION_SCHEMA_V1
            || self.execution_plan_sha256 == Digest::ZERO
        {
            return Err(route_message(
                "native tactic fault-injection marker is invalid",
            ));
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct NativeTacticFaultInjector {
    point: NativeTacticFaultPoint,
    decision_index: u64,
    fired: AtomicBool,
    exit_process: bool,
}

impl NativeTacticFaultInjector {
    pub fn process_exit(point: NativeTacticFaultPoint, decision_index: u64) -> Self {
        Self {
            point,
            decision_index,
            fired: AtomicBool::new(false),
            exit_process: true,
        }
    }

    pub fn decision_index(&self) -> u64 {
        self.decision_index
    }

    #[cfg(test)]
    fn returning_error(point: NativeTacticFaultPoint, decision_index: u64) -> Self {
        Self {
            point,
            decision_index,
            fired: AtomicBool::new(false),
            exit_process: false,
        }
    }

    pub(super) fn inject(
        &self,
        encountered: NativeTacticFaultPoint,
        execution_plan_sha256: Digest,
        seed: u64,
        decision_index: u64,
        seed_root: &Path,
    ) -> Result<(), NativeTacticRouteRunError> {
        if encountered != self.point
            || decision_index != self.decision_index
            || self
                .fired
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                .is_err()
        {
            return Ok(());
        }
        let marker = NativeTacticFaultInjectionMarker {
            schema: NATIVE_TACTIC_FAULT_INJECTION_SCHEMA_V1.into(),
            execution_plan_sha256,
            seed,
            decision_index,
            point: encountered,
        };
        marker.validate()?;
        let marker_path = seed_root.join(NATIVE_TACTIC_FAULT_INJECTION_FILE);
        if marker_path.exists() {
            let retained: NativeTacticFaultInjectionMarker =
                serde_json::from_slice(&fs::read(&marker_path).map_err(route_error)?)
                    .map_err(route_error)?;
            retained.validate()?;
            if retained != marker {
                return Err(route_message(
                    "retained native tactic fault injection differs from the requested fault",
                ));
            }
            return Ok(());
        }
        write_new(
            &marker_path,
            &serde_json::to_vec_pretty(&marker).map_err(route_error)?,
        )?;
        if self.exit_process {
            eprintln!(
                "huntctl: injected native tactic process loss at {} for seed {seed} decision {decision_index}",
                encountered.name()
            );
            std::process::exit(NATIVE_TACTIC_FAULT_EXIT_CODE);
        }
        Err(route_message(format!(
            "injected native tactic process loss at {}",
            encountered.name()
        )))
    }
}

pub(super) fn inject_tactic_fault(
    config: &NativeTacticRouteRunConfig<'_>,
    encountered: NativeTacticFaultPoint,
    execution_plan_sha256: Digest,
    seed: u64,
    decision_index: u64,
    seed_root: &Path,
) -> Result<(), NativeTacticRouteRunError> {
    if let Some(injector) = config.fault_injection {
        injector.inject(
            encountered,
            execution_plan_sha256,
            seed,
            decision_index,
            seed_root,
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durable_marker_makes_a_fault_one_shot_across_resume() {
        let root = std::env::temp_dir().join(format!(
            "dusklight-tactic-fault-marker-{}",
            std::process::id()
        ));
        if root.exists() {
            fs::remove_dir_all(&root).unwrap();
        }
        fs::create_dir_all(&root).unwrap();
        let injector =
            NativeTacticFaultInjector::returning_error(NativeTacticFaultPoint::BeforeDispatch, 3);
        assert!(
            injector
                .inject(
                    NativeTacticFaultPoint::BeforeDispatch,
                    Digest([7; 32]),
                    104_729,
                    3,
                    &root,
                )
                .is_err()
        );
        let marker: NativeTacticFaultInjectionMarker = serde_json::from_slice(
            &fs::read(root.join(NATIVE_TACTIC_FAULT_INJECTION_FILE)).unwrap(),
        )
        .unwrap();
        marker.validate().unwrap();
        assert_eq!(marker.seed, 104_729);
        assert_eq!(marker.decision_index, 3);

        let resumed =
            NativeTacticFaultInjector::returning_error(NativeTacticFaultPoint::BeforeDispatch, 3);
        resumed
            .inject(
                NativeTacticFaultPoint::BeforeDispatch,
                Digest([7; 32]),
                104_729,
                3,
                &root,
            )
            .unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn parser_names_every_required_native_loss_boundary() {
        assert_eq!(
            "before-dispatch".parse(),
            Ok(NativeTacticFaultPoint::BeforeDispatch)
        );
        assert_eq!(
            "during-execution".parse(),
            Ok(NativeTacticFaultPoint::DuringExecution)
        );
        assert_eq!(
            "after-native-completion".parse(),
            Ok(NativeTacticFaultPoint::AfterNativeCompletion)
        );
        assert_eq!(
            "after-recovery-point-commit".parse(),
            Ok(NativeTacticFaultPoint::AfterRecoveryPointCommit)
        );
        assert_eq!(
            "after-decision-commit".parse(),
            Ok(NativeTacticFaultPoint::AfterDecisionCommit)
        );
    }
}
