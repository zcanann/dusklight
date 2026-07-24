//! Minimal localhost HTTP boundary for the independent planner editor.

use crate::project::{ProjectSaveRequest, ProjectStore};
use crate::service::{PlannerServiceEnvelope, error_response, handle_envelope};
use crate::workspace::{
    BUILTIN_LIBRARY_VERSION, WorkspaceAssetCommandRequest, WorkspaceAssetSaveRequest,
    WorkspaceCreateRequest, WorkspaceExport, WorkspaceLibraryForkRequest, WorkspaceRegistry,
    WorkspaceTrashCommandRequest,
};
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;

const MAX_HEADER_BYTES: usize = 64 * 1024;
const MAX_BODY_BYTES: usize = 64 * 1024 * 1024;
const INDEX_HTML: &[u8] = include_bytes!("web/index.html");
const APP_CSS: &[u8] = include_bytes!("web/app.css");
const APP_JS: &[u8] = include_bytes!("web/app.js");

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlannerWebConfig {
    pub listen: SocketAddr,
    pub project_root: PathBuf,
}

pub fn serve_web(config: PlannerWebConfig) -> Result<(), PlannerWebError> {
    if !config.listen.ip().is_loopback() {
        return Err(web_error("planner web server must bind to loopback"));
    }
    let projects = ProjectStore::open(&config.project_root).map_err(PlannerWebError::project)?;
    let available_libraries =
        builtin_library_digests(&projects).map_err(|error| PlannerWebError(error.to_string()))?;
    let state = WebState {
        projects: Arc::new(Mutex::new(projects)),
        workspaces: Arc::new(Mutex::new(
            WorkspaceRegistry::open(config.project_root.join("workspaces"), available_libraries)
                .map_err(PlannerWebError::workspace)?,
        )),
    };
    let listener = TcpListener::bind(config.listen).map_err(PlannerWebError::io)?;
    for stream in listener.incoming() {
        let stream = stream.map_err(PlannerWebError::io)?;
        let state = state.clone();
        thread::Builder::new()
            .name("route-planner-http".into())
            .spawn(move || {
                if let Err(error) = handle_connection(stream, &state) {
                    eprintln!("route-planner web: {error}");
                }
            })
            .map_err(PlannerWebError::io)?;
    }
    Ok(())
}

fn builtin_library_digests(
    projects: &ProjectStore,
) -> Result<BTreeMap<(String, String), dusklight_route_planner::artifact::Digest>, String> {
    Ok(projects
        .list()
        .map_err(|error| error.to_string())?
        .projects
        .into_iter()
        .filter(|project| project.read_only)
        .map(|project| {
            (
                (project.id, BUILTIN_LIBRARY_VERSION.into()),
                project.revision_sha256,
            )
        })
        .collect())
}

#[derive(Clone)]
struct WebState {
    projects: Arc<Mutex<ProjectStore>>,
    workspaces: Arc<Mutex<WorkspaceRegistry>>,
}

fn handle_connection(mut stream: TcpStream, state: &WebState) -> Result<(), PlannerWebError> {
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(15)))
        .map_err(PlannerWebError::io)?;
    stream
        .set_write_timeout(Some(std::time::Duration::from_secs(15)))
        .map_err(PlannerWebError::io)?;
    let request = read_request(&mut stream)?;
    let response = dispatch(request, state);
    write_response(&mut stream, response)
}

#[derive(Debug, Eq, PartialEq)]
struct HttpRequest {
    method: String,
    target: String,
    body: Vec<u8>,
}

#[derive(Debug, Eq, PartialEq)]
struct HttpResponse {
    status: u16,
    reason: &'static str,
    content_type: &'static str,
    body: Vec<u8>,
}

