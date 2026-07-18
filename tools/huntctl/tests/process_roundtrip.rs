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
    client.ping().unwrap();
    assert_eq!(client.into_transport().child_id(), child_id);
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
