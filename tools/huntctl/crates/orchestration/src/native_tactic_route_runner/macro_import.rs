use super::*;

#[derive(Clone)]
pub(super) struct ImportedPromotedTactics {
    pub(super) entries: Vec<ImportedPromotedTactic>,
    pub(super) report: NativeTacticImportedMacroReport,
}

#[derive(Clone, PartialEq)]
pub(super) struct ImportedPromotedTactic {
    pub(super) entry: TacticCatalogEntry,
    pub(super) condition: TacticMacroEntryCondition,
}

pub fn tactic_macro_registry_identity(
    path: &Path,
) -> Result<(Digest, u64), NativeTacticRouteRunError> {
    let artifact = read_tactic_macro_registry(path).map_err(route_error)?;
    let promoted_count =
        u64::try_from(artifact.registry.promoted().count()).map_err(route_error)?;
    Ok((artifact.content_sha256, promoted_count))
}

pub(super) fn load_imported_promoted_tactics(
    config: &NativeTacticRouteRunConfig<'_>,
) -> Result<Option<ImportedPromotedTactics>, NativeTacticRouteRunError> {
    let (Some(expected), Some(path)) = (
        config.execution_plan.promoted_tactic_registry_sha256,
        config.promoted_tactic_registry,
    ) else {
        return Ok(None);
    };
    let artifact = read_tactic_macro_registry(path).map_err(route_error)?;
    if artifact.content_sha256 != expected {
        return Err(route_message(
            "promoted tactic registry differs from its sealed execution-plan identity",
        ));
    }
    let entries = promoted_tactic_entries(&artifact.registry)?;
    if entries.is_empty() {
        return Err(route_message(
            "promoted tactic registry contains no promoted tactics",
        ));
    }
    Ok(Some(ImportedPromotedTactics {
        report: NativeTacticImportedMacroReport {
            registry_path: path_text(path),
            registry_sha256: expected,
            promoted_count: u64::try_from(entries.len()).map_err(route_error)?,
        },
        entries,
    }))
}

pub(super) fn promoted_tactic_entries(
    registry: &TacticMacroPromotionRegistry,
) -> Result<Vec<ImportedPromotedTactic>, NativeTacticRouteRunError> {
    registry
        .promoted()
        .map(|record| {
            Ok(ImportedPromotedTactic {
                entry: record.candidate.catalog_entry().map_err(route_error)?,
                condition: record.candidate.entry_condition().map_err(route_error)?,
            })
        })
        .collect()
}

pub(super) fn candidate_tactic_entries(
    registry: &TacticMacroPromotionRegistry,
) -> Result<Vec<ImportedPromotedTactic>, NativeTacticRouteRunError> {
    registry
        .records()
        .map(|record| {
            Ok(ImportedPromotedTactic {
                entry: record.candidate.catalog_entry().map_err(route_error)?,
                condition: record.candidate.entry_condition().map_err(route_error)?,
            })
        })
        .collect()
}

pub(super) fn merge_promoted_tactic_entries(
    active: &mut Vec<ImportedPromotedTactic>,
    discovered: Vec<ImportedPromotedTactic>,
) -> Result<(), NativeTacticRouteRunError> {
    for tactic in discovered {
        match active
            .iter()
            .find(|existing| existing.entry.option_id() == tactic.entry.option_id())
        {
            Some(existing) if existing != &tactic => {
                return Err(route_message(
                    "promoted tactic option identity collides with different content",
                ));
            }
            Some(_) => {}
            None => active.push(tactic),
        }
    }
    active.sort_by(|left, right| left.entry.option_id().cmp(right.entry.option_id()));
    Ok(())
}
