//! Read-only projection of the native debug inventory definitions.
//!
//! The editor remains the source of friendly names, selectable equipment, and defaults. The
//! workbench parses those checked-in definitions so it cannot quietly drift into a second item
//! catalog or a second runtime mutation mechanism.

use super::*;

const ITEM_HEADER: &str = include_str!("../../../../../include/d/d_item_data.h");
const DEBUG_EDITOR: &str = include_str!("../../../../../src/dusk/ui/editor.cpp");
const NONE_SYMBOL: &str = "dItemNo_NONE_e";
const NONE_ID: u16 = 0xff;

pub(super) struct InventoryCatalog {
    pub items: Vec<GraphInventoryItem>,
    pub slots: Vec<GraphInventorySlot>,
}

pub(super) fn load(_repository_root: &Path) -> Result<InventoryCatalog, WorkbenchError> {
    let ids = parse_item_ids(ITEM_HEADER)?;
    let mut items = parse_debug_items(DEBUG_EDITOR, &ids)?;
    items.sort_by_key(|item| item.id);
    let defaults = parse_slot_defaults(DEBUG_EDITOR, &ids)?;
    let slots = (0..24)
        .map(|id| GraphInventorySlot {
            id,
            default_item: defaults.get(&id).copied().unwrap_or(NONE_ID),
            quantity: matches!(id, 4 | 11..=17 | 23),
        })
        .collect();
    Ok(InventoryCatalog { items, slots })
}

fn parse_item_ids(source: &str) -> Result<BTreeMap<String, u16>, WorkbenchError> {
    let mut ids = BTreeMap::new();
    for line in source.lines() {
        let trimmed = line.trim();
        let Some(hex_and_tail) = trimmed.strip_prefix("/* 0x") else {
            continue;
        };
        let Some((hex, tail)) = hex_and_tail.split_once(" */") else {
            continue;
        };
        let Some(symbol) = tail.trim().trim_end_matches(',').split_whitespace().next() else {
            continue;
        };
        if !symbol.starts_with("dItemNo_") {
            continue;
        }
        let id = u16::from_str_radix(hex, 16)
            .map_err(|_| WorkbenchError::new("native item enum contains an invalid hex ID"))?;
        ids.insert(symbol.to_owned(), id);
    }
    if ids.get(NONE_SYMBOL) != Some(&NONE_ID) {
        return Err(WorkbenchError::new(
            "native item enum no longer declares dItemNo_NONE_e as 0xff",
        ));
    }
    Ok(ids)
}

fn parse_debug_items(
    source: &str,
    ids: &BTreeMap<String, u16>,
) -> Result<Vec<GraphInventoryItem>, WorkbenchError> {
    let section = source
        .split_once("std::map<int, itemInfo> itemMap = {")
        .and_then(|(_, tail)| tail.split_once("\n};").map(|(section, _)| section))
        .ok_or_else(|| WorkbenchError::new("cannot locate native debug item map"))?;
    let mut items = Vec::new();
    for line in section.lines() {
        if !line.contains("ITEMTYPE_EQUIP_e") && !line.contains(NONE_SYMBOL) {
            continue;
        }
        let trimmed = line.trim();
        let symbol = trimmed
            .strip_prefix('{')
            .and_then(|tail| tail.split_once(',').map(|(symbol, _)| symbol.trim()))
            .ok_or_else(|| WorkbenchError::new("native debug item entry has changed shape"))?;
        let name = trimmed
            .split_once("{\"")
            .and_then(|(_, tail)| tail.split_once('\"').map(|(name, _)| name))
            .ok_or_else(|| WorkbenchError::new("native debug item name has changed shape"))?;
        let id = ids.get(symbol).copied().ok_or_else(|| {
            WorkbenchError::new(format!("native debug item {symbol} has no enum ID"))
        })?;
        items.push(GraphInventoryItem {
            id,
            name: name.to_owned(),
        });
    }
    if !items.iter().any(|item| item.id == NONE_ID) {
        return Err(WorkbenchError::new(
            "native debug item map has no None entry",
        ));
    }
    Ok(items)
}

fn parse_slot_defaults(
    source: &str,
    ids: &BTreeMap<String, u16>,
) -> Result<BTreeMap<u16, u16>, WorkbenchError> {
    let section = source
        .split_once("defaultInventory = {")
        .and_then(|(_, tail)| tail.split_once("\n};").map(|(section, _)| section))
        .ok_or_else(|| WorkbenchError::new("cannot locate native default inventory"))?;
    let mut defaults = BTreeMap::new();
    for line in section.lines() {
        let Some(tail) = line.trim().strip_prefix("DefaultInventoryEntry{SLOT_") else {
            continue;
        };
        let (slot, tail) = tail.split_once(',').ok_or_else(|| {
            WorkbenchError::new("native default inventory entry has changed shape")
        })?;
        let symbol = tail.trim().trim_end_matches("},");
        let slot = slot
            .parse::<u16>()
            .map_err(|_| WorkbenchError::new("native default inventory slot is invalid"))?;
        let item = ids.get(symbol).copied().ok_or_else(|| {
            WorkbenchError::new(format!("native default item {symbol} has no enum ID"))
        })?;
        defaults.insert(slot, item);
    }
    if defaults.len() != 22 {
        return Err(WorkbenchError::new(
            "native default inventory no longer contains 22 entries",
        ));
    }
    Ok(defaults)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projects_native_debug_inventory_without_a_duplicate_catalog() {
        let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../..")
            .canonicalize()
            .unwrap();
        let catalog = load(&repository).unwrap();
        assert_eq!(catalog.slots.len(), 24);
        assert_eq!(catalog.slots[0].default_item, 0x40);
        assert_eq!(catalog.slots[7].default_item, NONE_ID);
        assert!(catalog.slots[4].quantity);
        assert!(!catalog.slots[5].quantity);
        assert!(
            catalog
                .items
                .iter()
                .any(|item| item.id == 0x40 && item.name == "Gale Boomerang")
        );
        assert!(
            catalog
                .items
                .iter()
                .any(|item| item.id == NONE_ID && item.name == "None")
        );
    }
}