fn read_request(stream: &mut TcpStream) -> Result<HttpRequest, PlannerWebError> {
    let mut reader = BufReader::new(stream);
    let mut first = String::new();
    read_bounded_line(&mut reader, &mut first, MAX_HEADER_BYTES)?;
    let mut parts = first.split_whitespace();
    let method = parts
        .next()
        .ok_or_else(|| web_error("HTTP request omitted its method"))?;
    let target = parts
        .next()
        .ok_or_else(|| web_error("HTTP request omitted its target"))?;
    let version = parts
        .next()
        .ok_or_else(|| web_error("HTTP request omitted its version"))?;
    if parts.next().is_some() || !matches!(version, "HTTP/1.0" | "HTTP/1.1") {
        return Err(web_error("HTTP request line is unsupported"));
    }

    let mut header_bytes = first.len();
    let mut content_length = 0_usize;
    loop {
        let mut line = String::new();
        read_bounded_line(
            &mut reader,
            &mut line,
            MAX_HEADER_BYTES.saturating_sub(header_bytes),
        )?;
        header_bytes = header_bytes
            .checked_add(line.len())
            .ok_or_else(|| web_error("HTTP header size overflowed"))?;
        if line == "\r\n" || line == "\n" {
            break;
        }
        let Some((name, value)) = line.split_once(':') else {
            return Err(web_error("HTTP header is malformed"));
        };
        if name.eq_ignore_ascii_case("content-length") {
            content_length = value
                .trim()
                .parse::<usize>()
                .map_err(|_| web_error("HTTP content-length is invalid"))?;
            if content_length > MAX_BODY_BYTES {
                return Err(web_error("HTTP request body exceeds the planner limit"));
            }
        }
        if name.eq_ignore_ascii_case("transfer-encoding")
            && !value.trim().eq_ignore_ascii_case("identity")
        {
            return Err(web_error("chunked HTTP requests are unsupported"));
        }
    }
    let mut body = vec![0_u8; content_length];
    reader.read_exact(&mut body).map_err(PlannerWebError::io)?;
    Ok(HttpRequest {
        method: method.into(),
        target: target.split('?').next().unwrap_or(target).into(),
        body,
    })
}

fn read_bounded_line(
    reader: &mut BufReader<&mut TcpStream>,
    output: &mut String,
    remaining: usize,
) -> Result<(), PlannerWebError> {
    if remaining == 0 {
        return Err(web_error("HTTP headers exceed the planner limit"));
    }
    let count = reader.read_line(output).map_err(PlannerWebError::io)?;
    if count == 0 {
        return Err(web_error("HTTP connection ended before the request"));
    }
    if count > remaining {
        return Err(web_error("HTTP headers exceed the planner limit"));
    }
    Ok(())
}

fn dispatch(request: HttpRequest, state: &WebState) -> HttpResponse {
    match (request.method.as_str(), request.target.as_str()) {
        ("GET", "/") | ("GET", "/index.html") => asset("text/html; charset=utf-8", INDEX_HTML),
        ("GET", "/app.css") => asset("text/css; charset=utf-8", APP_CSS),
        ("GET", "/app.js") => asset("text/javascript; charset=utf-8", APP_JS),
        ("GET", "/api/health") => json_response(
            200,
            "OK",
            br#"{"schema":"dusklight.route-planner.web-health/v1","status":"ok"}"#.to_vec(),
        ),
        ("GET", "/api/projects") => project_response(|| {
            let store = state
                .projects
                .lock()
                .map_err(|_| "project store lock is poisoned".to_owned())?;
            store.list().map_err(|error| error.to_string())
        }),
        ("GET", "/api/libraries") => project_response(|| {
            let store = state
                .projects
                .lock()
                .map_err(|_| "project store lock is poisoned".to_owned())?;
            let libraries = store
                .list()
                .map_err(|error| error.to_string())?
                .projects
                .into_iter()
                .filter(|project| project.read_only)
                .collect::<Vec<_>>();
            Ok(serde_json::json!({
                "schema": "dusklight.route-planner.library-list/v1",
                "libraries": libraries,
            }))
        }),
        ("GET", "/api/workspaces") => project_response(|| {
            let registry = state
                .workspaces
                .lock()
                .map_err(|_| "workspace registry lock is poisoned".to_owned())?;
            registry.list().map_err(|error| error.to_string())
        }),
        ("POST", "/api/workspaces") => {
            let create = match serde_json::from_slice::<WorkspaceCreateRequest>(&request.body) {
                Ok(create) => create,
                Err(error) => {
                    return project_error_response(400, "Bad Request", &error.to_string());
                }
            };
            project_response(|| {
                let registry = state
                    .workspaces
                    .lock()
                    .map_err(|_| "workspace registry lock is poisoned".to_owned())?;
                registry.create(create).map_err(|error| error.to_string())
            })
        }
        ("POST", "/api/workspaces/import") => {
            let bundle = match serde_json::from_slice::<WorkspaceExport>(&request.body) {
                Ok(bundle) => bundle,
                Err(error) => {
                    return project_error_response(400, "Bad Request", &error.to_string());
                }
            };
            project_response(|| {
                let registry = state
                    .workspaces
                    .lock()
                    .map_err(|_| "workspace registry lock is poisoned".to_owned())?;
                registry.import(bundle).map_err(|error| error.to_string())
            })
        }
        ("GET", "/api/project-template") => project_response(|| {
            let store = state
                .projects
                .lock()
                .map_err(|_| "project store lock is poisoned".to_owned())?;
            store.blank_template().map_err(|error| error.to_string())
        }),
        ("POST", "/api/service") => {
            let response = match serde_json::from_slice::<PlannerServiceEnvelope>(&request.body) {
                Ok(envelope) => handle_envelope(envelope),
                Err(error) => error_response(None, "json", error.to_string()),
            };
            match serde_json::to_vec(&response) {
                Ok(body) => json_response(200, "OK", body),
                Err(error) => json_response(
                    500,
                    "Internal Server Error",
                    serde_json::to_vec(&error_response(None, "json", error.to_string()))
                        .unwrap_or_else(|_| b"{}".to_vec()),
                ),
            }
        }
        _ if request.target.starts_with("/api/workspaces/") => {
            dispatch_workspace_record(request, state)
        }
        _ => dispatch_project_record(request, state),
    }
}

