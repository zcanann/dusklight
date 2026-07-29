use super::*;

pub(super) fn compile_cutscene(args: &[String]) -> Result<(), Box<dyn Error>> {
    let program_path = required_path(args, "--program")?;
    let output = required_path(args, "--output")?;
    let program = CutsceneProgram::decode_canonical(&fs::read(program_path)?)?;
    let artifact = program.compile_artifact()?;
    let bytes = artifact.canonical_bytes()?;
    write_file(&output, &bytes)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": artifact.schema,
            "program_sha256": artifact.program_sha256,
            "transitions": artifact.transitions.len(),
            "output": output,
        }))?
    );
    Ok(())
}

pub(super) fn compile_return_place_mechanics(args: &[String]) -> Result<(), Box<dyn Error>> {
    let content_path = required_path(args, "--content-identity")?;
    let runtime_path = required_path(args, "--runtime-configuration")?;
    let output = required_path(args, "--output")?;
    let content = ContentIdentity::decode_canonical(&fs::read(content_path)?)?;
    let runtime = RuntimeConfiguration::decode_canonical(&fs::read(runtime_path)?)?;
    let mechanics = gz2e01_tower_return_place_mechanics(&content, &runtime)?;
    write_file(&output, &mechanics.canonical_bytes()?)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": mechanics.schema,
            "output": output,
            "sha256": mechanics.digest()?,
            "writers": mechanics.writers.len(),
            "gates": mechanics.gates.len(),
            "readers": mechanics.readers.len(),
            "transitions": mechanics.transitions.len(),
        }))?
    );
    Ok(())
}

pub(super) fn compile_title_boundary_mechanics(args: &[String]) -> Result<(), Box<dyn Error>> {
    let content_path = required_path(args, "--content-identity")?;
    let runtime_path = required_path(args, "--runtime-configuration")?;
    let output = required_path(args, "--output")?;
    let content = ContentIdentity::decode_canonical(&fs::read(content_path)?)?;
    let runtime = RuntimeConfiguration::decode_canonical(&fs::read(runtime_path)?)?;
    let mechanics = gz2e01_reset_to_opening_mechanics(&content, &runtime)?;
    write_file(&output, &mechanics.canonical_bytes()?)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": mechanics.schema,
            "output": output,
            "sha256": mechanics.digest()?,
            "transitions": mechanics.transitions.len(),
        }))?
    );
    Ok(())
}

pub(super) fn construct_message_flows(args: &[String]) -> Result<(), Box<dyn Error>> {
    let bundle_path = required_path(args, "--bundle")?;
    let runtime_path = required_path(args, "--runtime-configuration")?;
    let profile_path = required_path(args, "--profile")?;
    let output = required_path(args, "--output")?;
    let bundle = dusklight_route_planner::orig_discovery::ExtractedOrigBundle::decode_canonical(
        &fs::read(bundle_path)?,
    )?;
    let runtime = RuntimeConfiguration::decode_canonical(&fs::read(runtime_path)?)?;
    let profile = MessageFlowImportProfile::decode_canonical(&fs::read(profile_path)?)?;
    let set = MessageFlowProgramSet::build(&bundle, &runtime, &profile)?;
    write_file(&output, &set.canonical_bytes()?)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": set.schema,
            "output": output,
            "sha256": set.digest()?,
            "profile_sha256": set.profile_sha256,
            "bundle_sha256": set.bundle_sha256,
            "locale_bundle": set.locale_bundle,
            "programs": set.programs.len(),
        }))?
    );
    Ok(())
}

pub(super) fn construct_world_inventories(args: &[String]) -> Result<(), Box<dyn Error>> {
    let bundle_path = required_path(args, "--bundle")?;
    let output = required_path(args, "--output")?;
    let bundle = dusklight_route_planner::orig_discovery::ExtractedOrigBundle::decode_canonical(
        &fs::read(bundle_path)?,
    )?;
    let inventories = ExtractedOrigWorldInventories::build(&bundle)?;
    write_file(&output, &inventories.canonical_bytes()?)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": inventories.schema,
            "output": output,
            "sha256": inventories.digest()?,
            "source_bundle_sha256": inventories.source_bundle_sha256,
            "stages": inventories.inventories.len(),
            "sources": inventories.inventories.iter().map(|inventory| inventory.sources.len()).sum::<usize>(),
            "chunks": inventories.inventories.iter().map(|inventory| inventory.chunks.len()).sum::<usize>(),
            "room_transforms": inventories.stage_metadata.iter().map(|metadata| metadata.room_transforms.len()).sum::<usize>(),
            "file_lists": inventories.stage_metadata.iter().map(|metadata| metadata.file_lists.len()).sum::<usize>(),
            "room_reads": inventories.stage_metadata.iter().map(|metadata| metadata.room_reads.len()).sum::<usize>(),
            "cameras": inventories.stage_metadata.iter().map(|metadata| metadata.cameras.len()).sum::<usize>(),
            "camera_arrows": inventories.stage_metadata.iter().map(|metadata| metadata.camera_arrows.len()).sum::<usize>(),
            "paths": inventories.stage_metadata.iter().map(|metadata| metadata.paths.len()).sum::<usize>(),
            "path_points": inventories.stage_metadata.iter().map(|metadata| metadata.path_points.len()).sum::<usize>(),
            "placements": inventories.inventories.iter().map(|inventory| inventory.placements.len()).sum::<usize>(),
            "player_spawns": inventories.inventories.iter().map(|inventory| inventory.player_spawns.len()).sum::<usize>(),
            "scene_transitions": inventories.inventories.iter().map(|inventory| inventory.exits.len()).sum::<usize>(),
            "collision_coverage": inventories.coverage.collision,
        }))?
    );
    Ok(())
}

