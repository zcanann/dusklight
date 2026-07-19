use huntctl::client::WorkerClient;
use huntctl::transport::ProcessTransport;

#[test]
fn persistent_process_handles_hello_and_ping() {
    let executable = env!("CARGO_BIN_EXE_huntctl");
    let transport = ProcessTransport::spawn(executable, &["mock-worker".into()]).unwrap();
    let child_id = transport.child_id();
    let mut client = WorkerClient::new(transport);
    let hello = client.handshake().unwrap().clone();
    assert_eq!(hello.build.revision.len(), 40);
    assert!(
        hello
            .build
            .revision
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    );
    assert!(hello.capabilities.persistent_control);
    assert!(!hello.capabilities.engine_session);
    assert!(
        hello
            .capabilities
            .commands
            .iter()
            .any(|command| command == "session_audit")
    );
    let audit = client.session_audit().unwrap();
    assert!(!audit.reusable);
    assert_eq!(audit.target_boundary, "post_authenticated_run");
    assert_eq!(audit.blockers[0].code, "game_global_reconstruction");
    client.ping().unwrap();
    assert_eq!(client.into_transport().child_id(), child_id);
}

#[test]
fn session_audit_cli_reports_the_exact_reuse_refusal() {
    let executable = env!("CARGO_BIN_EXE_huntctl");
    let output = std::process::Command::new(executable)
        .args([
            "session",
            "audit",
            "--worker",
            executable,
            "--worker-arg",
            "mock-worker",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let audit: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(audit["reusable"], false);
    assert_eq!(audit["evaluated_boundary"], "pre_engine_boot");
    assert_eq!(audit["blockers"][0]["subsystem"], "game_state");
}

#[test]
fn graceful_shutdown_is_acknowledged() {
    let executable = env!("CARGO_BIN_EXE_huntctl");
    let transport = ProcessTransport::spawn(executable, &["mock-worker".into()]).unwrap();
    let mut client = WorkerClient::new(transport);
    client.handshake().unwrap();
    client.shutdown().unwrap();
}

#[test]
fn active_fidelity_profile_is_part_of_worker_identity() {
    let executable = env!("CARGO_BIN_EXE_huntctl");
    let mut observe =
        WorkerClient::new(ProcessTransport::spawn(executable, &["mock-worker".into()]).unwrap());
    let observe_identity = observe.handshake().unwrap().clone();
    observe.shutdown().unwrap();

    let mut shadow = WorkerClient::new(
        ProcessTransport::spawn(
            executable,
            &[
                "mock-worker".into(),
                "--mock-fidelity-profile".into(),
                "cursor_breakout_shadow".into(),
            ],
        )
        .unwrap(),
    );
    let shadow_identity = shadow.handshake().unwrap().clone();
    shadow.shutdown().unwrap();

    let differences = observe_identity.identity_differences(&shadow_identity);
    assert_eq!(differences.len(), 1);
    assert_eq!(differences[0].field, "build.fidelity_profile");
}