fn dispatch_workspace_record(request: HttpRequest, state: &WebState) -> HttpResponse {
    let target = request.target.clone();
    let remainder = target
        .strip_prefix("/api/workspaces/")
        .expect("workspace dispatch checked its prefix");
    let parts = remainder.split('/').collect::<Vec<_>>();
    let Some(workspace_id) = parts.first().copied().filter(|id| !id.is_empty()) else {
        return project_error_response(400, "Bad Request", "invalid workspace id");
    };
    match parts.as_slice() {
        [_] => match request.method.as_str() {
            "GET" => project_response(|| {
                let registry = state
                    .workspaces
                    .lock()
                    .map_err(|_| "workspace registry lock is poisoned".to_owned())?;
                registry
                    .load(workspace_id)
                    .map_err(|error| error.to_string())
            }),
            _ => project_error_response(405, "Method Not Allowed", "unsupported workspace method"),
        },
        [_, "export"] if request.method == "GET" => project_response(|| {
            let registry = state
                .workspaces
                .lock()
                .map_err(|_| "workspace registry lock is poisoned".to_owned())?;
            registry
                .export(workspace_id)
                .map_err(|error| error.to_string())
        }),
        [_, "assets", asset_id] if !asset_id.is_empty() => {
            dispatch_workspace_asset(request, state, workspace_id, asset_id)
        }
        [_, "trash"] if request.method == "GET" => project_response(|| {
            let registry = state
                .workspaces
                .lock()
                .map_err(|_| "workspace registry lock is poisoned".to_owned())?;
            registry
                .list_trash(workspace_id)
                .map_err(|error| error.to_string())
        }),
        [_, "trash", asset_id] if !asset_id.is_empty() => {
            dispatch_workspace_trash(request, state, workspace_id, asset_id)
        }
        [_, "library-references", library_id]
            if request.method == "POST" && !library_id.is_empty() =>
        {
            project_response(|| {
                let project = {
                    let projects = state
                        .projects
                        .lock()
                        .map_err(|_| "project store lock is poisoned".to_owned())?;
                    projects
                        .load(library_id)
                        .map_err(|error| error.to_string())?
                };
                if !project.read_only {
                    return Err("only read-only Library content can be referenced".into());
                }
                let registry = state
                    .workspaces
                    .lock()
                    .map_err(|_| "workspace registry lock is poisoned".to_owned())?;
                registry
                    .add_library_reference(workspace_id, &project.project, project.revision_sha256)
                    .map_err(|error| error.to_string())
            })
        }
        [_, "library-forks", library_id] if request.method == "POST" && !library_id.is_empty() => {
            let fork = match serde_json::from_slice::<WorkspaceLibraryForkRequest>(&request.body) {
                Ok(fork) => fork,
                Err(error) => {
                    return project_error_response(400, "Bad Request", &error.to_string());
                }
            };
            project_response(|| {
                let project = {
                    let projects = state
                        .projects
                        .lock()
                        .map_err(|_| "project store lock is poisoned".to_owned())?;
                    projects
                        .load(library_id)
                        .map_err(|error| error.to_string())?
                };
                if !project.read_only {
                    return Err("only read-only Library content can be forked".into());
                }
                let registry = state
                    .workspaces
                    .lock()
                    .map_err(|_| "workspace registry lock is poisoned".to_owned())?;
                registry
                    .fork_library(
                        workspace_id,
                        &project.project,
                        project.revision_sha256,
                        fork,
                    )
                    .map_err(|error| error.to_string())
            })
        }
        [_, "library-scenarios", library_id]
            if request.method == "POST" && !library_id.is_empty() =>
        {
            project_response(|| {
                let project = {
                    let projects = state
                        .projects
                        .lock()
                        .map_err(|_| "project store lock is poisoned".to_owned())?;
                    projects
                        .load(library_id)
                        .map_err(|error| error.to_string())?
                };
                if !project.read_only {
                    return Err("only read-only Library content can seed a scenario".into());
                }
                let registry = state
                    .workspaces
                    .lock()
                    .map_err(|_| "workspace registry lock is poisoned".to_owned())?;
                registry
                    .create_scenario_from_library(
                        workspace_id,
                        &project.project,
                        project.revision_sha256,
                    )
                    .map_err(|error| error.to_string())
            })
        }
        _ => project_error_response(404, "Not Found", "workspace endpoint not found"),
    }
}