pub(super) fn compile_message_flows(args: &[String]) -> Result<(), Box<dyn Error>> {
    let bundle_path = required_path(args, "--bundle")?;
    let runtime_path = required_path(args, "--runtime-configuration")?;
    let profile_path = required_path(args, "--profile")?;
    let output = required_path(args, "--output")?;
    let manifest_output = required_path(args, "--manifest")?;
    let bundle = dusklight_route_planner::orig_discovery::ExtractedOrigBundle::decode_canonical(
        &fs::read(bundle_path)?,
    )?;
    let runtime = RuntimeConfiguration::decode_canonical(&fs::read(runtime_path)?)?;
    let profile = MessageFlowImportProfile::decode_canonical(&fs::read(profile_path)?)?;
    let overlays = match option(args, "--overlays") {
        Some(path) => Some(MessageFlowResourceOverlaySet::decode_canonical(&fs::read(
            path,
        )?)?),
        None => None,
    };
    let set = CompiledMessageFlowSet::build(&bundle, &runtime, &profile, overlays.as_ref())?;
    let bytes = set.canonical_bytes()?;
    let mut sources = vec![FactPackSource {
        kind: SourceArtifactKind::SourceAudit,
        id: "message-flow/import-profile".into(),
        sha256: profile.digest()?,
    }];
    if let Some(overlays) = &overlays {
        sources.push(FactPackSource {
            kind: SourceArtifactKind::SourceAudit,
            id: "message-flow/resource-overlays".into(),
            sha256: overlays.digest()?,
        });
    }
    sources.extend(set.resources.iter().map(|resource| FactPackSource {
        kind: SourceArtifactKind::MessageArchive,
        id: format!(
            "message-flow/{}/group-{:03}",
            set.locale_bundle.to_ascii_lowercase(),
            resource.message_group
        ),
        sha256: resource.archive_sha256,
    }));
    let has_storage_bindings = profile.bindings.temporary_flags.is_some()
        || profile.bindings.persistent_flags.is_some()
        || profile.bindings.rupees.is_some()
        || profile.bindings.life.is_some()
        || !profile.bindings.item_ownership.is_empty()
        || !profile.bindings.switch_stores.is_empty();
    let storage_binding_coverage = if has_storage_bindings {
        FactPackCoverage {
            domain: CoverageDomain::StorageBindings,
            scope: "message-flow".into(),
            status: CoverageStatus::Partial,
            detail: "The exact import profile supplies one or more audited storage bindings; additional handler-owned stores remain open.".into(),
        }
    } else {
        FactPackCoverage {
            domain: CoverageDomain::StorageBindings,
            scope: "message-flow".into(),
            status: CoverageStatus::Unavailable,
            detail: "The structural import profile supplies no temporary, persistent, rupee, life, item-ownership, or switch-store bindings; every state-backed handler remains an explicit unknown requirement.".into(),
        }
    };
    let hard_guard_detail = if has_storage_bindings {
        "Branch predicates with audited bindings are executable; actor entry, interaction, and unsupported event guards remain separate."
    } else {
        "Encoded branch outcomes are retained, but this structural profile authorizes no state-backed predicate; actor entry, interaction, and event guards remain separate."
    };
    let manifest = FactPackManifest::build(
        format!(
            "message-flow.{}.{}",
            set.locale_bundle.to_ascii_lowercase(),
            &set.digest()?.to_string()[..24]
        ),
        bundle.content.clone(),
        ExtractorIdentity {
            name: "route-planner-message-flow".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            executable_sha256: Digest(Sha256::digest(fs::read(env::current_exe()?)?).into()),
            schema_sha256: Digest(Sha256::digest(COMPILED_MESSAGE_FLOW_SET_SCHEMA).into()),
        },
        sources,
        vec![
            FactPackCoverage {
                domain: CoverageDomain::MessageFlows,
                scope: "message-flow".into(),
                status: CoverageStatus::Partial,
                detail: "Every selected FLW1/FLI1 node is retained; known generic handlers compile and unsupported handlers remain explicit unknown requirements.".into(),
            },
            storage_binding_coverage,
            FactPackCoverage {
                domain: CoverageDomain::HardGuards,
                scope: "message-flow".into(),
                status: CoverageStatus::Partial,
                detail: hard_guard_detail.into(),
            },
            FactPackCoverage {
                domain: CoverageDomain::PhysicalFeasibility,
                scope: "message-flow".into(),
                status: CoverageStatus::Unavailable,
                detail: "Message resources do not establish actor reachability, trigger geometry, interruption timing, or player control.".into(),
            },
        ],
        COMPILED_MESSAGE_FLOW_SET_SCHEMA,
        set.digest()?,
    )?;
    manifest.verify_payload(&bytes)?;
    write_file(&output, &bytes)?;
    write_file(&manifest_output, &manifest.canonical_bytes()?)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": set.schema,
            "output": output,
            "manifest": manifest_output,
            "manifest_sha256": manifest.digest()?,
            "sha256": set.digest()?,
            "program_set_sha256": set.program_set_sha256,
            "overlay_set_sha256": set.overlay_set_sha256,
            "locale_bundle": set.locale_bundle,
            "resources": set.resources.len(),
            "aliases": set.facts.aliases.len(),
            "transitions": set.mechanics.transitions.len(),
            "readers": set.mechanics.readers.len(),
        }))?
    );
    Ok(())
}

