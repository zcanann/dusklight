//! Command adapters grouped by control-plane domain.

use std::env;
use std::error::Error;

pub(crate) mod args;
pub mod benchmark;
pub mod conformance;
pub mod controller;
pub mod corpus;
pub mod fixture;
pub mod harness;
pub mod identity;
pub mod learning;
pub mod milestone;
pub mod observation;
pub mod oracle;
pub mod search;
pub mod survey;
pub mod tape;
pub mod timeline;
pub mod trace;
pub mod worker;
pub mod world;

pub fn run() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = env::args().skip(1).collect();
    let Some(command) = args.first().map(String::as_str) else {
        return crate::usage_error();
    };
    match command {
        "hello" => worker::command_hello(&args[1..]),
        "ping" => worker::command_ping(&args[1..]),
        "session" => worker::command_session(&args[1..]),
        "pool" => worker::command_pool(&args[1..]),
        "benchmark" => benchmark::command_benchmark(&args[1..]),
        "campaign" => harness::command_campaign(&args[1..]),
        "conformance" => conformance::command_conformance(&args[1..]),
        "harness" => harness::command_harness(&args[1..]),
        "identity" => identity::command_identity(&args[1..]),
        "corpus" => corpus::command_corpus(&args[1..]),
        "controller" => controller::command_controller(&args[1..]),
        "milestone" => milestone::command_milestone(&args[1..]),
        "fixture" => fixture::command_fixture(&args[1..]),
        "tape" => tape::command_tape(&args[1..]),
        "trace" => trace::command_trace(&args[1..]),
        "timeline" => timeline::command_timeline(&args[1..]),
        "search" => search::command_search(&args[1..]),
        "survey" => survey::command_survey(&args[1..]),
        "learn" => learning::command_learn(&args[1..]),
        "observe" => observation::command_observe(&args[1..]),
        "oracle" => oracle::command_oracle(&args[1..]),
        "world" => world::command_world(&args[1..]),
        "run" | "replay" => worker::command_not_ready(command, &args[1..]),
        "mock-worker" => crate::mock_worker(&args[1..]),
        "mock-search-worker" => crate::mock_search_worker(&args[1..]),
        "mock-record-worker" => crate::mock_record_worker(&args[1..]),
        "help" | "--help" | "-h" => {
            crate::print_usage();
            Ok(())
        }
        _ => crate::usage_error(),
    }
}
