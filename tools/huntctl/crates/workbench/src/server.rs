//! Loopback-only HTTP transport for the route workbench.

use super::*;

pub fn serve(listener: TcpListener, mut config: WorkbenchConfig) -> Result<(), WorkbenchError> {
    let address = listener
        .local_addr()
        .map_err(|error| WorkbenchError::new(error.to_string()))?;
    if !address.ip().is_loopback() {
        return Err(WorkbenchError::new(
            "route workbench must bind to a loopback address",
        ));
    }
    configured_artifact_root(&config)?;
    for stream in listener.incoming() {
        match stream {
            Ok(mut stream) => {
                let response = handle_http(&mut stream, address, &mut config);
                let _ = write_http_response(&mut stream, response);
            }
            Err(error) => return Err(WorkbenchError::new(format!("HTTP accept failed: {error}"))),
        }
    }
    Ok(())
}

pub(super) struct HttpResponse {
    pub(super) status: u16,
    pub(super) reason: &'static str,
    pub(super) content_type: &'static str,
    pub(super) body: Vec<u8>,
}

pub(super) fn thumbnail_response(config: &WorkbenchConfig, request_path: &str) -> HttpResponse {
    let Some(filename) = request_path.strip_prefix("/api/thumbnails/") else {
        return json_error(404, "Not Found", "unknown thumbnail");
    };
    let Some(key) = filename.strip_suffix(".png") else {
        return json_error(404, "Not Found", "unknown thumbnail");
    };
    if !valid_sha256(key) || filename.len() != 68 {
        return json_error(404, "Not Found", "unknown thumbnail");
    }
    let path = thumbnail_cache_path(&config.state_root, key);
    if !thumbnail_file_is_valid(&path) {
        return json_error(404, "Not Found", "thumbnail is not available");
    }
    match fs::read(path) {
        Ok(body) => HttpResponse {
            status: 200,
            reason: "OK",
            content_type: "image/png",
            body,
        },
        Err(error) => json_error(
            500,
            "Internal Server Error",
            &format!("cannot read thumbnail: {error}"),
        ),
    }
}