pub(super) fn compile_message_entries(args: &[String]) -> Result<(), Box<dyn Error>> {
    let bundle_path = required_path(args, "--bundle")?;
    let message_flow_set_path = required_path(args, "--message-flow-set")?;
    let contracts_path = required_path(args, "--contracts")?;
    let output = required_path(args, "--output")?;
    let manifest_output = required_path(args, "--manifest")?;
    let bundle = dusklight_route_planner::orig_discovery::ExtractedOrigBundle::decode_canonical(
        &fs::read(bundle_path)?,
    )?;
    let message_flow_set =
        CompiledMessageFlowSet::decode_canonical(&fs::read(message_flow_set_path)?)?;
    let contracts = MessageFlowEntryContractSet::decode_canonical(&fs::read(contracts_path)?)?;
    let artifact = contracts.compile(&bundle, &message_flow_set)?;
    let bytes = artifact.canonical_bytes()?;

    let mut sources = vec![
        FactPackSource {
            kind: SourceArtifactKind::SourceAudit,
            id: "message-entry/contracts".into(),
            sha256: contracts.digest()?,
        },
        FactPackSource {
            kind: SourceArtifactKind::SourceAudit,
            id: "message-entry/compiled-message-flow-set".into(),
            sha256: message_flow_set.digest()?,
        },
    ];
    let mut stage_paths = Vec::new();
    for entry in &contracts.entries {
        stage_paths.push(entry.stage_archive_path.as_str());
        if let Some(placement) = &entry.speaker.placement {
            stage_paths.push(placement.archive_path.as_str());
        }
    }
    stage_paths.sort_unstable();
    stage_paths.dedup();
    for (index, path) in stage_paths.into_iter().enumerate() {
        let stage = bundle
            .stages
            .iter()
            .find(|stage| stage.relative_path == path)
            .ok_or("validated entry stage disappeared from the extracted bundle")?;
        sources.push(FactPackSource {
            kind: SourceArtifactKind::StageArchive,
            id: format!("message-entry/stage-{index:05}"),
            sha256: stage.archive_sha256,
        });
    }
    let unresolved_entries = contracts
        .entries
        .iter()
        .filter(|entry| !entry.unknown_requirements.is_empty())
        .count();
    let manifest = FactPackManifest::build(
        format!(
            "message-entry.{}",
            &artifact.digest()?.to_string()[..24]
        ),
        bundle.content.clone(),
        ExtractorIdentity {
            name: "route-planner-message-entry".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            executable_sha256: Digest(Sha256::digest(fs::read(env::current_exe()?)?).into()),
            schema_sha256: Digest(
                Sha256::digest(COMPILED_MESSAGE_FLOW_ENTRY_SET_SCHEMA).into(),
            ),
        },
        sources,
        vec![
            FactPackCoverage {
                domain: CoverageDomain::MessageFlows,
                scope: "message-entry".into(),
                status: CoverageStatus::Partial,
                detail: "Every authored entry is pinned to an exact compiled flow label; only audited actor and non-actor callers are present.".into(),
            },
            FactPackCoverage {
                domain: CoverageDomain::ActorPlacements,
                scope: "message-entry".into(),
                status: CoverageStatus::Partial,
                detail: "Actor-backed entries reproduce one raw placement record from the exact stage resource; caller reconstruction remains separately authored.".into(),
            },
            FactPackCoverage {
                domain: CoverageDomain::HardGuards,
                scope: "message-entry".into(),
                status: CoverageStatus::Partial,
                detail: "Stage, room, layer, and authored guards compile into entry transitions; unaudited conditions remain explicit unknown requirements.".into(),
            },
            FactPackCoverage {
                domain: CoverageDomain::PhysicalFeasibility,
                scope: "message-entry".into(),
                status: CoverageStatus::Partial,
                detail: "Authored interaction obligations are retained without inferring reachability from placement alone.".into(),
            },
        ],
        COMPILED_MESSAGE_FLOW_ENTRY_SET_SCHEMA,
        artifact.digest()?,
    )?;
    manifest.verify_payload(&bytes)?;
    write_file(&output, &bytes)?;
    write_file(&manifest_output, &manifest.canonical_bytes()?)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": artifact.schema,
            "output": output,
            "manifest": manifest_output,
            "manifest_sha256": manifest.digest()?,
            "sha256": artifact.digest()?,
            "contract_set_sha256": contracts.digest()?,
            "compiled_message_flow_set_sha256": contracts.compiled_message_flow_set_sha256,
            "entries": artifact.resolved_entries.len(),
            "transitions": artifact.mechanics.transitions.len(),
            "obligations": artifact.mechanics.obligations.len(),
            "entries_with_unknown_requirements": unresolved_entries,
        }))?
    );
    Ok(())
}

