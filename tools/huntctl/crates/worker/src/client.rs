use crate::transport::Transport;
pub use dusklight_automation_contracts::engine_session::{
    ENGINE_SESSION_REUSE_AUDIT_SCHEMA_V1, SessionReuseAudit, SessionReuseBlocker,
};
use serde::Deserialize;
use serde_json::json;
use std::error::Error;
use std::fmt;

pub const CONTROL_PROTOCOL_NAME: &str = "dusklight-automation";
pub const CONTROL_PROTOCOL_VERSION: u64 = 2;
pub const MAX_CONTROL_LINE_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct WorkerBuildIdentity {
    pub version: String,
    pub describe: String,
    pub revision: String,
    pub dirty_digest: String,
    pub branch: String,
    pub source_date: String,
    pub aurora_revision: String,
    pub compiler: String,
    pub compiler_target: String,
    pub build_type: String,
    pub feature_switches: String,
    pub feature_digest: String,
    pub fidelity_profile: String,
    pub platform: String,
    pub architecture: String,
    pub pointer_bits: u32,
    pub dirty: bool,
}

impl WorkerBuildIdentity {
    fn validate(&self) -> Result<(), ClientError> {
        for (field, value) in [
            ("build.version", self.version.as_str()),
            ("build.describe", self.describe.as_str()),
            ("build.branch", self.branch.as_str()),
            ("build.source_date", self.source_date.as_str()),
            ("build.compiler", self.compiler.as_str()),
            ("build.compiler_target", self.compiler_target.as_str()),
            ("build.build_type", self.build_type.as_str()),
            ("build.feature_switches", self.feature_switches.as_str()),
            ("build.fidelity_profile", self.fidelity_profile.as_str()),
            ("build.platform", self.platform.as_str()),
            ("build.architecture", self.architecture.as_str()),
        ] {
            if value.trim().is_empty() {
                return Err(ClientError::InvalidBuildIdentity {
                    field,
                    message: "must not be empty".into(),
                });
            }
        }

        validate_lower_hex("build.revision", &self.revision, 40)?;
        validate_lower_hex("build.aurora_revision", &self.aurora_revision, 40)?;
        validate_lower_hex("build.feature_digest", &self.feature_digest, 64)?;
        if self.dirty {
            validate_lower_hex("build.dirty_digest", &self.dirty_digest, 64)?;
        } else if !self.dirty_digest.is_empty() {
            return Err(ClientError::InvalidBuildIdentity {
                field: "build.dirty_digest",
                message: "must be empty when build.dirty is false".into(),
            });
        }
        if !matches!(self.pointer_bits, 32 | 64) {
            return Err(ClientError::InvalidBuildIdentity {
                field: "build.pointer_bits",
                message: format!("expected 32 or 64, received {}", self.pointer_bits),
            });
        }
        Ok(())
    }
}

