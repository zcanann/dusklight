//! Finalist-reduction policy layered over authenticated native evaluation.

mod boot;
mod route;

pub use boot::{golf_boot, minimize_boot};
pub use route::minimize_anchored_route;

use dusklight_automation_contracts::tape::{InputTape, RawPadState};
#[cfg(test)]
use dusklight_automation_contracts::tape::TapeBoot;
use dusklight_control::tape_chain::{ChainSegment, concatenate};
use dusklight_evaluation::*;
use dusklight_search::search::{
    Ancestry, Candidate, InterventionRange, MacroAction, SegmentProfile,
    tape_input_complexity, write_explicit_population,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

fn is_anchored_profile(profile: SegmentProfile) -> bool {
    matches!(
        profile,
        SegmentProfile::Fsp103ToFsp104 | SegmentProfile::LinkControlToTunnelCrawlStart
    )
}

fn directory_is_nonempty(path: &Path) -> Result<bool, EvaluateError> {
    Ok(path.is_dir() && fs::read_dir(path)?.next().transpose()?.is_some())
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<(), EvaluateError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_vec_pretty(value)?)?;
    Ok(())
}