pub(super) fn identify_orig(args: &[String]) -> Result<(), Box<dyn Error>> {
    let orig = required_path(args, "--orig")?;
    let output = required_path(args, "--output")?;
    let registry = load_supported_build_registry(args)?;
    let requested_content_id = option(args, "--content-id");
    let requested_identity = requested_content_id
        .as_deref()
        .map(|id| {
            registry
                .identities
                .iter()
                .find(|identity| identity.id == id)
                .ok_or_else(|| format!("content ID {id} is absent from the registry"))
        })
        .transpose()?;
    let product_id = requested_identity.map(|identity| identity.fingerprint.product_id.as_str());
    let scan = scan_orig_tree(&orig, product_id)?;
    let identification = registry.identify(&scan, requested_content_id.as_deref())?;
    let bytes = identification.canonical_bytes()?;
    write_file(&output, &bytes)?;
    let (status, content_id) = match &identification.support {
        OrigSupportStatus::Supported { content } => ("supported", Some(content.id.as_str())),
        OrigSupportStatus::Unsupported { .. } => ("unsupported", None),
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": identification.schema,
            "output": output,
            "status": status,
            "content_id": content_id,
            "scan_sha256": identification.scan_sha256,
            "product_id": scan.fingerprint.product_id,
        }))?
    );
    Ok(())
}

pub(super) fn cache_fact_pack(args: &[String]) -> Result<(), Box<dyn Error>> {
    let cache = required_path(args, "--cache")?;
    let payload_path = required_path(args, "--payload")?;
    let manifest_path = required_path(args, "--manifest")?;
    let receipt_path = required_path(args, "--receipt")?;
    let manifest = FactPackManifest::decode_canonical(&fs::read(manifest_path)?)?;
    let payload = fs::read(payload_path)?;
    let receipt = store_fact_pack(&cache, &manifest, &payload)?;
    write_file(&receipt_path, &receipt.canonical_bytes()?)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": receipt.schema,
            "receipt": receipt_path,
            "manifest_sha256": receipt.manifest_sha256,
            "payload_sha256": receipt.payload_sha256,
            "reused": receipt.reused,
        }))?
    );
    Ok(())
}

pub(super) fn materialize_fact_pack(args: &[String]) -> Result<(), Box<dyn Error>> {
    let cache = required_path(args, "--cache")?;
    let digest = option(args, "--manifest-sha256")
        .ok_or("missing required --manifest-sha256 <digest>")?
        .parse::<Digest>()?;
    let payload_output = required_path(args, "--payload")?;
    let manifest_output = required_path(args, "--manifest")?;
    let cached = load_fact_pack(&cache, digest)?;
    write_file(&payload_output, &cached.payload_bytes)?;
    write_file(&manifest_output, &cached.manifest_bytes)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "manifest_sha256": digest,
            "payload_sha256": cached.manifest.payload_sha256,
            "payload": payload_output,
            "manifest": manifest_output,
        }))?
    );
    Ok(())
}

pub(super) fn list_archive_resources(args: &[String]) -> Result<(), Box<dyn Error>> {
    let archive_path = required_path(args, "--archive")?;
    let output = required_path(args, "--output")?;
    let archive = fs::read(&archive_path)?;
    let archive_sha256 = Digest(Sha256::digest(&archive).into());
    let resource_names = list_rarc_resource_names(&archive)?;
    let bytes = serde_json::to_vec_pretty(&json!({
        "schema": "dusklight.route-planner.rarc-resource-list/v1",
        "archive_sha256": archive_sha256,
        "resource_names": resource_names,
    }))?;
    write_file(&output, &bytes)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": "dusklight.route-planner.rarc-resource-list/v1",
            "output": output,
            "archive_sha256": archive_sha256,
            "resources": resource_names.len(),
        }))?
    );
    Ok(())
}

pub(super) fn scan_orig(args: &[String]) -> Result<(), Box<dyn Error>> {
    let orig = required_path(args, "--orig")?;
    let output = required_path(args, "--output")?;
    let scan = scan_orig_tree(&orig, option(args, "--product-id").as_deref())?;
    let bytes = scan.canonical_bytes()?;
    write_file(&output, &bytes)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": scan.schema,
            "output": output,
            "sha256": scan.digest()?,
            "product_id": scan.fingerprint.product_id,
            "platform": scan.fingerprint.platform,
            "region": scan.fingerprint.region,
            "revision": scan.fingerprint.revision,
            "files": scan.files.len(),
            "extractable_archives": scan.extractable_archive_paths.len(),
        }))?
    );
    Ok(())
}

