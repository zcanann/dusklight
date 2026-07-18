//! Frozen, little-endian dense inference artifacts for Rust/C++ batch readers.

use crate::artifact::Digest;
use sha2::{Digest as _, Sha256};
use std::error::Error;
use std::fmt;

pub const FROZEN_INFERENCE_SCHEMA_V1: &str = "dusklight-frozen-inference/v1";
const MAGIC: &[u8; 8] = b"DSKFRZN\0";
const VERSION: u16 = 1;
const MAX_LAYERS: usize = 16;
const MAX_TENSOR_WIDTH: usize = 4096;
const MAX_PARAMETERS: usize = 16_000_000;
const MAX_BATCH_ROWS: usize = 8192;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum FrozenActivation {
    Linear = 0,
    Relu = 1,
}

impl FrozenActivation {
    fn apply(self, value: f32) -> f32 {
        match self {
            Self::Linear => value,
            Self::Relu => value.max(0.0),
        }
    }

    fn decode(value: u8) -> Result<Self, FrozenInferenceError> {
        match value {
            0 => Ok(Self::Linear),
            1 => Ok(Self::Relu),
            _ => Err(FrozenInferenceError::new(
                "frozen model activation is unsupported",
            )),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct FrozenDenseLayer {
    pub output_width: usize,
    pub activation: FrozenActivation,
    /// Row-major `[output][input]` weights.
    pub weights: Vec<f32>,
    pub biases: Vec<f32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FrozenInferenceModel {
    pub feature_schema_sha256: Digest,
    pub action_schema_sha256: Digest,
    pub objective_sha256: Digest,
    pub input_width: usize,
    pub actions: Vec<u32>,
    pub layers: Vec<FrozenDenseLayer>,
}

impl FrozenInferenceModel {
    pub fn new(
        feature_schema_sha256: Digest,
        action_schema_sha256: Digest,
        objective_sha256: Digest,
        input_width: usize,
        actions: Vec<u32>,
        layers: Vec<FrozenDenseLayer>,
    ) -> Result<Self, FrozenInferenceError> {
        let model = Self {
            feature_schema_sha256,
            action_schema_sha256,
            objective_sha256,
            input_width,
            actions,
            layers,
        };
        model.validate()?;
        Ok(model)
    }

    pub fn artifact_sha256(&self) -> Result<Digest, FrozenInferenceError> {
        let bytes = self.to_bytes()?;
        let mut hasher = Sha256::new();
        hasher.update(b"dusklight.frozen-inference-artifact/v1\0");
        hasher.update(bytes);
        Ok(Digest(hasher.finalize().into()))
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, FrozenInferenceError> {
        self.validate()?;
        let parameter_count = self.parameter_count()?;
        let capacity = 8_usize
            .checked_add(2 + 2 + 32 * 3 + 4 * 4)
            .and_then(|value| value.checked_add(self.actions.len() * 4))
            .and_then(|value| value.checked_add(self.layers.len() * 8))
            .and_then(|value| value.checked_add(parameter_count * 4))
            .ok_or_else(|| FrozenInferenceError::new("frozen model size overflowed"))?;
        let mut bytes = Vec::with_capacity(capacity);
        bytes.extend(MAGIC);
        push_u16(&mut bytes, VERSION);
        push_u16(&mut bytes, 0);
        bytes.extend(self.feature_schema_sha256.0);
        bytes.extend(self.action_schema_sha256.0);
        bytes.extend(self.objective_sha256.0);
        push_u32(&mut bytes, usize_u32(self.input_width)?);
        push_u32(&mut bytes, usize_u32(self.actions.len())?);
        push_u32(&mut bytes, usize_u32(self.layers.len())?);
        push_u32(&mut bytes, usize_u32(parameter_count)?);
        for action in &self.actions {
            push_u32(&mut bytes, *action);
        }
        for layer in &self.layers {
            bytes.push(layer.activation as u8);
            bytes.extend([0; 3]);
            push_u32(&mut bytes, usize_u32(layer.output_width)?);
            for value in layer.weights.iter().chain(&layer.biases) {
                bytes.extend(value.to_le_bytes());
            }
        }
        Ok(bytes)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, FrozenInferenceError> {
        let mut reader = Reader::new(bytes);
        if reader.take(8)? != MAGIC || reader.u16()? != VERSION || reader.u16()? != 0 {
            return Err(FrozenInferenceError::new(
                "frozen model header or version is invalid",
            ));
        }
        let feature_schema_sha256 = Digest(reader.array_32()?);
        let action_schema_sha256 = Digest(reader.array_32()?);
        let objective_sha256 = Digest(reader.array_32()?);
        let input_width = reader.usize_u32()?;
        let action_count = reader.usize_u32()?;
        let layer_count = reader.usize_u32()?;
        let declared_parameter_count = reader.usize_u32()?;
        if input_width == 0
            || input_width > MAX_TENSOR_WIDTH
            || action_count == 0
            || action_count > MAX_TENSOR_WIDTH
            || layer_count == 0
            || layer_count > MAX_LAYERS
            || declared_parameter_count > MAX_PARAMETERS
        {
            return Err(FrozenInferenceError::new(
                "frozen model dimensions exceed format bounds",
            ));
        }
        let actions = (0..action_count)
            .map(|_| reader.u32())
            .collect::<Result<Vec<_>, _>>()?;
        let mut layers = Vec::with_capacity(layer_count);
        let mut layer_input_width = input_width;
        let mut decoded_parameter_count = 0_usize;
        for _ in 0..layer_count {
            let activation = FrozenActivation::decode(reader.u8()?)?;
            if reader.take(3)? != [0; 3] {
                return Err(FrozenInferenceError::new(
                    "frozen model reserved bytes are nonzero",
                ));
            }
            let output_width = reader.usize_u32()?;
            if output_width == 0 || output_width > MAX_TENSOR_WIDTH {
                return Err(FrozenInferenceError::new(
                    "frozen layer width exceeds format bounds",
                ));
            }
            let weight_count = layer_input_width
                .checked_mul(output_width)
                .ok_or_else(|| FrozenInferenceError::new("frozen layer size overflowed"))?;
            decoded_parameter_count = decoded_parameter_count
                .checked_add(weight_count)
                .and_then(|value| value.checked_add(output_width))
                .filter(|value| *value <= declared_parameter_count && *value <= MAX_PARAMETERS)
                .ok_or_else(|| {
                    FrozenInferenceError::new(
                        "frozen layer parameters exceed the declared bounded total",
                    )
                })?;
            let weights = (0..weight_count)
                .map(|_| reader.f32())
                .collect::<Result<Vec<_>, _>>()?;
            let biases = (0..output_width)
                .map(|_| reader.f32())
                .collect::<Result<Vec<_>, _>>()?;
            layers.push(FrozenDenseLayer {
                output_width,
                activation,
                weights,
                biases,
            });
            layer_input_width = output_width;
        }
        if !reader.is_empty() {
            return Err(FrozenInferenceError::new(
                "frozen model contains trailing bytes",
            ));
        }
        let model = Self::new(
            feature_schema_sha256,
            action_schema_sha256,
            objective_sha256,
            input_width,
            actions,
            layers,
        )?;
        if model.parameter_count()? != declared_parameter_count {
            return Err(FrozenInferenceError::new(
                "frozen model parameter count is detached",
            ));
        }
        Ok(model)
    }

    /// Batch-only inference. Callers decide process placement after measuring it.
    pub fn infer_batch(&self, inputs: &[Vec<f32>]) -> Result<Vec<Vec<f32>>, FrozenInferenceError> {
        self.validate()?;
        if inputs.is_empty()
            || inputs.len() > MAX_BATCH_ROWS
            || inputs.iter().any(|row| {
                row.len() != self.input_width || row.iter().any(|value| !value.is_finite())
            })
        {
            return Err(FrozenInferenceError::new(
                "frozen inference batch is invalid",
            ));
        }
        inputs
            .iter()
            .map(|input| {
                let mut values = input.clone();
                for layer in &self.layers {
                    let mut output = Vec::with_capacity(layer.output_width);
                    for row in 0..layer.output_width {
                        let mut value = layer.biases[row];
                        let weights = &layer.weights[row * values.len()..(row + 1) * values.len()];
                        for (weight, input) in weights.iter().zip(&values) {
                            value += weight * input;
                        }
                        output.push(layer.activation.apply(value));
                    }
                    values = output;
                }
                if values.iter().any(|value| !value.is_finite()) {
                    return Err(FrozenInferenceError::new(
                        "frozen inference output became non-finite",
                    ));
                }
                Ok(values)
            })
            .collect()
    }

    fn validate(&self) -> Result<(), FrozenInferenceError> {
        if [
            self.feature_schema_sha256,
            self.action_schema_sha256,
            self.objective_sha256,
        ]
        .contains(&Digest::ZERO)
            || self.input_width == 0
            || self.input_width > MAX_TENSOR_WIDTH
            || self.actions.is_empty()
            || self.actions.len() > MAX_TENSOR_WIDTH
            || !self.actions.windows(2).all(|pair| pair[0] < pair[1])
            || self.layers.is_empty()
            || self.layers.len() > MAX_LAYERS
        {
            return Err(FrozenInferenceError::new(
                "frozen model identity or topology is invalid",
            ));
        }
        let mut input_width = self.input_width;
        for (index, layer) in self.layers.iter().enumerate() {
            if layer.output_width == 0
                || layer.output_width > MAX_TENSOR_WIDTH
                || layer.weights.len() != input_width.saturating_mul(layer.output_width)
                || layer.biases.len() != layer.output_width
                || layer
                    .weights
                    .iter()
                    .chain(&layer.biases)
                    .any(|value| !value.is_finite())
                || (index + 1 == self.layers.len()
                    && (layer.output_width != self.actions.len()
                        || layer.activation != FrozenActivation::Linear))
            {
                return Err(FrozenInferenceError::new(
                    "frozen layer topology or parameters are invalid",
                ));
            }
            input_width = layer.output_width;
        }
        if self.parameter_count()? > MAX_PARAMETERS {
            return Err(FrozenInferenceError::new(
                "frozen model exceeds its parameter bound",
            ));
        }
        Ok(())
    }

    fn parameter_count(&self) -> Result<usize, FrozenInferenceError> {
        self.layers.iter().try_fold(0_usize, |count, layer| {
            count
                .checked_add(layer.weights.len())
                .and_then(|value| value.checked_add(layer.biases.len()))
                .ok_or_else(|| FrozenInferenceError::new("frozen parameter count overflowed"))
        })
    }
}

fn usize_u32(value: usize) -> Result<u32, FrozenInferenceError> {
    value
        .try_into()
        .map_err(|_| FrozenInferenceError::new("frozen model integer exceeds u32"))
}

fn push_u16(bytes: &mut Vec<u8>, value: u16) {
    bytes.extend(value.to_le_bytes());
}

fn push_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend(value.to_le_bytes());
}

struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8], FrozenInferenceError> {
        let end = self
            .offset
            .checked_add(count)
            .filter(|end| *end <= self.bytes.len())
            .ok_or_else(|| FrozenInferenceError::new("frozen model is truncated"))?;
        let value = &self.bytes[self.offset..end];
        self.offset = end;
        Ok(value)
    }

    fn u8(&mut self) -> Result<u8, FrozenInferenceError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, FrozenInferenceError> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }

    fn u32(&mut self) -> Result<u32, FrozenInferenceError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn usize_u32(&mut self) -> Result<usize, FrozenInferenceError> {
        self.u32()?
            .try_into()
            .map_err(|_| FrozenInferenceError::new("frozen model integer exceeds usize"))
    }

    fn f32(&mut self) -> Result<f32, FrozenInferenceError> {
        Ok(f32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn array_32(&mut self) -> Result<[u8; 32], FrozenInferenceError> {
        Ok(self.take(32)?.try_into().unwrap())
    }

    fn is_empty(&self) -> bool {
        self.offset == self.bytes.len()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FrozenInferenceError(String);

impl FrozenInferenceError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for FrozenInferenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for FrozenInferenceError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn model() -> FrozenInferenceModel {
        FrozenInferenceModel::new(
            Digest([1; 32]),
            Digest([2; 32]),
            Digest([3; 32]),
            2,
            vec![10, 20],
            vec![
                FrozenDenseLayer {
                    output_width: 2,
                    activation: FrozenActivation::Relu,
                    weights: vec![1.0, -1.0, 0.5, 0.5],
                    biases: vec![0.0, 1.0],
                },
                FrozenDenseLayer {
                    output_width: 2,
                    activation: FrozenActivation::Linear,
                    weights: vec![2.0, 0.0, 0.0, -1.0],
                    biases: vec![0.25, 0.5],
                },
            ],
        )
        .unwrap()
    }

    #[test]
    fn versioned_bytes_round_trip_and_batch_inference_is_exact() {
        let model = model();
        let bytes = model.to_bytes().unwrap();
        let decoded = FrozenInferenceModel::from_bytes(&bytes).unwrap();
        assert_eq!(decoded, model);
        assert_ne!(model.artifact_sha256().unwrap(), Digest::ZERO);
        assert_eq!(
            decoded
                .infer_batch(&[vec![3.0, 1.0], vec![-4.0, 2.0]])
                .unwrap(),
            vec![vec![4.25, -2.5], vec![0.25, 0.5]]
        );
    }

    #[test]
    fn malformed_or_detached_artifacts_fail_closed() {
        let fixture = model();
        let mut bytes = fixture.to_bytes().unwrap();
        bytes[8] = 2;
        assert!(FrozenInferenceModel::from_bytes(&bytes).is_err());

        let bytes = fixture.to_bytes().unwrap();
        assert!(FrozenInferenceModel::from_bytes(&bytes[..bytes.len() - 1]).is_err());
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(FrozenInferenceModel::from_bytes(&trailing).is_err());

        // The declared parameter count is the fourth u32 after the three
        // identity digests. Detaching it from the tensor payload must fail.
        let mut detached_count = bytes;
        detached_count[116..120].copy_from_slice(&0_u32.to_le_bytes());
        assert!(FrozenInferenceModel::from_bytes(&detached_count).is_err());

        let mut invalid = fixture;
        invalid.actions.reverse();
        assert!(invalid.to_bytes().is_err());
        assert!(invalid.infer_batch(&[vec![0.0, 0.0]]).is_err());

        let mut non_finite = model();
        non_finite.layers[0].weights[0] = f32::NAN;
        assert!(non_finite.to_bytes().is_err());
    }
}
