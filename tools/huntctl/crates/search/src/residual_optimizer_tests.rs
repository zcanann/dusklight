use super::*;
use dusklight_automation_contracts::tape::InputFrame;

fn parent(frame_count: usize) -> (InputTape, Vec<u8>) {
    let tape = InputTape {
        frames: (0..frame_count)
            .map(|_| InputFrame {
                owned_ports: 1,
                ..InputFrame::default()
            })
            .collect(),
        ..InputTape::default()
    };
    let bytes = tape.encode().unwrap();
    (tape, bytes)
}

fn space() -> ResidualSearchSpace {
    ResidualSearchSpace {
        schema: RESIDUAL_SEARCH_SPACE_SCHEMA_V1.into(),
        start_frame: 0,
        end_frame_exclusive: 96,
        candidate_slots: 4,
        ports: vec![0],
        analog_channels: vec![
            AnalogChannel::MainX,
            AnalogChannel::MainY,
            AnalogChannel::CameraX,
            AnalogChannel::CameraY,
        ],
        analog_delta_values: vec![-64, -16, -4, 4, 16, 64],
        button_masks: vec![0x0010, 0x0020, 0x0040, 0x0100, 0x0200, 0x0400],
        duration_values: vec![1, 2, 4, 8, 16, 32],
    }
}

#[test]
fn random_sampler_is_seeded_independent_and_compiles_unique_raw_tapes() {
    let (parent, bytes) = parent(96);
    let mut first = ResidualRandomSampler::new(space(), &bytes, 104_729).unwrap();
    let mut second = ResidualRandomSampler::new(space(), &bytes, 104_729).unwrap();
    let left = first.sample(&parent, &bytes, 64).unwrap();
    let right = second.sample(&parent, &bytes, 64).unwrap();
    assert_eq!(
        left.proposals
            .iter()
            .map(|proposal| proposal.compiled.report.realized_tape_sha256)
            .collect::<Vec<_>>(),
        right
            .proposals
            .iter()
            .map(|proposal| proposal.compiled.report.realized_tape_sha256)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        left.proposals
            .iter()
            .map(|proposal| proposal.compiled.report.realized_tape_sha256)
            .collect::<BTreeSet<_>>()
            .len(),
        64
    );
    assert!(left.proposals.iter().any(|proposal| {
        proposal.candidate.analog.len() + proposal.candidate.buttons.len() > 1
    }));
    assert!(left.proposals.iter().all(|proposal| {
        proposal.compiled.report.realized_tape_authoritative
            && proposal.compiled.report.parent_tape_sha256 == sha256(&bytes)
    }));
    let snapshot = first.snapshot().unwrap();
    assert_eq!(snapshot.rejected_invalid_genomes, left.rejected_invalid);
    assert_eq!(
        snapshot.rejected_duplicate_tapes,
        left.rejected_duplicate_tape
    );
    assert_eq!(
        snapshot.attempted_genomes,
        snapshot.produced_candidates
            + snapshot.rejected_invalid_genomes
            + snapshot.rejected_duplicate_tapes
    );
}

#[test]
fn random_snapshot_resumes_without_repeating_or_skipping() {
    let (parent, bytes) = parent(96);
    let mut uninterrupted = ResidualRandomSampler::new(space(), &bytes, 17).unwrap();
    let all = uninterrupted.sample(&parent, &bytes, 24).unwrap();

    let mut interrupted = ResidualRandomSampler::new(space(), &bytes, 17).unwrap();
    let prefix = interrupted.sample(&parent, &bytes, 9).unwrap();
    let snapshot = interrupted.snapshot().unwrap();
    let mut resumed = ResidualRandomSampler::restore(space(), &bytes, snapshot).unwrap();
    let suffix = resumed.sample(&parent, &bytes, 15).unwrap();
    let joined = prefix
        .proposals
        .iter()
        .chain(&suffix.proposals)
        .map(|proposal| proposal.compiled.report.realized_tape_sha256)
        .collect::<Vec<_>>();
    assert_eq!(
        joined,
        all.proposals
            .iter()
            .map(|proposal| proposal.compiled.report.realized_tape_sha256)
            .collect::<Vec<_>>()
    );
}

