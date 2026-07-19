//! Portable engine-session reuse audit shared by workers and run evidence.

use serde::{Deserialize, Serialize};
use std::error::Error;
use std::fmt;

pub const ENGINE_SESSION_REUSE_AUDIT_SCHEMA_V1: &str = "dusklight-engine-session-reuse-audit/v1";
pub const POST_AUTHENTICATED_RUN_BOUNDARY: &str = "post_authenticated_run";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SessionReuseBlocker {
    pub code: String,
    pub subsystem: String,
    pub required_guarantee: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SessionReuseAudit {
    pub schema: String,
    pub reusable: bool,
    pub evaluated_boundary: String,
    pub target_boundary: String,
    pub blockers: Vec<SessionReuseBlocker>,
}

impl SessionReuseAudit {
    pub fn validate(&self) -> Result<(), SessionReuseAuditError> {
        if self.schema != ENGINE_SESSION_REUSE_AUDIT_SCHEMA_V1 {
            return Err(audit_error("unsupported audit schema"));
        }
        if !matches!(
            self.evaluated_boundary.as_str(),
            "pre_engine_boot" | POST_AUTHENTICATED_RUN_BOUNDARY
        ) || self.target_boundary != POST_AUTHENTICATED_RUN_BOUNDARY
        {
            return Err(audit_error("audit lifecycle boundaries are not canonical"));
        }
        if self.reusable != self.blockers.is_empty() {
            return Err(audit_error(
                "reusable status contradicts the blocker inventory",
            ));
        }
        if !self
            .blockers
            .windows(2)
            .all(|pair| pair[0].code < pair[1].code)
        {
            return Err(audit_error("blockers must be unique and code-sorted"));
        }
        for blocker in &self.blockers {
            if !canonical_identifier(&blocker.code)
                || !canonical_identifier(&blocker.subsystem)
                || blocker.required_guarantee.trim().is_empty()
            {
                return Err(audit_error(
                    "blocker identity or required guarantee is invalid",
                ));
            }
        }
        Ok(())
    }
}

fn canonical_identifier(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionReuseAuditError(String);

impl fmt::Display for SessionReuseAuditError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for SessionReuseAuditError {}

fn audit_error(message: impl Into<String>) -> SessionReuseAuditError {
    SessionReuseAuditError(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_audit() -> SessionReuseAudit {
        SessionReuseAudit {
            schema: ENGINE_SESSION_REUSE_AUDIT_SCHEMA_V1.into(),
            reusable: false,
            evaluated_boundary: "pre_engine_boot".into(),
            target_boundary: POST_AUTHENTICATED_RUN_BOUNDARY.into(),
            blockers: vec![SessionReuseBlocker {
                code: "game_global_reconstruction".into(),
                subsystem: "game_state".into(),
                required_guarantee: "game globals reconstruct from a clean origin".into(),
            }],
        }
    }

    #[test]
    fn canonical_reuse_refusal_is_accepted_at_each_evaluated_boundary() {
        let mut audit = valid_audit();
        audit.validate().unwrap();
        audit.evaluated_boundary = POST_AUTHENTICATED_RUN_BOUNDARY.into();
        audit.validate().unwrap();
    }

    #[test]
    fn reusable_session_cannot_retain_blockers() {
        let mut audit = valid_audit();
        audit.reusable = true;
        let error = audit.validate().unwrap_err().to_string();
        assert!(error.contains("contradicts the blocker inventory"));
    }

    #[test]
    fn blockers_must_be_unique_and_sorted() {
        let mut audit = valid_audit();
        audit.blockers.push(audit.blockers[0].clone());
        let error = audit.validate().unwrap_err().to_string();
        assert!(error.contains("unique and code-sorted"));
    }
}
