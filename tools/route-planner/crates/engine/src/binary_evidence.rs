//! Exact executable evidence for version-scoped mechanics audits.
//!
//! The extractor deliberately stops at function bytes and mechanically obvious
//! instruction shapes. Semantic bindings (for example, identifying a caller as
//! a return-place writer site) remain a separate audited layer.

use crate::artifact::Digest;
use crate::{PlannerContractError, canonical_json, validate_label};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

pub const BINARY_FUNCTION_EVIDENCE_SCHEMA: &str =
    "dusklight.route-planner.binary-function-evidence/v1";
pub const BINARY_RANGE_EVIDENCE_SCHEMA: &str = "dusklight.route-planner.binary-range-evidence/v1";
const DOL_TEXT_SECTION_COUNT: usize = 7;
const DOL_DATA_SECTION_COUNT: usize = 11;
const DOL_SECTION_COUNT: usize = DOL_TEXT_SECTION_COUNT + DOL_DATA_SECTION_COUNT;
const DOL_ADDRESS_TABLE_OFFSET: usize = 0x48;
const DOL_SIZE_TABLE_OFFSET: usize = 0x90;
const MAX_RETAINED_RANGE_BYTES: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BinaryFunctionShape {
    ImmediateReturn,
    Other,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DolSectionKind {
    Text,
    Data,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BinaryFunctionEvidence {
    pub schema: String,
    pub executable_sha256: Digest,
    pub symbol_table_sha256: Digest,
    pub symbol: String,
    pub virtual_address: u32,
    pub function_size: u32,
    pub text_section_index: u8,
    pub file_offset: u32,
    pub code_sha256: Digest,
    pub code_hex: String,
    pub shape: BinaryFunctionShape,
}

/// Exact bytes addressed through one loadable DOL text or data section.
///
/// This artifact deliberately assigns no semantic meaning to the bytes. A
/// source audit or instruction-level reference must separately establish why a
/// retained constant is relevant to a planner rule.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BinaryRangeEvidence {
    pub schema: String,
    pub executable_sha256: Digest,
    pub virtual_address: u32,
    pub byte_size: u32,
    pub section_kind: DolSectionKind,
    pub section_index: u8,
    pub file_offset: u32,
    pub bytes_sha256: Digest,
    pub bytes_hex: String,
}

impl BinaryFunctionEvidence {
    pub fn validate(&self) -> Result<(), PlannerContractError> {
        if self.schema != BINARY_FUNCTION_EVIDENCE_SCHEMA {
            return Err(PlannerContractError::new(
                "binary_function_evidence.schema",
                "is unsupported",
            ));
        }
        if self.executable_sha256 == Digest::ZERO || self.symbol_table_sha256 == Digest::ZERO {
            return Err(PlannerContractError::new(
                "binary_function_evidence.source",
                "must contain nonzero exact source identities",
            ));
        }
        validate_label("binary_function_evidence.symbol", &self.symbol)?;
        if self.function_size == 0 || !self.virtual_address.is_multiple_of(4) {
            return Err(PlannerContractError::new(
                "binary_function_evidence.function",
                "must have a nonzero size and aligned virtual address",
            ));
        }
        if usize::from(self.text_section_index) >= DOL_TEXT_SECTION_COUNT {
            return Err(PlannerContractError::new(
                "binary_function_evidence.text_section_index",
                "is outside the DOL text-section table",
            ));
        }
        let code = decode_hex(&self.code_hex, "binary_function_evidence.code_hex")?;
        if code.len() != self.function_size as usize {
            return Err(PlannerContractError::new(
                "binary_function_evidence.code_hex",
                "length does not match the symbol size",
            ));
        }
        if Digest(Sha256::digest(&code).into()) != self.code_sha256 {
            return Err(PlannerContractError::new(
                "binary_function_evidence.code_sha256",
                "does not match the retained function bytes",
            ));
        }
        if classify_function(&code) != self.shape {
            return Err(PlannerContractError::new(
                "binary_function_evidence.shape",
                "does not match the retained function bytes",
            ));
        }
        Ok(())
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, PlannerContractError> {
        self.validate()?;
        canonical_json(self)
    }

    pub fn digest(&self) -> Result<Digest, PlannerContractError> {
        Ok(Digest(Sha256::digest(self.canonical_bytes()?).into()))
    }

    pub fn decode_canonical(bytes: &[u8]) -> Result<Self, PlannerContractError> {
        let evidence: Self = serde_json::from_slice(bytes)?;
        evidence.validate()?;
        if evidence.canonical_bytes()? != bytes {
            return Err(PlannerContractError::new(
                "binary_function_evidence",
                "is not canonical JSON",
            ));
        }
        Ok(evidence)
    }
}

impl BinaryRangeEvidence {
    pub fn validate(&self) -> Result<(), PlannerContractError> {
        if self.schema != BINARY_RANGE_EVIDENCE_SCHEMA {
            return Err(PlannerContractError::new(
                "binary_range_evidence.schema",
                "is unsupported",
            ));
        }
        if self.executable_sha256 == Digest::ZERO {
            return Err(PlannerContractError::new(
                "binary_range_evidence.executable_sha256",
                "must be nonzero",
            ));
        }
        if self.byte_size == 0 || self.byte_size as usize > MAX_RETAINED_RANGE_BYTES {
            return Err(PlannerContractError::new(
                "binary_range_evidence.byte_size",
                "must be nonzero and within the retained-range limit",
            ));
        }
        let section_count = match self.section_kind {
            DolSectionKind::Text => DOL_TEXT_SECTION_COUNT,
            DolSectionKind::Data => DOL_DATA_SECTION_COUNT,
        };
        if usize::from(self.section_index) >= section_count {
            return Err(PlannerContractError::new(
                "binary_range_evidence.section_index",
                "is outside the selected DOL section table",
            ));
        }
        let bytes = decode_hex(&self.bytes_hex, "binary_range_evidence.bytes_hex")?;
        if bytes.len() != self.byte_size as usize {
            return Err(PlannerContractError::new(
                "binary_range_evidence.bytes_hex",
                "length does not match byte_size",
            ));
        }
        if Digest(Sha256::digest(&bytes).into()) != self.bytes_sha256 {
            return Err(PlannerContractError::new(
                "binary_range_evidence.bytes_sha256",
                "does not match the retained bytes",
            ));
        }
        Ok(())
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, PlannerContractError> {
        self.validate()?;
        canonical_json(self)
    }

    pub fn digest(&self) -> Result<Digest, PlannerContractError> {
        Ok(Digest(Sha256::digest(self.canonical_bytes()?).into()))
    }

    pub fn decode_canonical(bytes: &[u8]) -> Result<Self, PlannerContractError> {
        let evidence: Self = serde_json::from_slice(bytes)?;
        evidence.validate()?;
        if evidence.canonical_bytes()? != bytes {
            return Err(PlannerContractError::new(
                "binary_range_evidence",
                "is not canonical JSON",
            ));
        }
        Ok(evidence)
    }
}

pub fn extract_dol_function_evidence(
    dol: &[u8],
    symbol_table: &[u8],
    symbol: &str,
) -> Result<BinaryFunctionEvidence, PlannerContractError> {
    validate_label("binary_function_evidence.symbol", symbol)?;
    let symbol_text = std::str::from_utf8(symbol_table).map_err(|_| {
        PlannerContractError::new("binary_function_evidence.symbol_table", "is not UTF-8 text")
    })?;
    let (virtual_address, function_size) = parse_function_symbol(symbol_text, symbol)?;
    let function_end = virtual_address.checked_add(function_size).ok_or_else(|| {
        PlannerContractError::new(
            "binary_function_evidence.function",
            "virtual address range overflows",
        )
    })?;

    let mut matching_section = None;
    for section_index in 0..DOL_TEXT_SECTION_COUNT {
        let section_offset = read_be_u32(dol, section_index * 4, "dol.text_offset")?;
        let section_address = read_be_u32(
            dol,
            DOL_ADDRESS_TABLE_OFFSET + section_index * 4,
            "dol.text_address",
        )?;
        let section_size = read_be_u32(
            dol,
            DOL_SIZE_TABLE_OFFSET + section_index * 4,
            "dol.text_size",
        )?;
        if section_size == 0 {
            continue;
        }
        let section_end = section_address.checked_add(section_size).ok_or_else(|| {
            PlannerContractError::new("dol.text_section", "virtual address range overflows")
        })?;
        if virtual_address >= section_address && function_end <= section_end {
            if matching_section.is_some() {
                return Err(PlannerContractError::new(
                    "dol.text_section",
                    "function is covered by multiple text sections",
                ));
            }
            matching_section = Some((section_index, section_offset, section_address));
        }
    }
    let (section_index, section_offset, section_address) = matching_section.ok_or_else(|| {
        PlannerContractError::new(
            "dol.text_section",
            "function is not wholly contained in one text section",
        )
    })?;
    let file_offset = section_offset
        .checked_add(virtual_address - section_address)
        .ok_or_else(|| PlannerContractError::new("dol.function", "file offset overflows"))?;
    let file_end = file_offset
        .checked_add(function_size)
        .ok_or_else(|| PlannerContractError::new("dol.function", "file range overflows"))?;
    let code = dol
        .get(file_offset as usize..file_end as usize)
        .ok_or_else(|| PlannerContractError::new("dol.function", "file range is truncated"))?;

    let evidence = BinaryFunctionEvidence {
        schema: BINARY_FUNCTION_EVIDENCE_SCHEMA.into(),
        executable_sha256: Digest(Sha256::digest(dol).into()),
        symbol_table_sha256: Digest(Sha256::digest(symbol_table).into()),
        symbol: symbol.into(),
        virtual_address,
        function_size,
        text_section_index: section_index as u8,
        file_offset,
        code_sha256: Digest(Sha256::digest(code).into()),
        code_hex: encode_hex(code),
        shape: classify_function(code),
    };
    evidence.validate()?;
    Ok(evidence)
}

pub fn extract_dol_range_evidence(
    dol: &[u8],
    virtual_address: u32,
    byte_size: u32,
) -> Result<BinaryRangeEvidence, PlannerContractError> {
    if byte_size == 0 || byte_size as usize > MAX_RETAINED_RANGE_BYTES {
        return Err(PlannerContractError::new(
            "binary_range_evidence.byte_size",
            "must be nonzero and within the retained-range limit",
        ));
    }
    let virtual_end = virtual_address.checked_add(byte_size).ok_or_else(|| {
        PlannerContractError::new(
            "binary_range_evidence.range",
            "virtual address range overflows",
        )
    })?;

    let mut matching_section = None;
    for global_index in 0..DOL_SECTION_COUNT {
        let section_offset = read_be_u32(dol, global_index * 4, "dol.section_offset")?;
        let section_address = read_be_u32(
            dol,
            DOL_ADDRESS_TABLE_OFFSET + global_index * 4,
            "dol.section_address",
        )?;
        let section_size = read_be_u32(
            dol,
            DOL_SIZE_TABLE_OFFSET + global_index * 4,
            "dol.section_size",
        )?;
        if section_size == 0 {
            continue;
        }
        let section_end = section_address.checked_add(section_size).ok_or_else(|| {
            PlannerContractError::new("dol.section", "virtual address range overflows")
        })?;
        let file_end = section_offset
            .checked_add(section_size)
            .ok_or_else(|| PlannerContractError::new("dol.section", "file range overflows"))?;
        if file_end as usize > dol.len() {
            return Err(PlannerContractError::new(
                "dol.section",
                "declared file range is truncated",
            ));
        }
        if virtual_address >= section_address && virtual_end <= section_end {
            if matching_section.is_some() {
                return Err(PlannerContractError::new(
                    "dol.section",
                    "range is covered by multiple loadable sections",
                ));
            }
            matching_section = Some((global_index, section_offset, section_address));
        }
    }
    let (global_index, section_offset, section_address) = matching_section.ok_or_else(|| {
        PlannerContractError::new(
            "dol.section",
            "range is not wholly contained in one loadable section",
        )
    })?;
    let file_offset = section_offset
        .checked_add(virtual_address - section_address)
        .ok_or_else(|| PlannerContractError::new("dol.range", "file offset overflows"))?;
    let file_end = file_offset
        .checked_add(byte_size)
        .ok_or_else(|| PlannerContractError::new("dol.range", "file range overflows"))?;
    let bytes = dol
        .get(file_offset as usize..file_end as usize)
        .ok_or_else(|| PlannerContractError::new("dol.range", "file range is truncated"))?;
    let (section_kind, section_index) = if global_index < DOL_TEXT_SECTION_COUNT {
        (DolSectionKind::Text, global_index)
    } else {
        (DolSectionKind::Data, global_index - DOL_TEXT_SECTION_COUNT)
    };
    let evidence = BinaryRangeEvidence {
        schema: BINARY_RANGE_EVIDENCE_SCHEMA.into(),
        executable_sha256: Digest(Sha256::digest(dol).into()),
        virtual_address,
        byte_size,
        section_kind,
        section_index: section_index as u8,
        file_offset,
        bytes_sha256: Digest(Sha256::digest(bytes).into()),
        bytes_hex: encode_hex(bytes),
    };
    evidence.validate()?;
    Ok(evidence)
}

fn parse_function_symbol(
    symbol_table: &str,
    symbol: &str,
) -> Result<(u32, u32), PlannerContractError> {
    let prefix = format!("{symbol} = .text:0x");
    let mut matches = symbol_table.lines().filter_map(|line| {
        let suffix = line.strip_prefix(&prefix)?;
        let (address, comment) = suffix.split_once(';')?;
        if !comment.contains("type:function") {
            return None;
        }
        let size = comment.split_once("size:0x")?.1;
        let size = size
            .chars()
            .take_while(|character| character.is_ascii_hexdigit())
            .collect::<String>();
        Some((address.to_owned(), size))
    });
    let (address, size) = matches.next().ok_or_else(|| {
        PlannerContractError::new(
            "binary_function_evidence.symbol",
            "has no exact text-function record",
        )
    })?;
    if matches.next().is_some() {
        return Err(PlannerContractError::new(
            "binary_function_evidence.symbol",
            "has multiple exact text-function records",
        ));
    }
    let address = u32::from_str_radix(&address, 16).map_err(|_| {
        PlannerContractError::new(
            "binary_function_evidence.symbol",
            "has an invalid virtual address",
        )
    })?;
    let size = u32::from_str_radix(&size, 16).map_err(|_| {
        PlannerContractError::new(
            "binary_function_evidence.symbol",
            "has an invalid function size",
        )
    })?;
    Ok((address, size))
}

fn classify_function(code: &[u8]) -> BinaryFunctionShape {
    if code == [0x4e, 0x80, 0x00, 0x20] {
        BinaryFunctionShape::ImmediateReturn
    } else {
        BinaryFunctionShape::Other
    }
}

fn read_be_u32(
    bytes: &[u8],
    offset: usize,
    field: &'static str,
) -> Result<u32, PlannerContractError> {
    let value = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| PlannerContractError::new(field, "is truncated"))?;
    Ok(u32::from_be_bytes(
        value.try_into().expect("four-byte slice"),
    ))
}

fn encode_hex(bytes: &[u8]) -> String {
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(encoded, "{byte:02x}").expect("writing to String cannot fail");
    }
    encoded
}

fn decode_hex(value: &str, field: &'static str) -> Result<Vec<u8>, PlannerContractError> {
    if !value.len().is_multiple_of(2) {
        return Err(PlannerContractError::new(
            field,
            "must contain complete bytes",
        ));
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let pair = std::str::from_utf8(pair).expect("hex input is ASCII-sized");
            u8::from_str_radix(pair, 16)
                .map_err(|_| PlannerContractError::new(field, "contains a non-hex byte"))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (Vec<u8>, Vec<u8>) {
        let mut dol = vec![0; 0x140];
        dol[0..4].copy_from_slice(&0x100u32.to_be_bytes());
        dol[0x48..0x4c].copy_from_slice(&0x8000_0000u32.to_be_bytes());
        dol[0x90..0x94].copy_from_slice(&0x40u32.to_be_bytes());
        dol[0x110..0x114].copy_from_slice(&[0x4e, 0x80, 0x00, 0x20]);
        dol[0x1c..0x20].copy_from_slice(&0x120u32.to_be_bytes());
        dol[0x64..0x68].copy_from_slice(&0x8040_0000u32.to_be_bytes());
        dol[0xac..0xb0].copy_from_slice(&0x20u32.to_be_bytes());
        dol[0x128..0x12d].copy_from_slice(b"Msgus");
        let symbols = b"writer__Fv = .text:0x80000010; // type:function size:0x4 scope:global\n";
        (dol, symbols.to_vec())
    }

    #[test]
    fn extracts_and_seals_an_immediate_return_function() {
        let (dol, symbols) = fixture();
        let evidence = extract_dol_function_evidence(&dol, &symbols, "writer__Fv").unwrap();
        assert_eq!(evidence.virtual_address, 0x8000_0010);
        assert_eq!(evidence.file_offset, 0x110);
        assert_eq!(evidence.code_hex, "4e800020");
        assert_eq!(evidence.shape, BinaryFunctionShape::ImmediateReturn);
        assert_eq!(
            BinaryFunctionEvidence::decode_canonical(&evidence.canonical_bytes().unwrap()).unwrap(),
            evidence
        );
    }

    #[test]
    fn rejects_missing_duplicate_and_truncated_function_records() {
        let (dol, symbols) = fixture();
        assert_eq!(
            extract_dol_function_evidence(&dol, &symbols, "missing")
                .unwrap_err()
                .field(),
            "binary_function_evidence.symbol"
        );

        let mut duplicate = symbols.clone();
        duplicate.extend_from_slice(&symbols);
        assert_eq!(
            extract_dol_function_evidence(&dol, &duplicate, "writer__Fv")
                .unwrap_err()
                .field(),
            "binary_function_evidence.symbol"
        );

        assert_eq!(
            extract_dol_function_evidence(&dol[..0x112], &symbols, "writer__Fv")
                .unwrap_err()
                .field(),
            "dol.function"
        );
    }

    #[test]
    fn canonical_decode_rejects_a_forged_shape() {
        let (dol, symbols) = fixture();
        let mut evidence = extract_dol_function_evidence(&dol, &symbols, "writer__Fv").unwrap();
        evidence.shape = BinaryFunctionShape::Other;
        assert_eq!(
            evidence.canonical_bytes().unwrap_err().field(),
            "binary_function_evidence.shape"
        );
    }

    #[test]
    fn extracts_unaligned_data_range_by_virtual_address() {
        let (dol, _) = fixture();
        let evidence = extract_dol_range_evidence(&dol, 0x8040_0008, 5).unwrap();
        assert_eq!(evidence.section_kind, DolSectionKind::Data);
        assert_eq!(evidence.section_index, 0);
        assert_eq!(evidence.file_offset, 0x128);
        assert_eq!(evidence.bytes_hex, "4d73677573");
        assert_eq!(
            BinaryRangeEvidence::decode_canonical(&evidence.canonical_bytes().unwrap()).unwrap(),
            evidence
        );
    }

    #[test]
    fn range_extraction_rejects_zero_cross_section_overlap_and_truncation() {
        let (dol, _) = fixture();
        assert_eq!(
            extract_dol_range_evidence(&dol, 0x8040_0000, 0)
                .unwrap_err()
                .field(),
            "binary_range_evidence.byte_size"
        );
        assert_eq!(
            extract_dol_range_evidence(&dol, 0x8040_001f, 2)
                .unwrap_err()
                .field(),
            "dol.section"
        );

        let mut overlap = dol.clone();
        overlap[0x20..0x24].copy_from_slice(&0x130u32.to_be_bytes());
        overlap[0x68..0x6c].copy_from_slice(&0x8040_0000u32.to_be_bytes());
        overlap[0xb0..0xb4].copy_from_slice(&0x10u32.to_be_bytes());
        assert_eq!(
            extract_dol_range_evidence(&overlap, 0x8040_0008, 4)
                .unwrap_err()
                .field(),
            "dol.section"
        );

        assert_eq!(
            extract_dol_range_evidence(&dol[..0x13f], 0x8040_0008, 4)
                .unwrap_err()
                .field(),
            "dol.section"
        );
    }

    #[test]
    fn canonical_range_decode_rejects_tampered_bytes() {
        let (dol, _) = fixture();
        let mut evidence = extract_dol_range_evidence(&dol, 0x8040_0008, 5).unwrap();
        evidence.bytes_hex = "4d73677500".into();
        assert_eq!(
            evidence.canonical_bytes().unwrap_err().field(),
            "binary_range_evidence.bytes_sha256"
        );
    }
}
