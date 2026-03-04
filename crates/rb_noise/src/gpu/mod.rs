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

/// Result of GPU noise generation containing all 6 base layers.
#[derive(Debug)]
pub struct GpuNoiseResult {
    pub continentalness: Vec<f32>,
    pub temperature: Vec<f32>,
    pub tectonic: Vec<f32>,
    pub erosion: Vec<f32>,
    pub peaks_valleys: Vec<f32>,
    pub humidity: Vec<f32>,
}

impl GpuNoiseResult {
    /// Convert f32 results to f64 for compatibility with CPU pipeline.
    pub fn to_f64_vecs(self) -> GpuNoiseResultF64 {
        GpuNoiseResultF64 {
            continentalness: self.continentalness.into_iter().map(|v| v as f64).collect(),
            temperature: self.temperature.into_iter().map(|v| v as f64).collect(),
            tectonic: self.tectonic.into_iter().map(|v| v as f64).collect(),
            erosion: self.erosion.into_iter().map(|v| v as f64).collect(),
            peaks_valleys: self.peaks_valleys.into_iter().map(|v| v as f64).collect(),
            humidity: self.humidity.into_iter().map(|v| v as f64).collect(),
        }
    }
}

/// F64 version of GPU noise results for CPU pipeline compatibility.
#[derive(Debug)]
pub struct GpuNoiseResultF64 {
    pub continentalness: Vec<f64>,
    pub temperature: Vec<f64>,
    pub tectonic: Vec<f64>,
    pub erosion: Vec<f64>,
    pub peaks_valleys: Vec<f64>,
    pub humidity: Vec<f64>,
}