fn validate_lower_hex(
    field: &'static str,
    value: &str,
    expected_length: usize,
) -> Result<(), ClientError> {
    if value.len() != expected_length
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(ClientError::InvalidBuildIdentity {
            field,
            message: format!("must be exactly {expected_length} lowercase hexadecimal characters"),
        });
    }
    Ok(())
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct WorkerCapabilities {
    pub persistent_control: bool,
    pub engine_session: bool,
    pub headless: bool,
    pub scenario_load: bool,
    pub input_tape: bool,
    pub batch_run: bool,
    pub commands: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HelloResponse {
    pub build: WorkerBuildIdentity,
    pub capabilities: WorkerCapabilities,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IdentityDifference {
    pub field: &'static str,
    pub expected: String,
    pub actual: String,
}

impl IdentityDifference {
    pub fn message(&self) -> String {
        format!(
            "{}: expected {}, received {}",
            self.field, self.expected, self.actual
        )
    }
}

impl HelloResponse {
    /// Returns every build or protocol-capability mismatch. Keeping this as a
    /// typed field list makes CLI rejection useful without weakening the exact
    /// comparison used to form deterministic worker pools.
    pub fn identity_differences(&self, actual: &Self) -> Vec<IdentityDifference> {
        let mut differences = Vec::new();
        macro_rules! compare {
            ($field:literal, $expected:expr, $actual:expr) => {
                if $expected != $actual {
                    differences.push(IdentityDifference {
                        field: $field,
                        expected: format!("{:?}", $expected),
                        actual: format!("{:?}", $actual),
                    });
                }
            };
        }

        compare!("build.version", self.build.version, actual.build.version);
        compare!("build.describe", self.build.describe, actual.build.describe);
        compare!("build.revision", self.build.revision, actual.build.revision);
        compare!(
            "build.dirty_digest",
            self.build.dirty_digest,
            actual.build.dirty_digest
        );
        compare!("build.branch", self.build.branch, actual.build.branch);
        compare!(
            "build.source_date",
            self.build.source_date,
            actual.build.source_date
        );
        compare!(
            "build.aurora_revision",
            self.build.aurora_revision,
            actual.build.aurora_revision
        );
        compare!("build.compiler", self.build.compiler, actual.build.compiler);
        compare!(
            "build.compiler_target",
            self.build.compiler_target,
            actual.build.compiler_target
        );
        compare!(
            "build.build_type",
            self.build.build_type,
            actual.build.build_type
        );
        compare!(
            "build.feature_switches",
            self.build.feature_switches,
            actual.build.feature_switches
        );
        compare!(
            "build.feature_digest",
            self.build.feature_digest,
            actual.build.feature_digest
        );
        compare!(
            "build.fidelity_profile",
            self.build.fidelity_profile,
            actual.build.fidelity_profile
        );
        compare!("build.platform", self.build.platform, actual.build.platform);
        compare!(
            "build.architecture",
            self.build.architecture,
            actual.build.architecture
        );
        compare!(
            "build.pointer_bits",
            self.build.pointer_bits,
            actual.build.pointer_bits
        );
        compare!("build.dirty", self.build.dirty, actual.build.dirty);
        compare!(
            "capabilities.persistent_control",
            self.capabilities.persistent_control,
            actual.capabilities.persistent_control
        );
        compare!(
            "capabilities.engine_session",
            self.capabilities.engine_session,
            actual.capabilities.engine_session
        );
        compare!(
            "capabilities.headless",
            self.capabilities.headless,
            actual.capabilities.headless
        );
        compare!(
            "capabilities.scenario_load",
            self.capabilities.scenario_load,
            actual.capabilities.scenario_load
        );
        compare!(
            "capabilities.input_tape",
            self.capabilities.input_tape,
            actual.capabilities.input_tape
        );
        compare!(
            "capabilities.batch_run",
            self.capabilities.batch_run,
            actual.capabilities.batch_run
        );
        compare!(
            "capabilities.commands",
            self.capabilities.commands,
            actual.capabilities.commands
        );
        differences
    }
}

#[derive(Clone, Debug, Deserialize)]
struct ProtocolDescriptor {
    name: String,
    version: u64,
}

#[derive(Clone, Debug, Deserialize)]
struct WorkerErrorBody {
    code: String,
    message: String,
}

#[derive(Clone, Debug, Deserialize)]
struct Envelope {
    protocol: ProtocolDescriptor,
    #[serde(rename = "type")]
    response_type: String,
    ok: bool,
    id: Option<u64>,
    build: Option<WorkerBuildIdentity>,
    capabilities: Option<WorkerCapabilities>,
    audit: Option<SessionReuseAudit>,
    result: Option<String>,
    episode_shard: Option<String>,
    error: Option<WorkerErrorBody>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BatchComplete {
    pub result: String,
    pub episode_shard: String,
}

#[derive(Debug)]
pub enum ClientError {
    Io(std::io::Error),
    Json(serde_json::Error),
    WorkerClosed,
    ResponseTooLarge(usize),
    RequestMismatch {
        expected: u64,
        received: Option<u64>,
    },
    ProtocolName(String),
    ProtocolVersion(u64),
    UnexpectedResponse {
        expected: &'static str,
        received: String,
    },
    MissingField(&'static str),
    InvalidBuildIdentity {
        field: &'static str,
        message: String,
    },
    InvalidSessionAudit(String),
    Worker {
        code: String,
        message: String,
    },
}

impl fmt::Display for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "worker I/O error: {error}"),
            Self::Json(error) => write!(f, "invalid worker JSON: {error}"),
            Self::WorkerClosed => f.write_str("worker closed its transport"),
            Self::ResponseTooLarge(size) => {
                write!(f, "worker response is {size} bytes; limit is 1 MiB")
            }
            Self::RequestMismatch { expected, received } => {
                write!(f, "expected request id {expected}, received {received:?}")
            }
            Self::ProtocolName(name) => write!(f, "unexpected worker protocol {name:?}"),
            Self::ProtocolVersion(version) => {
                write!(f, "unsupported worker protocol version {version}")
            }
            Self::UnexpectedResponse { expected, received } => {
                write!(f, "expected {expected} response, received {received}")
            }
            Self::MissingField(field) => write!(f, "worker response is missing {field}"),
            Self::InvalidBuildIdentity { field, message } => {
                write!(f, "invalid worker build identity {field}: {message}")
            }
            Self::InvalidSessionAudit(message) => {
                write!(f, "invalid engine-session reuse audit: {message}")
            }
            Self::Worker { code, message } => write!(f, "worker error {code}: {message}"),
        }
    }
}