pub(super) fn handle_http(
    stream: &mut TcpStream,
    server_address: SocketAddr,
    config: &mut WorkbenchConfig,
) -> HttpResponse {
    match read_http_request(stream) {
        Ok(request) => {
            if !origin_allowed(request.origin.as_deref(), server_address) {
                return json_error(403, "Forbidden", "cross-origin requests are not allowed");
            }
            match (request.method.as_str(), request.path.as_str()) {
                ("GET", "/") => {
                    html_response(include_bytes!("../../../assets/route_workbench.html"))
                }
                ("GET", "/api/graph") => load_authoritative_timeline(&config.timeline_path)
                    .and_then(|timeline| {
                        let artifact_root = configured_artifact_root(config)?;
                        let mut graph =
                            graph_with_drafts(&timeline, &artifact_root, &config.state_root)?;
                        graph.projects = project_catalog_projection(
                            &config.repository_root,
                            &config.timeline_path,
                        )?;
                        append_generated_search_segments(
                            &mut graph,
                            &timeline,
                            &config.repository_root.join("build/search"),
                            &config.state_root,
                        )?;
                        decorate_graph_thumbnails(&mut graph, config)?;
                        Ok(graph)
                    })
                    .and_then(|graph| json_response(&graph))
                    .unwrap_or_else(|error| {
                        json_error(500, "Internal Server Error", &error.to_string())
                    }),
                ("POST", "/api/play") => {
                    let result = serde_json::from_slice::<BrowserPlayRequest>(&request.body)
                        .map_err(|error| {
                            WorkbenchError::new(format!("invalid play request: {error}"))
                        })
                        .and_then(|browser_request| {
                            validate_playback_origin(&browser_request)?;
                            let timeline = load_authoritative_timeline(&config.timeline_path)?;
                            let accelerated =
                                browser_request.mode == PlaybackMode::ResumeAccelerated;
                            let playback = PlaybackSettings {
                                speed_percent: if accelerated {
                                    0
                                } else {
                                    browser_request.speed_percent
                                },
                                fast: accelerated,
                            };
                            let (response, _child) = match &browser_request.selection {
                                BrowserSelection::Draft { id } => play_draft(
                                    &timeline,
                                    config,
                                    id,
                                    playback.speed_percent,
                                    playback.fast,
                                )?,
                                BrowserSelection::Segment { id } => play_segment(
                                    &timeline,
                                    config,
                                    id,
                                    &browser_request.stop,
                                    SegmentPlaybackOptions {
                                        handoff: browser_request.handoff,
                                        playback,
                                    },
                                )?,
                                BrowserSelection::Project { id } => play_project(
                                    &timeline,
                                    config,
                                    id,
                                    browser_request.handoff,
                                    playback,
                                )?,
                            };
                            Ok(response)
                        });
                    match result {
                        Ok(response) => json_response(&response).unwrap_or_else(|error| {
                            json_error(500, "Internal Server Error", &error.to_string())
                        }),
                        Err(error) => json_error(400, "Bad Request", &error.to_string()),
                    }
                }
                ("POST", "/api/thumbnails/capture") => {
                    let result =
                        serde_json::from_slice::<BrowserThumbnailCaptureRequest>(&request.body)
                            .map_err(|error| {
                                WorkbenchError::new(format!(
                                    "invalid thumbnail capture request: {error}"
                                ))
                            })
                            .and_then(|capture_request| {
                                let timeline = load_authoritative_timeline(&config.timeline_path)?;
                                capture_thumbnail(&timeline, config, &capture_request)
                                    .map(|(response, _child)| response)
                            });
                    match result {
                        Ok(response) => json_response(&response).unwrap_or_else(|error| {
                            json_error(500, "Internal Server Error", &error.to_string())
                        }),
                        Err(error) => json_error(400, "Bad Request", &error.to_string()),
                    }
                }
                ("GET", path) if path.starts_with("/api/thumbnails/") => {
                    thumbnail_response(config, path)
                }
                ("POST", "/api/record") => {
                    let result = serde_json::from_slice::<BrowserRecordRequest>(&request.body)
                        .map_err(|error| {
                            WorkbenchError::new(format!("invalid record request: {error}"))
                        })
                        .and_then(|record_request| {
                            let timeline = load_authoritative_timeline(&config.timeline_path)?;
                            record_continuation(&timeline, config, record_request)
                        });
                    match result {
                        Ok(response) => json_response(&response).unwrap_or_else(|error| {
                            json_error(500, "Internal Server Error", &error.to_string())
                        }),
                        Err(error) => json_error(400, "Bad Request", &error.to_string()),
                    }
                }
                ("POST", "/api/segments/delete/preview") => {
                    let result =
                        serde_json::from_slice::<BrowserSegmentDeletePreviewRequest>(&request.body)
                            .map_err(|error| {
                                WorkbenchError::new(format!(
                                    "invalid segment delete preview request: {error}"
                                ))
                            })
                            .and_then(|delete_request| {
                                preview_segment_deletion(
                                    &config.timeline_path,
                                    &config.state_root,
                                    &delete_request.id,
                                )
                            });
                    match result {
                        Ok(response) => json_response(&response).unwrap_or_else(|error| {
                            json_error(500, "Internal Server Error", &error.to_string())
                        }),
                        Err(error) => json_error(400, "Bad Request", &error.to_string()),
                    }
                }
                ("POST", "/api/segments/delete/apply") => {
                    let result =
                        serde_json::from_slice::<BrowserSegmentDeleteApplyRequest>(&request.body)
                            .map_err(|error| {
                                SegmentDeleteError::Invalid(WorkbenchError::new(format!(
                                    "invalid segment delete apply request: {error}"
                                )))
                            })
                            .and_then(|delete_request| {
                                apply_segment_deletion(
                                    &config.timeline_path,
                                    &config.state_root,
                                    &delete_request,
                                )
                            });
                    match result {
                        Ok(response) => json_response(&response).unwrap_or_else(|error| {
                            json_error(500, "Internal Server Error", &error.to_string())
                        }),
                        Err(error @ SegmentDeleteError::Conflict(_)) => {
                            json_error(409, "Conflict", &error.to_string())
                        }
                        Err(error) => json_error(400, "Bad Request", &error.to_string()),
                    }
                }
                ("POST", "/api/segments/delete-siblings/preview") => {
                    let result =
                        serde_json::from_slice::<BrowserSiblingDeletePreviewRequest>(&request.body)
                            .map_err(|error| {
                                WorkbenchError::new(format!(
                                    "invalid sibling delete preview request: {error}"
                                ))
                            })
                            .and_then(|delete_request| {
                                preview_sibling_deletion(
                                    &config.timeline_path,
                                    &config.repository_root,
                                    &config.state_root,
                                    &delete_request.keep_id,
                                )
                            });
                    match result {
                        Ok(response) => json_response(&response).unwrap_or_else(|error| {
                            json_error(500, "Internal Server Error", &error.to_string())
                        }),
                        Err(error) => json_error(400, "Bad Request", &error.to_string()),
                    }
                }
                ("POST", "/api/segments/delete-siblings/apply") => {
                    let result =
                        serde_json::from_slice::<BrowserSiblingDeleteApplyRequest>(&request.body)
                            .map_err(|error| {
                                SegmentDeleteError::Invalid(WorkbenchError::new(format!(
                                    "invalid sibling delete apply request: {error}"
                                )))
                            })
                            .and_then(|delete_request| {
                                apply_sibling_deletion(
                                    &config.timeline_path,
                                    &config.repository_root,
                                    &config.state_root,
                                    &delete_request,
                                )
                            });
                    match result {
                        Ok(response) => json_response(&response).unwrap_or_else(|error| {
                            json_error(500, "Internal Server Error", &error.to_string())
                        }),
                        Err(error @ SegmentDeleteError::Conflict(_)) => {
                            json_error(409, "Conflict", &error.to_string())
                        }
                        Err(error) => json_error(400, "Bad Request", &error.to_string()),
                    }
                }
                ("POST", "/api/drafts/delete/preview") => {
                    let result =
                        serde_json::from_slice::<BrowserDraftDeletePreviewRequest>(&request.body)
                            .map_err(|error| {
                                WorkbenchError::new(format!(
                                    "invalid delete preview request: {error}"
                                ))
                            })
                            .and_then(|delete_request| {
                                preview_draft_deletion(&config.state_root, &delete_request.id)
                            });
                    match result {
                        Ok(response) => json_response(&response).unwrap_or_else(|error| {
                            json_error(500, "Internal Server Error", &error.to_string())
                        }),
                        Err(error) => json_error(400, "Bad Request", &error.to_string()),
                    }
                }
                ("POST", "/api/drafts/delete/apply") => {
                    let result =
                        serde_json::from_slice::<BrowserDraftDeleteApplyRequest>(&request.body)
                            .map_err(|error| {
                                WorkbenchError::new(format!(
                                    "invalid delete apply request: {error}"
                                ))
                            })
                            .and_then(|delete_request| {
                                apply_draft_deletion(&config.state_root, &delete_request)
                            });
                    match result {
                        Ok(response) => json_response(&response).unwrap_or_else(|error| {
                            json_error(500, "Internal Server Error", &error.to_string())
                        }),
                        Err(error) => json_error(400, "Bad Request", &error.to_string()),
                    }
                }
                ("POST", "/api/drafts/rename") => {
                    let result = serde_json::from_slice::<BrowserDraftRenameRequest>(&request.body)
                        .map_err(|error| {
                            DraftRenameError::Invalid(WorkbenchError::new(format!(
                                "invalid draft rename request: {error}"
                            )))
                        })
                        .and_then(|rename_request| {
                            rename_draft_label(&config.state_root, &rename_request)
                        });
                    match result {
                        Ok(response) => json_response(&response).unwrap_or_else(|error| {
                            json_error(500, "Internal Server Error", &error.to_string())
                        }),
                        Err(error @ DraftRenameError::Conflict(_)) => {
                            json_error(409, "Conflict", &error.to_string())
                        }
                        Err(error) => json_error(400, "Bad Request", &error.to_string()),
                    }
                }
                ("POST", "/api/segments/rename") => {
                    let result =
                        serde_json::from_slice::<BrowserSegmentRenameRequest>(&request.body)
                            .map_err(|error| {
                                SegmentRenameError::Invalid(WorkbenchError::new(format!(
                                    "invalid segment rename request: {error}"
                                )))
                            })
                            .and_then(|rename_request| {
                                rename_segment(&config.timeline_path, &rename_request)
                            });
                    match result {
                        Ok(response) => json_response(&response).unwrap_or_else(|error| {
                            json_error(500, "Internal Server Error", &error.to_string())
                        }),
                        Err(error @ SegmentRenameError::Conflict(_)) => {
                            json_error(409, "Conflict", &error.to_string())
                        }
                        Err(error) => json_error(400, "Bad Request", &error.to_string()),
                    }
                }
                ("POST", "/api/workspace/boot") => {
                    let result =
                        serde_json::from_slice::<BrowserBootOverrideUpdateRequest>(&request.body)
                            .map_err(|error| {
                                WorkbenchError::new(format!(
                                    "invalid boot override request: {error}"
                                ))
                            })
                            .and_then(|update| {
                                update_boot_override(&config.repository_root, &update)
                            });
                    match result {
                        Ok(response) => json_response(&response).unwrap_or_else(|error| {
                            json_error(500, "Internal Server Error", &error.to_string())
                        }),
                        Err(error) => json_error(400, "Bad Request", &error.to_string()),
                    }
                }
                ("POST", "/api/workspace/stage-options") => {
                    let result =
                        serde_json::from_slice::<BrowserStageBootOptionsRequest>(&request.body)
                            .map_err(|error| {
                                WorkbenchError::new(format!(
                                    "invalid stage-options request: {error}"
                                ))
                            })
                            .and_then(|request| {
                                crate::stage_catalog::stage_boot_options(
                                    &config.repository_root,
                                    &request.stage,
                                )
                            });
                    match result {
                        Ok(response) => json_response(&response).unwrap_or_else(|error| {
                            json_error(500, "Internal Server Error", &error.to_string())
                        }),
                        Err(error) => json_error(400, "Bad Request", &error.to_string()),
                    }
                }
                ("POST", "/api/workspace/folders/create") => {
                    let result = serde_json::from_slice::<BrowserWorkspaceFolderCreateRequest>(
                        &request.body,
                    )
                    .map_err(|error| {
                        WorkbenchError::new(format!("invalid create-folder request: {error}"))
                    })
                    .and_then(|create| create_workspace_folder(&config.repository_root, &create));
                    match result {
                        Ok(response) => json_response(&response).unwrap_or_else(|error| {
                            json_error(500, "Internal Server Error", &error.to_string())
                        }),
                        Err(error) => json_error(400, "Bad Request", &error.to_string()),
                    }
                }
                ("POST", "/api/workspace/tapes/create") => {
                    let result =
                        serde_json::from_slice::<BrowserWorkspaceTapeCreateRequest>(&request.body)
                            .map_err(|error| {
                                WorkbenchError::new(format!("invalid create-tape request: {error}"))
                            })
                            .and_then(|create| {
                                create_workspace_tape(&config.repository_root, &create)
                            });
                    match result {
                        Ok(response) => json_response(&response).unwrap_or_else(|error| {
                            json_error(500, "Internal Server Error", &error.to_string())
                        }),
                        Err(error) => json_error(400, "Bad Request", &error.to_string()),
                    }
                }
                ("POST", "/api/workspace/tapes/clone") => {
                    let result =
                        serde_json::from_slice::<BrowserWorkspaceTapeCloneRequest>(&request.body)
                            .map_err(|error| {
                                WorkbenchError::new(format!("invalid clone-tape request: {error}"))
                            })
                            .and_then(|clone| {
                                clone_workspace_tape(&config.repository_root, &clone)
                            });
                    match result {
                        Ok(response) => json_response(&response).unwrap_or_else(|error| {
                            json_error(500, "Internal Server Error", &error.to_string())
                        }),
                        Err(error) => json_error(400, "Bad Request", &error.to_string()),
                    }
                }
                ("POST", "/api/workspace/move") => {
                    let result =
                        serde_json::from_slice::<BrowserWorkspaceMoveRequest>(&request.body)
                            .map_err(|error| {
                                WorkbenchError::new(format!(
                                    "invalid workspace-move request: {error}"
                                ))
                            })
                            .and_then(|move_request| {
                                move_workspace_node(
                                    &config.repository_root,
                                    &mut config.timeline_path,
                                    &move_request,
                                )
                            });
                    match result {
                        Ok(response) => json_response(&response).unwrap_or_else(|error| {
                            json_error(500, "Internal Server Error", &error.to_string())
                        }),
                        Err(error) => json_error(400, "Bad Request", &error.to_string()),
                    }
                }
                ("POST", "/api/workspace/delete") => {
                    let result =
                        serde_json::from_slice::<BrowserWorkspaceDeleteRequest>(&request.body)
                            .map_err(|error| {
                                WorkbenchError::new(format!(
                                    "invalid workspace-delete request: {error}"
                                ))
                            })
                            .and_then(|delete_request| {
                                delete_workspace_node(
                                    &config.repository_root,
                                    &config.timeline_path,
                                    &config.state_root,
                                    &delete_request,
                                )
                            });
                    match result {
                        Ok(response) => json_response(&response).unwrap_or_else(|error| {
                            json_error(500, "Internal Server Error", &error.to_string())
                        }),
                        Err(error) => json_error(400, "Bad Request", &error.to_string()),
                    }
                }
                ("POST", "/api/milestone-program") => {
                    let result = serde_json::from_slice::<BrowserMilestoneProgramUpdateRequest>(
                        &request.body,
                    )
                    .map_err(|error| {
                        MilestoneProgramUpdateError::Invalid(WorkbenchError::new(format!(
                            "invalid milestone program update request: {error}"
                        )))
                    })
                    .and_then(|update_request| {
                        let timeline = load_authoritative_timeline(&config.timeline_path)
                            .map_err(MilestoneProgramUpdateError::Invalid)?;
                        let artifact_root = configured_artifact_root(config)
                            .map_err(MilestoneProgramUpdateError::Invalid)?;
                        update_milestone_program(&timeline, &artifact_root, &update_request)
                    });
                    match result {
                        Ok(response) => json_response(&response).unwrap_or_else(|error| {
                            json_error(500, "Internal Server Error", &error.to_string())
                        }),
                        Err(error @ MilestoneProgramUpdateError::Stale { .. }) => {
                            json_error(409, "Conflict", &error.to_string())
                        }
                        Err(error) => json_error(400, "Bad Request", &error.to_string()),
                    }
                }
                _ => json_error(404, "Not Found", "unknown route workbench endpoint"),
            }
        }
        Err(error) => json_error(400, "Bad Request", &error.to_string()),
    }
}