pub(super) fn extract_orig(args: &[String]) -> Result<(), Box<dyn Error>> {
    let orig = required_path(args, "--orig")?;
    let output = required_path(args, "--output")?;
    let manifest_output = required_path(args, "--manifest")?;
    let content = if let Some(content_path) = option(args, "--content-identity") {
        if option(args, "--registry").is_some() || option(args, "--content-id").is_some() {
            return Err(
                "--content-identity cannot be combined with --registry or --content-id".into(),
            );
        }
        ContentIdentity::decode_canonical(&fs::read(content_path)?)?
    } else {
        let registry = load_supported_build_registry(args)?;
        let requested_content_id = option(args, "--content-id");
        let requested_identity = requested_content_id
            .as_deref()
            .map(|id| {
                registry
                    .identities
                    .iter()
                    .find(|identity| identity.id == id)
                    .ok_or_else(|| format!("content ID {id} is absent from the registry"))
            })
            .transpose()?;
        let product_id =
            requested_identity.map(|identity| identity.fingerprint.product_id.as_str());
        let scan = scan_orig_tree(&orig, product_id)?;
        match registry
            .identify(&scan, requested_content_id.as_deref())?
            .support
        {
            OrigSupportStatus::Supported { content } => content,
            OrigSupportStatus::Unsupported { fingerprint } => {
                return Err(format!(
                    "unsupported orig fingerprint for {} revision {} (executable {}, game data {}, resources {})",
                    fingerprint.product_id,
                    fingerprint.revision,
                    fingerprint.executable_sha256,
                    fingerprint.game_data_sha256,
                    fingerprint.resource_manifest_sha256,
                )
                .into());
            }
        }
    };
    let bundle = extract_orig_bundle(&orig, &content)?;
    let bytes = bundle.canonical_bytes()?;
    let mut sources = vec![FactPackSource {
        kind: SourceArtifactKind::Executable,
        id: "orig/sys/main.dol".into(),
        sha256: bundle.input_scan.fingerprint.executable_sha256,
    }];
    sources.extend(bundle.stages.iter().map(|record| FactPackSource {
        kind: SourceArtifactKind::StageArchive,
        id: format!(
            "orig/stage/{}",
            Digest(Sha256::digest(record.relative_path.as_bytes()).into())
        ),
        sha256: record.archive_sha256,
    }));
    sources.extend(bundle.message_flows.iter().map(|record| FactPackSource {
        kind: SourceArtifactKind::MessageArchive,
        id: format!(
            "orig/message/{}",
            Digest(Sha256::digest(record.relative_path.as_bytes()).into())
        ),
        sha256: record.archive_sha256,
    }));
    sources.extend(bundle.ignored_archives.iter().map(|record| FactPackSource {
        kind: SourceArtifactKind::MessageArchive,
        id: format!(
            "orig/message/{}",
            Digest(Sha256::digest(record.relative_path.as_bytes()).into())
        ),
        sha256: record.archive_sha256,
    }));
    let manifest = FactPackManifest::build(
        format!("{}.orig-extraction", content.id),
        content,
        ExtractorIdentity {
            name: "route-planner-orig-extraction".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            executable_sha256: Digest(Sha256::digest(fs::read(env::current_exe()?)?).into()),
            schema_sha256: Digest(Sha256::digest(EXTRACTED_ORIG_BUNDLE_SCHEMA).into()),
        },
        sources,
        vec![
            FactPackCoverage {
                domain: CoverageDomain::Topology,
                scope: "orig".into(),
                status: CoverageStatus::Partial,
                detail: "Decoded DZS/DZR chunks include STAG, SCLS, MULT, FILI, RTBL, RCAM, RARO, RPAT, and RPPN metadata; unresolved chunk semantics and physical reachability remain explicit.".into(),
            },
            FactPackCoverage {
                domain: CoverageDomain::ActorPlacements,
                scope: "orig".into(),
                status: CoverageStatus::Partial,
                detail: "Recognized placement chunks retain parameters, transforms, layer, and raw bytes.".into(),
            },
            FactPackCoverage {
                domain: CoverageDomain::MessageFlows,
                scope: "orig".into(),
                status: CoverageStatus::Partial,
                detail: "FLW1/FLI1 graphs and known temporary, persistent, and switch accesses are decoded for discovered language bundles.".into(),
            },
            FactPackCoverage {
                domain: CoverageDomain::PhysicalFeasibility,
                scope: "orig".into(),
                status: CoverageStatus::Unavailable,
                detail: "Resource extraction does not infer collision reachability, interaction geometry, or timing witnesses.".into(),
            },
        ],
        EXTRACTED_ORIG_BUNDLE_SCHEMA,
        bundle.digest()?,
    )?;
    let manifest_bytes = manifest.canonical_bytes()?;
    write_file(&output, &bytes)?;
    write_file(&manifest_output, &manifest_bytes)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": bundle.schema,
            "output": output,
            "manifest": manifest_output,
            "manifest_sha256": manifest.digest()?,
            "sha256": bundle.digest()?,
            "product_id": bundle.content.fingerprint.product_id,
            "files": bundle.input_scan.files.len(),
            "stage_archives": bundle.stages.len(),
            "message_archives": bundle.message_flows.len(),
            "ignored_archives": bundle.ignored_archives.len(),
        }))?
    );
    Ok(())
}

pub(super) fn extract_stage_data(args: &[String]) -> Result<(), Box<dyn Error>> {
    let archive_path = required_path(args, "--archive")?;
    let resource_name = option(args, "--resource")
        .ok_or_else(|| "missing required --resource <stage.dzs|room.dzr>".to_owned())?;
    let output = required_path(args, "--output")?;
    let archive = fs::read(&archive_path)?;
    let resource = extract_unique_rarc_resource(&archive, &resource_name)?;
    let stage = parse_stage_data(&resource)?;
    let archive_sha256 = Digest(Sha256::digest(&archive).into());
    let resource_sha256 = Digest(Sha256::digest(&resource).into());
    let bytes = serde_json::to_vec_pretty(&json!({
        "schema": EXTRACTED_STAGE_DATA_SCHEMA,
        "archive": archive_path,
        "archive_sha256": archive_sha256,
        "resource": resource_name,
        "resource_sha256": resource_sha256,
        "stage": stage,
    }))?;
    write_file(&output, &bytes)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": EXTRACTED_STAGE_DATA_SCHEMA,
            "output": output,
            "archive_sha256": archive_sha256,
            "resource_sha256": resource_sha256,
            "chunks": stage.chunks.len(),
            "room_transforms": stage.room_transforms.len(),
            "file_lists": stage.file_lists.len(),
            "room_reads": stage.room_read_table.len(),
            "cameras": stage.cameras.len(),
            "camera_arrows": stage.camera_arrows.len(),
            "paths": stage.paths.len(),
            "path_points": stage.path_points.len(),
            "scene_transitions": stage.scene_transitions.len(),
            "map_events": stage.map_events.len(),
            "demo_archive_banks": stage.demo_archive_banks.len(),
            "actor_placements": stage.actor_placements.len(),
            "treasure_placements": stage.treasure_placements.len(),
            "player_spawns": stage.player_spawns.len(),
        }))?
    );
    Ok(())
}