impl Error for ClientError {}
impl From<std::io::Error> for ClientError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}
impl From<serde_json::Error> for ClientError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}

pub struct WorkerClient<T> {
    transport: T,
    next_request_id: u64,
    hello: Option<HelloResponse>,
}

impl<T: Transport> WorkerClient<T> {
    pub fn new(transport: T) -> Self {
        Self {
            transport,
            next_request_id: 1,
            hello: None,
        }
    }
    pub fn hello(&self) -> Option<&HelloResponse> {
        self.hello.as_ref()
    }
    pub fn into_transport(self) -> T {
        self.transport
    }

    pub fn handshake(&mut self) -> Result<&HelloResponse, ClientError> {
        let response = self.command("hello", "hello")?;
        let build = response.build.ok_or(ClientError::MissingField("build"))?;
        build.validate()?;
        self.hello = Some(HelloResponse {
            build,
            capabilities: response
                .capabilities
                .ok_or(ClientError::MissingField("capabilities"))?,
        });
        Ok(self.hello.as_ref().expect("hello was assigned"))
    }

    pub fn ping(&mut self) -> Result<(), ClientError> {
        self.command("ping", "pong").map(drop)
    }
    pub fn session_audit(&mut self) -> Result<SessionReuseAudit, ClientError> {
        let audit = self
            .command("session_audit", "session_audit")?
            .audit
            .ok_or(ClientError::MissingField("audit"))?;
        audit
            .validate()
            .map_err(|error| ClientError::InvalidSessionAudit(error.to_string()))?;
        Ok(audit)
    }
    pub fn shutdown(&mut self) -> Result<(), ClientError> {
        self.command("shutdown", "shutdown").map(drop)
    }

    /// Receives the batch supplied on the engine worker's launch command line.
    pub fn await_initial_batch(&mut self) -> Result<BatchComplete, ClientError> {
        self.require_batch_capability()?;
        let response = self.receive_response(0, "batch_complete")?;
        batch_complete(response)
    }

