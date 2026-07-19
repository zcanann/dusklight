//! Terminal outcomes shared by harness execution and immutable evidence.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HarnessTerminalReason {
    Reached,
    Exhausted,
    Impossible,
    Unsupported,
    CapabilityMismatch,
    IdentityMismatch,
    HostTimeout,
    Cancelled,
    WorkerCrashed,
    GameCrashed,
    ProtocolFailure,
    Hung,
    TargetLost,
    Nondeterministic,
    Rejected,
}

impl HarnessTerminalReason {
    pub const ALL: [Self; 15] = [
        Self::Reached,
        Self::Exhausted,
        Self::Impossible,
        Self::Unsupported,
        Self::CapabilityMismatch,
        Self::IdentityMismatch,
        Self::HostTimeout,
        Self::Cancelled,
        Self::WorkerCrashed,
        Self::GameCrashed,
        Self::ProtocolFailure,
        Self::Hung,
        Self::TargetLost,
        Self::Nondeterministic,
        Self::Rejected,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Reached => "reached",
            Self::Exhausted => "exhausted",
            Self::Impossible => "impossible",
            Self::Unsupported => "unsupported",
            Self::CapabilityMismatch => "capability_mismatch",
            Self::IdentityMismatch => "identity_mismatch",
            Self::HostTimeout => "host_timeout",
            Self::Cancelled => "cancelled",
            Self::WorkerCrashed => "worker_crashed",
            Self::GameCrashed => "game_crashed",
            Self::ProtocolFailure => "protocol_failure",
            Self::Hung => "hung",
            Self::TargetLost => "target_lost",
            Self::Nondeterministic => "nondeterministic",
            Self::Rejected => "rejected",
        }
    }
}
