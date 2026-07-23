//! Bounded extraction of a canonical GameCube disc filesystem.
//!
//! This is intentionally only a container reader.  Exact build recognition is
//! still performed by `orig_discovery` over the extracted bytes, so a product
//! code or filename never grants planner support.

use crate::PlannerContractError;
use serde::Serialize;
use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

pub const GCM_EXTRACTION_REPORT_SCHEMA_V1: &str =
    "dusklight.route-planner.gcm-extraction-report/v1";
const DISC_HEADER_BYTES: u64 = 0x440;
const BI2_BYTES: u64 = 0x2000;
const APPLOADER_OFFSET: u64 = 0x2440;
const APPLOADER_HEADER_BYTES: u64 = 0x20;
const MAX_FST_BYTES: u64 = 64 * 1024 * 1024;
const MAX_FST_ENTRIES: usize = 100_000;
const MAX_COMPONENT_BYTES: usize = 255;
const COPY_BUFFER_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GcmExtractionReport {
    pub schema: &'static str,
    pub product_id: String,
    pub file_count: u32,
    pub directory_count: u32,
    pub extracted_bytes: u64,
    pub fst_offset: u64,
    pub fst_bytes: u64,
    pub dol_offset: u64,
    pub dol_bytes: u64,
    pub apploader_bytes: u64,
}

#[derive(Clone, Debug)]
struct FileEntry {
    relative_path: PathBuf,
    offset: u64,
    bytes: u64,
}

#[derive(Clone, Debug)]
struct ParsedDisc {
    product_id: String,
    fst_offset: u64,
    fst_bytes: u64,
    dol_offset: u64,
    dol_bytes: u64,
    apploader_bytes: u64,
    directories: Vec<PathBuf>,
    files: Vec<FileEntry>,
}

/// Extracts one GameCube GCM/ISO into the conventional `sys/` + `files/`
/// layout consumed by `scan-orig`.  `output` must not already exist.
pub fn extract_gamecube_disc(
    iso: impl AsRef<Path>,
    output: impl AsRef<Path>,
) -> Result<GcmExtractionReport, PlannerContractError> {
    let supplied_iso = iso.as_ref();
    let output = output.as_ref();
    let iso = fs::canonicalize(supplied_iso).map_err(|error| io_error("gcm.iso", error))?;
    let metadata = fs::metadata(&iso).map_err(|error| io_error("gcm.iso", error))?;
    if !metadata.is_file() {
        return Err(PlannerContractError::new(
            "gcm.iso",
            "must resolve to a regular file",
        ));
    }
    if fs::symlink_metadata(output).is_ok() {
        return Err(PlannerContractError::new(
            "gcm.output",
            "must not already exist",
        ));
    }
    let parent = output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let parent_metadata =
        fs::metadata(parent).map_err(|error| io_error("gcm.output.parent", error))?;
    if !parent_metadata.is_dir() {
        return Err(PlannerContractError::new(
            "gcm.output.parent",
            "must resolve to an existing directory",
        ));
    }

    let mut source = File::open(&iso).map_err(|error| io_error("gcm.iso", error))?;
    let disc_bytes = source
        .metadata()
        .map_err(|error| io_error("gcm.iso", error))?
        .len();
    let parsed = parse_disc(&mut source, disc_bytes)?;
    fs::create_dir(output).map_err(|error| io_error("gcm.output", error))?;
    fs::create_dir(output.join("sys")).map_err(|error| io_error("gcm.output.sys", error))?;
    fs::create_dir(output.join("files")).map_err(|error| io_error("gcm.output.files", error))?;
    for directory in &parsed.directories {
        fs::create_dir_all(output.join("files").join(directory))
            .map_err(|error| io_error("gcm.output.directory", error))?;
    }

    let mut buffer = vec![0_u8; COPY_BUFFER_BYTES];
    let mut extracted_bytes = 0_u64;
    for (name, offset, bytes) in [
        ("boot.bin", 0, DISC_HEADER_BYTES),
        ("bi2.bin", DISC_HEADER_BYTES, BI2_BYTES),
        ("apploader.img", APPLOADER_OFFSET, parsed.apploader_bytes),
        ("main.dol", parsed.dol_offset, parsed.dol_bytes),
        ("fst.bin", parsed.fst_offset, parsed.fst_bytes),
    ] {
        copy_range(
            &mut source,
            &output.join("sys").join(name),
            offset,
            bytes,
            &mut buffer,
        )?;
        extracted_bytes = extracted_bytes
            .checked_add(bytes)
            .ok_or_else(|| PlannerContractError::new("gcm.extracted_bytes", "overflowed"))?;
    }
    for entry in &parsed.files {
        copy_range(
            &mut source,
            &output.join("files").join(&entry.relative_path),
            entry.offset,
            entry.bytes,
            &mut buffer,
        )?;
        extracted_bytes = extracted_bytes
            .checked_add(entry.bytes)
            .ok_or_else(|| PlannerContractError::new("gcm.extracted_bytes", "overflowed"))?;
    }
    Ok(GcmExtractionReport {
        schema: GCM_EXTRACTION_REPORT_SCHEMA_V1,
        product_id: parsed.product_id,
        file_count: parsed.files.len() as u32,
        directory_count: parsed.directories.len() as u32,
        extracted_bytes,
        fst_offset: parsed.fst_offset,
        fst_bytes: parsed.fst_bytes,
        dol_offset: parsed.dol_offset,
        dol_bytes: parsed.dol_bytes,
        apploader_bytes: parsed.apploader_bytes,
    })
}