struct HttpRequest {
    method: String,
    path: String,
    origin: Option<String>,
    body: Vec<u8>,
}

fn read_http_request(stream: &mut TcpStream) -> Result<HttpRequest, WorkbenchError> {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|error| WorkbenchError::new(error.to_string()))?;
    let mut bytes = Vec::new();
    let header_end = loop {
        if bytes.len() >= MAX_HTTP_HEADER {
            return Err(WorkbenchError::new("HTTP header is too large"));
        }
        let mut chunk = [0_u8; 4096];
        let count = stream
            .read(&mut chunk)
            .map_err(|error| WorkbenchError::new(format!("cannot read HTTP request: {error}")))?;
        if count == 0 {
            return Err(WorkbenchError::new("incomplete HTTP request"));
        }
        bytes.extend_from_slice(&chunk[..count]);
        if let Some(index) = find_bytes(&bytes, b"\r\n\r\n") {
            break index + 4;
        }
    };
    let header = std::str::from_utf8(&bytes[..header_end])
        .map_err(|_| WorkbenchError::new("HTTP header is not UTF-8"))?;
    let mut lines = header.split("\r\n");
    let mut request_line = lines
        .next()
        .ok_or_else(|| WorkbenchError::new("missing HTTP request line"))?
        .split_whitespace();
    let method = request_line.next().unwrap_or_default().to_string();
    let path = request_line.next().unwrap_or_default().to_string();
    if request_line.next() != Some("HTTP/1.1") || request_line.next().is_some() {
        return Err(WorkbenchError::new("invalid HTTP/1.1 request line"));
    }
    let mut content_length = 0_usize;
    let mut origin = None;
    for line in lines.filter(|line| !line.is_empty()) {
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| WorkbenchError::new("malformed HTTP header"))?;
        match name.trim().to_ascii_lowercase().as_str() {
            "content-length" => {
                content_length = value
                    .trim()
                    .parse()
                    .map_err(|_| WorkbenchError::new("invalid Content-Length"))?;
            }
            "origin" => origin = Some(value.trim().to_string()),
            _ => {}
        }
    }
    if content_length > MAX_HTTP_BODY {
        return Err(WorkbenchError::new("HTTP body is too large"));
    }
    while bytes.len() - header_end < content_length {
        let mut chunk = [0_u8; 4096];
        let count = stream
            .read(&mut chunk)
            .map_err(|error| WorkbenchError::new(format!("cannot read HTTP body: {error}")))?;
        if count == 0 {
            return Err(WorkbenchError::new("incomplete HTTP body"));
        }
        bytes.extend_from_slice(&chunk[..count]);
    }
    Ok(HttpRequest {
        method,
        path,
        origin,
        body: bytes[header_end..header_end + content_length].to_vec(),
    })
}

