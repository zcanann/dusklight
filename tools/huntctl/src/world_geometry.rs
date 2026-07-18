//! Offline readers for original game collision resources.
//!
//! This module only consumes caller-owned bytes. It does not link against the
//! game, inspect live process memory, issue collision queries, or mutate the
//! source buffers.

use crate::artifact::Digest;
use serde::Serialize;
use sha2::{Digest as ShaDigest, Sha256};
use std::error::Error;
use std::fmt;

const KCL_HEADER_SIZE: usize = 0x38;
const KCL_PRISM_SIZE: usize = 0x10;
const PLC_HEADER_SIZE: usize = 0x08;
const PLC_CODE_SIZE: usize = 0x14;
const RARC_HEADER_SIZE: usize = 0x40;
const RARC_FILE_ENTRY_SIZE: usize = 0x14;
const MAX_DECOMPRESSED_ARCHIVE_SIZE: usize = 256 * 1024 * 1024;
// G_CM3D_F_ABS_MIN in c_m3d.cpp; this keeps offline prism reconstruction
// aligned with dBgWKCol::GetTriPnt's degeneracy decision.
const GAME_GEOMETRY_ABS_MIN: f32 = 32.0 * f32::EPSILON;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorldGeometryError(String);

impl WorldGeometryError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for WorldGeometryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for WorldGeometryError {}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vec3 {
    fn cross(self, rhs: Self) -> Self {
        Self {
            x: self.y * rhs.z - self.z * rhs.y,
            y: self.z * rhs.x - self.x * rhs.z,
            z: self.x * rhs.y - self.y * rhs.x,
        }
    }

    fn dot(self, rhs: Self) -> f32 {
        self.x * rhs.x + self.y * rhs.y + self.z * rhs.z
    }

    fn add_scaled(self, rhs: Self, scale: f32) -> Self {
        Self {
            x: self.x + rhs.x * scale,
            y: self.y + rhs.y * scale,
            z: self.z + rhs.z * scale,
        }
    }