#[test]
fn categorical_cem_updates_only_from_exact_rank_and_resumes_byte_exactly() {
    let (parent, bytes) = parent(96);
    let config = ResidualCemConfig {
        population: 12,
        elites: 3,
        smoothing_millionths: 250_000,
        seed: 31,
    };
    let mut optimizer = ResidualCemOptimizer::new(space(), &bytes, config).unwrap();
    let first = optimizer.ask(&parent, &bytes).unwrap();
    assert_eq!(first.proposals.len(), 12);
    assert!(optimizer.ask(&parent, &bytes).is_err());
    let mut ranking = first
        .proposals
        .iter()
        .map(|proposal| proposal.candidate.content_sha256)
        .collect::<Vec<_>>();
    ranking.sort();
    let before = optimizer.snapshot().unwrap();
    assert_eq!(before.rejected_invalid_genomes, first.rejected_invalid);
    assert_eq!(
        before.rejected_duplicate_tapes,
        first.rejected_duplicate_tape
    );
    assert!(optimizer.tell(&ranking[..11]).is_err());
    optimizer.tell(&ranking).unwrap();
    let updated = optimizer.snapshot().unwrap();
    assert_eq!(updated.generation, 1);
    assert_ne!(updated.distributions, before.distributions);

    let restored = ResidualCemOptimizer::restore(space(), config, &bytes, updated.clone()).unwrap();
    assert_eq!(restored.snapshot().unwrap(), updated);
}

#[test]
fn random_and_cem_share_the_same_genome_renderer_and_complete_basis_catalog() {
    let (parent, bytes) = parent(96);
    let search_space = space();
    let genome = ResidualGenome {
        genes: vec![
            ResidualGene {
                enabled: true,
                kind: ResidualGeneKind::Analog,
                port_index: 0,
                channel_index: 0,
                basis_index: 7,
                start_index: 90,
                duration_index: 3,
                delta_indices: [0, 1, 4, 5],
                button_index: 0,
                button_mode: ResidualGeneButtonMode::Press,
            },
            ResidualGene {
                enabled: true,
                kind: ResidualGeneKind::Button,
                port_index: 0,
                channel_index: 3,
                basis_index: 0,
                start_index: 95,
                duration_index: 2,
                delta_indices: [0; 4],
                button_index: 3,
                button_mode: ResidualGeneButtonMode::Press,
            },
            ResidualGene {
                enabled: false,
                kind: ResidualGeneKind::Analog,
                port_index: 0,
                channel_index: 0,
                basis_index: 0,
                start_index: 0,
                duration_index: 0,
                delta_indices: [0; 4],
                button_index: 0,
                button_mode: ResidualGeneButtonMode::Release,
            },
            ResidualGene {
                enabled: false,
                kind: ResidualGeneKind::Button,
                port_index: 0,
                channel_index: 0,
                basis_index: 0,
                start_index: 0,
                duration_index: 0,
                delta_indices: [0; 4],
                button_index: 0,
                button_mode: ResidualGeneButtonMode::Release,
            },
        ],
    };
    let candidate = genome.candidate(&bytes, &search_space).unwrap();
    assert!(matches!(
        candidate.analog[0].basis,
        TemporalBasis::CubicControlCurve { .. }
    ));
    assert_eq!(candidate.buttons[0].duration_frames, 4);
    assert!(compile_genome(&parent, &bytes, &search_space, &genome).is_ok());
    assert_eq!(BASIS_COUNT, 8);
}

