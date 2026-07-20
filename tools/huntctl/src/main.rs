use huntctl::Digest;
use huntctl::client::{CONTROL_PROTOCOL_NAME, CONTROL_PROTOCOL_VERSION};
use huntctl::controller_program::ControllerProgram;
use huntctl::milestone_dsl;
use huntctl::tape::InputTape;
use huntctl::tape_program::PROGRAM_SCHEMA;
use serde_json::{Value, json};
use sha2::{Digest as ShaDigest, Sha256};
use std::env;
use std::error::Error;
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::time::Duration;

mod cli;

fn main() {
    suppress_windows_error_dialogs();
    if let Err(error) = run() {
        eprintln!("huntctl: {error}");
        std::process::exit(2);
    }
}

#[cfg(windows)]
fn suppress_windows_error_dialogs() {
    // Hunt workers are unattended and fan out native child processes. Preserve
    // their nonzero exit status and WER evidence, but never let a crashed child
    // block the farm on an interactive Windows error dialog. The process error
    // mode is inherited by children unless a launcher explicitly resets it.
    const SEM_FAILCRITICALERRORS: u32 = 0x0001;
    const SEM_NOGPFAULTERRORBOX: u32 = 0x0002;
    const SEM_NOOPENFILEERRORBOX: u32 = 0x8000;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn SetErrorMode(mode: u32) -> u32;
    }

    unsafe {
        SetErrorMode(SEM_FAILCRITICALERRORS | SEM_NOGPFAULTERRORBOX | SEM_NOOPENFILEERRORBOX);
    }
}

#[cfg(not(windows))]
fn suppress_windows_error_dialogs() {}

fn run() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = env::args().skip(1).collect();
    let Some(command) = args.first().map(String::as_str) else {
        return usage_error();
    };
    match command {
        "hello" => cli::worker::command_hello(&args[1..]),
        "ping" => cli::worker::command_ping(&args[1..]),
        "session" => cli::worker::command_session(&args[1..]),
        "pool" => cli::worker::command_pool(&args[1..]),
        "benchmark" => cli::benchmark::command_benchmark(&args[1..]),
        "campaign" => cli::harness::command_campaign(&args[1..]),
        "conformance" => cli::conformance::command_conformance(&args[1..]),
        "harness" => cli::harness::command_harness(&args[1..]),
        "identity" => cli::identity::command_identity(&args[1..]),
        "corpus" => cli::corpus::command_corpus(&args[1..]),
        "controller" => cli::controller::command_controller(&args[1..]),
        "milestone" => cli::milestone::command_milestone(&args[1..]),
        "fixture" => cli::fixture::command_fixture(&args[1..]),
        "tape" => cli::tape::command_tape(&args[1..]),
        "trace" => cli::trace::command_trace(&args[1..]),
        "timeline" => cli::timeline::command_timeline(&args[1..]),
        "search" => cli::search::command_search(&args[1..]),
        "learn" => cli::learning::command_learn(&args[1..]),
        "observe" => cli::observation::command_observe(&args[1..]),
        "oracle" => cli::oracle::command_oracle(&args[1..]),
        "world" => cli::world::command_world(&args[1..]),
        "run" | "replay" => cli::worker::command_not_ready(command, &args[1..]),
        "mock-worker" => mock_worker(&args[1..]),
        "mock-search-worker" => mock_search_worker(&args[1..]),
        "mock-record-worker" => mock_record_worker(&args[1..]),
        "help" | "--help" | "-h" => {
            print_usage();
            Ok(())
        }
        _ => usage_error(),
    }
}

fn option(args: &[String], name: &str) -> Option<String> {
    args.windows(2)
        .find(|pair| pair[0] == name)
        .map(|pair| pair[1].clone())
}

fn flag(args: &[String], name: &str) -> bool {
    args.iter().any(|argument| argument == name)
}

fn repeated_option(args: &[String], name: &str) -> Vec<String> {
    args.windows(2)
        .filter(|pair| pair[0] == name)
        .map(|pair| pair[1].clone())
        .collect()
}

fn required_path(args: &[String], name: &str) -> Result<PathBuf, Box<dyn Error>> {
    option(args, name)
        .map(PathBuf::from)
        .ok_or_else(|| format!("missing required {name} <path>").into())
}

fn usize_option(args: &[String], name: &str, default: usize) -> Result<usize, Box<dyn Error>> {
    Ok(option(args, name)
        .map(|value| value.parse())
        .transpose()?
        .unwrap_or(default))
}

fn u64_option(args: &[String], name: &str, default: u64) -> Result<u64, Box<dyn Error>> {
    Ok(option(args, name)
        .map(|value| value.parse())
        .transpose()?
        .unwrap_or(default))
}

fn u32_option(args: &[String], name: &str, default: u32) -> Result<u32, Box<dyn Error>> {
    Ok(option(args, name)
        .map(|value| value.parse())
        .transpose()?
        .unwrap_or(default))
}

fn timeout_option(args: &[String]) -> Result<Duration, Box<dyn Error>> {
    if let Some(milliseconds) = option(args, "--timeout-ms") {
        return Ok(Duration::from_millis(milliseconds.parse()?));
    }
    Ok(Duration::from_secs(
        option(args, "--timeout-seconds")
            .map(|value| value.parse())
            .transpose()?
            .unwrap_or(300),
    ))
}

