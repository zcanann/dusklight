//! Scenario-fixture compilation and inspection adapters.

use huntctl::scenario_fixture::ScenarioFixture;
use serde_json::json;
use std::error::Error;
use std::fs;

pub(crate) fn command_fixture(args: &[String]) -> Result<(), Box<dyn Error>> {
    match args.first().map(String::as_str) {
        Some("compile") if args.len() == 3 => {
            let fixture: ScenarioFixture = serde_json::from_slice(&fs::read(&args[1])?)?;
            let bytes = fixture.encode()?;
            fs::write(&args[2], &bytes)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": fixture.schema,
                    "name": fixture.name,
                    "encoded_bytes": bytes.len(),
                    "output": args[2]
                }))?
            );
            Ok(())
        }
        Some("inspect") if args.len() == 2 => {
            let fixture = ScenarioFixture::decode(&fs::read(&args[1])?)?;
            println!("{}", serde_json::to_string_pretty(&fixture)?);
            Ok(())
        }
        _ => Err(
            "fixture commands: compile SOURCE.json OUTPUT.fixture, inspect INPUT.fixture".into(),
        ),
    }
}