#[test]
fn checked_ordon_canary_surface_can_express_the_exact_q125_repair() {
    let repository_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../..");
    let segment_root = repository_root.join("routes/Glitch Exhibition/intro/segments");
    let degraded_bytes =
        std::fs::read(segment_root.join("to_ordon_spring_degraded_q131.tape")).unwrap();
    let q125_bytes = std::fs::read(segment_root.join("to_ordon_spring_q125.tape")).unwrap();
    let degraded = InputTape::decode(&degraded_bytes).unwrap().tape;
    let search_space = ResidualSearchSpace {
        schema: RESIDUAL_SEARCH_SPACE_SCHEMA_V1.into(),
        start_frame: 0,
        end_frame_exclusive: 126,
        candidate_slots: 4,
        ports: vec![0],
        analog_channels: vec![
            AnalogChannel::MainX,
            AnalogChannel::MainY,
            AnalogChannel::CameraX,
            AnalogChannel::CameraY,
        ],
        analog_delta_values: vec![-64, -32, -16, -8, -4, 4, 8, 16, 32, 64],
        button_masks: vec![1, 2, 4, 8, 16, 32, 64, 256, 512, 1024, 2048, 4096],
        duration_values: vec![1, 2, 4, 8, 16, 32],
    };
    search_space.validate().unwrap();
    assert!(search_space.ports.contains(&0));
    assert!(search_space.button_masks.contains(&0x0100));
    assert!(search_space.duration_values.contains(&1));
    assert!((search_space.start_frame..search_space.end_frame_exclusive).contains(&100));

    // This witness proves only that the sealed residual language can express
    // the known repair. It is intentionally never inserted into CEM's
    // population, replay, rank, or proposal distribution.
    let witness = ResidualCandidate::seal(
        &degraded_bytes,
        Vec::new(),
        vec![ButtonResidual {
            port: 0,
            buttons: 0x0100,
            start_frame: 100,
            duration_frames: 1,
            mode: ButtonResidualMode::Press,
        }],
    )
    .unwrap();
    let compiled = compile_residual_candidate(&degraded, &degraded_bytes, &witness).unwrap();

    assert_eq!(compiled.bytes, q125_bytes);
    assert_eq!(witness.buttons.len(), 1);
    assert_eq!(witness.buttons[0].start_frame, 100);
}

#[test]
fn detached_spaces_genomes_and_snapshots_fail_closed() {
    let (parent, bytes) = parent(96);
    let mut invalid = space();
    invalid.analog_delta_values.push(0);
    assert!(invalid.validate().is_err());

    let mut sampler = ResidualRandomSampler::new(space(), &bytes, 1).unwrap();
    sampler.sample(&parent, &bytes, 2).unwrap();
    let mut snapshot = sampler.snapshot().unwrap();
    snapshot.search_space_sha256 = Digest([9; 32]);
    snapshot.content_sha256 = snapshot.compute_identity().unwrap();
    assert!(ResidualRandomSampler::restore(space(), &bytes, snapshot).is_err());

    let mut short_parent = parent.clone();
    short_parent.frames.truncate(40);
    assert!(space().validate_parent(&short_parent).is_err());

    let config = ResidualCemConfig {
        population: 4,
        elites: 1,
        smoothing_millionths: 250_000,
        seed: 9,
    };
    let mut cem = ResidualCemOptimizer::new(space(), &bytes, config).unwrap();
    cem.ask(&parent, &bytes).unwrap();
    let cem_snapshot = cem.snapshot().unwrap();
    let different_config = ResidualCemConfig {
        smoothing_millionths: 500_000,
        ..config
    };
    assert!(
        ResidualCemOptimizer::restore(space(), different_config, &bytes, cem_snapshot.clone())
            .is_err()
    );
    let mut detached_pending = cem_snapshot;
    detached_pending.pending[0].candidate_sha256 = Digest([8; 32]);
    detached_pending.content_sha256 = detached_pending.compute_identity().unwrap();
    assert!(ResidualCemOptimizer::restore(space(), config, &bytes, detached_pending).is_err());
}

fn sha256(bytes: &[u8]) -> Digest {
    Digest(Sha256::digest(bytes).into())
}
