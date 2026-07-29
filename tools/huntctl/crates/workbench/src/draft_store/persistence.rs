use super::*;

pub(crate) fn read_draft_launch(directory: &Path, manifest: &DraftManifest) -> Option<DraftLaunch> {
    let bytes = fs::read(directory.join(DRAFT_LAUNCH)).ok()?;
    let launch: DraftLaunch = serde_json::from_slice(&bytes).ok()?;
    (launch.schema == "dusklight.route-workbench.launch.v2"
        && launch.id == manifest.id
        && launch.session_token == manifest.session_token)
        .then_some(launch)
}

pub(crate) fn graph_drafts_from_manifests(
    timeline: &Timeline,
    repository_root: &Path,
    state_root: &Path,
    manifests: BTreeMap<String, DraftManifest>,
) -> Result<Vec<GraphDraft>, WorkbenchError> {
    let root = drafts_root(state_root)?;
    let mut memo = BTreeMap::new();
    let mut anchor_digests = BTreeMap::new();
    for id in manifests.keys() {
        let _ = validate_draft_structure(
            id,
            &manifests,
            timeline,
            repository_root,
            &root,
            &mut memo,
            &mut anchor_digests,
        );
    }
    Ok(manifests
        .into_values()
        .map(|manifest| {
            let mut error = manifest.error.clone();
            if error.is_none() && manifest.status == DraftStatus::Ready {
                error = memo
                    .get(&manifest.id)
                    .and_then(|result| result.clone().err());
            }
            let playable = manifest.status == DraftStatus::Ready && error.is_none();
            GraphDraft {
                id: manifest.id,
                label: manifest.label,
                parent: manifest.parent,
                created_unix_ms: manifest.created_unix_ms,
                status: manifest.status,
                frame_count: manifest.frames,
                playable,
                endpoint_kind: manifest.endpoint_kind,
                verification: manifest.verification,
                tape_sha256: manifest.tape_sha256,
                result_tape_sha256: manifest.result_tape_sha256,
                tape_bytes: manifest.tape_bytes,
                thumbnail: None,
                error,
            }
        })
        .collect())
}