fn dispatch_workspace_asset(
    request: HttpRequest,
    state: &WebState,
    workspace_id: &str,
    asset_id: &str,
) -> HttpResponse {
    match request.method.as_str() {
        "GET" => project_response(|| {
            let registry = state
                .workspaces
                .lock()
                .map_err(|_| "workspace registry lock is poisoned".to_owned())?;
            registry
                .load_asset(workspace_id, asset_id)
                .map_err(|error| error.to_string())
        }),
        "PUT" => {
            let save = match serde_json::from_slice::<WorkspaceAssetSaveRequest>(&request.body) {
                Ok(save) => save,
                Err(error) => {
                    return project_error_response(400, "Bad Request", &error.to_string());
                }
            };
            project_response(|| {
                let registry = state
                    .workspaces
                    .lock()
                    .map_err(|_| "workspace registry lock is poisoned".to_owned())?;
                registry
                    .save_asset(workspace_id, asset_id, save)
                    .map_err(|error| error.to_string())
            })
        }
        "POST" => {
            let command =
                match serde_json::from_slice::<WorkspaceAssetCommandRequest>(&request.body) {
                    Ok(command) => command,
                    Err(error) => {
                        return project_error_response(400, "Bad Request", &error.to_string());
                    }
                };
            project_response(|| {
                let registry = state
                    .workspaces
                    .lock()
                    .map_err(|_| "workspace registry lock is poisoned".to_owned())?;
                registry
                    .command_asset(workspace_id, asset_id, command)
                    .map_err(|error| error.to_string())
            })
        }
        _ => project_error_response(405, "Method Not Allowed", "unsupported asset method"),
    }
}

fn dispatch_workspace_trash(
    request: HttpRequest,
    state: &WebState,
    workspace_id: &str,
    asset_id: &str,
) -> HttpResponse {
    if request.method != "POST" {
        return project_error_response(405, "Method Not Allowed", "unsupported trash method");
    }
    let command = match serde_json::from_slice::<WorkspaceTrashCommandRequest>(&request.body) {
        Ok(command) => command,
        Err(error) => return project_error_response(400, "Bad Request", &error.to_string()),
    };
    project_response(|| {
        let registry = state
            .workspaces
            .lock()
            .map_err(|_| "workspace registry lock is poisoned".to_owned())?;
        registry
            .command_trash(workspace_id, asset_id, command)
            .map_err(|error| error.to_string())
    })
}

fn dispatch_project_record(request: HttpRequest, state: &WebState) -> HttpResponse {
    let Some(id) = request.target.strip_prefix("/api/projects/") else {
        return HttpResponse {
            status: 404,
            reason: "Not Found",
            content_type: "text/plain; charset=utf-8",
            body: b"route planner endpoint not found\n".to_vec(),
        };
    };
    if id.is_empty() || id.contains('/') {
        return project_error_response(400, "Bad Request", "invalid project id");
    }
    match request.method.as_str() {
        "GET" => project_response(|| {
            let store = state
                .projects
                .lock()
                .map_err(|_| "project store lock is poisoned".to_owned())?;
            store.load(id).map_err(|error| error.to_string())
        }),
        "PUT" => {
            let save = match serde_json::from_slice::<ProjectSaveRequest>(&request.body) {
                Ok(save) => save,
                Err(error) => {
                    return project_error_response(400, "Bad Request", &error.to_string());
                }
            };
            project_response(|| {
                let store = state
                    .projects
                    .lock()
                    .map_err(|_| "project store lock is poisoned".to_owned())?;
                store.save(id, save).map_err(|error| error.to_string())
            })
        }
        _ => project_error_response(405, "Method Not Allowed", "unsupported project method"),
    }
}

