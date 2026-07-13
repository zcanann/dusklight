use huntctl::client::WorkerClient;
use huntctl::transport::ProcessTransport;

#[test]
fn persistent_process_handles_hello_and_ping() {
    let executable = env!("CARGO_BIN_EXE_huntctl");
    let transport = ProcessTransport::spawn(executable, &["mock-worker".into()]).unwrap();
    let child_id = transport.child_id();
    let mut client = WorkerClient::new(transport);
    let hello = client.handshake().unwrap().clone();
    assert_eq!(hello.build.revision, "mock");
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
