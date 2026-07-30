use super::TacticAssetDescription;
use std::error::Error;
use std::fmt;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TacticAssetError {
    InvalidOptionId,
    EmptyCatalog,
    CatalogTooLarge,
    DuplicateOptionId,
    UnknownOptionId(String),
    InvalidAsset(String),
    Serialization(String),
}

impl fmt::Display for TacticAssetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidOptionId => formatter.write_str("tactic option ID is invalid"),
            Self::EmptyCatalog => formatter.write_str("tactic catalog is empty"),
            Self::CatalogTooLarge => formatter.write_str("tactic catalog exceeds its finite bound"),
            Self::DuplicateOptionId => {
                formatter.write_str("tactic catalog option IDs are not unique")
            }
            Self::UnknownOptionId(option_id) => {
                write!(formatter, "tactic catalog has no option named {option_id}")
            }
            Self::InvalidAsset(message) => write!(formatter, "tactic asset is invalid: {message}"),
            Self::Serialization(message) => {
                write!(formatter, "tactic asset serialization failed: {message}")
            }
        }
    }
}

impl Error for TacticAssetError {}

pub(super) fn checked(
    description: TacticAssetDescription,
) -> Result<TacticAssetDescription, TacticAssetError> {
    description.validate()?;
    Ok(description)
}

pub(super) fn validate_option_id(value: &str) -> Result<(), TacticAssetError> {
    if value.is_empty()
        || value.len() > 96
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b'/'))
    {
        return Err(TacticAssetError::InvalidOptionId);
    }
    Ok(())
}

pub(super) fn invalid(message: impl Into<String>) -> TacticAssetError {
    TacticAssetError::InvalidAsset(message.into())
}

pub(super) fn serialization(error: serde_json::Error) -> TacticAssetError {
    TacticAssetError::Serialization(error.to_string())
}
