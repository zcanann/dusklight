//! Minimal localhost HTTP boundary for the independent planner editor.

use crate::service::{PlannerServiceEnvelope, error_response, handle_envelope};
use std::error::Error;
use std::fmt;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::thread;

const MAX_HEADER_BYTES: usize = 64 * 1024;
const MAX_BODY_BYTES: usize = 64 * 1024 * 1024;
const INDEX_HTML: &[u8] = include_bytes!("web/index.html");
const APP_CSS: &[u8] = include_bytes!("web/app.css");
const APP_JS: &[u8] = include_bytes!("web/app.js");

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PlannerWebConfig {
    pub listen: SocketAddr,
}

pub fn serve_web(config: PlannerWebConfig) -> Result<(), PlannerWebError> {
    if !config.listen.ip().is_loopback() {
        return Err(web_error("planner web server must bind to loopback"));
    }
    let listener = TcpListener::bind(config.listen).map_err(PlannerWebError::io)?;
    for stream in listener.incoming() {
        let stream = stream.map_err(PlannerWebError::io)?;
        thread::Builder::new()
            .name("route-planner-http".into())
            .spawn(move || {
                if let Err(error) = handle_connection(stream) {
                    eprintln!("route-planner web: {error}");
                }
            })
            .map_err(PlannerWebError::io)?;
    }
    Ok(())
}

fn handle_connection(mut stream: TcpStream) -> Result<(), PlannerWebError> {
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(15)))
        .map_err(PlannerWebError::io)?;
    stream
        .set_write_timeout(Some(std::time::Duration::from_secs(15)))
        .map_err(PlannerWebError::io)?;
    let request = read_request(&mut stream)?;
    let response = dispatch(request);
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

fn dispatch(request: HttpRequest) -> HttpResponse {
    match (request.method.as_str(), request.target.as_str()) {
        ("GET", "/") | ("GET", "/index.html") => asset("text/html; charset=utf-8", INDEX_HTML),
        ("GET", "/app.css") => asset("text/css; charset=utf-8", APP_CSS),
        ("GET", "/app.js") => asset("text/javascript; charset=utf-8", APP_JS),
        ("GET", "/api/health") => json_response(
            200,
            "OK",
            br#"{"schema":"dusklight.route-planner.web-health/v1","status":"ok"}"#.to_vec(),
        ),
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
        _ => HttpResponse {
            status: 404,
            reason: "Not Found",
            content_type: "text/plain; charset=utf-8",
            body: b"route planner endpoint not found\n".to_vec(),
        },
    }
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

    #[test]
    fn static_assets_and_health_are_local_and_cacheless() {
        let index = dispatch(HttpRequest {
            method: "GET".into(),
            target: "/".into(),
            body: Vec::new(),
        });
        assert_eq!(index.status, 200);
        assert_eq!(index.content_type, "text/html; charset=utf-8");
        assert!(
            index
                .body
                .windows(13)
                .any(|window| window == b"Route Planner")
        );

        let health = dispatch(HttpRequest {
            method: "GET".into(),
            target: "/api/health".into(),
            body: Vec::new(),
        });
        assert_eq!(health.status, 200);
        assert!(
            health
                .body
                .windows(13)
                .any(|window| window == b"\"status\":\"ok\"")
        );
    }

    #[test]
    fn typed_service_rejects_an_unknown_protocol_through_http() {
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
        let response = dispatch(HttpRequest {
            method: "POST".into(),
            target: "/api/service".into(),
            body: serde_json::to_vec(&envelope).unwrap(),
        });
        assert_eq!(response.status, 200);
        let body: serde_json::Value = serde_json::from_slice(&response.body).unwrap();
        assert_eq!(body["outcome"]["status"], "error");
        assert_eq!(body["outcome"]["field"], "schema");
    }
}