fn usage_error<T>() -> Result<T, Box<dyn Error>> {
    print_usage();
    Err("invalid command line".into())
}

fn print_usage() {
    eprintln!("Trace typed facts:\n  huntctl trace facts INPUT.trace [--boundary-index N]\n");
    eprintln!("Engine sessions:\n  huntctl session audit --worker PATH [--worker-arg ARG]...\n");
    eprintln!(
        "Objective campaigns:\n  huntctl conformance --suite SUITE.json --executable DUSKLIGHT --game-data GAME.iso --output build/DIR [--repository-root DIR] [--fidelity headless|unpaced-headful|realtime-headful]\n  huntctl campaign --suite SUITE.json --case ID --output build/DIR --dry-run [--repository-root DIR] [--proposer scripted|random|structured|learned]...\n  huntctl campaign --suite SUITE.json --case ID --output build/DIR --run-request REQUEST.json --definition TOURNAMENT.json [--repository-root DIR] [--workers N]\n"
    );
    eprintln!(
        "Usage:\n  huntctl hello --worker PATH [--worker-arg ARG]...\n  huntctl ping --worker PATH [--worker-arg ARG]...\n  huntctl pool health --worker PATH [--worker-arg ARG]... [--workers N] [--checks N] [--allow-mixed-builds]\n  huntctl controller compile SOURCE.duskctl OUTPUT.dctl\n  huntctl controller inspect INPUT.dctl\n  huntctl controller flatten INPUT.dctl OUTPUT.tape\n  huntctl controller derive-waypoint-policy INPUT.trace OUTPUT.dctl --source-frame N --duration N --interval N --waypoint-phase N --lookahead N (--roll-frames LIST | --roll-period N --roll-phase N) --roll-hold N --magnitude N --terminal-extension D\n  huntctl controller waypoint-policy OUTPUT.dctl --duration N --waypoints START:X:Y:Z[;...] --roll-frames LIST --roll-hold N --magnitude N [--button-layers START:DURATION:MASK,...]\n  huntctl milestone compile SOURCE.milestones OUTPUT.dmsp\n  huntctl milestone inspect INPUT.dmsp\n  huntctl milestone format SOURCE.milestones\n  huntctl tape inspect INPUT.tape [--frames]\n  huntctl tape compile PROGRAM.tas OUTPUT.tape\n  huntctl tape run INPUT.tape --game PATH --dvd PATH --state-root DIR [--milestone-program FILE] [--milestones IDS] [--milestone-goal ID] [--milestone-result FILE] [--gameplay-trace FILE] [--gameplay-trace-channels LIST] [--headful] [--timeout-seconds N] [--game-arg ARG]...\n  huntctl tape prove INPUT.tape --game PATH --dvd PATH --state-root DIR --milestone-goal ID [--milestone-program FILE] [--proof FILE] [--repetitions N] [--timeout-seconds N] [--game-arg ARG]...\n  huntctl tape concat OUTPUT.tape INPUT.tape INPUT.tape...\n  huntctl trace inspect INPUT.trace\n  huntctl trace timeline INPUT.trace\n  huntctl trace compare INPUT.trace INPUT.trace...\n  huntctl trace state-hashes INPUT.trace [--output SERIES.json]\n  huntctl trace compare-state LEFT.trace RIGHT.trace\n  huntctl trace route-diagnostics INPUT.trace --source-boundary-frame N --terminal-frame N [--corner-yaw-threshold-s16 N] [--output REPORT.json]\n  huntctl timeline parse ROUTE.timeline\n  huntctl timeline inspect ROUTE.timeline\n  huntctl timeline status --timeline FILE [--continuation NAME] [--select ORIGINAL_SEGMENT=REPLACEMENT_SEGMENT]... [--output FILE]\n  huntctl timeline rebase-compatible --timeline FILE --continuation NAME --select ORIGINAL_SEGMENT=REPLACEMENT_SEGMENT --name NEW_NAME\n  huntctl timeline store init ROOT\n  huntctl timeline store import --store ROOT --timeline FILE --ref REF\n  huntctl timeline store import-evaluation --store ROOT --evaluation FILE --segment NAME --fingerprint VALUE [--ref REF]\n  huntctl timeline store fork --store ROOT --from REF --to REF [--lineage NAME]\n  huntctl timeline store append --store ROOT --ref REF --timeline FILE --continuation NAME\n  huntctl timeline store replay-repair --store ROOT --from REF --to REF --timeline FILE --continuation NAME\n  huntctl timeline store promote --store ROOT --ref REF --object ID\n  huntctl timeline store resolve|show|verify|gc ...\n  huntctl search seed --segment ID --output DIR [--candidate FILE] [--size N] [--rng-seed N]\n  huntctl search collect --population MANIFEST --input EVALUATION.json... --output RESULTS.json\n  huntctl search evolve --population MANIFEST --results RESULTS --output DIR [--size N] [--elites N] [--rng-seed N]\n  huntctl search rank --population MANIFEST --results RESULTS\n  huntctl search inspect CANDIDATE.json\n  huntctl search mock-evaluate --population MANIFEST --output RESULTS.json [--attempts N]\n  huntctl corpus init ROOT\n  huntctl corpus ingest ROOT --tape INPUT.tape --scenario ID --build BUILD.json [--scenario-json METADATA.json]\n  huntctl corpus list ROOT\n  huntctl corpus show ROOT ARTIFACT_SHA256\n  huntctl corpus verify ROOT\n  huntctl run --worker PATH\n  huntctl replay --worker PATH\n  huntctl mock-worker [--mock-revision REVISION]\n\nSearch segment IDs: boot_to_fsp103, fsp103_to_fsp104\nTAS DSL: dusktape 1 (legacy JSON schema: {PROGRAM_SCHEMA})"
    );
    eprintln!(
        "\nBenchmarks:\n  huntctl benchmark import-skybook --source CHECKOUT --output MANIFEST.json [--revision FULL_GIT_REVISION] [--repository URL]\n  huntctl benchmark validate-skybook-selection --manifest MANIFEST.json --selection SELECTION.json\n  huntctl benchmark route-cold-process --timeline FILE --segment ID --goal GOAL --game PATH --dvd PATH --artifact-root RELATIVE_ROOT [--output REPORT.json] [--repository-root ROOT] [--repetitions N] [--timeout-seconds N]\n  huntctl benchmark cold-process --request REQUEST.json --artifact-root RELATIVE_ROOT --output REPORT.json [--repository-root ROOT] [--repetitions N] [--prefix-ticks N]\n  huntctl benchmark validate-cold-process --report REPORT.json"
    );
    eprintln!(
        "\nCore harness:\n  huntctl harness validate-suite --suite SUITE.json [--repository-root DIR]\n  huntctl harness seal-suite --input DRAFT.json --output SUITE.json [--repository-root DIR]\n  huntctl harness validate-run-request --request REQUEST.json [--repository-root DIR]\n  huntctl harness seal-run-request --input DRAFT.json --output REQUEST.json [--repository-root DIR]\n  huntctl harness validate-run-result --result RESULT.json --request REQUEST.json --artifact-root DIR [--repository-root DIR]\n  huntctl harness seal-run-result --input DRAFT.json --output RESULT.json --request REQUEST.json --artifact-root DIR [--repository-root DIR]\n  huntctl harness execute --request REQUEST.json [--repository-root DIR] [--attempt N]\n  huntctl harness inspect-objective --request REQUEST.json [--result RESULT.json --artifact-root DIR] [--repository-root DIR]"
    );
    eprintln!(
        "\nRun identity:\n  huntctl identity compare --mode replay|trace-merge|model-training|checkpoint-restore|cross-build-comparison|cross-fidelity-comparison --expected EXPECTED.json --actual ACTUAL.json"
    );
    eprintln!(
        "\nRoute workbench:\n  huntctl timeline workbench --timeline FILE --game PATH [--dvd PATH] [--state-root DIR] [--port N] [--no-open]"
    );
    eprintln!(
        "  huntctl timeline prune-thumbnails --timeline FILE --state-root DIR [--repository-root DIR] [--apply]"
    );
    eprintln!(
        "\nScenario fixtures:\n  huntctl fixture compile SOURCE.json OUTPUT.fixture\n  huntctl fixture inspect INPUT.fixture\n  huntctl tape compile PROGRAM.tas OUTPUT.tape [--fixture FIXTURE.json]"
    );
    eprintln!(
        "\nTape editing:\n  huntctl tape slice INPUT.tape OUTPUT.tape --start N --frames N\n  huntctl tape layer BASE.tape OVERLAY.tape OUTPUT.tape --start N\n  huntctl tape resample INPUT.tape OUTPUT.tape\n  huntctl tape diff LEFT.tape RIGHT.tape"
    );
    eprintln!(
        "\nTransition corpus lifecycle:\n  huntctl corpus query --input BATCH.dtcz [--input MORE.dtcz] [--action N] [--terminal BOOL] [--minimum-reward R] [--limit N]\n  huntctl corpus compare --left LEFT.dtcz --right RIGHT.dtcz\n  huntctl corpus merge|compact --input BATCH.dtcz [--input MORE.dtcz] --output OUTPUT.dtcz\n  huntctl corpus shard --input BATCH.dtcz --output-directory DIR [--maximum-transitions N]\n  huntctl corpus refeature --source SOURCE.json --output OUTPUT.dtcz [--view movement-state/v1|movement-state/v2]\n  huntctl corpus validate-transitions --input BATCH.dtcz [--input MORE.dtcz]\n  huntctl corpus quarantine --input BATCH.dtcz [--input MORE.dtcz] --quarantine-root DIR [--apply]\n  huntctl corpus gc-content --store ROOT --trash-root DIR (--manifest ROOT.json | --reference SHA256)... [--apply]\n  huntctl corpus verify-episodes --store ROOT\n  huntctl corpus gc-episodes --store ROOT --trash-root DIR --retain-episode SHA256 [--retain-episode SHA256]... [--reference BLOB_SHA256]... [--apply]\n  huntctl corpus export-arrow --input BATCH.dtcz [--input MORE.dtcz] --output ANALYSIS.arrow"
    );
    eprintln!(
        "\nTape recording:\n  huntctl tape record SEED.tape OUTPUT.tape --game PATH --dvd PATH --state-root DIR [--capacity N] [--timeout-seconds N] [--game-arg ARG]..."
    );
    eprintln!(concat!(
        "\nNative search:\n",
        "  huntctl search evaluate --population MANIFEST --output DIR (--run-request REQUEST.json [--repository-root DIR] | --game PATH --dvd PATH) [--results FILE] [--episode-store DIR] [--workers N] [--repetitions N]\n",
        "  huntctl search run --segment ID [--candidate FILE] --output DIR (--run-request REQUEST.json [--repository-root DIR] | --game PATH --dvd PATH) [--generations N] [--size N] [--elites N] [--workers N] [--repetitions N]\n",
        "  huntctl search beam --candidate SEED.json --options OPTIONS.json [--q-priors PRIORS.json] --output DIR (--run-request REQUEST.json [--repository-root DIR] | --game PATH --dvd PATH) [--beam-width N] [--maximum-depth N] [--candidate-budget N] [--workers N] [--repetitions N]\n",
        "  huntctl search continuous --method cem|cma-es --candidate SEED.json --axes AXES.json --output DIR (--run-request REQUEST.json [--repository-root DIR] | --game PATH --dvd PATH) [--generations N] [--population N] [--elites N] [--initial-sigma S] [--candidate-budget N] [--rng-seed N]\n",
        "  huntctl search bayesian --candidate SEED.json --axes AXES.json --output DIR (--run-request REQUEST.json [--repository-root DIR] | --game PATH --dvd PATH) [--generations N] [--batch-size N] [--initial-samples N] [--acquisition-pool N] [--length-scale L] [--observation-noise N] [--exploration X] [--candidate-budget N] [--rng-seed N]\n",
        "  huntctl search tournament --definition TOURNAMENT.json --output DIR (--run-request REQUEST.json [--repository-root DIR] | --game PATH --dvd PATH) [--anchored-prefix PREFIX.tape --milestones PROGRAM.dmsp --segment ID --source-milestone NAME --source-boundary-fingerprint VALUE --goal-milestone NAME] [--workers N] [--repetitions N]\n",
        "  huntctl search prepare-tournament-lane --candidate CANDIDATE.json --proposal-envelopes ENVELOPES.json --output DIR\n",
        "  huntctl search minimize-route --candidate FILE --anchored-prefix PREFIX.tape --milestones PROGRAM.dmsp --segment ID --source-milestone NAME --source-boundary-fingerprint VALUE --goal-milestone NAME --output DIR (--run-request REQUEST.json [--repository-root DIR] | --game PATH --dvd PATH) [--candidate-budget N] [--workers N] [--repetitions N] [--resume]\n",
        "  huntctl search golf-inputs --candidate FILE --anchored-prefix PREFIX.tape --milestones PROGRAM.dmsp --segment ID --source-milestone NAME --source-boundary-fingerprint VALUE --goal-milestone NAME --output DIR (--run-request REQUEST.json [--repository-root DIR] | --game PATH --dvd PATH) [--candidate-budget N] [--workers N] [--repetitions N]\n",
        "  huntctl search minimize-boot --candidate FILE --output DIR (--run-request REQUEST.json [--repository-root DIR] | --game PATH --dvd PATH) [--workers N] [--repetitions N]\n",
        "  huntctl search golf-boot --candidate FILE --output DIR (--run-request REQUEST.json [--repository-root DIR] | --game PATH --dvd PATH) [--workers N] [--repetitions N] [--resume]\n",
        "  huntctl search golf-option --plan ROLL.json --execution EXECUTION.json --tape INPUT.tape --output PROPOSALS.json [--cancellation-tick N --condition-index N] [--heading-step N] [--magnitude-step N] [--duration-step N] [--phase-step N] [--button-step N] [--cancellation-step N]\n",
        "  huntctl search golf-path --plan PATH.json --execution EXECUTION.json --tape INPUT.tape --output PROPOSALS.json [--cancellation-tick N --condition-index N] [--point-step N] [--duration-step N] [--phase-step N] [--cancellation-step N]\n",
        "  huntctl search candidate-from-tape --input INPUT.tape --output CANDIDATE.json --segment ID [--start N] [--frames N] [--normalize-port-one]\n  huntctl search candidate-to-tape --input CANDIDATE.json --output OUTPUT.tape\n  huntctl search suffix-batch --candidate CANDIDATE.json --output BATCH.json --method deletion|delete-hold|button-edge|heading|corner|corner-wide|collision|fine-heading|fine-terminal|lane-shift|fine-lane-shift|early-lane-shift|magnitude|asymmetric-lane-shift|post-collision|recovery-bias|timing|path|terminal --source-boundary-fingerprint VALUE [--source-frame N] [--maximum-ticks N] [--candidate-budget N]\n  huntctl search suffix-refine --candidate CANDIDATE.json --batch BATCH.json --results RESULT.json --output BATCH.json [--candidate-budget N]\n  huntctl search suffix-select --candidate CANDIDATE.json --batch BATCH.json --id ID --output CANDIDATE.json\n  huntctl search suffix-promote-failure --candidate CANDIDATE.json --batch BATCH.json --results RESULT.json --output CANDIDATE.json"
    ));
    eprintln!(
        "  huntctl search run-route --timeline FILE --lineage NAME --segment TIMELINE_SEGMENT [--source-goal GOAL] [--progress-goal GOAL ...] [--goal GOAL] [--candidate FILE] --output DIR (--run-request REQUEST.json [--repository-root DIR] | --game PATH --dvd PATH) [--generations N] [--size N] [--elites N] [--workers N] [--repetitions N]"
    );
    eprintln!(
        "  huntctl search golf-route-inputs --timeline FILE --segment TIMELINE_SEGMENT [--anchor-segment TIMELINE_SEGMENT] [--source-goal GOAL] [--goal GOAL] --output DIR (--run-request REQUEST.json [--repository-root DIR] | --game PATH --dvd PATH) [--candidate-budget N] [--workers N] [--repetitions N]"
    );
    eprintln!(
        "\nObservation views:\n  huntctl observe spec movement-state/v2 [--output SPEC.json]\n  huntctl observe inspect SPEC.json\n\nNative fitted Q:\n  huntctl learn benchmark\n  huntctl learn extract-trace --trace INPUT.trace --tape INPUT.tape --episode-context CONTEXT.json --start-frame N --end-frame N --output BATCH.dtcz [--artifact-store ROOT] [--view movement-state/v1|movement-state/v2] [--terminal]\n  huntctl learn dataset --source SOURCE.json [--source MORE.json] --output DATASET.json [--withheld-objective ID] [--validation-percent N] [--test-percent N] [--artifact-store ROOT]\n  huntctl learn diff-episodes --success-trace SUCCESS.trace --failure-trace FAILURE.trace --output DIFF.json [--success-evidence SUCCESS.json --failure-evidence FAILURE.json]\n  huntctl learn inspect --input BATCH.dtcz\n  huntctl learn baseline --method nearest-neighbor|tabular --input BATCH.dtcz [--input MORE.dtcz] [--query-transition N] [--query-side state|next-state] [--discount D] [--neighbors N --feature INDEX:SCALE:continuous|categorical ...] [--axis INDEX:ORIGIN:WIDTH ...]\n  huntctl learn calibrate (--dataset DATASET.json [--split validation|test|withheld] | --training TRAIN.dtcz --held-out TEST.dtcz) --output REPORT.json [--iterations N] [--n-step N] [--trees N] [--max-depth N] [--seed N] [--discount D] [--all-continuous | --categorical-feature N ...]\n  huntctl learn double-q (--dataset DATASET.json | --input BATCH.dtcz [--input MORE.dtcz]) [--model-output MODEL.json] [--artifact-store ROOT] [--query-transition N] [--query-side state|next-state] [--epochs N] [--hidden-width N] [--learning-rate R] [--target-sync-steps N] [--gradient-clip V] [--seed N] [--discount D]\n  huntctl learn fit (--dataset DATASET.json | --input BATCH.dtcz [--input MORE.dtcz]) [--model-output MODEL.json] [--artifact-store ROOT] [--query-transition N] [--query-side state|next-state] [--iterations N] [--n-step N] [--trees N] [--max-depth N] [--seed N] [--discount D] [--shaping SPEC.json --shaping-report REPORT.json] [--all-continuous | --categorical-feature N ...]"
    );
    eprintln!("  huntctl learn inspect-episode --input IMMUTABLE-EPISODE.json");
    eprintln!(
        "  huntctl learn cql (--dataset DATASET.json | --input BATCH.dtcz [--input MORE.dtcz]) [--model-output MODEL.json] [--artifact-store ROOT] [--query-transition N] [--query-side state|next-state] [--epochs N] [--hidden-width N] [--learning-rate R] [--target-sync-steps N] [--conservative-weight A] [--temperature T] [--gradient-clip V] [--seed N] [--discount D]"
    );
    eprintln!(
        "  huntctl learn iql (--dataset DATASET.json | --input BATCH.dtcz [--input MORE.dtcz]) [--model-output MODEL.json] [--artifact-store ROOT] [--query-transition N] [--query-side state|next-state] [--epochs N] [--hidden-width N] [--learning-rate R] [--discount D] [--expectile T] [--advantage-beta B] [--max-advantage-weight W] [--target-sync-steps N] [--gradient-clip G] [--seed N]"
    );
    eprintln!(
        "  huntctl learn ensemble-q (--dataset DATASET.json | --input BATCH.dtcz [--input MORE.dtcz]) [--model-output MODEL.json] [--artifact-store ROOT] [--query-transition N] [--query-side state|next-state] [--members N] [--epochs N] [--hidden-width N] [--learning-rate R] [--discount D] [--target-sync-steps N] [--gradient-clip G] [--seed N] [--critic-seed N]"
    );
    eprintln!(
        "  huntctl learn prioritized-q (--dataset DATASET.json | --input BATCH.dtcz [--input MORE.dtcz]) [--model-output MODEL.json] [--artifact-store ROOT] [--query-transition N] [--query-side state|next-state] [--epochs N] [--hidden-width N] [--learning-rate R] [--discount D] [--target-sync-steps N] [--gradient-clip G] [--seed N] [--priority-alpha A] [--importance-beta-start B] [--importance-beta-end B] [--priority-epsilon E] [--importance-weight-cap W] [--replay-seed N]"
    );
    eprintln!(
        "  huntctl learn ablate-q --component dueling-heads|n-step|distributional-values|noisy-exploration (--dataset DATASET.json [--split validation|test|withheld] | --training TRAIN.dtcz --held-out TEST.dtcz) --output REPORT.json [--epochs N] [--hidden-width N] [--learning-rate R] [--discount D] [--target-sync-steps N] [--gradient-clip G] [--seed N] [--n-step N] [--distribution-atoms N] [--distribution-min V] [--distribution-max V] [--noisy-stddev V]"
    );
    eprintln!(
        "  huntctl learn option-values --input BATCH.json --model-output MODEL.json [--artifact-store ROOT] [--query-sample N] [--query-side state|next-state] [--iterations N] [--trees N] [--max-depth N] [--min-samples-leaf N] [--features-per-split N] [--max-thresholds N] [--categorical-feature INDEX] [--discount D] [--seed N]"
    );
    eprintln!(
        "\nSemantic oracles:\n  huntctl oracle evaluate --program ORACLES.json --trace RUN.trace [--supplemental OBSERVATIONS.json] [--run-outcome OUTCOME.json] [--output REPORT.json]\n  huntctl oracle compose --manifest COMPOSITION.json [--output EVIDENCE.json]\n  huntctl oracle compare --program ORACLES.json --evidence COMPARISON.json [--output REPORT.json]"
    );
    eprintln!(
        "\nOffline world geometry:\n  huntctl world inventory --stage-dir STAGE_DIR --stage STAGE_ID --output INVENTORY.json [--artifact-store ROOT]\n  huntctl world spatial-index --stage-dir STAGE_DIR --stage STAGE_ID --output INDEX.json [--artifact-store ROOT]\n  huntctl world query point --stage-dir STAGE_DIR --stage STAGE_ID --room N --point X,Y,Z [--max-distance D] [--limit K] [FILTERS]\n  huntctl world query aabb --stage-dir STAGE_DIR --stage STAGE_ID --room N --min X,Y,Z --max X,Y,Z [--limit K] [FILTERS]\n  huntctl world query ray --stage-dir STAGE_DIR --stage STAGE_ID --room N --origin X,Y,Z --direction X,Y,Z --max-distance D [--limit K] [FILTERS]\n  FILTERS: [--load-triggers-only] [--trigger-id ID] [--destination-stage ID] [--destination-room N] [--destination-point N]\n  huntctl world kcl --archive ROOM.arc --prism INDEX [--point X,Y,Z] [--kcl-name room.kcl] [--plc-name room.plc]\n  huntctl world kcl --kcl room.kcl --plc room.plc --prism INDEX [--point X,Y,Z]"
    );
}