pub(crate) fn validate_draft_structure(
    id: &str,
    manifests: &BTreeMap<String, DraftManifest>,
    timeline: &Timeline,
    repository_root: &Path,
    drafts_root: &Path,
    memo: &mut BTreeMap<String, Result<(), String>>,
    anchor_digests: &mut BTreeMap<(String, usize), Result<String, String>>,
) -> Result<(), String> {
    if let Some(result) = memo.get(id) {
        return result.clone();
    }
    memo.insert(
        id.to_owned(),
        Err("draft parent graph contains a cycle".into()),
    );
    let result = (|| {
        let manifest = manifests
            .get(id)
            .ok_or_else(|| "parent draft is missing".to_owned())?;
        if manifest.status != DraftStatus::Ready {
            return Err(format!("parent draft {id:?} is not ready"));
        }
        for (name, digest) in [
            ("parent", Some(manifest.parent_tape_sha256.as_str())),
            ("continuation", manifest.tape_sha256.as_deref()),
            ("result", manifest.result_tape_sha256.as_deref()),
        ] {
            if !digest.is_some_and(valid_sha256) {
                return Err(format!("draft {id:?} has invalid {name} tape digest"));
            }
        }
        if manifest.tape_bytes.is_none() || manifest.frames.is_none() {
            return Err(format!("draft {id:?} lacks finalized tape metadata"));
        }
        let draft_directory = fs::canonicalize(drafts_root.join(id))
            .map_err(|_| format!("draft {id:?} directory is missing"))?;
        let tape_path = fs::canonicalize(draft_directory.join(DRAFT_TAPE))
            .map_err(|_| format!("draft {id:?} continuation tape is missing"))?;
        if !draft_directory.starts_with(drafts_root) || !tape_path.starts_with(&draft_directory) {
            return Err(format!("draft {id:?} continuation tape escapes the store"));
        }
        let tape_bytes = fs::read(&tape_path)
            .map_err(|_| format!("draft {id:?} continuation tape is unreadable"))?;
        if manifest.tape_bytes != Some(tape_bytes.len() as u64)
            || manifest.tape_sha256.as_deref()
                != Some(format!("{:x}", Sha256::digest(&tape_bytes)).as_str())
        {
            return Err(format!("draft {id:?} continuation tape was tampered"));
        }
        let continuation = InputTape::decode(&tape_bytes)
            .map_err(|_| format!("draft {id:?} continuation tape is invalid"))?
            .tape;
        if continuation.frames.is_empty()
            || manifest.frames != Some(continuation.frames.len() as u64)
        {
            return Err(format!(
                "draft {id:?} continuation frame metadata is inconsistent"
            ));
        }
        match &manifest.parent {
            DraftParent::Milestone {
                id,
                program_sha256,
                definition_sha256,
                boundary_fingerprint,
            } => {
                let program = milestone_program_projection(timeline, repository_root)
                    .map_err(|error| error.to_string())?
                    .ok_or_else(|| "Boot parent has no authored milestone program".to_owned())?;
                let definition = program
                    .definitions
                    .iter()
                    .find(|definition| definition.name == *id)
                    .ok_or_else(|| "Boot parent milestone no longer exists".to_owned())?;
                if program.program_sha256 != *program_sha256
                    || definition.definition_sha256 != *definition_sha256
                    || !is_exact_boot_boundary_predicate(definition)
                    || !manifest.start_boundary_verified
                    || !boundary_fingerprint
                        .as_deref()
                        .is_some_and(native_fingerprint)
                    || manifest.parent_tape_sha256
                        != tape_digest(&InputTape::default()).map_err(|error| error.to_string())?
                {
                    return Err("Boot milestone parent proof is missing or stale".into());
                }
            }
            DraftParent::Segment {
                id: segment_id,
                terminal_milestone: _,
                boundary_fingerprint,
            } => {
                let segment = timeline
                    .segments
                    .get(segment_id)
                    .ok_or_else(|| "parent segment no longer exists".to_owned())?;
                if segment.end_fingerprint != *boundary_fingerprint
                    || !manifest.start_boundary_verified
                {
                    return Err("curated segment anchor is no longer exact".into());
                }
                let key = (segment_id.clone(), 0);
                let parent_digest = if let Some(result) = anchor_digests.get(&key) {
                    result.clone()?
                } else {
                    let result = materialize_segment_chain(timeline, repository_root, segment_id)
                        .map_err(|error| error.to_string())
                        .and_then(|materialized| {
                            tape_digest(&materialized.tape).map_err(|error| error.to_string())
                        });
                    anchor_digests.insert(key, result.clone());
                    result?
                };
                if manifest.parent_tape_sha256 != parent_digest {
                    return Err("curated parent tape digest changed".into());
                }
            }
            DraftParent::Draft {
                id: parent_id,
                parent_tape_sha256,
            } => {
                validate_draft_structure(
                    parent_id,
                    manifests,
                    timeline,
                    repository_root,
                    drafts_root,
                    memo,
                    anchor_digests,
                )?;
                let parent = &manifests[parent_id];
                if parent.result_tape_sha256.as_deref() != Some(parent_tape_sha256)
                    || manifest.parent_tape_sha256 != *parent_tape_sha256
                {
                    return Err("draft parent chain digest mismatch".into());
                }
            }
        }
        Ok(())
    })();
    memo.insert(id.to_owned(), result.clone());
    result
}

pub(crate) fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

pub(crate) fn valid_draft_id(id: &str) -> bool {
    id.starts_with("draft-")
        && id.len() <= 80
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

#[cfg(windows)]
pub(crate) fn process_is_alive(pid: u32) -> bool {
    type Handle = *mut std::ffi::c_void;
    unsafe extern "system" {
        fn OpenProcess(access: u32, inherit: i32, process_id: u32) -> Handle;
        fn GetExitCodeProcess(process: Handle, exit_code: *mut u32) -> i32;
        fn CloseHandle(object: Handle) -> i32;
    }
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    const STILL_ACTIVE: u32 = 259;
    // SAFETY: handles are checked for null, the output pointer is valid for the
    // duration of the call, and every opened handle is closed exactly once.
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return false;
        }
        let mut exit_code = 0;
        let ok = GetExitCodeProcess(process, &mut exit_code);
        let _ = CloseHandle(process);
        ok != 0 && exit_code == STILL_ACTIVE
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn process_is_alive(pid: u32) -> bool {
    Path::new("/proc").join(pid.to_string()).exists()
}

#[cfg(all(unix, not(target_os = "linux")))]
pub(crate) fn process_is_alive(pid: u32) -> bool {
    unsafe extern "C" {
        fn kill(pid: i32, signal: i32) -> i32;
    }
    let Ok(pid) = i32::try_from(pid) else {
        return false;
    };
    // SAFETY: signal zero does not deliver a signal; it only asks the kernel
    // whether this process exists and is visible to the caller.
    let visible = unsafe { kill(pid, 0) == 0 };
    visible || std::io::Error::last_os_error().kind() == std::io::ErrorKind::PermissionDenied
}

#[cfg(not(any(windows, unix)))]
pub(crate) fn process_is_alive(_pid: u32) -> bool {
    false
}

pub(crate) fn write_draft_manifest(
    directory: &Path,
    manifest: &DraftManifest,
    finalized: bool,
) -> Result<(), WorkbenchError> {
    let bytes = serde_json::to_vec(manifest)
        .map_err(|error| WorkbenchError::new(format!("cannot encode draft manifest: {error}")))?;
    let target = directory.join(if finalized {
        DRAFT_FINAL_MANIFEST
    } else {
        DRAFT_MANIFEST
    });
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temporary = directory.join(format!(".draft-{nonce}.tmp"));
    let mut file = fs::File::create(&temporary).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot write draft manifest {}: {error}",
            directory.display()
        ))
    })?;
    file.write_all(&bytes)
        .and_then(|()| file.sync_all())
        .map_err(|error| {
            WorkbenchError::new(format!("cannot flush {}: {error}", temporary.display()))
        })?;
    match fs::rename(&temporary, &target) {
        Ok(()) => Ok(()),
        Err(_) if target.is_file() => {
            let _ = fs::remove_file(&temporary);
            Ok(())
        }
        Err(error) => Err(WorkbenchError::new(format!(
            "cannot atomically install {}: {error}",
            target.display()
        ))),
    }
}