fn parse_disc(source: &mut File, disc_bytes: u64) -> Result<ParsedDisc, PlannerContractError> {
    if disc_bytes < APPLOADER_OFFSET + APPLOADER_HEADER_BYTES {
        return Err(PlannerContractError::new(
            "gcm.iso",
            "is shorter than the system area",
        ));
    }
    let mut header = [0_u8; DISC_HEADER_BYTES as usize];
    read_exact_at(source, 0, &mut header, "gcm.header")?;
    let product = &header[..6];
    if !product
        .iter()
        .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
    {
        return Err(PlannerContractError::new(
            "gcm.product_id",
            "is not canonical ASCII",
        ));
    }
    let product_id = String::from_utf8(product.to_vec())
        .map_err(|_| PlannerContractError::new("gcm.product_id", "is not UTF-8"))?;
    let dol_offset = u64::from(be_u32(&header, 0x420)?);
    let fst_offset = u64::from(be_u32(&header, 0x424)?);
    let fst_bytes = u64::from(be_u32(&header, 0x428)?);
    if fst_bytes < 12 || fst_bytes > MAX_FST_BYTES {
        return Err(PlannerContractError::new(
            "gcm.fst",
            "size is outside the bounded domain",
        ));
    }
    checked_range(dol_offset, 0x100, disc_bytes, "gcm.dol")?;
    checked_range(fst_offset, fst_bytes, disc_bytes, "gcm.fst")?;
    let mut dol_header = [0_u8; 0x100];
    read_exact_at(source, dol_offset, &mut dol_header, "gcm.dol")?;
    let dol_bytes = dol_size(&dol_header)?;
    checked_range(dol_offset, dol_bytes, disc_bytes, "gcm.dol")?;
    let mut apploader_header = [0_u8; APPLOADER_HEADER_BYTES as usize];
    read_exact_at(
        source,
        APPLOADER_OFFSET,
        &mut apploader_header,
        "gcm.apploader",
    )?;
    let apploader_code_bytes = u64::from(be_u32(&apploader_header, 0x14)?);
    let apploader_trailer_bytes = u64::from(be_u32(&apploader_header, 0x18)?);
    let apploader_bytes = APPLOADER_HEADER_BYTES
        .checked_add(apploader_code_bytes)
        .and_then(|value| value.checked_add(apploader_trailer_bytes))
        .ok_or_else(|| PlannerContractError::new("gcm.apploader", "size overflowed"))?;
    checked_range(
        APPLOADER_OFFSET,
        apploader_bytes,
        disc_bytes,
        "gcm.apploader",
    )?;
    let mut fst = vec![0_u8; fst_bytes as usize];
    read_exact_at(source, fst_offset, &mut fst, "gcm.fst")?;
    let (directories, files) = parse_fst(&fst, disc_bytes)?;
    validate_file_ranges(&files, disc_bytes)?;
    Ok(ParsedDisc {
        product_id,
        fst_offset,
        fst_bytes,
        dol_offset,
        dol_bytes,
        apploader_bytes,
        directories,
        files,
    })
}

fn dol_size(header: &[u8; 0x100]) -> Result<u64, PlannerContractError> {
    let mut maximum = 0_u64;
    for index in 0..7 {
        let offset = u64::from(be_u32(header, index * 4)?);
        let bytes = u64::from(be_u32(header, 0x90 + index * 4)?);
        maximum = maximum.max(offset.checked_add(bytes).ok_or_else(|| {
            PlannerContractError::new("gcm.dol", "text section range overflowed")
        })?);
    }
    for index in 0..11 {
        let offset = u64::from(be_u32(header, 0x1c + index * 4)?);
        let bytes = u64::from(be_u32(header, 0xac + index * 4)?);
        maximum = maximum.max(offset.checked_add(bytes).ok_or_else(|| {
            PlannerContractError::new("gcm.dol", "data section range overflowed")
        })?);
    }
    if maximum < 0x100 {
        return Err(PlannerContractError::new(
            "gcm.dol",
            "contains no bounded sections",
        ));
    }
    Ok(maximum)
}