fn project_response<T: serde::Serialize>(
    operation: impl FnOnce() -> Result<T, String>,
) -> HttpResponse {
    match operation() {
        Ok(value) => match serde_json::to_vec(&value) {
            Ok(body) => json_response(200, "OK", body),
            Err(error) => project_error_response(500, "Internal Server Error", &error.to_string()),
        },
        Err(error) => project_error_response(400, "Bad Request", &error),
    }
}

fn project_error_response(status: u16, reason: &'static str, detail: &str) -> HttpResponse {
    let body = serde_json::to_vec(&serde_json::json!({ "error": detail }))
        .unwrap_or_else(|_| b"{\"error\":\"project request failed\"}".to_vec());
    json_response(status, reason, body)
}

fn asset(content_type: &'static str, body: &[u8]) -> HttpResponse {
    HttpResponse {
        status: 200,
        reason: "OK",
        content_type,
        body: body.to_vec(),
    }
}

fn json_response(status: u16, reason: &'static str, body: Vec<u8>) -> HttpResponse {
    HttpResponse {
        status,
        reason,
        content_type: "application/json; charset=utf-8",
        body,
    }
}

fn write_response(stream: &mut TcpStream, response: HttpResponse) -> Result<(), PlannerWebError> {
    write!(
        stream,
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nContent-Security-Policy: default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src 'self'\r\nConnection: close\r\n\r\n",
        response.status,
        response.reason,
        response.content_type,
        response.body.len()
    )
    .map_err(PlannerWebError::io)?;
    stream
        .write_all(&response.body)
        .map_err(PlannerWebError::io)?;
    stream.flush().map_err(PlannerWebError::io)
}

#[derive(Debug)]
pub struct PlannerWebError(String);

impl PlannerWebError {
    fn io(error: std::io::Error) -> Self {
        Self(error.to_string())
    }

    fn project(error: crate::project::ProjectError) -> Self {
        Self(error.to_string())
    }

    fn workspace(error: crate::workspace::WorkspaceError) -> Self {
        Self(error.to_string())
    }
}

impl fmt::Display for PlannerWebError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for PlannerWebError {}

