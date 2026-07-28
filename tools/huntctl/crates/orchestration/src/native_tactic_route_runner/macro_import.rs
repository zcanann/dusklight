use super::*;

pub(super) struct ImportedPromotedTactics {
    pub(super) entries: Vec<TacticCatalogEntry>,
    pub(super) report: NativeTacticImportedMacroReport,
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
    let entries = artifact
        .registry
        .promoted()
        .map(|record| record.candidate.catalog_entry().map_err(route_error))
        .collect::<Result<Vec<_>, _>>()?;
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
