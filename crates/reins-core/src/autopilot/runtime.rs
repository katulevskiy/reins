//! Traits the app implements for Autopilot (spec §6.3): running the model, and hearing how a download goes.

use super::types::{ModelInput, ModelOutput};
use crate::error::ForeignError;

/// Runs the ONNX model (ONNX Runtime for Android in the app; the `ort` crate on desktop). The core owns tokenization,
/// calibration and everything after; this only does the forward pass. Called off the async runtime.
#[uniffi::export(with_foreign)]
pub trait ModelRuntime: Send + Sync {
    /// Loads (or keeps loaded) the ONNX file at `path`.
    fn load(&self, path: String) -> Result<(), ForeignError>;
    fn run(&self, input: ModelInput) -> Result<ModelOutput, ForeignError>;
    /// Frees the loaded model (it was deleted or replaced).
    fn unload(&self);
}

/// Told how a model download goes.
#[uniffi::export(with_foreign)]
pub trait DownloadProgress: Send + Sync {
    /// `total` is 0 when not known.
    fn progress(&self, downloaded: u64, total: u64);
}