fn web_error(message: impl Into<String>) -> PlannerWebError {
    PlannerWebError(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::PlannerServiceRequest;
    use dusklight_route_planner::logic::{FACT_CATALOG_SCHEMA, FactCatalog};
    use dusklight_route_planner::transition::{MECHANICS_CATALOG_SCHEMA, MechanicsCatalog};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static NEXT_TEST_ROOT: AtomicU64 = AtomicU64::new(0);

    fn state() -> (WebState, PathBuf) {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "dusklight-route-web-{}-{nonce}-{}",
            std::process::id(),
            NEXT_TEST_ROOT.fetch_add(1, Ordering::Relaxed),
        ));
        let projects = ProjectStore::open(&root).unwrap();
        let available_libraries = builtin_library_digests(&projects).unwrap();
        let state = WebState {
            projects: Arc::new(Mutex::new(projects)),
            workspaces: Arc::new(Mutex::new(
                WorkspaceRegistry::open(root.join("workspaces"), available_libraries).unwrap(),
            )),
        };
        (state, root)
    }

    #[test]
    fn static_assets_and_health_are_local_and_cacheless() {
        let (state, root) = state();
        let index = dispatch(
            HttpRequest {
                method: "GET".into(),
                target: "/".into(),
                body: Vec::new(),
            },
            &state,
        );
        assert_eq!(index.status, 200);
        assert_eq!(index.content_type, "text/html; charset=utf-8");
        assert!(
            index
                .body
                .windows(13)
                .any(|window| window == b"Route Planner")
        );
        let app = dispatch(
            HttpRequest {
                method: "GET".into(),
                target: "/app.js".into(),
                body: Vec::new(),
            },
            &state,
        );
        assert_eq!(app.status, 200);
        assert!(
            String::from_utf8(app.body)
                .unwrap()
                .contains("evaluate_transition")
        );

        let health = dispatch(
            HttpRequest {
                method: "GET".into(),
                target: "/api/health".into(),
                body: Vec::new(),
            },
            &state,
        );
        assert_eq!(health.status, 200);
        assert!(
            health
                .body
                .windows(13)
                .any(|window| window == b"\"status\":\"ok\"")
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn typed_service_rejects_an_unknown_protocol_through_http() {
        let (state, root) = state();
        let envelope = PlannerServiceEnvelope {
            schema: "dusklight.route-planner.service/v999".into(),
            request: PlannerServiceRequest::Compose {
                request_id: "web-test".into(),
                facts: Box::new(FactCatalog {
                    schema: FACT_CATALOG_SCHEMA.into(),
                    aliases: Vec::new(),
                    derived_facts: Vec::new(),
                }),
                mechanics: Box::new(MechanicsCatalog {
                    schema: MECHANICS_CATALOG_SCHEMA.into(),
                    transitions: Vec::new(),
                    obligations: Vec::new(),
                    writers: Vec::new(),
                    gates: Vec::new(),
                    readers: Vec::new(),
                    reconstruction_rules: Vec::new(),
                    obstructions: Vec::new(),
                    resolvers: Vec::new(),
                    techniques: Vec::new(),
                    microtraces: Vec::new(),
                    goals: Vec::new(),
                }),
                packs: Vec::new(),
                route_local_overlays: Vec::new(),
                ephemeral_what_if_overlays: Vec::new(),
            },
        };
        let response = dispatch(
            HttpRequest {
                method: "POST".into(),
                target: "/api/service".into(),
                body: serde_json::to_vec(&envelope).unwrap(),
            },
            &state,
        );
        assert_eq!(response.status, 200);
        let body: serde_json::Value = serde_json::from_slice(&response.body).unwrap();
        assert_eq!(body["outcome"]["status"], "error");
        assert_eq!(body["outcome"]["field"], "schema");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn project_http_lists_loads_saves_and_rejects_stale_revisions() {
        let (state, root) = state();
        let list = dispatch(
            HttpRequest {
                method: "GET".into(),
                target: "/api/projects".into(),
                body: Vec::new(),
            },
            &state,
        );
        assert_eq!(list.status, 200);
        let list: serde_json::Value = serde_json::from_slice(&list.body).unwrap();
        assert_eq!(list["projects"].as_array().unwrap().len(), 6);

        let template = dispatch(
            HttpRequest {
                method: "GET".into(),
                target: "/api/project-template".into(),
                body: Vec::new(),
            },
            &state,
        );
        let mut record: serde_json::Value = serde_json::from_slice(&template.body).unwrap();
        record["project"]["id"] = "http-route".into();
        record["project"]["label"] = "HTTP route".into();
        let request = serde_json::json!({
            "schema": crate::project::WEB_PROJECT_SAVE_SCHEMA,
            "expected_revision_sha256": null,
            "project": record["project"],
        });
        let saved = dispatch(
            HttpRequest {
                method: "PUT".into(),
                target: "/api/projects/http-route".into(),
                body: serde_json::to_vec(&request).unwrap(),
            },
            &state,
        );
        assert_eq!(saved.status, 200);
        let conflict = dispatch(
            HttpRequest {
                method: "PUT".into(),
                target: "/api/projects/http-route".into(),
                body: serde_json::to_vec(&request).unwrap(),
            },
            &state,
        );
        assert_eq!(conflict.status, 400);
        assert!(
            String::from_utf8(conflict.body)
                .unwrap()
                .contains("revision conflict")
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn workspace_http_creates_lists_and_loads_file_backed_workspaces() {
        let (state, root) = state();
        let create = serde_json::json!({
            "schema": crate::workspace::WORKSPACE_CREATE_SCHEMA,
            "id": "ordon-route",
            "label": "Ordon route",
        });
        let created = dispatch(
            HttpRequest {
                method: "POST".into(),
                target: "/api/workspaces".into(),
                body: serde_json::to_vec(&create).unwrap(),
            },
            &state,
        );
        assert_eq!(created.status, 200);
        let created: serde_json::Value = serde_json::from_slice(&created.body).unwrap();
        assert_eq!(created["manifest"]["id"], "ordon-route");
        assert!(created["manifest"].get("catalog").is_none());
        assert_eq!(created["assets"].as_array().unwrap().len(), 0);

        let list = dispatch(
            HttpRequest {
                method: "GET".into(),
                target: "/api/workspaces".into(),
                body: Vec::new(),
            },
            &state,
        );
        let list: serde_json::Value = serde_json::from_slice(&list.body).unwrap();
        assert_eq!(list["workspaces"].as_array().unwrap().len(), 1);
        assert_eq!(list["workspaces"][0]["label"], "Ordon route");

        let loaded = dispatch(
            HttpRequest {
                method: "GET".into(),
                target: "/api/workspaces/ordon-route".into(),
                body: Vec::new(),
            },
            &state,
        );
        assert_eq!(loaded.status, 200);
        let loaded: serde_json::Value = serde_json::from_slice(&loaded.body).unwrap();
        assert_eq!(loaded["manifest"]["id"], "ordon-route");
        assert!(root.join("workspaces/ordon-route/workspace.json").is_file());

        let referenced = dispatch(
            HttpRequest {
                method: "POST".into(),
                target: "/api/workspaces/ordon-route/library-references/demo-forest-keyed-door"
                    .into(),
                body: Vec::new(),
            },
            &state,
        );
        assert_eq!(referenced.status, 200);
        let referenced: serde_json::Value = serde_json::from_slice(&referenced.body).unwrap();
        assert_eq!(
            referenced["manifest"]["mounted_libraries"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert!(referenced["assets"].as_array().unwrap().is_empty());

        let imported = dispatch(
            HttpRequest {
                method: "POST".into(),
                target: "/api/workspaces/ordon-route/library-scenarios/demo-forest-keyed-door"
                    .into(),
                body: Vec::new(),
            },
            &state,
        );
        assert_eq!(imported.status, 200);
        let imported: serde_json::Value = serde_json::from_slice(&imported.body).unwrap();
        assert_eq!(
            imported["manifest"]["mounted_libraries"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        for kind in ["scenario", "route_graph", "state_seed", "layout"] {
            assert!(
                imported["assets"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|asset| asset["kind"] == kind),
                "missing imported {kind}"
            );
        }
        assert!(
            imported["assets"]
                .as_array()
                .unwrap()
                .iter()
                .all(|asset| asset["id"] == "custom.roll"
                    || asset["kind"] == "route_book"
                    || asset.get("revision_sha256").is_some())
        );

        let fork = serde_json::json!({
            "schema": crate::workspace::WORKSPACE_LIBRARY_FORK_SCHEMA,
            "namespace": "forest-alternate",
        });
        let forked = dispatch(
            HttpRequest {
                method: "POST".into(),
                target: "/api/workspaces/ordon-route/library-forks/demo-forest-keyed-door".into(),
                body: serde_json::to_vec(&fork).unwrap(),
            },
            &state,
        );
        assert_eq!(forked.status, 200);
        let forked: serde_json::Value = serde_json::from_slice(&forked.body).unwrap();
        assert!(forked["assets"].as_array().unwrap().iter().any(|asset| {
            asset["id"] == "route-graph.forest-alternate"
                && asset["kind"] == "route_graph"
                && asset.get("revision_sha256").is_some()
        }));

        let asset = crate::workspace::WorkspaceAsset {
            schema: crate::workspace::WORKSPACE_ASSET_SCHEMA.into(),
            header: crate::workspace::WorkspaceAssetHeader {
                id: "custom.roll".into(),
                label: "Roll".into(),
                kind: crate::workspace::WorkspaceAssetKind::CustomNodeDefinition,
                version: 1,
                origin: None,
            },
            references: Vec::new(),
            payload: crate::workspace::WorkspaceAssetPayload::CustomNodeDefinition(
                crate::workspace::CustomNodeDefinitionAsset {
                    inputs: Vec::new(),
                    outputs: Vec::new(),
                    guard: dusklight_route_planner::logic::PredicateExpression::True,
                    effects: Vec::new(),
                    evidence_status: crate::workspace::CustomNodeEvidenceStatus::Hypothetical,
                    evidence: Vec::new(),
                },
            ),
        };
        let save = crate::workspace::WorkspaceAssetSaveRequest {
            schema: crate::workspace::WORKSPACE_ASSET_SAVE_SCHEMA.into(),
            relative_path: "custom-nodes/roll.json".into(),
            expected_revision_sha256: None,
            asset,
        };
        let saved = dispatch(
            HttpRequest {
                method: "PUT".into(),
                target: "/api/workspaces/ordon-route/assets/custom.roll".into(),
                body: serde_json::to_vec(&save).unwrap(),
            },
            &state,
        );
        assert_eq!(saved.status, 200);
        let saved: serde_json::Value = serde_json::from_slice(&saved.body).unwrap();
        assert_eq!(saved["asset"]["header"]["id"], "custom.roll");
        assert!(
            root.join("workspaces/ordon-route/custom-nodes/roll.json")
                .is_file()
        );
        let loaded_asset = dispatch(
            HttpRequest {
                method: "GET".into(),
                target: "/api/workspaces/ordon-route/assets/custom.roll".into(),
                body: Vec::new(),
            },
            &state,
        );
        assert_eq!(loaded_asset.status, 200);
        let loaded_asset: serde_json::Value = serde_json::from_slice(&loaded_asset.body).unwrap();
        assert_eq!(
            loaded_asset["asset"]["payload"]["evidence_status"],
            "hypothetical"
        );
        let revision = loaded_asset["revision_sha256"].as_str().unwrap();
        let rename = serde_json::json!({
            "schema": crate::workspace::WORKSPACE_ASSET_COMMAND_SCHEMA,
            "command": {
                "kind": "rename",
                "expected_revision_sha256": revision,
                "label": "Roll quickly",
            },
        });
        let renamed = dispatch(
            HttpRequest {
                method: "POST".into(),
                target: "/api/workspaces/ordon-route/assets/custom.roll".into(),
                body: serde_json::to_vec(&rename).unwrap(),
            },
            &state,
        );
        assert_eq!(renamed.status, 200);
        let renamed: serde_json::Value = serde_json::from_slice(&renamed.body).unwrap();
        let renamed_asset = renamed["assets"]
            .as_array()
            .unwrap()
            .iter()
            .find(|asset| asset["id"] == "custom.roll")
            .unwrap();
        assert_eq!(renamed_asset["label"], "Roll quickly");
        let trash = serde_json::json!({
            "schema": crate::workspace::WORKSPACE_ASSET_COMMAND_SCHEMA,
            "command": {
                "kind": "delete_to_trash",
                "expected_revision_sha256": renamed_asset["revision_sha256"],
                "allow_broken_references": false,
            },
        });
        let trashed = dispatch(
            HttpRequest {
                method: "POST".into(),
                target: "/api/workspaces/ordon-route/assets/custom.roll".into(),
                body: serde_json::to_vec(&trash).unwrap(),
            },
            &state,
        );
        assert_eq!(trashed.status, 200);
        let trash_list = dispatch(
            HttpRequest {
                method: "GET".into(),
                target: "/api/workspaces/ordon-route/trash".into(),
                body: Vec::new(),
            },
            &state,
        );
        let trash_list: serde_json::Value = serde_json::from_slice(&trash_list.body).unwrap();
        assert_eq!(trash_list.as_array().unwrap().len(), 1);
        let restore = serde_json::json!({
            "schema": crate::workspace::WORKSPACE_TRASH_COMMAND_SCHEMA,
            "expected_revision_sha256": trash_list[0]["revision_sha256"],
            "command": "restore",
        });
        let restored = dispatch(
            HttpRequest {
                method: "POST".into(),
                target: "/api/workspaces/ordon-route/trash/custom.roll".into(),
                body: serde_json::to_vec(&restore).unwrap(),
            },
            &state,
        );
        assert_eq!(restored.status, 200);

        let exported = dispatch(
            HttpRequest {
                method: "GET".into(),
                target: "/api/workspaces/ordon-route/export".into(),
                body: Vec::new(),
            },
            &state,
        );
        assert_eq!(exported.status, 200);
        let mut bundle =
            serde_json::from_slice::<crate::workspace::WorkspaceExport>(&exported.body).unwrap();
        assert_eq!(bundle.schema, crate::workspace::WORKSPACE_EXPORT_SCHEMA);
        assert!(bundle.assets.iter().any(|record| {
            record.asset.header.id == "custom.roll"
                && record.relative_path == std::path::Path::new("custom-nodes/roll.json")
        }));
        bundle.manifest.id = "ordon-route-copy".into();
        bundle.manifest.label = "Ordon route copy".into();
        let imported_workspace = dispatch(
            HttpRequest {
                method: "POST".into(),
                target: "/api/workspaces/import".into(),
                body: serde_json::to_vec(&bundle).unwrap(),
            },
            &state,
        );
        assert_eq!(imported_workspace.status, 200);
        let imported_workspace: serde_json::Value =
            serde_json::from_slice(&imported_workspace.body).unwrap();
        assert_eq!(imported_workspace["manifest"]["id"], "ordon-route-copy");
        assert_eq!(
            imported_workspace["assets"].as_array().unwrap().len(),
            bundle.assets.len()
        );
        assert!(
            root.join("workspaces/ordon-route-copy/custom-nodes/roll.json")
                .is_file()
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