fn parse_fst(
    fst: &[u8],
    disc_bytes: u64,
) -> Result<(Vec<PathBuf>, Vec<FileEntry>), PlannerContractError> {
    let root_kind_name = be_u32(fst, 0)?;
    let entry_count = be_u32(fst, 8)? as usize;
    if root_kind_name >> 24 != 1
        || root_kind_name & 0x00ff_ffff != 0
        || !(1..=MAX_FST_ENTRIES).contains(&entry_count)
        || entry_count
            .checked_mul(12)
            .is_none_or(|bytes| bytes > fst.len())
    {
        return Err(PlannerContractError::new(
            "gcm.fst.root",
            "is invalid or unbounded",
        ));
    }
    let string_offset = entry_count * 12;
    let mut stack = vec![(0_usize, entry_count, PathBuf::new())];
    let mut directories = Vec::new();
    let mut files = Vec::new();
    let mut paths = BTreeSet::new();
    for index in 1..entry_count {
        while stack.last().is_some_and(|(_, end, _)| index >= *end) {
            stack.pop();
        }
        let Some((parent_index, parent_end, parent_path)) = stack.last().cloned() else {
            return Err(PlannerContractError::new(
                "gcm.fst",
                "entry escapes the root directory",
            ));
        };
        let base = index * 12;
        let kind_name = be_u32(fst, base)?;
        let directory = kind_name >> 24 == 1;
        if kind_name >> 24 > 1 {
            return Err(PlannerContractError::new(
                "gcm.fst.entry",
                "has an unknown type",
            ));
        }
        let name = fst_name(fst, string_offset, (kind_name & 0x00ff_ffff) as usize)?;
        let relative_path = parent_path.join(&name);
        let text = relative_path
            .to_str()
            .ok_or_else(|| PlannerContractError::new("gcm.fst.path", "is not UTF-8"))?;
        if !paths.insert(text.to_owned()) {
            return Err(PlannerContractError::new("gcm.fst.path", "is duplicated"));
        }
        if directory {
            let declared_parent = be_u32(fst, base + 4)? as usize;
            let next = be_u32(fst, base + 8)? as usize;
            if declared_parent != parent_index || next <= index || next > parent_end {
                return Err(PlannerContractError::new(
                    "gcm.fst.directory",
                    "has invalid nesting",
                ));
            }
            directories.push(relative_path.clone());
            stack.push((index, next, relative_path));
        } else {
            let offset = u64::from(be_u32(fst, base + 4)?);
            let bytes = u64::from(be_u32(fst, base + 8)?);
            checked_range(offset, bytes, disc_bytes, "gcm.fst.file")?;
            files.push(FileEntry {
                relative_path,
                offset,
                bytes,
            });
        }
    }
    Ok((directories, files))
}

fn fst_name(
    fst: &[u8],
    string_offset: usize,
    relative_offset: usize,
) -> Result<String, PlannerContractError> {
    let start = string_offset
        .checked_add(relative_offset)
        .filter(|start| *start < fst.len())
        .ok_or_else(|| PlannerContractError::new("gcm.fst.name", "offset is out of range"))?;
    let tail = &fst[start..];
    let end = tail
        .iter()
        .position(|byte| *byte == 0)
        .ok_or_else(|| PlannerContractError::new("gcm.fst.name", "is not null terminated"))?;
    let name = &tail[..end];
    if name.is_empty()
        || name.len() > MAX_COMPONENT_BYTES
        || name == b"."
        || name == b".."
        || name
            .iter()
            .any(|byte| *byte == b'/' || *byte == b'\\' || byte.is_ascii_control())
    {
        return Err(PlannerContractError::new(
            "gcm.fst.name",
            "is not a safe path component",
        ));
    }
    String::from_utf8(name.to_vec())
        .map_err(|_| PlannerContractError::new("gcm.fst.name", "is not UTF-8"))
}

fn validate_file_ranges(files: &[FileEntry], disc_bytes: u64) -> Result<(), PlannerContractError> {
    let mut ranges = files
        .iter()
        .filter(|entry| entry.bytes != 0)
        .map(|entry| (entry.offset, entry.offset + entry.bytes))
        .collect::<Vec<_>>();
    ranges.sort_unstable();
    if ranges.windows(2).any(|pair| pair[0].1 > pair[1].0)
        || ranges.last().is_some_and(|range| range.1 > disc_bytes)
    {
        return Err(PlannerContractError::new(
            "gcm.fst.files",
            "contain overlapping or out-of-disc ranges",
        ));
    }
    Ok(())
}