    /// Submits another batch to the same authenticated in-process checkpoint.
    pub fn run_batch(
        &mut self,
        batch: &str,
        result: &str,
        winner_tape: Option<&str>,
    ) -> Result<BatchComplete, ClientError> {
        self.require_batch_capability()?;
        if batch.is_empty() || result.is_empty() || winner_tape.is_some_and(str::is_empty) {
            return Err(ClientError::MissingField("run_batch path"));
        }
        let response = self.command_value(
            json!({
                "command": "run_batch",
                "batch": batch,
                "result": result,
                "winner_tape": winner_tape,
            }),
            "batch_complete",
        )?;
        batch_complete(response)
    }

    fn require_batch_capability(&self) -> Result<(), ClientError> {
        if !self.hello.as_ref().is_some_and(|hello| {
            hello.capabilities.persistent_control && hello.capabilities.batch_run
        }) {
            return Err(ClientError::UnexpectedResponse {
                expected: "batch-capable hello",
                received: "worker without persistent batch capability".into(),
            });
        }
        Ok(())
    }

    fn command(
        &mut self,
        command: &str,
        expected_type: &'static str,
    ) -> Result<Envelope, ClientError> {
        self.command_value(json!({"command": command}), expected_type)
    }

    fn command_value(
        &mut self,
        mut request: serde_json::Value,
        expected_type: &'static str,
    ) -> Result<Envelope, ClientError> {
        let request_id = self.next_request_id;
        self.next_request_id = self.next_request_id.checked_add(1).unwrap_or(1);
        request["id"] = request_id.into();
        self.transport
            .send_line(&serde_json::to_string(&request)?)?;
        self.receive_response(request_id, expected_type)
    }

    fn receive_response(
        &mut self,
        request_id: u64,
        expected_type: &'static str,
    ) -> Result<Envelope, ClientError> {
        let line = self
            .transport
            .receive_line()?
            .ok_or(ClientError::WorkerClosed)?;
        if line.len() > MAX_CONTROL_LINE_BYTES {
            return Err(ClientError::ResponseTooLarge(line.len()));
        }
        let response: Envelope = serde_json::from_str(&line)?;
        if response.id != Some(request_id) {
            return Err(ClientError::RequestMismatch {
                expected: request_id,
                received: response.id,
            });
        }
        if response.protocol.name != CONTROL_PROTOCOL_NAME {
            return Err(ClientError::ProtocolName(response.protocol.name));
        }
        if response.protocol.version != CONTROL_PROTOCOL_VERSION {
            return Err(ClientError::ProtocolVersion(response.protocol.version));
        }
        if !response.ok {
            let error = response.error.ok_or(ClientError::MissingField("error"))?;
            return Err(ClientError::Worker {
                code: error.code,
                message: error.message,
            });
        }
        if response.response_type != expected_type {
            return Err(ClientError::UnexpectedResponse {
                expected: expected_type,
                received: response.response_type,
            });
        }
        Ok(response)
    }
}