pub(crate) fn write_draft_launch(
    directory: &Path,
    launch: &DraftLaunch,
) -> Result<(), WorkbenchError> {
    let bytes = serde_json::to_vec(launch)
        .map_err(|error| WorkbenchError::new(format!("cannot encode draft launch: {error}")))?;
    let target = directory.join(DRAFT_LAUNCH);
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temporary = directory.join(format!(".launch-{nonce}.tmp"));
    let mut file = fs::File::create(&temporary)
        .map_err(|error| WorkbenchError::new(format!("cannot create launch state: {error}")))?;
    file.write_all(&bytes)
        .and_then(|()| file.sync_all())
        .map_err(|error| WorkbenchError::new(format!("cannot flush launch state: {error}")))?;
    fs::rename(&temporary, &target)
        .map_err(|error| WorkbenchError::new(format!("cannot install launch state: {error}")))
}

pub(crate) fn random_session_token() -> Result<String, WorkbenchError> {
    let mut bytes = [0_u8; 16];
    fill_random(&mut bytes)?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

#[cfg(windows)]
pub(crate) fn fill_random(output: &mut [u8]) -> Result<(), WorkbenchError> {
    #[link(name = "bcrypt")]
    unsafe extern "system" {
        fn BCryptGenRandom(
            algorithm: *mut std::ffi::c_void,
            buffer: *mut u8,
            length: u32,
            flags: u32,
        ) -> i32;
    }
    const BCRYPT_USE_SYSTEM_PREFERRED_RNG: u32 = 2;
    // SAFETY: `output` is a live writable slice and its length fits in u32.
    let status = unsafe {
        BCryptGenRandom(
            std::ptr::null_mut(),
            output.as_mut_ptr(),
            output.len() as u32,
            BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        )
    };
    if status == 0 {
        Ok(())
    } else {
        Err(WorkbenchError::new(format!(
            "system random generator failed with status {status}"
        )))
    }
}

#[cfg(not(windows))]
pub(crate) fn fill_random(output: &mut [u8]) -> Result<(), WorkbenchError> {
    fs::File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(output))
        .map_err(|error| WorkbenchError::new(format!("system random generator failed: {error}")))
}

pub(crate) fn tape_digest(tape: &InputTape) -> Result<String, WorkbenchError> {
    let encoded = tape
        .encode()
        .map_err(|error| WorkbenchError::new(error.to_string()))?;
    Ok(format!("{:x}", Sha256::digest(encoded)))
}

pub(crate) fn read_draft_tape(directory: &Path) -> Result<(Vec<u8>, InputTape), WorkbenchError> {
    let directory = fs::canonicalize(directory)
        .map_err(|error| WorkbenchError::new(format!("cannot resolve draft directory: {error}")))?;
    let path = fs::canonicalize(directory.join(DRAFT_TAPE))
        .map_err(|error| WorkbenchError::new(format!("cannot resolve draft tape: {error}")))?;
    if !path.starts_with(&directory) || !path.is_file() {
        return Err(WorkbenchError::new(
            "draft tape escapes its draft directory",
        ));
    }
    let bytes = fs::read(&path).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot read draft tape {}: {error}",
            path.display()
        ))
    })?;
    let decoded = InputTape::decode(&bytes).map_err(|error| {
        WorkbenchError::new(format!("invalid draft tape {}: {error}", path.display()))
    })?;
    Ok((bytes, decoded.tape))
}