pub(super) fn extract_function_evidence(args: &[String]) -> Result<(), Box<dyn Error>> {
    let dol_path = required_path(args, "--dol")?;
    let symbols_path = required_path(args, "--symbols")?;
    let symbol = option(args, "--symbol")
        .ok_or_else(|| "missing required --symbol <exact-name>".to_owned())?;
    let output = required_path(args, "--output")?;
    let evidence =
        extract_dol_function_evidence(&fs::read(&dol_path)?, &fs::read(&symbols_path)?, &symbol)?;
    write_file(&output, &evidence.canonical_bytes()?)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": evidence.schema,
            "output": output,
            "sha256": evidence.digest()?,
            "symbol": evidence.symbol,
            "virtual_address": evidence.virtual_address,
            "function_size": evidence.function_size,
            "file_offset": evidence.file_offset,
            "code_sha256": evidence.code_sha256,
            "shape": evidence.shape,
        }))?
    );
    Ok(())
}

pub(super) fn extract_binary_range_evidence(args: &[String]) -> Result<(), Box<dyn Error>> {
    let dol_path = required_path(args, "--dol")?;
    let virtual_address = required_u32(args, "--virtual-address")?;
    let byte_size = required_u32(args, "--size")?;
    let output = required_path(args, "--output")?;
    let evidence = extract_dol_range_evidence(&fs::read(&dol_path)?, virtual_address, byte_size)?;
    write_file(&output, &evidence.canonical_bytes()?)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": evidence.schema,
            "output": output,
            "sha256": evidence.digest()?,
            "virtual_address": evidence.virtual_address,
            "byte_size": evidence.byte_size,
            "section_kind": evidence.section_kind,
            "section_index": evidence.section_index,
            "file_offset": evidence.file_offset,
            "bytes_sha256": evidence.bytes_sha256,
        }))?
    );
    Ok(())
}

pub(super) fn extract_jstudio_stb(args: &[String]) -> Result<(), Box<dyn Error>> {
    let archive_path = required_path(args, "--archive")?;
    let resource_name = option(args, "--resource")
        .ok_or_else(|| "missing required --resource <file.stb>".to_owned())?;
    let output = required_path(args, "--output")?;
    let archive = fs::read(&archive_path)?;
    let resource = extract_unique_rarc_resource(&archive, &resource_name)?;
    let program = parse_jstudio_stb(
        Digest(Sha256::digest(&archive).into()),
        &resource_name,
        &resource,
    )?;
    write_file(&output, &program.canonical_bytes()?)?;
    let object_count = program
        .blocks
        .iter()
        .filter(|block| {
            matches!(
                block.body,
                dusklight_route_planner::jstudio_import::JstudioStbBlockBody::Object { .. }
            )
        })
        .count();
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": program.schema,
            "output": output,
            "sha256": program.digest()?,
            "archive_sha256": program.source.archive_sha256,
            "resource_sha256": program.source.resource_sha256,
            "blocks": program.blocks.len(),
            "objects": object_count,
            "coverage": program.coverage,
        }))?
    );
    Ok(())
}

pub(super) fn extract_demo_actor_program(args: &[String]) -> Result<(), Box<dyn Error>> {
    let archive_path = required_path(args, "--archive")?;
    let resource_name = option(args, "--resource")
        .ok_or_else(|| "missing required --resource <file.stb>".to_owned())?;
    let content_path = required_path(args, "--content-identity")?;
    let output = required_path(args, "--output")?;
    let archive = fs::read(&archive_path)?;
    let resource = extract_unique_rarc_resource(&archive, &resource_name)?;
    let content = ContentIdentity::decode_canonical(&fs::read(content_path)?)?;
    let program = extract_gz2e01_demo_actor_program(
        &content,
        Digest(Sha256::digest(&archive).into()),
        &resource_name,
        &resource,
    )?;
    write_file(&output, &program.canonical_bytes()?)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": program.schema,
            "output": output,
            "sha256": program.digest()?,
            "source_program_sha256": program.source_program_sha256,
            "source_resource_sha256": program.source_resource_sha256,
            "coverage": program.coverage,
        }))?
    );
    Ok(())
}

pub(super) fn resolve_jstudio_stb(args: &[String]) -> Result<(), Box<dyn Error>> {
    let archive_path = required_path(args, "--archive")?;
    let resource_name = option(args, "--resource")
        .ok_or_else(|| "missing required --resource <file.stb>".to_owned())?;
    let content_path = required_path(args, "--content-identity")?;
    let output = required_path(args, "--output")?;
    let archive = fs::read(&archive_path)?;
    let resource = extract_unique_rarc_resource(&archive, &resource_name)?;
    let content = ContentIdentity::decode_canonical(&fs::read(content_path)?)?;
    let profile = match option(args, "--profile") {
        Some(profile_path) => JstudioAdaptorProfile::decode_canonical(&fs::read(profile_path)?)?,
        None => bundled_gz2e01_adaptor_profile()?,
    };
    let program = resolve_jstudio_stb_semantics(
        &content,
        &profile,
        Digest(Sha256::digest(&archive).into()),
        &resource_name,
        &resource,
    )?;
    write_file(&output, &program.canonical_bytes()?)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": program.schema,
            "output": output,
            "sha256": program.digest()?,
            "source_program_sha256": program.source_program_sha256,
            "profile_sha256": program.profile_sha256,
            "coverage": program.coverage,
        }))?
    );
    Ok(())
}

