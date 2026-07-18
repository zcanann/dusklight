//! Canonical, bounded scenario loadouts for deterministic stage fixtures.

use serde::{Deserialize, Serialize};
use std::error::Error;
use std::fmt;

pub const SCENARIO_FIXTURE_SCHEMA: &str = "dusklight-scenario-fixture/v1";
pub const MAGIC: [u8; 8] = *b"DUSKFXTR";
pub const MAJOR_VERSION: u16 = 1;
pub const MINOR_VERSION: u16 = 0;
pub const HEADER_SIZE: usize = 32;
const RECORD_HEADER_SIZE: usize = 8;
const MAX_ENCODED_SIZE: usize = u16::MAX as usize;
const MAX_NAME_BYTES: usize = 64;
const MAX_SETTING_KEY_BYTES: usize = 96;
const MAX_SETTING_STRING_BYTES: usize = 1024;
const MAX_INVENTORY: usize = 256;
const MAX_EQUIPMENT: usize = 64;
const MAX_FLAGS: usize = 4096;
const MAX_SETTINGS: usize = 256;

const TAG_NAME: u16 = 1;
const TAG_FORM: u16 = 2;
const TAG_HEALTH: u16 = 3;
const TAG_RNG: u16 = 4;
const TAG_VIDEO_MODE: u16 = 5;
const TAG_INVENTORY: u16 = 6;
const TAG_EQUIPMENT: u16 = 7;
const TAG_FLAG: u16 = 8;
const TAG_SETTING: u16 = 9;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlayerForm {
    Human,
    Wolf,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FixtureVideoMode {
    Automatic,
    NtscInterlaced,
    NtscProgressive,
    Pal50,
    Pal60,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RngStreamId {
    Primary,
    Secondary,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HealthFixture {
    pub current: u16,
    pub maximum: u16,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RngFixture {
    pub stream: RngStreamId,
    pub state0: i32,
    pub state1: i32,
    pub state2: i32,
    pub call_count: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InventoryFixture {
    pub slot: u16,
    pub item: u16,
    #[serde(default = "one")]
    pub quantity: u16,
}

const fn one() -> u16 {
    1
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EquipmentFixture {
    pub slot: u16,
    pub item: u16,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FixtureFlagDomain {
    Event,
    Temporary,
    Dungeon,
    Switch,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FlagFixture {
    pub domain: FixtureFlagDomain,
    /// -1 denotes a global flag; switch and room-local flag domains use 0-63.
    #[serde(default = "global_room")]
    pub room: i8,
    pub index: u16,
    pub value: bool,
}

const fn global_room() -> i8 {
    -1
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum FixtureSettingValue {
    Boolean {
        value: bool,
    },
    Integer {
        value: i64,
    },
    /// Exact IEEE-754 binary64 bits. This avoids cross-language decimal drift.
    FloatBits {
        bits: u64,
    },
    String {
        value: String,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SettingFixture {
    pub key: String,
    pub value: FixtureSettingValue,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScenarioFixture {
    pub schema: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub form: Option<PlayerForm>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health: Option<HealthFixture>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rng: Vec<RngFixture>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_mode: Option<FixtureVideoMode>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inventory: Vec<InventoryFixture>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub equipment: Vec<EquipmentFixture>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub flags: Vec<FlagFixture>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub settings: Vec<SettingFixture>,
}

impl ScenarioFixture {
    pub fn validate(&self) -> Result<(), FixtureError> {
        if self.schema != SCENARIO_FIXTURE_SCHEMA {
            return Err(FixtureError::InvalidSchema);
        }
        validate_name(&self.name)?;
        if self.health.is_some_and(|health| {
            health.maximum == 0 || health.current == 0 || health.current > health.maximum
        }) {
            return Err(FixtureError::InvalidHealth);
        }
        if self.rng.len() > 2
            || self.inventory.len() > MAX_INVENTORY
            || self.equipment.len() > MAX_EQUIPMENT
            || self.flags.len() > MAX_FLAGS
            || self.settings.len() > MAX_SETTINGS
        {
            return Err(FixtureError::LimitExceeded);
        }

        let mut rng = self.rng.clone();
        rng.sort_unstable();
        reject_duplicate_keys(&rng, |entry| entry.stream)?;

        let mut inventory = self.inventory.clone();
        inventory.sort_unstable();
        if inventory.iter().any(|entry| entry.quantity == 0) {
            return Err(FixtureError::InvalidInventory);
        }
        reject_duplicate_keys(&inventory, |entry| entry.slot)?;

        let mut equipment = self.equipment.clone();
        equipment.sort_unstable();
        reject_duplicate_keys(&equipment, |entry| entry.slot)?;

        let mut flags = self.flags.clone();
        flags.sort_unstable();
        if flags.iter().any(|flag| {
            !(-1..=63).contains(&flag.room)
                || (flag.domain == FixtureFlagDomain::Switch && flag.room < 0)
        }) {
            return Err(FixtureError::InvalidFlag);
        }
        reject_duplicate_keys(&flags, |entry| (entry.domain, entry.room, entry.index))?;

        let mut setting_keys = self
            .settings
            .iter()
            .map(|setting| setting.key.as_str())
            .collect::<Vec<_>>();
        setting_keys.sort_unstable();
        if setting_keys.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(FixtureError::DuplicateKey);
        }
        for setting in &self.settings {
            validate_setting(setting)?;
        }
        Ok(())
    }

    /// Emits canonical little-endian TLV records. Collection order in JSON is
    /// irrelevant; the wire representation sorts every keyed collection.
    pub fn encode(&self) -> Result<Vec<u8>, FixtureError> {
        self.validate()?;
        let mut records = Vec::new();
        push_record(&mut records, TAG_NAME, self.name.as_bytes())?;
        if let Some(form) = self.form {
            push_record(&mut records, TAG_FORM, &[encode_form(form), 0, 0, 0])?;
        }
        if let Some(health) = self.health {
            let mut payload = [0_u8; 4];
            put_u16(&mut payload[0..2], health.current);
            put_u16(&mut payload[2..4], health.maximum);
            push_record(&mut records, TAG_HEALTH, &payload)?;
        }
        let mut rng = self.rng.clone();
        rng.sort_unstable();
        for entry in rng {
            let mut payload = [0_u8; 24];
            payload[0] = encode_rng_stream(entry.stream);
            put_i32(&mut payload[4..8], entry.state0);
            put_i32(&mut payload[8..12], entry.state1);
            put_i32(&mut payload[12..16], entry.state2);
            put_u64(&mut payload[16..24], entry.call_count);
            push_record(&mut records, TAG_RNG, &payload)?;
        }
        if let Some(mode) = self.video_mode {
            push_record(
                &mut records,
                TAG_VIDEO_MODE,
                &[encode_video_mode(mode), 0, 0, 0],
            )?;
        }
        let mut inventory = self.inventory.clone();
        inventory.sort_unstable();
        for entry in inventory {
            let mut payload = [0_u8; 8];
            put_u16(&mut payload[0..2], entry.slot);
            put_u16(&mut payload[2..4], entry.item);
            put_u16(&mut payload[4..6], entry.quantity);
            push_record(&mut records, TAG_INVENTORY, &payload)?;
        }
        let mut equipment = self.equipment.clone();
        equipment.sort_unstable();
        for entry in equipment {
            let mut payload = [0_u8; 4];
            put_u16(&mut payload[0..2], entry.slot);
            put_u16(&mut payload[2..4], entry.item);
            push_record(&mut records, TAG_EQUIPMENT, &payload)?;
        }
        let mut flags = self.flags.clone();
        flags.sort_unstable();
        for entry in flags {
            let mut payload = [0_u8; 8];
            payload[0] = encode_flag_domain(entry.domain);
            payload[1] = entry.room as u8;
            payload[2] = u8::from(entry.value);
            put_u16(&mut payload[4..6], entry.index);
            push_record(&mut records, TAG_FLAG, &payload)?;
        }
        let mut settings = self.settings.iter().collect::<Vec<_>>();
        settings.sort_unstable_by(|left, right| left.key.cmp(&right.key));
        for setting in settings {
            let (kind, value) = encode_setting_value(&setting.value);
            let mut payload = Vec::with_capacity(4 + setting.key.len() + value.len());
            payload.push(setting.key.len() as u8);
            payload.push(kind);
            payload.extend_from_slice(&(value.len() as u16).to_le_bytes());
            payload.extend_from_slice(setting.key.as_bytes());
            payload.extend_from_slice(&value);
            push_record(&mut records, TAG_SETTING, &payload)?;
        }

        let total = HEADER_SIZE
            .checked_add(records.len())
            .ok_or(FixtureError::LimitExceeded)?;
        if total > MAX_ENCODED_SIZE {
            return Err(FixtureError::LimitExceeded);
        }
        let record_count = count_records(&records)?;
        let mut output = vec![0_u8; HEADER_SIZE];
        output[..8].copy_from_slice(&MAGIC);
        put_u16(&mut output[8..10], MAJOR_VERSION);
        put_u16(&mut output[10..12], MINOR_VERSION);
        put_u16(&mut output[12..14], HEADER_SIZE as u16);
        put_u16(&mut output[14..16], record_count);
        put_u32(&mut output[16..20], total as u32);
        output.extend_from_slice(&records);
        Ok(output)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, FixtureError> {
        if bytes.len() < HEADER_SIZE {
            return Err(FixtureError::Truncated);
        }
        if bytes[..8] != MAGIC {
            return Err(FixtureError::BadMagic);
        }
        if get_u16(&bytes[8..10]) != MAJOR_VERSION || get_u16(&bytes[10..12]) != MINOR_VERSION {
            return Err(FixtureError::UnsupportedVersion);
        }
        if get_u16(&bytes[12..14]) as usize != HEADER_SIZE
            || bytes[20..HEADER_SIZE].iter().any(|byte| *byte != 0)
        {
            return Err(FixtureError::InvalidHeader);
        }
        if get_u32(&bytes[16..20]) as usize != bytes.len() || bytes.len() > MAX_ENCODED_SIZE {
            return Err(FixtureError::InvalidSize);
        }
        let declared_records = get_u16(&bytes[14..16]) as usize;
        let mut cursor = HEADER_SIZE;
        let mut records = 0_usize;
        let mut fixture = ScenarioFixture {
            schema: SCENARIO_FIXTURE_SCHEMA.into(),
            name: String::new(),
            form: None,
            health: None,
            rng: Vec::new(),
            video_mode: None,
            inventory: Vec::new(),
            equipment: Vec::new(),
            flags: Vec::new(),
            settings: Vec::new(),
        };
        while cursor < bytes.len() {
            if bytes.len() - cursor < RECORD_HEADER_SIZE {
                return Err(FixtureError::Truncated);
            }
            let tag = get_u16(&bytes[cursor..cursor + 2]);
            let flags = get_u16(&bytes[cursor + 2..cursor + 4]);
            let length = get_u32(&bytes[cursor + 4..cursor + 8]) as usize;
            if flags != 0 {
                return Err(FixtureError::InvalidRecord);
            }
            cursor += RECORD_HEADER_SIZE;
            let padded = align4(length).ok_or(FixtureError::InvalidSize)?;
            if padded > bytes.len() - cursor {
                return Err(FixtureError::Truncated);
            }
            let payload = &bytes[cursor..cursor + length];
            if bytes[cursor + length..cursor + padded]
                .iter()
                .any(|byte| *byte != 0)
            {
                return Err(FixtureError::NonCanonical);
            }
            decode_record(tag, payload, &mut fixture)?;
            cursor += padded;
            records += 1;
        }
        if records != declared_records {
            return Err(FixtureError::InvalidRecordCount);
        }
        fixture.validate()?;
        if fixture.encode()?.as_slice() != bytes {
            return Err(FixtureError::NonCanonical);
        }
        Ok(fixture)
    }
}

fn decode_record(
    tag: u16,
    payload: &[u8],
    fixture: &mut ScenarioFixture,
) -> Result<(), FixtureError> {
    match tag {
        TAG_NAME => {
            if !fixture.name.is_empty() {
                return Err(FixtureError::DuplicateKey);
            }
            fixture.name = std::str::from_utf8(payload)
                .map_err(|_| FixtureError::InvalidName)?
                .to_owned();
        }
        TAG_FORM => {
            require_singleton(&fixture.form, payload, 4)?;
            if payload[1..].iter().any(|byte| *byte != 0) {
                return Err(FixtureError::InvalidRecord);
            }
            fixture.form = Some(decode_form(payload[0])?);
        }
        TAG_HEALTH => {
            require_singleton(&fixture.health, payload, 4)?;
            fixture.health = Some(HealthFixture {
                current: get_u16(&payload[0..2]),
                maximum: get_u16(&payload[2..4]),
            });
        }
        TAG_RNG => {
            require_length(payload, 24)?;
            if payload[1..4].iter().any(|byte| *byte != 0) {
                return Err(FixtureError::InvalidRecord);
            }
            fixture.rng.push(RngFixture {
                stream: decode_rng_stream(payload[0])?,
                state0: get_i32(&payload[4..8]),
                state1: get_i32(&payload[8..12]),
                state2: get_i32(&payload[12..16]),
                call_count: get_u64(&payload[16..24]),
            });
        }
        TAG_VIDEO_MODE => {
            require_singleton(&fixture.video_mode, payload, 4)?;
            if payload[1..].iter().any(|byte| *byte != 0) {
                return Err(FixtureError::InvalidRecord);
            }
            fixture.video_mode = Some(decode_video_mode(payload[0])?);
        }
        TAG_INVENTORY => {
            require_length(payload, 8)?;
            if payload[6..8].iter().any(|byte| *byte != 0) {
                return Err(FixtureError::InvalidRecord);
            }
            fixture.inventory.push(InventoryFixture {
                slot: get_u16(&payload[0..2]),
                item: get_u16(&payload[2..4]),
                quantity: get_u16(&payload[4..6]),
            });
        }
        TAG_EQUIPMENT => {
            require_length(payload, 4)?;
            fixture.equipment.push(EquipmentFixture {
                slot: get_u16(&payload[0..2]),
                item: get_u16(&payload[2..4]),
            });
        }
        TAG_FLAG => {
            require_length(payload, 8)?;
            if payload[3] != 0 || payload[6] != 0 || payload[7] != 0 || payload[2] > 1 {
                return Err(FixtureError::InvalidRecord);
            }
            fixture.flags.push(FlagFixture {
                domain: decode_flag_domain(payload[0])?,
                room: payload[1] as i8,
                index: get_u16(&payload[4..6]),
                value: payload[2] != 0,
            });
        }
        TAG_SETTING => {
            if payload.len() < 4 {
                return Err(FixtureError::InvalidRecord);
            }
            let key_len = payload[0] as usize;
            let kind = payload[1];
            let value_len = get_u16(&payload[2..4]) as usize;
            if 4 + key_len + value_len != payload.len() {
                return Err(FixtureError::InvalidRecord);
            }
            let key = std::str::from_utf8(&payload[4..4 + key_len])
                .map_err(|_| FixtureError::InvalidSetting)?
                .to_owned();
            let value = decode_setting_value(kind, &payload[4 + key_len..])?;
            fixture.settings.push(SettingFixture { key, value });
        }
        _ => return Err(FixtureError::UnknownRecord(tag)),
    }
    Ok(())
}

fn validate_name(name: &str) -> Result<(), FixtureError> {
    if name.is_empty()
        || name.len() > MAX_NAME_BYTES
        || name.starts_with(' ')
        || name.ends_with(' ')
        || !name.bytes().all(|byte| (0x20..=0x7e).contains(&byte))
    {
        return Err(FixtureError::InvalidName);
    }
    Ok(())
}

fn validate_setting(setting: &SettingFixture) -> Result<(), FixtureError> {
    if setting.key.is_empty()
        || setting.key.len() > MAX_SETTING_KEY_BYTES
        || !setting
            .key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        || matches!(
            &setting.value,
            FixtureSettingValue::String { value } if value.len() > MAX_SETTING_STRING_BYTES
        )
    {
        return Err(FixtureError::InvalidSetting);
    }
    if let FixtureSettingValue::FloatBits { bits } = setting.value
        && !f64::from_bits(bits).is_finite()
    {
        return Err(FixtureError::InvalidSetting);
    }
    Ok(())
}

fn reject_duplicate_keys<T, K: PartialEq>(
    values: &[T],
    key: impl Fn(&T) -> K,
) -> Result<(), FixtureError> {
    if values.windows(2).any(|pair| key(&pair[0]) == key(&pair[1])) {
        Err(FixtureError::DuplicateKey)
    } else {
        Ok(())
    }
}

fn push_record(output: &mut Vec<u8>, tag: u16, payload: &[u8]) -> Result<(), FixtureError> {
    let padded = align4(payload.len()).ok_or(FixtureError::LimitExceeded)?;
    let added = RECORD_HEADER_SIZE
        .checked_add(padded)
        .ok_or(FixtureError::LimitExceeded)?;
    if HEADER_SIZE + output.len() + added > MAX_ENCODED_SIZE {
        return Err(FixtureError::LimitExceeded);
    }
    output.extend_from_slice(&tag.to_le_bytes());
    output.extend_from_slice(&0_u16.to_le_bytes());
    output.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    output.extend_from_slice(payload);
    output.resize(output.len() + (padded - payload.len()), 0);
    Ok(())
}

fn count_records(records: &[u8]) -> Result<u16, FixtureError> {
    let mut cursor = 0_usize;
    let mut count = 0_u16;
    while cursor < records.len() {
        if records.len() - cursor < RECORD_HEADER_SIZE {
            return Err(FixtureError::InvalidRecord);
        }
        let length = get_u32(&records[cursor + 4..cursor + 8]) as usize;
        cursor = cursor
            .checked_add(RECORD_HEADER_SIZE)
            .and_then(|value| value.checked_add(align4(length)?))
            .ok_or(FixtureError::LimitExceeded)?;
        count = count.checked_add(1).ok_or(FixtureError::LimitExceeded)?;
    }
    if cursor != records.len() {
        return Err(FixtureError::InvalidRecord);
    }
    Ok(count)
}

fn require_length(payload: &[u8], length: usize) -> Result<(), FixtureError> {
    if payload.len() == length {
        Ok(())
    } else {
        Err(FixtureError::InvalidRecord)
    }
}

fn require_singleton<T>(
    value: &Option<T>,
    payload: &[u8],
    length: usize,
) -> Result<(), FixtureError> {
    if value.is_some() {
        return Err(FixtureError::DuplicateKey);
    }
    require_length(payload, length)
}

fn encode_form(value: PlayerForm) -> u8 {
    match value {
        PlayerForm::Human => 0,
        PlayerForm::Wolf => 1,
    }
}

fn decode_form(value: u8) -> Result<PlayerForm, FixtureError> {
    match value {
        0 => Ok(PlayerForm::Human),
        1 => Ok(PlayerForm::Wolf),
        _ => Err(FixtureError::InvalidRecord),
    }
}

fn encode_video_mode(value: FixtureVideoMode) -> u8 {
    match value {
        FixtureVideoMode::Automatic => 0,
        FixtureVideoMode::NtscInterlaced => 1,
        FixtureVideoMode::NtscProgressive => 2,
        FixtureVideoMode::Pal50 => 3,
        FixtureVideoMode::Pal60 => 4,
    }
}

fn decode_video_mode(value: u8) -> Result<FixtureVideoMode, FixtureError> {
    match value {
        0 => Ok(FixtureVideoMode::Automatic),
        1 => Ok(FixtureVideoMode::NtscInterlaced),
        2 => Ok(FixtureVideoMode::NtscProgressive),
        3 => Ok(FixtureVideoMode::Pal50),
        4 => Ok(FixtureVideoMode::Pal60),
        _ => Err(FixtureError::InvalidRecord),
    }
}

fn encode_rng_stream(value: RngStreamId) -> u8 {
    match value {
        RngStreamId::Primary => 0,
        RngStreamId::Secondary => 1,
    }
}

fn decode_rng_stream(value: u8) -> Result<RngStreamId, FixtureError> {
    match value {
        0 => Ok(RngStreamId::Primary),
        1 => Ok(RngStreamId::Secondary),
        _ => Err(FixtureError::InvalidRecord),
    }
}

fn encode_flag_domain(value: FixtureFlagDomain) -> u8 {
    match value {
        FixtureFlagDomain::Event => 0,
        FixtureFlagDomain::Temporary => 1,
        FixtureFlagDomain::Dungeon => 2,
        FixtureFlagDomain::Switch => 3,
    }
}

fn decode_flag_domain(value: u8) -> Result<FixtureFlagDomain, FixtureError> {
    match value {
        0 => Ok(FixtureFlagDomain::Event),
        1 => Ok(FixtureFlagDomain::Temporary),
        2 => Ok(FixtureFlagDomain::Dungeon),
        3 => Ok(FixtureFlagDomain::Switch),
        _ => Err(FixtureError::InvalidRecord),
    }
}

fn encode_setting_value(value: &FixtureSettingValue) -> (u8, Vec<u8>) {
    match value {
        FixtureSettingValue::Boolean { value } => (1, vec![u8::from(*value)]),
        FixtureSettingValue::Integer { value } => (2, value.to_le_bytes().to_vec()),
        FixtureSettingValue::FloatBits { bits } => (3, bits.to_le_bytes().to_vec()),
        FixtureSettingValue::String { value } => (4, value.as_bytes().to_vec()),
    }
}

fn decode_setting_value(kind: u8, bytes: &[u8]) -> Result<FixtureSettingValue, FixtureError> {
    match kind {
        1 if bytes.len() == 1 && bytes[0] <= 1 => Ok(FixtureSettingValue::Boolean {
            value: bytes[0] != 0,
        }),
        2 if bytes.len() == 8 => Ok(FixtureSettingValue::Integer {
            value: i64::from_le_bytes(bytes.try_into().unwrap()),
        }),
        3 if bytes.len() == 8 => {
            let bits = u64::from_le_bytes(bytes.try_into().unwrap());
            if !f64::from_bits(bits).is_finite() {
                return Err(FixtureError::InvalidSetting);
            }
            Ok(FixtureSettingValue::FloatBits { bits })
        }
        4 => Ok(FixtureSettingValue::String {
            value: std::str::from_utf8(bytes)
                .map_err(|_| FixtureError::InvalidSetting)?
                .to_owned(),
        }),
        _ => Err(FixtureError::InvalidSetting),
    }
}

fn align4(value: usize) -> Option<usize> {
    value.checked_add(3).map(|value| value & !3)
}

fn get_u16(bytes: &[u8]) -> u16 {
    u16::from_le_bytes(bytes.try_into().unwrap())
}

fn get_u32(bytes: &[u8]) -> u32 {
    u32::from_le_bytes(bytes.try_into().unwrap())
}

fn get_i32(bytes: &[u8]) -> i32 {
    i32::from_le_bytes(bytes.try_into().unwrap())
}

fn get_u64(bytes: &[u8]) -> u64 {
    u64::from_le_bytes(bytes.try_into().unwrap())
}

fn put_u16(bytes: &mut [u8], value: u16) {
    bytes.copy_from_slice(&value.to_le_bytes());
}

fn put_u32(bytes: &mut [u8], value: u32) {
    bytes.copy_from_slice(&value.to_le_bytes());
}

fn put_i32(bytes: &mut [u8], value: i32) {
    bytes.copy_from_slice(&value.to_le_bytes());
}

fn put_u64(bytes: &mut [u8], value: u64) {
    bytes.copy_from_slice(&value.to_le_bytes());
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FixtureError {
    Truncated,
    BadMagic,
    UnsupportedVersion,
    InvalidSchema,
    InvalidHeader,
    InvalidSize,
    InvalidRecordCount,
    InvalidRecord,
    UnknownRecord(u16),
    NonCanonical,
    InvalidName,
    InvalidHealth,
    InvalidInventory,
    InvalidFlag,
    InvalidSetting,
    DuplicateKey,
    LimitExceeded,
}

impl fmt::Display for FixtureError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated => formatter.write_str("scenario fixture is truncated"),
            Self::BadMagic => formatter.write_str("scenario fixture magic is invalid"),
            Self::UnsupportedVersion => {
                formatter.write_str("scenario fixture version is unsupported")
            }
            Self::InvalidSchema => formatter.write_str("scenario fixture JSON schema is invalid"),
            Self::InvalidHeader => formatter.write_str("scenario fixture header is invalid"),
            Self::InvalidSize => formatter.write_str("scenario fixture size is invalid"),
            Self::InvalidRecordCount => {
                formatter.write_str("scenario fixture record count is invalid")
            }
            Self::InvalidRecord => formatter.write_str("scenario fixture record is invalid"),
            Self::UnknownRecord(tag) => {
                write!(formatter, "scenario fixture record tag {tag} is unknown")
            }
            Self::NonCanonical => formatter.write_str("scenario fixture encoding is noncanonical"),
            Self::InvalidName => formatter.write_str("scenario fixture name is invalid"),
            Self::InvalidHealth => formatter.write_str("scenario fixture health is invalid"),
            Self::InvalidInventory => formatter.write_str("scenario fixture inventory is invalid"),
            Self::InvalidFlag => formatter.write_str("scenario fixture flag is invalid"),
            Self::InvalidSetting => formatter.write_str("scenario fixture setting is invalid"),
            Self::DuplicateKey => {
                formatter.write_str("scenario fixture contains a duplicate keyed record")
            }
            Self::LimitExceeded => {
                formatter.write_str("scenario fixture exceeds a bounded format limit")
            }
        }
    }
}

impl Error for FixtureError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> ScenarioFixture {
        ScenarioFixture {
            schema: SCENARIO_FIXTURE_SCHEMA.into(),
            name: "wolf combat loadout".into(),
            form: Some(PlayerForm::Wolf),
            health: Some(HealthFixture {
                current: 20,
                maximum: 40,
            }),
            rng: vec![
                RngFixture {
                    stream: RngStreamId::Secondary,
                    state0: 4,
                    state1: 5,
                    state2: 6,
                    call_count: 8,
                },
                RngFixture {
                    stream: RngStreamId::Primary,
                    state0: 1,
                    state1: 2,
                    state2: 3,
                    call_count: 7,
                },
            ],
            video_mode: Some(FixtureVideoMode::NtscProgressive),
            inventory: vec![
                InventoryFixture {
                    slot: 4,
                    item: 0x2a,
                    quantity: 1,
                },
                InventoryFixture {
                    slot: 1,
                    item: 0x10,
                    quantity: 30,
                },
            ],
            equipment: vec![EquipmentFixture { slot: 2, item: 9 }],
            flags: vec![FlagFixture {
                domain: FixtureFlagDomain::Switch,
                room: 1,
                index: 12,
                value: true,
            }],
            settings: vec![
                SettingFixture {
                    key: "game.damageMultiplier".into(),
                    value: FixtureSettingValue::Integer { value: 2 },
                },
                SettingFixture {
                    key: "game.enableMirrorMode".into(),
                    value: FixtureSettingValue::Boolean { value: false },
                },
            ],
        }
    }

    #[test]
    fn canonical_round_trip_covers_every_fixture_domain() {
        const GOLDEN_HEX: &str = concat!(
            "4455534b465854520100000020000c002c010000000000000000000000000000",
            "0100000013000000776f6c6620636f6d626174206c6f61646f75740002000000",
            "0400000001000000030000000400000014002800040000001800000000000000",
            "0100000002000000030000000700000000000000040000001800000001000000",
            "0400000005000000060000000800000000000000050000000400000002000000",
            "0600000008000000010010001e000000060000000800000004002a0001000000",
            "0700000004000000020009000800000008000000030101000c00000009000000",
            "210000001502080067616d652e64616d6167654d756c7469706c696572020000",
            "0000000000000000090000001a0000001501010067616d652e656e61626c654d",
            "6972726f724d6f6465000000"
        );
        let fixture = fixture();
        let encoded = fixture.encode().unwrap();
        assert_eq!(
            encoded
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>(),
            GOLDEN_HEX
        );
        assert_eq!(&encoded[..8], &MAGIC);
        assert_eq!(get_u16(&encoded[14..16]), 12);
        let decoded = ScenarioFixture::decode(&encoded).unwrap();

        let mut expected = fixture;
        expected.rng.sort_unstable();
        expected.inventory.sort_unstable();
        expected.equipment.sort_unstable();
        expected.flags.sort_unstable();
        expected
            .settings
            .sort_unstable_by(|left, right| left.key.cmp(&right.key));
        assert_eq!(decoded, expected);
        assert_eq!(decoded.encode().unwrap(), encoded);
    }

    #[test]
    fn rejects_corruption_duplicates_and_unbounded_values() {
        let encoded = fixture().encode().unwrap();
        for end in 0..encoded.len() {
            assert!(ScenarioFixture::decode(&encoded[..end]).is_err());
        }

        let mut reserved = encoded.clone();
        reserved[20] = 1;
        assert_eq!(
            ScenarioFixture::decode(&reserved),
            Err(FixtureError::InvalidHeader)
        );

        let mut duplicate = fixture();
        duplicate.inventory.push(InventoryFixture {
            slot: 1,
            item: 99,
            quantity: 1,
        });
        assert_eq!(duplicate.encode(), Err(FixtureError::DuplicateKey));

        let mut nonfinite = fixture();
        nonfinite.settings.push(SettingFixture {
            key: "game.hudScale".into(),
            value: FixtureSettingValue::FloatBits {
                bits: f64::NAN.to_bits(),
            },
        });
        assert_eq!(nonfinite.encode(), Err(FixtureError::InvalidSetting));
    }
}