fn mock_worker(args: &[String]) -> Result<(), Box<dyn Error>> {
    let revision_label = option(args, "--mock-revision").unwrap_or_else(|| "mock".into());
    let mut revision = format!("{:x}", Sha256::digest(revision_label.as_bytes()));
    revision.truncate(40);
    let aurora_revision = "2".repeat(40);
    let feature_digest = "3".repeat(64);
    let fidelity_profile =
        option(args, "--mock-fidelity-profile").unwrap_or_else(|| "observe_only".into());
    let stdin = io::stdin();
    let mut stdout = io::stdout().lock();
    for line in stdin.lock().lines() {
        let request: Value = serde_json::from_str(&line?)?;
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        let command = request.get("command").and_then(Value::as_str).unwrap_or("");
        let mut response = match command {
            "hello" => json!({
                "protocol": {"name": CONTROL_PROTOCOL_NAME, "version": CONTROL_PROTOCOL_VERSION},
                "type": "hello", "ok": true,
                "build": {
                    "version": "mock", "describe": revision_label, "revision": revision, "branch": "test",
                    "dirty_digest": "", "source_date": "1970-01-01", "aurora_revision": aurora_revision,
                    "compiler": "mock-compiler", "compiler_target": "mock-target", "build_type": "test",
                    "feature_switches": "mock=ON", "feature_digest": feature_digest, "fidelity_profile": fidelity_profile, "platform": env::consts::OS,
                    "architecture": env::consts::ARCH, "pointer_bits": usize::BITS, "dirty": false
                },
                "capabilities": {
                    "persistent_control": true, "engine_session": false, "headless": false,
                    "scenario_load": false, "input_tape": false, "batch_run": false,
                    "commands": ["hello", "ping", "session_audit", "shutdown"]
                }
            }),
            "ping" => success_response("pong"),
            "session_audit" => json!({
                "protocol": {"name": CONTROL_PROTOCOL_NAME, "version": CONTROL_PROTOCOL_VERSION},
                "type": "session_audit", "ok": true,
                "audit": {
                    "schema": "dusklight-engine-session-reuse-audit/v1",
                    "reusable": false,
                    "evaluated_boundary": "pre_engine_boot",
                    "target_boundary": "post_authenticated_run",
                    "blockers": [{
                        "code": "game_global_reconstruction",
                        "subsystem": "game_state",
                        "required_guarantee": "global game state reconstructs from a clean origin"
                    }]
                }
            }),
            "shutdown" => success_response("shutdown"),
            _ => json!({
                "protocol": {"name": CONTROL_PROTOCOL_NAME, "version": CONTROL_PROTOCOL_VERSION},
                "type": "error", "ok": false,
                "error": {"code": "unknown_command", "message": "unsupported command"}
            }),
        };
        response["id"] = id;
        writeln!(stdout, "{}", serde_json::to_string(&response)?)?;
        stdout.flush()?;
        if command == "shutdown" {
            break;
        }
    }
    Ok(())
}