fn copy_range(
    source: &mut File,
    output: &Path,
    offset: u64,
    bytes: u64,
    buffer: &mut [u8],
) -> Result<(), PlannerContractError> {
    source
        .seek(SeekFrom::Start(offset))
        .map_err(|error| io_error("gcm.iso.seek", error))?;
    let mut destination = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)
        .map_err(|error| io_error("gcm.output.file", error))?;
    let mut remaining = bytes;
    while remaining != 0 {
        let count = remaining.min(buffer.len() as u64) as usize;
        source
            .read_exact(&mut buffer[..count])
            .map_err(|error| io_error("gcm.iso.read", error))?;
        destination
            .write_all(&buffer[..count])
            .map_err(|error| io_error("gcm.output.write", error))?;
        remaining -= count as u64;
    }
    destination
        .sync_all()
        .map_err(|error| io_error("gcm.output.sync", error))?;
    Ok(())
}

fn read_exact_at(
    source: &mut File,
    offset: u64,
    output: &mut [u8],
    field: &str,
) -> Result<(), PlannerContractError> {
    source
        .seek(SeekFrom::Start(offset))
        .map_err(|error| io_error(field, error))?;
    source
        .read_exact(output)
        .map_err(|error| io_error(field, error))
}

fn checked_range(
    offset: u64,
    bytes: u64,
    limit: u64,
    field: &str,
) -> Result<(), PlannerContractError> {
    if offset.checked_add(bytes).is_none_or(|end| end > limit) {
        return Err(PlannerContractError::new(
            field,
            "range exceeds the disc image",
        ));
    }
    Ok(())
}

fn be_u32(bytes: &[u8], offset: usize) -> Result<u32, PlannerContractError> {
    let value: [u8; 4] = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| PlannerContractError::new("gcm", "integer is truncated"))?
        .try_into()
        .expect("slice length checked");
    Ok(u32::from_be_bytes(value))
}

fn io_error(field: &str, error: std::io::Error) -> PlannerContractError {
    PlannerContractError::new(field, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static NEXT_TEST_ROOT: AtomicU64 = AtomicU64::new(0);

    fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
    }

    fn synthetic_disc(path: &Path, malicious_name: Option<&[u8]>) {
        let mut bytes = vec![0_u8; 0x6000];
        bytes[..6].copy_from_slice(b"GZ2E01");
        put_u32(&mut bytes, 0x420, 0x3000);
        put_u32(&mut bytes, 0x424, 0x3200);
        put_u32(&mut bytes, 0x428, 0x100);
        put_u32(&mut bytes, 0x3000, 0x100);
        put_u32(&mut bytes, 0x3090, 4);
        bytes[0x3100..0x3104].copy_from_slice(b"DOL!");
        put_u32(&mut bytes, 0x2454, 4);
        bytes[0x2460..0x2464].copy_from_slice(b"APP!");
        let fst = &mut bytes[0x3200..0x3300];
        put_u32(fst, 0, 0x0100_0000);
        put_u32(fst, 8, 3);
        put_u32(fst, 12, 0x0100_0000);
        put_u32(fst, 16, 0);
        put_u32(fst, 20, 3);
        put_u32(fst, 24, 4);
        put_u32(fst, 28, 0x4000);
        put_u32(fst, 32, 5);
        let strings = malicious_name.unwrap_or(b"res\0file.bin\0");
        fst[36..36 + strings.len()].copy_from_slice(strings);
        bytes[0x4000..0x4005].copy_from_slice(b"HELLO");
        fs::write(path, bytes).unwrap();
    }

    fn root() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let ordinal = NEXT_TEST_ROOT.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "dusklight-gcm-test-{}-{nonce}-{ordinal}",
            std::process::id()
        ))
    }

    #[test]
    fn extracts_system_and_fst_files_without_trusting_the_product_id() {
        let root = root();
        fs::create_dir(&root).unwrap();
        let iso = root.join("disc.iso");
        let output = root.join("tree");
        synthetic_disc(&iso, None);
        let report = extract_gamecube_disc(&iso, &output).unwrap();
        assert_eq!(report.product_id, "GZ2E01");
        assert_eq!(report.file_count, 1);
        assert_eq!(
            fs::read(output.join("files/res/file.bin")).unwrap(),
            b"HELLO"
        );
        assert_eq!(fs::read(output.join("sys/main.dol")).unwrap().len(), 0x104);
        assert!(extract_gamecube_disc(&iso, &output).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_traversal_components_before_creating_output() {
        let root = root();
        fs::create_dir(&root).unwrap();
        let iso = root.join("disc.iso");
        let output = root.join("tree");
        synthetic_disc(&iso, Some(b"..\0file.bin\0"));
        assert!(extract_gamecube_disc(&iso, &output).is_err());
        assert!(!output.exists());
        fs::remove_dir_all(root).unwrap();
    }
}
