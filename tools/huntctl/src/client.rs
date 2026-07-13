use crate::transport::Transport;
use serde::Deserialize;
use serde_json::json;
use std::error::Error;
use std::fmt;

pub const CONTROL_PROTOCOL_NAME: &str = "dusklight-automation";
pub const CONTROL_PROTOCOL_VERSION: u64 = 1;
pub const MAX_CONTROL_LINE_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct WorkerBuildIdentity {
    pub version: String,
    pub describe: String,
    pub revision: String,
    pub branch: String,
    pub source_date: String,
    pub build_type: String,
    pub platform: String,
    pub architecture: String,
    pub pointer_bits: u32,
    pub dirty: bool,
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
    error: Option<WorkerErrorBody>,
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
        self.hello = Some(HelloResponse {
            build: response.build.ok_or(ClientError::MissingField("build"))?,
            capabilities: response
                .capabilities
                .ok_or(ClientError::MissingField("capabilities"))?,
        });
        Ok(self.hello.as_ref().expect("hello was assigned"))
    }

    pub fn ping(&mut self) -> Result<(), ClientError> {
        self.command("ping", "pong").map(drop)
    }
    pub fn shutdown(&mut self) -> Result<(), ClientError> {
        self.command("shutdown", "shutdown").map(drop)
    }

    fn command(
        &mut self,
        command: &str,
        expected_type: &'static str,
    ) -> Result<Envelope, ClientError> {
        let request_id = self.next_request_id;
        self.next_request_id = self.next_request_id.checked_add(1).unwrap_or(1);
        self.transport.send_line(&serde_json::to_string(
            &json!({"id": request_id, "command": command}),
        )?)?;
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
