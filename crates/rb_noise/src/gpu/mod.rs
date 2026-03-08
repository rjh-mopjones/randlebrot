//! GPU-accelerated noise generation using wgpu compute shaders.
//!
//! This module provides an optional GPU backend for noise layer generation,
//! offering significant speedups (10-50x) over the CPU/Rayon implementation.
//!
//! # Usage
//!
//! Enable the `gpu` feature in Cargo.toml, then use `NoiseBackend::Gpu`:
//!
//! ```ignore
//! let map = BiomeMap::generate(seed, 1024, 512, NoiseBackend::Gpu);
//! ```

mod context;
mod perm_table;
mod pipelines;

pub use context::GpuNoiseContext;
pub use perm_table::generate_permutation_table;
pub use pipelines::NoisePipelines;

/// Result of GPU noise generation containing all base layers.
///
/// Erosion is no longer generated on GPU — it's fully derived on CPU
/// from heightmap + rock_hardness + humidity.
/// Temperature is derived on CPU from light_level + elevation + humidity.
#[derive(Debug)]
pub struct GpuNoiseResult {
    pub continentalness: Vec<f32>,
    pub peaks_valleys: Vec<f32>,
    pub humidity: Vec<f32>,
    pub light_level: Vec<f32>,
    pub rock_hardness: Vec<f32>,
}

impl GpuNoiseResult {
    /// Convert f32 results to f64 for compatibility with CPU pipeline.
    pub fn to_f64_vecs(self) -> GpuNoiseResultF64 {
        GpuNoiseResultF64 {
            continentalness: self.continentalness.into_iter().map(|v| v as f64).collect(),
            peaks_valleys: self.peaks_valleys.into_iter().map(|v| v as f64).collect(),
            humidity: self.humidity.into_iter().map(|v| v as f64).collect(),
            light_level: self.light_level.into_iter().map(|v| v as f64).collect(),
            rock_hardness: self.rock_hardness.into_iter().map(|v| v as f64).collect(),
        }
    }
}

/// F64 version of GPU noise results for CPU pipeline compatibility.
#[derive(Debug)]
pub struct GpuNoiseResultF64 {
    pub continentalness: Vec<f64>,
    pub peaks_valleys: Vec<f64>,
    pub humidity: Vec<f64>,
    pub light_level: Vec<f64>,
    pub rock_hardness: Vec<f64>,
}