    fn finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PointTriangleQuery {
    pub point: Vec3,
    pub signed_plane_distance: f32,
    pub closest_point: Vec3,
    pub distance: f32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KclSourceIndices {
    pub position: u16,
    pub face_normal: u16,
    pub edge_normal_1: u16,
    pub edge_normal_2: u16,
    pub edge_normal_3: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CollisionCode {
    pub raw: [u32; 5],
    pub exit_id: u8,
    pub polygon_color: u8,
    pub special_code: u8,
    pub link_no: u8,
    pub wall_code: u8,
    pub attribute_0: u8,
    pub attribute_1: u8,
    pub ground_code: u8,
    pub camera_move_background: u8,
    pub room_camera: u8,
    pub room_path: u8,
    pub room_path_point: u8,
    pub room_info: u8,
    pub sound_id: u8,
    pub room: u8,
}

impl CollisionCode {
    fn decode(raw: [u32; 5]) -> Self {
        Self {
            raw,
            exit_id: (raw[0] & 0x3f) as u8,
            polygon_color: ((raw[0] >> 6) & 0xff) as u8,
            special_code: ((raw[0] >> 24) & 0x0f) as u8,
            link_no: (raw[1] & 0xff) as u8,
            wall_code: ((raw[1] >> 8) & 0x0f) as u8,
            attribute_0: ((raw[1] >> 12) & 0x0f) as u8,
            attribute_1: ((raw[1] >> 16) & 0x07) as u8,
            ground_code: ((raw[1] >> 19) & 0x1f) as u8,
            camera_move_background: (raw[2] & 0xff) as u8,
            room_camera: ((raw[2] >> 8) & 0xff) as u8,
            room_path: ((raw[2] >> 16) & 0xff) as u8,
            room_path_point: ((raw[2] >> 24) & 0xff) as u8,
            room_info: (raw[4] & 0xff) as u8,
            sound_id: ((raw[4] >> 11) & 0xff) as u8,
            room: ((raw[4] >> 20) & 0xff) as u8,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CollisionPlane {
    pub anchor: Vec3,
    pub normal: Vec3,
    pub d: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KclPrismInspection {
    pub stable_id: String,
    pub prism_index: u16,
    pub height: f32,
    pub source_indices: KclSourceIndices,
    pub attribute: u16,
    pub code: CollisionCode,
    pub plane: CollisionPlane,
    pub triangle: [Vec3; 3],
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KclInspection {
    pub format: &'static str,
    pub kcl_sha256: Digest,
    pub plc_sha256: Digest,
    pub position_count: usize,
    pub normal_count: usize,
    /// Includes the reserved zero prism used as the block-list terminator.
    pub prism_table_count: usize,
    pub plc_code_count: usize,
    pub prism: KclPrismInspection,
}

#[derive(Clone, Copy)]
struct KclLayout {
    position_offset: usize,
    normal_offset: usize,
    prism_offset: usize,
    position_count: usize,
    normal_count: usize,
    prism_count: usize,
}

/// A validated, borrowed view over one KCL/PLC pair.
pub struct KclPlc<'a> {
    kcl: &'a [u8],
    plc: &'a [u8],
    layout: KclLayout,
    plc_count: usize,
    kcl_digest: Digest,
    plc_digest: Digest,
}

impl<'a> KclPlc<'a> {
    pub fn parse(kcl: &'a [u8], plc: &'a [u8]) -> Result<Self, WorldGeometryError> {
        let layout = parse_kcl_layout(kcl)?;
        let plc_count = parse_plc_layout(plc)?;
        Ok(Self {
            kcl,
            plc,
            layout,
            plc_count,
            kcl_digest: sha256(kcl),
            plc_digest: sha256(plc),
        })
    }

    pub fn inspect_prism(&self, prism_index: u16) -> Result<KclInspection, WorldGeometryError> {
        let prism_index = usize::from(prism_index);
        if prism_index == 0 {
            return Err(WorldGeometryError::new(
                "KCL prism index 0 is reserved as the block-list terminator",
            ));
        }
        if prism_index >= self.layout.prism_count {
            return Err(WorldGeometryError::new(format!(
                "KCL prism index {prism_index} is outside table count {}",
                self.layout.prism_count
            )));
        }

        let offset = checked_add(
            self.layout.prism_offset,
            checked_mul(prism_index, KCL_PRISM_SIZE, "KCL prism offset")?,
            "KCL prism offset",
        )?;
        let height = read_f32(self.kcl, offset, "KCL prism height")?;
        if !height.is_finite() {
            return Err(WorldGeometryError::new(format!(
                "KCL prism {prism_index} has a non-finite height"
            )));
        }
        let source_indices = KclSourceIndices {
            position: read_u16(self.kcl, offset + 4, "KCL position index")?,
            face_normal: read_u16(self.kcl, offset + 6, "KCL face-normal index")?,
            edge_normal_1: read_u16(self.kcl, offset + 8, "KCL edge-normal 1 index")?,
            edge_normal_2: read_u16(self.kcl, offset + 10, "KCL edge-normal 2 index")?,
            edge_normal_3: read_u16(self.kcl, offset + 12, "KCL edge-normal 3 index")?,
        };
        let attribute = read_u16(self.kcl, offset + 14, "KCL attribute")?;
        if usize::from(attribute) >= self.plc_count {
            return Err(WorldGeometryError::new(format!(
                "KCL prism {prism_index} attribute {attribute} is outside PLC code count {}",
                self.plc_count
            )));
        }

        let anchor = self.position(source_indices.position, prism_index)?;
        let face_normal = self.normal(source_indices.face_normal, prism_index, "face")?;
        let edge_1 = self.normal(source_indices.edge_normal_1, prism_index, "edge 1")?;
        let edge_2 = self.normal(source_indices.edge_normal_2, prism_index, "edge 2")?;
        let edge_3 = self.normal(source_indices.edge_normal_3, prism_index, "edge 3")?;

        // This is the same prism reconstruction used by dBgWKCol::GetTriPnt,
        // performed on immutable source bytes rather than through the game.
        let toward_vertex_2 = face_normal.cross(edge_1);
        let toward_vertex_1 = edge_2.cross(face_normal);
        let denominator_2 = toward_vertex_2.dot(edge_3);
        let denominator_1 = toward_vertex_1.dot(edge_3);
        if !denominator_1.is_finite()
            || !denominator_2.is_finite()
            || denominator_1.abs() < GAME_GEOMETRY_ABS_MIN
            || denominator_2.abs() < GAME_GEOMETRY_ABS_MIN
        {
            return Err(WorldGeometryError::new(format!(
                "KCL prism {prism_index} has degenerate edge normals"
            )));
        }
        let vertex_1 = anchor.add_scaled(toward_vertex_1, height / denominator_1);
        let vertex_2 = anchor.add_scaled(toward_vertex_2, height / denominator_2);
        if !vertex_1.finite() || !vertex_2.finite() {
            return Err(WorldGeometryError::new(format!(
                "KCL prism {prism_index} reconstructs non-finite vertices"
            )));
        }

        let code_offset = checked_add(
            PLC_HEADER_SIZE,
            checked_mul(usize::from(attribute), PLC_CODE_SIZE, "PLC code offset")?,
            "PLC code offset",
        )?;
        let mut raw_code = [0_u32; 5];
        for (index, word) in raw_code.iter_mut().enumerate() {
            *word = read_u32(self.plc, code_offset + index * 4, "PLC code word")?;
        }
        let code = CollisionCode::decode(raw_code);
        let stable_id = format!(
            "kcl-sha256:{}/plc-sha256:{}/prism/{prism_index}",
            self.kcl_digest, self.plc_digest
        );

        let plane_d = -face_normal.dot(anchor);
        if !plane_d.is_finite() {
            return Err(WorldGeometryError::new(format!(
                "KCL prism {prism_index} reconstructs a non-finite plane"
            )));
        }

        Ok(KclInspection {
            format: "dusklight-kcl-inspection/v1",
            kcl_sha256: self.kcl_digest,
            plc_sha256: self.plc_digest,
            position_count: self.layout.position_count,
            normal_count: self.layout.normal_count,
            prism_table_count: self.layout.prism_count,
            plc_code_count: self.plc_count,
            prism: KclPrismInspection {
                stable_id,
                prism_index: prism_index as u16,
                height,
                source_indices,
                attribute,
                code,
                plane: CollisionPlane {
                    anchor,
                    normal: face_normal,
                    d: plane_d,
                },
                triangle: [anchor, vertex_1, vertex_2],
            },
        })
    }

    fn position(&self, index: u16, prism: usize) -> Result<Vec3, WorldGeometryError> {
        let index = usize::from(index);
        if index >= self.layout.position_count {
            return Err(WorldGeometryError::new(format!(
                "KCL prism {prism} position index {index} is outside position count {}",
                self.layout.position_count
            )));
        }
        read_vec3(
            self.kcl,
            self.layout.position_offset + index * 12,
            "KCL position",
        )
    }

    fn normal(&self, index: u16, prism: usize, kind: &str) -> Result<Vec3, WorldGeometryError> {
        let index = usize::from(index);
        if index >= self.layout.normal_count {
            return Err(WorldGeometryError::new(format!(
                "KCL prism {prism} {kind} normal index {index} is outside normal count {}",
                self.layout.normal_count
            )));
        }
        read_vec3(
            self.kcl,
            self.layout.normal_offset + index * 12,
            "KCL normal",
        )
    }
}

/// Measures a point against an already reconstructed prism without consulting
/// the running game. Calculations use f64 intermediates so large world-space
/// coordinates do not unnecessarily erode the precision of source f32 data.
pub fn query_prism_point(
    prism: &KclPrismInspection,
    point: Vec3,
) -> Result<PointTriangleQuery, WorldGeometryError> {
    if !point.finite() {
        return Err(WorldGeometryError::new(
            "point query contains a non-finite component",
        ));
    }

    let point64 = Vec3d::from(point);
    let normal = Vec3d::from(prism.plane.normal);
    let normal_length = normal.length_squared().sqrt();
    if !normal_length.is_finite() || normal_length <= f64::EPSILON {
        return Err(WorldGeometryError::new(
            "collision plane has a zero or non-finite normal",
        ));
    }
    // The anchor form avoids cancellation from the serialized f32 plane D and
    // guarantees that querying the source anchor reports exactly zero.
    let signed_plane_distance =
        normal.dot(point64 - Vec3d::from(prism.plane.anchor)) / normal_length;
    let closest = closest_point_on_triangle(point64, prism.triangle.map(Vec3d::from));
    let distance = (point64 - closest).length_squared().sqrt();
    let result = PointTriangleQuery {
        point,
        signed_plane_distance: signed_plane_distance as f32,
        closest_point: closest.to_vec3(),
        distance: distance as f32,
    };
    if !result.signed_plane_distance.is_finite()
        || !result.closest_point.finite()
        || !result.distance.is_finite()
    {
        return Err(WorldGeometryError::new(
            "point query produced a non-finite result",
        ));
    }
    Ok(result)
}

#[derive(Clone, Copy)]
struct Vec3d {
    x: f64,
    y: f64,
    z: f64,
}

impl Vec3d {
    fn dot(self, rhs: Self) -> f64 {
        self.x * rhs.x + self.y * rhs.y + self.z * rhs.z
    }

    fn cross(self, rhs: Self) -> Self {
        Self {
            x: self.y * rhs.z - self.z * rhs.y,
            y: self.z * rhs.x - self.x * rhs.z,
            z: self.x * rhs.y - self.y * rhs.x,
        }
    }

    fn length_squared(self) -> f64 {
        self.dot(self)
    }

    fn to_vec3(self) -> Vec3 {
        Vec3 {
            x: self.x as f32,
            y: self.y as f32,
            z: self.z as f32,
        }
    }
}

impl From<Vec3> for Vec3d {
    fn from(value: Vec3) -> Self {
        Self {
            x: f64::from(value.x),
            y: f64::from(value.y),
            z: f64::from(value.z),
        }
    }
}

impl std::ops::Add for Vec3d {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        Self {
            x: self.x + rhs.x,
            y: self.y + rhs.y,
            z: self.z + rhs.z,
        }
    }
}

impl std::ops::Sub for Vec3d {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self::Output {
        Self {
            x: self.x - rhs.x,
            y: self.y - rhs.y,
            z: self.z - rhs.z,
        }
    }
}

impl std::ops::Mul<f64> for Vec3d {
    type Output = Self;

    fn mul(self, rhs: f64) -> Self::Output {
        Self {
            x: self.x * rhs,
            y: self.y * rhs,
            z: self.z * rhs,
        }
    }
}

fn closest_point_on_segment(point: Vec3d, start: Vec3d, end: Vec3d) -> Vec3d {
    let edge = end - start;
    let length_squared = edge.length_squared();
    if length_squared <= f64::EPSILON {
        return start;
    }
    start + edge * ((point - start).dot(edge) / length_squared).clamp(0.0, 1.0)
}

fn closest_point_on_triangle(point: Vec3d, triangle: [Vec3d; 3]) -> Vec3d {
    let [a, b, c] = triangle;
    let ab = b - a;
    let ac = c - a;
    let bc = c - b;
    let edge_scale = ab
        .length_squared()
        .max(ac.length_squared())
        .max(bc.length_squared());
    let twice_area_squared = ab.cross(ac).length_squared();

    // The usual Voronoi-region formula divides by triangle area. Collapse to
    // the best of the three bounded segments when the source triangle is
    // degenerate or too ill-conditioned for that division.
    if edge_scale <= f64::EPSILON
        || twice_area_squared <= 64.0 * f64::EPSILON * edge_scale * edge_scale
    {
        return [(a, b), (b, c), (c, a)]
            .map(|(start, end)| closest_point_on_segment(point, start, end))
            .into_iter()
            .min_by(|lhs, rhs| {
                (point - *lhs)
                    .length_squared()
                    .total_cmp(&(point - *rhs).length_squared())
            })
            .expect("three triangle edges");
    }

    // Voronoi-region tests from Real-Time Collision Detection, section 5.1.5.
    let ap = point - a;
    let d1 = ab.dot(ap);
    let d2 = ac.dot(ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }

    let bp = point - b;
    let d3 = ab.dot(bp);
    let d4 = ac.dot(bp);
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }

    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        return a + ab * (d1 / (d1 - d3));
    }

    let cp = point - c;
    let d5 = ab.dot(cp);
    let d6 = ac.dot(cp);
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }

    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        return a + ac * (d2 / (d2 - d6));
    }

    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && d4 - d3 >= 0.0 && d5 - d6 >= 0.0 {
        return b + bc * ((d4 - d3) / ((d4 - d3) + (d5 - d6)));
    }

    let inverse_sum = 1.0 / (va + vb + vc);
    a + ab * (vb * inverse_sum) + ac * (vc * inverse_sum)
}

/// Extracts one named file from an uncompressed RARC or a Yaz0-wrapped RARC.
/// The returned bytes are newly owned; the archive input remains untouched.
pub fn extract_rarc_resource(
    archive: &[u8],
    resource_name: &str,
) -> Result<Vec<u8>, WorldGeometryError> {
    if resource_name.is_empty() || resource_name.as_bytes().contains(&0) {
        return Err(WorldGeometryError::new(
            "invalid empty/NUL RARC resource name",
        ));
    }
    let decoded = if archive.starts_with(b"Yaz0") {
        decode_yaz0(archive)?
    } else {
        archive.to_vec()
    };
    extract_uncompressed_rarc_resource(&decoded, resource_name)
}

fn parse_kcl_layout(kcl: &[u8]) -> Result<KclLayout, WorldGeometryError> {
    require_range(kcl, 0, KCL_HEADER_SIZE, "KCL header")?;
    let position_offset = read_u32(kcl, 0, "KCL position offset")? as usize;
    let normal_offset = read_u32(kcl, 4, "KCL normal offset")? as usize;
    let prism_offset = read_u32(kcl, 8, "KCL prism offset")? as usize;
    let block_offset = read_u32(kcl, 12, "KCL block offset")? as usize;
    if position_offset < KCL_HEADER_SIZE
        || position_offset > normal_offset
        || normal_offset > prism_offset
        || prism_offset > block_offset
        || block_offset > kcl.len()
    {
        return Err(WorldGeometryError::new(
            "KCL table offsets are out of order or outside the file",
        ));
    }
    let position_bytes = normal_offset - position_offset;
    let normal_bytes = prism_offset - normal_offset;
    let prism_bytes = block_offset - prism_offset;
    // Retail KCL resources may contain up to eleven opaque alignment bytes
    // between the Vec tables and the next table (F_SP103 room 1 has eight).
    // Counts therefore use complete Vec records; the prism span must remain
    // exact because runtime block lists address that table by record index.
    if !prism_bytes.is_multiple_of(KCL_PRISM_SIZE) {
        return Err(WorldGeometryError::new(
            "KCL prism-table span is not aligned to its record size",
        ));
    }
    let position_count = position_bytes / 12;
    let normal_count = normal_bytes / 12;
    if position_count == 0 || normal_count == 0 {
        return Err(WorldGeometryError::new(
            "KCL position and normal tables must contain complete records",
        ));
    }
    let prism_count = prism_bytes / KCL_PRISM_SIZE;
    if prism_count < 2 || prism_count > usize::from(u16::MAX) + 1 {
        return Err(WorldGeometryError::new(
            "KCL prism table must contain a reserved entry and at least one addressable prism",
        ));
    }
    Ok(KclLayout {
        position_offset,
        normal_offset,
        prism_offset,
        position_count,
        normal_count,
        prism_count,
    })
}

fn parse_plc_layout(plc: &[u8]) -> Result<usize, WorldGeometryError> {
    require_range(plc, 0, PLC_HEADER_SIZE, "PLC header")?;
    if &plc[0..4] != b"SPLC" {
        return Err(WorldGeometryError::new("PLC magic is not SPLC"));
    }
    let code_size = usize::from(read_u16(plc, 4, "PLC code size")?);
    if code_size != PLC_CODE_SIZE {
        return Err(WorldGeometryError::new(format!(
            "unsupported PLC code size {code_size}; expected {PLC_CODE_SIZE}"
        )));
    }
    let code_count = usize::from(read_u16(plc, 6, "PLC code count")?);
    let required = checked_add(
        PLC_HEADER_SIZE,
        checked_mul(code_count, code_size, "PLC table size")?,
        "PLC table size",
    )?;
    require_range(plc, 0, required, "PLC code table")?;
    Ok(code_count)
}

fn decode_yaz0(input: &[u8]) -> Result<Vec<u8>, WorldGeometryError> {
    require_range(input, 0, 16, "Yaz0 header")?;
    if &input[0..4] != b"Yaz0" {
        return Err(WorldGeometryError::new("Yaz0 magic is missing"));
    }
    let output_size = read_u32(input, 4, "Yaz0 output size")? as usize;
    if output_size > MAX_DECOMPRESSED_ARCHIVE_SIZE {
        return Err(WorldGeometryError::new(format!(
            "Yaz0 output size {output_size} exceeds offline limit {MAX_DECOMPRESSED_ARCHIVE_SIZE}"
        )));
    }
    let mut output = Vec::with_capacity(output_size);
    let mut cursor = 16_usize;
    let mut code = 0_u8;
    let mut remaining_bits = 0_u8;
    while output.len() < output_size {
        if remaining_bits == 0 {
            code = *input
                .get(cursor)
                .ok_or_else(|| WorldGeometryError::new("truncated Yaz0 code byte"))?;
            cursor += 1;
            remaining_bits = 8;
        }
        if code & 0x80 != 0 {
            let byte = *input
                .get(cursor)
                .ok_or_else(|| WorldGeometryError::new("truncated Yaz0 literal"))?;
            cursor += 1;
            output.push(byte);
        } else {
            let first = *input
                .get(cursor)
                .ok_or_else(|| WorldGeometryError::new("truncated Yaz0 back-reference"))?;
            let second = *input
                .get(cursor + 1)
                .ok_or_else(|| WorldGeometryError::new("truncated Yaz0 back-reference"))?;
            cursor += 2;
            let distance = usize::from((u16::from(first & 0x0f) << 8) | u16::from(second)) + 1;
            if distance > output.len() {
                return Err(WorldGeometryError::new(
                    "invalid Yaz0 back-reference distance",
                ));
            }
            let mut length = usize::from(first >> 4);
            if length == 0 {
                length = usize::from(
                    *input
                        .get(cursor)
                        .ok_or_else(|| WorldGeometryError::new("truncated Yaz0 long length"))?,
                ) + 0x12;
                cursor += 1;
            } else {
                length += 2;
            }
            if length > output_size - output.len() {
                return Err(WorldGeometryError::new(
                    "Yaz0 back-reference exceeds declared output size",
                ));
            }
            for _ in 0..length {
                let source = output.len() - distance;
                let byte = output[source];
                output.push(byte);
            }
        }
        code <<= 1;
        remaining_bits -= 1;
    }
    Ok(output)
}

fn extract_uncompressed_rarc_resource(
    archive: &[u8],
    resource_name: &str,
) -> Result<Vec<u8>, WorldGeometryError> {
    require_range(archive, 0, RARC_HEADER_SIZE, "RARC header")?;
    if &archive[0..4] != b"RARC" {
        return Err(WorldGeometryError::new(
            "archive is neither RARC nor Yaz0-wrapped RARC",
        ));
    }
    let info_base = 0x20_usize;
    let file_count = read_u32(archive, info_base + 8, "RARC file count")? as usize;
    let file_table = relative_offset(
        info_base,
        read_u32(archive, info_base + 12, "RARC file-table offset")?,
        "RARC file table",
    )?;
    let string_table = relative_offset(
        info_base,
        read_u32(archive, info_base + 20, "RARC string-table offset")?,
        "RARC string table",
    )?;
    let data_base = relative_offset(
        info_base,
        read_u32(archive, 12, "RARC data offset")?,
        "RARC data",
    )?;
    require_range(
        archive,
        file_table,
        checked_mul(file_count, RARC_FILE_ENTRY_SIZE, "RARC file table")?,
        "RARC file table",
    )?;
    if string_table >= archive.len() || data_base > archive.len() {
        return Err(WorldGeometryError::new(
            "RARC table offset is outside the archive",
        ));
    }

    let mut match_range = None;
    for index in 0..file_count {
        let entry = file_table + index * RARC_FILE_ENTRY_SIZE;
        let flags = read_u16(archive, entry + 4, "RARC entry flags")?;
        if flags & 0x0100 == 0 {
            continue;
        }
        let name_offset = usize::from(read_u16(archive, entry + 6, "RARC name offset")?);
        let name_start = checked_add(string_table, name_offset, "RARC resource name")?;
        let name = nul_terminated(archive, name_start, "RARC resource name")?;
        if name != resource_name.as_bytes() {
            continue;
        }
        let resource_offset = relative_offset(
            data_base,
            read_u32(archive, entry + 8, "RARC resource offset")?,
            "RARC resource",
        )?;
        let resource_size = read_u32(archive, entry + 12, "RARC resource size")? as usize;
        require_range(archive, resource_offset, resource_size, "RARC resource")?;
        if match_range
            .replace((resource_offset, resource_size))
            .is_some()
        {
            return Err(WorldGeometryError::new(format!(
                "RARC contains multiple files named {resource_name:?}"
            )));
        }
    }
    let (offset, size) = match_range.ok_or_else(|| {
        WorldGeometryError::new(format!("RARC resource {resource_name:?} was not found"))
    })?;
    Ok(archive[offset..offset + size].to_vec())
}

fn sha256(bytes: &[u8]) -> Digest {
    Digest(Sha256::digest(bytes).into())
}

fn relative_offset(base: usize, relative: u32, context: &str) -> Result<usize, WorldGeometryError> {
    checked_add(base, relative as usize, context)
}

fn checked_add(lhs: usize, rhs: usize, context: &str) -> Result<usize, WorldGeometryError> {
    lhs.checked_add(rhs)
        .ok_or_else(|| WorldGeometryError::new(format!("{context} offset overflow")))
}

fn checked_mul(lhs: usize, rhs: usize, context: &str) -> Result<usize, WorldGeometryError> {
    lhs.checked_mul(rhs)
        .ok_or_else(|| WorldGeometryError::new(format!("{context} size overflow")))
}

fn require_range(
    bytes: &[u8],
    offset: usize,
    size: usize,
    context: &str,
) -> Result<(), WorldGeometryError> {
    let end = checked_add(offset, size, context)?;
    if end > bytes.len() {
        return Err(WorldGeometryError::new(format!(
            "{context} range {offset:#x}..{end:#x} exceeds file size {:#x}",
            bytes.len()
        )));
    }
    Ok(())
}

fn read_u16(bytes: &[u8], offset: usize, context: &str) -> Result<u16, WorldGeometryError> {
    require_range(bytes, offset, 2, context)?;
    Ok(u16::from_be_bytes(
        bytes[offset..offset + 2].try_into().unwrap(),
    ))
}

fn read_u32(bytes: &[u8], offset: usize, context: &str) -> Result<u32, WorldGeometryError> {
    require_range(bytes, offset, 4, context)?;
    Ok(u32::from_be_bytes(
        bytes[offset..offset + 4].try_into().unwrap(),
    ))
}

fn read_f32(bytes: &[u8], offset: usize, context: &str) -> Result<f32, WorldGeometryError> {
    Ok(f32::from_bits(read_u32(bytes, offset, context)?))
}

fn read_vec3(bytes: &[u8], offset: usize, context: &str) -> Result<Vec3, WorldGeometryError> {
    let value = Vec3 {
        x: read_f32(bytes, offset, context)?,
        y: read_f32(bytes, offset + 4, context)?,
        z: read_f32(bytes, offset + 8, context)?,
    };
    if !value.finite() {
        return Err(WorldGeometryError::new(format!(
            "{context} contains a non-finite component"
        )));
    }
    Ok(value)
}

fn nul_terminated<'a>(
    bytes: &'a [u8],
    start: usize,
    context: &str,
) -> Result<&'a [u8], WorldGeometryError> {
    let tail = bytes
        .get(start..)
        .ok_or_else(|| WorldGeometryError::new(format!("{context} starts outside file")))?;
    let length = tail
        .iter()
        .position(|byte| *byte == 0)
        .ok_or_else(|| WorldGeometryError::new(format!("unterminated {context}")))?;
    Ok(&tail[..length])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put_u16(bytes: &mut [u8], offset: usize, value: u16) {
        bytes[offset..offset + 2].copy_from_slice(&value.to_be_bytes());
    }

    fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
    }

    fn put_f32(bytes: &mut [u8], offset: usize, value: f32) {
        put_u32(bytes, offset, value.to_bits());
    }

    fn put_vec3(bytes: &mut [u8], offset: usize, value: Vec3) {
        put_f32(bytes, offset, value.x);
        put_f32(bytes, offset + 4, value.y);
        put_f32(bytes, offset + 8, value.z);
    }

    fn fixture() -> (Vec<u8>, Vec<u8>) {
        let position_offset = KCL_HEADER_SIZE;
        let normal_offset = position_offset + 12;
        let prism_offset = normal_offset + 4 * 12;
        let block_offset = prism_offset + 2 * KCL_PRISM_SIZE;
        let mut kcl = vec![0_u8; block_offset + 4];
        put_u32(&mut kcl, 0, position_offset as u32);
        put_u32(&mut kcl, 4, normal_offset as u32);
        put_u32(&mut kcl, 8, prism_offset as u32);
        put_u32(&mut kcl, 12, block_offset as u32);
        put_vec3(
            &mut kcl,
            position_offset,
            Vec3 {
                x: 10.0,
                y: 20.0,
                z: 30.0,
            },
        );
        for (index, normal) in [
            Vec3 {
                x: 0.0,
                y: 1.0,
                z: 0.0,
            },
            Vec3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
            Vec3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
            Vec3 {
                x: 1.0,
                y: 0.0,
                z: 1.0,
            },
        ]
        .into_iter()
        .enumerate()
        {
            put_vec3(&mut kcl, normal_offset + index * 12, normal);
        }
        let prism = prism_offset + KCL_PRISM_SIZE;
        put_f32(&mut kcl, prism, 1.0);
        for (offset, value) in [(4, 0), (6, 0), (8, 1), (10, 2), (12, 3), (14, 0)] {
            put_u16(&mut kcl, prism + offset, value);
        }

        let mut plc = vec![0_u8; PLC_HEADER_SIZE + PLC_CODE_SIZE];
        plc[0..4].copy_from_slice(b"SPLC");
        put_u16(&mut plc, 4, PLC_CODE_SIZE as u16);
        put_u16(&mut plc, 6, 1);
        for (index, word) in [
            1 | (7 << 6) | (8 << 24),
            9 | (2 << 8) | (3 << 12) | (4 << 16) | (5 << 19),
            6 | (7 << 8) | (8 << 16) | (9 << 24),
            0x1234_5678,
            10 | (11 << 11) | (12 << 20),
        ]
        .into_iter()
        .enumerate()
        {
            put_u32(&mut plc, PLC_HEADER_SIZE + index * 4, word);
        }
        (kcl, plc)
    }

    #[test]
    fn inspects_content_addressed_prism_geometry_and_raw_code() {
        let (kcl, plc) = fixture();
        let inspection = KclPlc::parse(&kcl, &plc).unwrap().inspect_prism(1).unwrap();
        assert_eq!(inspection.position_count, 1);
        assert_eq!(inspection.normal_count, 4);
        assert_eq!(inspection.prism_table_count, 2);
        assert_eq!(inspection.plc_code_count, 1);
        assert_eq!(inspection.kcl_sha256, sha256(&kcl));
        assert_eq!(inspection.plc_sha256, sha256(&plc));
        assert_eq!(inspection.prism.attribute, 0);
        assert_eq!(inspection.prism.code.exit_id, 1);
        assert_eq!(inspection.prism.code.raw[3], 0x1234_5678);
        assert_eq!(inspection.prism.code.ground_code, 5);
        assert_eq!(inspection.prism.code.room, 12);
        assert_eq!(inspection.prism.plane.d, -20.0);
        assert_eq!(
            inspection.prism.triangle,
            [
                Vec3 {
                    x: 10.0,
                    y: 20.0,
                    z: 30.0
                },
                Vec3 {
                    x: 10.0,
                    y: 20.0,
                    z: 31.0
                },
                Vec3 {
                    x: 11.0,
                    y: 20.0,
                    z: 30.0
                },
            ]
        );
        assert!(inspection.prism.stable_id.starts_with("kcl-sha256:"));
    }

    #[test]
    fn measures_point_to_prism_plane_and_triangle() {
        let (kcl, plc) = fixture();
        let prism = KclPlc::parse(&kcl, &plc)
            .unwrap()
            .inspect_prism(1)
            .unwrap()
            .prism;
        let query = query_prism_point(
            &prism,
            Vec3 {
                x: 10.25,
                y: 22.0,
                z: 30.25,
            },
        )
        .unwrap();
        assert_eq!(query.signed_plane_distance, 2.0);
        assert_eq!(
            query.closest_point,
            Vec3 {
                x: 10.25,
                y: 20.0,
                z: 30.25,
            }
        );
        assert_eq!(query.distance, 2.0);

        let outside = query_prism_point(
            &prism,
            Vec3 {
                x: 12.0,
                y: 22.0,
                z: 32.0,
            },
        )
        .unwrap();
        assert_eq!(
            outside.closest_point,
            Vec3 {
                x: 10.5,
                y: 20.0,
                z: 30.5,
            }
        );
        assert!((outside.distance - 8.5_f32.sqrt()).abs() < 1.0e-6);
    }

    #[test]
    fn closest_point_falls_back_for_degenerate_triangle() {
        let point = Vec3d {
            x: 1.0,
            y: 2.0,
            z: 0.0,
        };
        let closest = closest_point_on_triangle(
            point,
            [
                Vec3d {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                Vec3d {
                    x: 2.0,
                    y: 0.0,
                    z: 0.0,
                },
                Vec3d {
                    x: 4.0,
                    y: 0.0,
                    z: 0.0,
                },
            ],
        );
        assert_eq!(closest.x, 1.0);
        assert_eq!(closest.y, 0.0);
        assert_eq!(closest.z, 0.0);
    }

    #[test]
    fn rejects_reserved_out_of_range_and_cross_file_indices() {
        let (kcl, plc) = fixture();
        let parsed = KclPlc::parse(&kcl, &plc).unwrap();
        assert!(
            parsed
                .inspect_prism(0)
                .unwrap_err()
                .to_string()
                .contains("reserved")
        );
        assert!(
            parsed
                .inspect_prism(2)
                .unwrap_err()
                .to_string()
                .contains("outside table")
        );

        let mut invalid_attribute = kcl;
        let prism_offset = read_u32(&invalid_attribute, 8, "test").unwrap() as usize;
        put_u16(
            &mut invalid_attribute,
            prism_offset + KCL_PRISM_SIZE + 14,
            1,
        );
        assert!(
            KclPlc::parse(&invalid_attribute, &plc)
                .unwrap()
                .inspect_prism(1)
                .unwrap_err()
                .to_string()
                .contains("outside PLC")
        );
    }

    #[test]
    fn rejects_degenerate_geometry_and_invalid_table_order() {
        let (mut kcl, plc) = fixture();
        let normal_offset = read_u32(&kcl, 4, "test").unwrap() as usize;
        put_vec3(
            &mut kcl,
            normal_offset + 3 * 12,
            Vec3 {
                x: 0.0,
                y: 1.0,
                z: 0.0,
            },
        );
        assert!(
            KclPlc::parse(&kcl, &plc)
                .unwrap()
                .inspect_prism(1)
                .unwrap_err()
                .to_string()
                .contains("degenerate")
        );

        put_u32(&mut kcl, 8, (normal_offset - 1) as u32);
        assert!(KclPlc::parse(&kcl, &plc).is_err());
    }

    fn rarc_with(name: &str, resource: &[u8]) -> Vec<u8> {
        let file_table = 0x40_usize;
        let string_table = 0x60_usize;
        let data = 0x80_usize;
        let mut archive = vec![0_u8; data + resource.len()];
        archive[0..4].copy_from_slice(b"RARC");
        put_u32(&mut archive, 12, (data - 0x20) as u32);
        put_u32(&mut archive, 0x28, 1);
        put_u32(&mut archive, 0x2c, (file_table - 0x20) as u32);
        put_u32(&mut archive, 0x34, (string_table - 0x20) as u32);
        put_u16(&mut archive, file_table + 4, 0x0100);
        put_u32(&mut archive, file_table + 8, 0);
        put_u32(&mut archive, file_table + 12, resource.len() as u32);
        archive[string_table..string_table + name.len()].copy_from_slice(name.as_bytes());
        archive[string_table + name.len()] = 0;
        archive[data..].copy_from_slice(resource);
        archive
    }

    fn yaz0_literals(source: &[u8]) -> Vec<u8> {
        let mut output = Vec::with_capacity(16 + source.len() + source.len().div_ceil(8));
        output.extend_from_slice(b"Yaz0");
        output.extend_from_slice(&(source.len() as u32).to_be_bytes());
        output.extend_from_slice(&[0; 8]);
        for chunk in source.chunks(8) {
            output.push(0xff);
            output.extend_from_slice(chunk);
        }
        output
    }

    #[test]
    fn extracts_named_resource_from_rarc_and_yaz0_without_mutating_input() {
        let rarc = rarc_with("room.kcl", b"immutable collision bytes");
        let original = rarc.clone();
        assert_eq!(
            extract_rarc_resource(&rarc, "room.kcl").unwrap(),
            b"immutable collision bytes"
        );
        assert_eq!(rarc, original);
        assert_eq!(
            extract_rarc_resource(&yaz0_literals(&rarc), "room.kcl").unwrap(),
            b"immutable collision bytes"
        );
        assert!(extract_rarc_resource(&rarc, "room.plc").is_err());
    }

    #[test]
    fn rejects_truncated_yaz0_and_oversized_declared_output() {
        assert!(extract_rarc_resource(b"Yaz0", "room.kcl").is_err());
        let mut oversized = vec![0_u8; 16];
        oversized[0..4].copy_from_slice(b"Yaz0");
        put_u32(
            &mut oversized,
            4,
            (MAX_DECOMPRESSED_ARCHIVE_SIZE as u32) + 1,
        );
        assert!(extract_rarc_resource(&oversized, "room.kcl").is_err());
    }
}