fn mock_search_worker(args: &[String]) -> Result<(), Box<dyn Error>> {
    let mode = option(args, "--mock-mode").unwrap_or_else(|| "hit".into());
    if mode == "timeout" {
        std::thread::sleep(Duration::from_secs(30));
        return Ok(());
    }
    let result_path = required_path(args, "--milestone-result")?;
    if mode == "malformed" {
        fs::write(result_path, b"{}")?;
        return Ok(());
    }
    let goal = option(args, "--milestone-goal").ok_or("mock worker missing milestone goal")?;
    let requested = option(args, "--milestones").ok_or("mock worker missing milestone list")?;
    let logical_tick_budget = option(args, "--automation-tick-budget")
        .ok_or("mock worker missing automation tick budget")?
        .parse::<u64>()?;
    if logical_tick_budget == 0 {
        return Err("mock worker received a zero automation tick budget".into());
    }
    if mode == "protocol-failure" || mode == "game-crash" {
        writeln!(io::stdout(), "mock native partial stdout before {mode}")?;
        writeln!(io::stderr(), "mock native partial stderr before {mode}")?;
        io::stdout().flush()?;
        io::stderr().flush()?;
        std::process::exit(if mode == "protocol-failure" { 3 } else { 86 });
    }
    let state_root = option(args, "--automation-data-root").unwrap_or_default();
    let input_tape = option(args, "--input-tape")
        .map(|path| -> Result<_, Box<dyn Error>> { Ok(InputTape::decode(&fs::read(path)?)?.tape) })
        .transpose()?
        .unwrap_or_default();
    let controller_duration = option(args, "--input-controller")
        .map(|path| -> Result<_, Box<dyn Error>> {
            Ok(ControllerProgram::decode(&fs::read(path)?)?.duration_frames)
        })
        .transpose()?;
    if input_tape.frames.is_empty() && controller_duration.is_none() {
        return Err("mock worker requires an input tape or controller".into());
    }
    let planned_ticks = controller_duration
        .map(u64::from)
        .unwrap_or(input_tape.frames.len() as u64);
    if planned_ticks > logical_tick_budget {
        return Err(format!(
            "mock worker input requires {planned_ticks} ticks but received budget {logical_tick_budget}"
        )
        .into());
    }
    if let Some(path) = option(args, "--gameplay-trace") {
        fs::write(path, mock_gameplay_trace(&input_tape.boot))?;
    }
    let program_digest = option(args, "--milestone-program")
        .map(|path| -> Result<_, Box<dyn Error>> {
            Ok(Digest(milestone_dsl::decode(&fs::read(path)?)?.program_sha256).to_string())
        })
        .transpose()?;
    let second_attempt = state_root.contains("attempt-002") || state_root.contains("repeat-002");
    let unstable_miss = mode == "unstable-goal" && second_attempt;
    let coordinate_golf_tick = if mode == "coordinate-golf" {
        let pulse_timestamps: Vec<_> = input_tape
            .frames
            .iter()
            .enumerate()
            .filter_map(|(index, frame)| (frame.pads[0].buttons != 0).then_some(index))
            .collect();
        match pulse_timestamps.as_slice() {
            [10, 20] | [10, 19] => Some(100_u64),
            [9, 19] => Some(90_u64),
            _ => None,
        }
    } else {
        None
    };
    let hit_goal = if mode == "coordinate-golf" {
        coordinate_golf_tick.is_some()
    } else {
        mode != "miss" && mode != "target-lost" && !unstable_miss
    };
    if let Some(path) = option(args, "--realized-input-tape") {
        let realized = if let Some(duration) = controller_duration {
            let frame_count = if hit_goal {
                1
            } else if mode == "target-lost" {
                duration.saturating_sub(1)
            } else {
                duration
            };
            InputTape {
                boot: input_tape.boot.clone(),
                frames: vec![
                    huntctl::tape::InputFrame {
                        owned_ports: 1,
                        ..huntctl::tape::InputFrame::default()
                    };
                    usize::try_from(frame_count)?
                ],
                ..InputTape::default()
            }
        } else {
            input_tape.clone()
        };
        fs::write(path, realized.encode()?)?;
    }
    let milestones: Vec<Value> = requested
        .split(',')
        .map(|id| {
            let hit = hit_goal
                || ((mode == "miss" || unstable_miss)
                    && id == "gameplay-ready-f-sp103"
                    && goal == "entered-f-sp104");
            let base_tick = match id {
                "gameplay-ready-f-sp103" => coordinate_golf_tick.unwrap_or(77),
                "exit-f-sp103-to-f-sp104" => 572,
                "entered-f-sp104" => 603,
                _ => 0,
            };
            let tick = base_tick + u64::from(mode == "unstable-tick" && second_attempt);
            let tape_frame = tick + u64::from(mode == "unstable-frame" && second_attempt);
            let mut digest_character = match id {
                "gameplay-ready-f-sp103" => "1",
                "exit-f-sp103-to-f-sp104" => "2",
                "entered-f-sp104" => "3",
                _ => "0",
            };
            if mode == "unstable-fingerprint" && second_attempt {
                digest_character = "a";
            }
            json!({
                "id": id,
                "hit": hit,
                "sim_tick": hit.then_some(tick),
                "tape_frame": hit.then_some(tape_frame),
                "evidence": hit.then(|| json!({
                    "boot": input_tape.boot,
                    "boundary_fingerprint": {
                        "schema": "dusklight.milestone-boundary/v4",
                        "algorithm": "xxh3-128",
                        "canonical_encoding": "little-endian-fixed-v4",
                        "digest": digest_character.repeat(32)
                    }
                }))
            })
        })
        .collect();
    fs::write(
        result_path,
        serde_json::to_vec_pretty(&json!({
            "schema": {
                "name": "dusklight.automation.milestones",
                "version": 5
            },
            "boot": input_tape.boot,
            "boot_origin_established": true,
            "goal": goal,
            "goal_reached": hit_goal,
            "program_digest": program_digest,
            "milestones": milestones
        }))?,
    )?;
    if let Some(path) = option(args, "--automation-phase-timing") {
        fs::write(
            path,
            serde_json::to_vec_pretty(&json!({
                "schema": "dusklight-native-lifecycle-timing/v1",
                "clock": "steady_clock",
                "process_entry_micros": 0,
                "cli_configured_micros": 0,
                "aurora_initialized_micros": 0,
                "engine_ready_micros": 0,
                "stage_ready_micros": 0,
                "first_simulation_tick_micros": 0,
                "last_simulation_tick_micros": 0,
                "proof_artifacts_written_micros": 0,
                "engine_shutdown_micros": 1,
                "exit_ready_micros": 1
            }))?,
        )?;
    }
    if mode == "miss"
        || mode == "target-lost"
        || unstable_miss
        || (mode == "coordinate-golf" && !hit_goal)
    {
        std::process::exit(2);
    }
    Ok(())
}