pub(super) fn origin_allowed(origin: Option<&str>, server: SocketAddr) -> bool {
    let Some(origin) = origin else {
        return true;
    };
    let port = server.port();
    let allowed = match server.ip() {
        IpAddr::V4(ip) => vec![
            format!("http://{ip}:{port}"),
            format!("http://localhost:{port}"),
        ],
        IpAddr::V6(ip) => vec![
            format!("http://[{ip}]:{port}"),
            format!("http://localhost:{port}"),
        ],
    };
    allowed.iter().any(|candidate| candidate == origin)
}

fn json_response(value: &impl Serialize) -> Result<HttpResponse, WorkbenchError> {
    Ok(HttpResponse {
        status: 200,
        reason: "OK",
        content_type: "application/json; charset=utf-8",
        body: serde_json::to_vec(value).map_err(|error| WorkbenchError::new(error.to_string()))?,
    })
}

fn html_response(body: &'static [u8]) -> HttpResponse {
    HttpResponse {
        status: 200,
        reason: "OK",
        content_type: "text/html; charset=utf-8",
        body: body.to_vec(),
    }
}

fn json_error(status: u16, reason: &'static str, message: &str) -> HttpResponse {
    #[derive(Serialize)]
    struct ErrorBody<'a> {
        error: &'a str,
    }
    HttpResponse {
        status,
        reason,
        content_type: "application/json; charset=utf-8",
        body: serde_json::to_vec(&ErrorBody { error: message }).unwrap_or_default(),
    }
}

fn write_http_response(stream: &mut TcpStream, response: HttpResponse) -> std::io::Result<()> {
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    write!(
        stream,
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n",
        response.status,
        response.reason,
        response.content_type,
        response.body.len()
    )?;
    stream.write_all(&response.body)
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}