pub(super) fn resolve_cutscene_package_command(args: &[String]) -> Result<(), Box<dyn Error>> {
    let content_path = required_path(args, "--content-identity")?;
    let topology_path = required_path(args, "--topology")?;
    let semantics_path = required_path(args, "--semantics")?;
    let output = required_path(args, "--output")?;
    let content = ContentIdentity::decode_canonical(&fs::read(content_path)?)?;
    let topology = CutsceneWrapperTopology::decode_canonical(&fs::read(topology_path)?)?;
    let semantics = JstudioSemanticProgram::decode_canonical(&fs::read(semantics_path)?)?;
    let profile = match option(args, "--profile") {
        Some(profile_path) => {
            CutscenePackageRuntimeProfile::decode_canonical(&fs::read(profile_path)?)?
        }
        None => bundled_gz2e01_cutscene_runtime_profile()?,
    };
    let package = resolve_cutscene_package(&content, &topology, &semantics, &profile)?;
    write_file(&output, &package.canonical_bytes()?)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": package.schema,
            "output": output,
            "sha256": package.digest()?,
            "event_name": package.event_name,
            "demo_archive_name": package.demo_archive_name,
            "stb_file": package.stb_file,
            "coverage": package.coverage,
        }))?
    );
    Ok(())
}

pub(super) fn resolve_cutscene_outer_command(args: &[String]) -> Result<(), Box<dyn Error>> {
    let content_path = required_path(args, "--content-identity")?;
    let runtime_path = required_path(args, "--runtime-configuration")?;
    let topology_path = required_path(args, "--topology")?;
    let package_path = required_path(args, "--package")?;
    let stage_resource_path = required_path(args, "--stage-resource-file")?;
    let event_list_resource_path = required_path(args, "--event-list-resource-file")?;
    let output = required_path(args, "--output")?;
    let content = ContentIdentity::decode_canonical(&fs::read(content_path)?)?;
    let runtime = RuntimeConfiguration::decode_canonical(&fs::read(runtime_path)?)?;
    let topology = CutsceneWrapperTopology::decode_canonical(&fs::read(topology_path)?)?;
    let package =
        dusklight_route_planner::cutscene_runtime::ResolvedCutscenePackage::decode_canonical(
            &fs::read(package_path)?,
        )?;
    let stage_resource = fs::read(stage_resource_path)?;
    let event_list_resource = fs::read(event_list_resource_path)?;
    let stage = parse_stage_data(&stage_resource)?;
    let event_list = parse_event_list(&event_list_resource)?;
    let profile = match option(args, "--profile") {
        Some(profile_path) => {
            CutsceneOuterRuntimeProfile::decode_canonical(&fs::read(profile_path)?)?
        }
        None => bundled_gz2e01_cutscene_outer_runtime_profile()?,
    };
    let resolved = resolve_cutscene_outer_event(
        &content,
        &runtime,
        &topology,
        &package,
        topology.source.stage_archive_sha256,
        &stage_resource,
        &event_list_resource,
        &stage,
        &event_list,
        &profile,
    )?;
    write_file(&output, &resolved.canonical_bytes()?)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": resolved.schema,
            "output": output,
            "sha256": resolved.digest()?,
            "event_name": resolved.event_name,
            "event_finish_flags": resolved.event_finish_flags,
            "skip_cut_enabled": resolved.skip_cut_enabled,
            "skip_cut_type": resolved.skip_cut_type,
            "transitions": resolved.transitions.len(),
            "coverage": resolved.coverage,
        }))?
    );
    Ok(())
}

pub(super) fn compile_cutscene_corruption_hypothesis_command(
    args: &[String],
) -> Result<(), Box<dyn Error>> {
    let content_path = required_path(args, "--content-identity")?;
    let runtime_path = required_path(args, "--runtime-configuration")?;
    let outer_path = required_path(args, "--outer-event")?;
    let output = required_path(args, "--output")?;
    let content = ContentIdentity::decode_canonical(&fs::read(content_path)?)?;
    let runtime = RuntimeConfiguration::decode_canonical(&fs::read(runtime_path)?)?;
    let outer =
        dusklight_route_planner::cutscene_outer::ResolvedCutsceneOuterEvent::decode_canonical(
            &fs::read(outer_path)?,
        )?;
    let profile = match option(args, "--outer-profile") {
        Some(profile_path) => {
            CutsceneOuterRuntimeProfile::decode_canonical(&fs::read(profile_path)?)?
        }
        None => bundled_gz2e01_cutscene_outer_runtime_profile()?,
    };
    let hypothesis = compile_actor_corruption_hypothesis(&content, &runtime, &outer, &profile)?;
    write_file(&output, &hypothesis.canonical_bytes()?)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": hypothesis.schema,
            "output": output,
            "sha256": hypothesis.digest()?,
            "id": hypothesis.id,
            "producer_transition": hypothesis.producer.id,
            "unknown_requirements": hypothesis.producer.activation.unknown_requirements.len(),
            "coverage": hypothesis.coverage,
        }))?
    );
    Ok(())
}