fn batch_complete(response: Envelope) -> Result<BatchComplete, ClientError> {
    Ok(BatchComplete {
        result: response.result.ok_or(ClientError::MissingField("result"))?,
        episode_shard: response
            .episode_shard
            .ok_or(ClientError::MissingField("episode_shard"))?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::LineTransport;
    use std::io::Cursor;

    fn valid_build_identity() -> WorkerBuildIdentity {
        WorkerBuildIdentity {
            version: "test".into(),
            describe: "test-build".into(),
            revision: "1".repeat(40),
            dirty_digest: String::new(),
            branch: "test".into(),
            source_date: "2026-07-18".into(),
            aurora_revision: "2".repeat(40),
            compiler: "test-compiler".into(),
            compiler_target: "test-target".into(),
            build_type: "Debug".into(),
            feature_switches: "test=ON".into(),
            feature_digest: "3".repeat(64),
            fidelity_profile: "observe_only".into(),
            platform: "test-platform".into(),
            architecture: "test-architecture".into(),
            pointer_bits: 64,
            dirty: false,
        }
    }

    #[test]
    fn complete_build_identity_is_accepted() {
        valid_build_identity().validate().unwrap();
    }

    #[test]
    fn placeholder_revision_is_rejected() {
        let mut identity = valid_build_identity();
        identity.revision = "unknown".into();
        let error = identity.validate().unwrap_err().to_string();
        assert!(error.contains("build.revision"));
        assert!(error.contains("40 lowercase hexadecimal"));
    }

    #[test]
    fn dirty_state_requires_a_digest() {
        let mut identity = valid_build_identity();
        identity.dirty = true;
        let error = identity.validate().unwrap_err().to_string();
        assert!(error.contains("build.dirty_digest"));
        assert!(error.contains("64 lowercase hexadecimal"));
    }

    #[test]
    fn fidelity_profile_must_be_explicit() {
        let mut identity = valid_build_identity();
        identity.fidelity_profile.clear();
        let error = identity.validate().unwrap_err().to_string();
        assert!(error.contains("build.fidelity_profile"));
        assert!(error.contains("must not be empty"));
    }

    #[test]
    fn batch_client_sends_typed_request_and_accepts_bound_outputs() {
        let responses = concat!(
            "{\"protocol\":{\"name\":\"dusklight-automation\",\"version\":2},",
            "\"type\":\"hello\",\"ok\":true,\"id\":1,",
            "\"build\":{\"version\":\"test\",\"describe\":\"test-build\",",
            "\"revision\":\"1111111111111111111111111111111111111111\",",
            "\"dirty_digest\":\"\",\"branch\":\"test\",\"source_date\":\"2026-07-18\",",
            "\"aurora_revision\":\"2222222222222222222222222222222222222222\",",
            "\"compiler\":\"test\",\"compiler_target\":\"test\",\"build_type\":\"Debug\",",
            "\"feature_switches\":\"test=ON\",",
            "\"feature_digest\":\"3333333333333333333333333333333333333333333333333333333333333333\",",
            "\"fidelity_profile\":\"observe_only\",\"platform\":\"test\",",
            "\"architecture\":\"x86_64\",\"pointer_bits\":64,\"dirty\":false},",
            "\"capabilities\":{\"persistent_control\":true,\"engine_session\":false,",
            "\"headless\":true,\"scenario_load\":false,\"input_tape\":true,",
            "\"batch_run\":true,\"commands\":[\"hello\",\"run_batch\",\"shutdown\"]}}\n",
            "{\"protocol\":{\"name\":\"dusklight-automation\",\"version\":2},",
            "\"type\":\"batch_complete\",\"ok\":true,\"id\":0,",
            "\"result\":\"first.json\",\"episode_shard\":\"first.json.episodes.dseps\"}\n",
            "{\"protocol\":{\"name\":\"dusklight-automation\",\"version\":2},",
            "\"type\":\"batch_complete\",\"ok\":true,\"id\":2,",
            "\"result\":\"next.json\",\"episode_shard\":\"next.json.episodes.dseps\"}\n"
        );
        let reader = Cursor::new(responses.as_bytes().to_vec());
        let writer = Cursor::new(Vec::<u8>::new());
        let transport = LineTransport::new(std::io::BufReader::new(reader), writer);
        let mut client = WorkerClient::new(transport);
        client.handshake().unwrap();
        assert_eq!(client.await_initial_batch().unwrap().result, "first.json");
        assert_eq!(
            client
                .run_batch("next.batch.json", "next.json", None)
                .unwrap()
                .episode_shard,
            "next.json.episodes.dseps"
        );
        let transport = client.into_transport();
        let (_, writer) = transport.into_parts();
        let written = String::from_utf8(writer.into_inner()).unwrap();
        let requests = written.lines().collect::<Vec<_>>();
        assert_eq!(requests.len(), 2);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(requests[1]).unwrap(),
            json!({
                "id": 2,
                "command": "run_batch",
                "batch": "next.batch.json",
                "result": "next.json",
                "winner_tape": null,
            })
        );
    }
}