fn mock_gameplay_trace(boot: &huntctl::tape::TapeBoot) -> Vec<u8> {
    let mut bytes = vec![0_u8; 225];
    bytes[..8].copy_from_slice(b"DUSKTRCE");
    bytes[8..10].copy_from_slice(&4_u16.to_le_bytes());
    bytes[10..12].copy_from_slice(&128_u16.to_le_bytes());
    bytes[12..16].copy_from_slice(&30_u32.to_le_bytes());
    bytes[16..20].copy_from_slice(&1_u32.to_le_bytes());
    bytes[20..28].copy_from_slice(&1_u64.to_le_bytes());
    bytes[28..32].copy_from_slice(&1_u32.to_le_bytes());
    bytes[32..34].copy_from_slice(&1_u16.to_le_bytes());
    bytes[34..36].copy_from_slice(&64_u16.to_le_bytes());
    bytes[36..44].copy_from_slice(&128_u64.to_le_bytes());
    bytes[44..52].copy_from_slice(&192_u64.to_le_bytes());
    bytes[52..60].copy_from_slice(&1_u64.to_le_bytes());
    if let huntctl::tape::TapeBoot::Stage {
        stage,
        room,
        point,
        layer,
        save_slot,
        fixture,
        ..
    } = boot
    {
        bytes[64] = 1;
        bytes[65] = save_slot.unwrap_or(0);
        bytes[66] = *room as u8;
        bytes[67] = *layer as u8;
        bytes[68..70].copy_from_slice(&point.to_le_bytes());
        bytes[70] = stage.len() as u8;
        bytes[72..72 + stage.len()].copy_from_slice(stage.as_bytes());
        if let Some(fixture) = fixture {
            let encoded = fixture.encode().expect("validated mock fixture");
            let fixture_offset = bytes.len() as u64;
            bytes[88..96].copy_from_slice(&fixture_offset.to_le_bytes());
            bytes[96..100].copy_from_slice(&(encoded.len() as u32).to_le_bytes());
            bytes.extend_from_slice(&encoded);
        }
    }
    bytes[128..130].copy_from_slice(&0_u16.to_le_bytes());
    bytes[130..132].copy_from_slice(&1_u16.to_le_bytes());
    bytes[132..136].copy_from_slice(&3_u32.to_le_bytes());
    bytes[136..140].copy_from_slice(&32_u32.to_le_bytes());
    bytes[140..144].copy_from_slice(&1_u32.to_le_bytes());
    bytes[144..152].copy_from_slice(&192_u64.to_le_bytes());
    bytes[152..160].copy_from_slice(&1_u64.to_le_bytes());
    bytes[160..168].copy_from_slice(&193_u64.to_le_bytes());
    bytes[168..176].copy_from_slice(&32_u64.to_le_bytes());
    bytes[192] = 1;
    bytes[193..201].copy_from_slice(&1_u64.to_le_bytes());
    bytes[201..209].copy_from_slice(&0_u64.to_le_bytes());
    bytes[209..217].copy_from_slice(&u64::MAX.to_le_bytes());
    bytes[217..221].copy_from_slice(&1_u32.to_le_bytes());
    bytes[221] = 2;
    bytes[222] = 1;
    bytes
}

fn mock_record_worker(args: &[String]) -> Result<(), Box<dyn Error>> {
    let output = required_path(args, "--record-input-tape")?;
    let tape = InputTape {
        frames: vec![huntctl::tape::InputFrame::default()],
        ..InputTape::default()
    };
    fs::write(output, tape.encode()?)?;
    Ok(())
}

fn success_response(response_type: &str) -> Value {
    json!({
        "protocol": {"name": CONTROL_PROTOCOL_NAME, "version": CONTROL_PROTOCOL_VERSION},
        "type": response_type, "ok": true
    })
}