pub(super) fn extract_cutscene_wrapper(args: &[String]) -> Result<(), Box<dyn Error>> {
    let archive_path = required_path(args, "--archive")?;
    let stage_resource_name = option(args, "--stage-resource").unwrap_or_else(|| "room.dzr".into());
    let event_list_resource_name =
        option(args, "--event-list-resource").unwrap_or_else(|| "event_list.dat".into());
    let event_name = option(args, "--event-name")
        .ok_or_else(|| "missing required --event-name <name>".to_owned())?;
    let layer = option(args, "--layer")
        .ok_or_else(|| "missing required --layer <0..255>".to_owned())?
        .parse::<u8>()?;
    let output = required_path(args, "--output")?;
    let archive = fs::read(&archive_path)?;
    let stage_resource = extract_unique_rarc_resource(&archive, &stage_resource_name)?;
    let event_list_resource = extract_unique_rarc_resource(&archive, &event_list_resource_name)?;
    let stage = parse_stage_data(&stage_resource)?;
    let event_list = parse_event_list(&event_list_resource)?;
    let topology = CutsceneWrapperTopology::build(
        CutsceneWrapperSourceIdentity {
            stage_archive_sha256: Digest(Sha256::digest(&archive).into()),
            stage_resource_sha256: Digest(Sha256::digest(&stage_resource).into()),
            event_list_resource_sha256: Digest(Sha256::digest(&event_list_resource).into()),
        },
        &stage,
        &event_list,
        &event_name,
        layer,
    )?;
    write_file(&output, &topology.canonical_bytes()?)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": topology.schema,
            "output": output,
            "sha256": topology.digest()?,
            "event_name": topology.event_name,
            "demo_archive_name": topology.demo_archive_name,
            "package_stb_file": topology.package_stb_file,
            "normal_exit": topology.normal_exit,
            "skip_exit": topology.skip_exit,
            "coverage": topology.coverage,
        }))?
    );
    Ok(())
}

pub(super) fn extract_resource(args: &[String]) -> Result<(), Box<dyn Error>> {
    let archive_path = required_path(args, "--archive")?;
    let resource_name = option(args, "--resource")
        .ok_or_else(|| "missing required --resource <basename>".to_owned())?;
    let output = required_path(args, "--output")?;
    let archive = fs::read(&archive_path)?;
    let resource = extract_unique_rarc_resource(&archive, &resource_name)?;
    let archive_sha256 = Digest(Sha256::digest(&archive).into());
    let resource_sha256 = Digest(Sha256::digest(&resource).into());
    write_file(&output, &resource)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": "dusklight.route-planner.extracted-resource/v1",
            "output": output,
            "archive": archive_path,
            "archive_sha256": archive_sha256,
            "resource": resource_name,
            "resource_sha256": resource_sha256,
            "bytes": resource.len(),
        }))?
    );
    Ok(())
}

pub(super) fn extract_event_list(args: &[String]) -> Result<(), Box<dyn Error>> {
    let archive_path = required_path(args, "--archive")?;
    let resource_name = option(args, "--resource").unwrap_or_else(|| "event_list.dat".into());
    let output = required_path(args, "--output")?;
    let archive = fs::read(&archive_path)?;
    let resource = extract_unique_rarc_resource(&archive, &resource_name)?;
    let event_list = parse_event_list(&resource)?;
    let archive_sha256 = Digest(Sha256::digest(&archive).into());
    let resource_sha256 = Digest(Sha256::digest(&resource).into());
    let bytes = serde_json::to_vec_pretty(&json!({
        "schema": EXTRACTED_EVENT_LIST_SCHEMA,
        "archive": archive_path,
        "archive_sha256": archive_sha256,
        "resource": resource_name,
        "resource_sha256": resource_sha256,
        "event_list": event_list,
    }))?;
    write_file(&output, &bytes)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": EXTRACTED_EVENT_LIST_SCHEMA,
            "output": output,
            "archive_sha256": archive_sha256,
            "resource_sha256": resource_sha256,
            "events": event_list.events.len(),
            "staff": event_list.staff.len(),
            "cuts": event_list.cuts.len(),
            "data": event_list.data.len(),
        }))?
    );
    Ok(())
}

pub(super) fn extract_message_flow(args: &[String]) -> Result<(), Box<dyn Error>> {
    let archive_path = required_path(args, "--archive")?;
    let resource_name = option(args, "--resource")
        .ok_or_else(|| "missing required --resource <basename>".to_owned())?;
    let output = required_path(args, "--output")?;
    let archive = fs::read(&archive_path)?;
    let resource = extract_unique_rarc_resource(&archive, &resource_name)?;
    let flow = parse_message_flow(&resource)?;
    let archive_sha256 = Digest(Sha256::digest(&archive).into());
    let resource_sha256 = Digest(Sha256::digest(&resource).into());
    let bytes = serde_json::to_vec_pretty(&json!({
        "schema": "dusklight.route-planner.extracted-message-flow/v1",
        "archive": archive_path,
        "archive_sha256": archive_sha256,
        "resource": resource_name,
        "resource_sha256": resource_sha256,
        "flow": flow,
    }))?;
    write_file(&output, &bytes)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": "dusklight.route-planner.extracted-message-flow/v1",
            "output": output,
            "archive_sha256": archive_sha256,
            "resource_sha256": resource_sha256,
            "nodes": flow.node_count,
            "labels": flow.labels.len(),
        }))?
    );
    Ok(())
}
