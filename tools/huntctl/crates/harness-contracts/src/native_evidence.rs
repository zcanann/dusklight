//! Closed native diagnostics that may supplement ordinary objective evidence.

use crate::objective_suite::{ArtifactReference, ObjectiveBoot, ObjectiveSeed};
use serde::{Deserialize, Serialize};
use std::error::Error;
use std::fmt;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HarnessNativeEvidenceRequest {
    EyeShredderV4,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HarnessNativeEvidenceArtifacts {
    pub oracle_result: ArtifactReference,
    pub semantic_trace: ArtifactReference,
}

impl HarnessNativeEvidenceRequest {
    pub fn validate_for(
        self,
        boot: &ObjectiveBoot,
        input: &ObjectiveSeed,
    ) -> Result<(), HarnessNativeEvidenceError> {
        match self {
            Self::EyeShredderV4
                if matches!(boot, ObjectiveBoot::Process)
                    && matches!(input, ObjectiveSeed::Tape { .. } | ObjectiveSeed::TapeSource { .. }) =>
            {
                Ok(())
            }
            Self::EyeShredderV4 => Err(evidence_error(
                "eye_shredder_v4 evidence requires process boot and tape input",
            )),
        }
    }
}

#[derive(Debug)]
pub struct HarnessNativeEvidenceError(String);

impl fmt::Display for HarnessNativeEvidenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for HarnessNativeEvidenceError {}

fn evidence_error(message: impl Into<String>) -> HarnessNativeEvidenceError {
    HarnessNativeEvidenceError(message.into())
}

#[cfg(test)]
mod tests;
