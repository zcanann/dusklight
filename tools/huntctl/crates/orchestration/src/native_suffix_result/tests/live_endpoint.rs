use super::*;

#[test]
fn result_has_no_portable_machine_image() {
    let mut request = request(false);
    request.schema = NATIVE_CACHED_SUFFIX_BATCH_SCHEMA.into();
    request.checkpoint_cache = Some(
        dusklight_search::suffix_batch::NativeCheckpointCacheRequest {
            capacity_bytes: 671_088_640,
            capacity_entries: 2,
            source_identity: Some("a".repeat(32)),
            source_route_ticks: 40,
            retain_candidate_checkpoints: false,
            retain_live_endpoint: true,
        },
    );
    let mut result = result(false, false);
    result.schema = NATIVE_SUFFIX_BATCH_RESULT_SCHEMA_V9.into();
    result.restore_identity = Some("a".repeat(32));
    result.restore_micros = vec![0];
    result.timing.phases["checkpoint_restore"]["micros"] = Value::from(0);
    result.checkpoint_cache = Some(
        serde_json::from_value(serde_json::json!({
            "source_kind": "direct_process_local_continuation",
            "source_identity": "a".repeat(32),
            "source_route_ticks": 40,
            "capacity_bytes": 671_088_640_u64,
            "capacity_entries": 2,
            "resident_bytes": 0,
            "resident_checkpoint_bytes": 0,
            "resident_host_snapshot_bytes": 0,
            "resident_entries": 0,
            "insertions": 0,
            "replacements": 0,
            "evictions": 0,
            "hits": 0,
            "misses": 0,
            "source_pinned": false,
            "batch_capture_attempts": 0,
            "batch_capture_successes": 0,
            "batch_capture_micros": 0,
            "live_endpoint_capacity_entries": 1,
            "live_endpoint_resident_entries": 1,
            "live_endpoint_resident_host_snapshot_bytes": 64,
            "batch_live_retention_attempts": 1,
            "batch_live_retention_successes": 1,
            "batch_live_retention_nanos": 1,
            "batch_live_consumptions": 1,
            "batch_live_invalidations": 0
        }))
        .unwrap(),
    );
    result.candidates[0].retained_checkpoint = Some(NativeRetainedCheckpointResult {
        storage_kind: "live_endpoint".into(),
        restore_identity: "b".repeat(32),
        image_digest: None,
        semantic_digest: None,
        checkpoint_bytes: 0,
        host_snapshot_bytes: 64,
        machine_capture_micros: 0,
        host_snapshot_capture_nanos: 1,
        capture_micros: 0,
        route_ticks: 42,
    });

    let validated = result.validate_against(&request, &terminal()).unwrap();
    let retained = validated.candidates[0]
        .retained_checkpoint
        .as_ref()
        .unwrap();
    assert_eq!(retained.storage_kind, "live_endpoint");
    assert_eq!(retained.checkpoint_bytes, 0);
    assert!(retained.image_digest.is_none());
    assert!(retained.semantic_digest.is_none());
}
