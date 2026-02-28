use bevy::prelude::*;
use std::sync::Arc;

use crate::chunk_hierarchy::{CacheConfig, ChunkHierarchy};
use crate::strategy::{ContinentalnessStrategy, TemperatureStrategy};

/// Bevy resource wrapping the chunk hierarchy and noise strategies.
#[derive(Resource)]
pub struct WorldChunks {
    /// Chunk hierarchy for continentalness noise.
    pub continentalness_hierarchy: ChunkHierarchy,
    /// Chunk hierarchy for temperature noise.
    pub temperature_hierarchy: ChunkHierarchy,
    /// Continentalness noise strategy.
    pub continentalness_strategy: Arc<ContinentalnessStrategy>,
    /// Temperature noise strategy.
    pub temperature_strategy: Arc<TemperatureStrategy>,
}

impl WorldChunks {
    /// Create a new WorldChunks resource with the given seed.
    pub fn new(seed: u32) -> Self {
        let config = CacheConfig::default();

        Self {
            continentalness_hierarchy: ChunkHierarchy::new(config.clone()),
            temperature_hierarchy: ChunkHierarchy::new(config),
            continentalness_strategy: Arc::new(ContinentalnessStrategy::new(seed)),
            temperature_strategy: Arc::new(TemperatureStrategy::new(seed.wrapping_add(1))),
        }
    }

    /// Create with custom cache configuration.
    pub fn with_config(seed: u32, config: CacheConfig) -> Self {
        Self {
            continentalness_hierarchy: ChunkHierarchy::new(config.clone()),
            temperature_hierarchy: ChunkHierarchy::new(config),
            continentalness_strategy: Arc::new(ContinentalnessStrategy::new(seed)),
            temperature_strategy: Arc::new(TemperatureStrategy::new(seed.wrapping_add(1))),
        }
    }

    /// Sample continentalness at a world position.
    pub fn sample_continentalness(
        &mut self,
        x: f64,
        y: f64,
        detail_level: rb_core::DetailLevel,
    ) -> f64 {
        self.continentalness_hierarchy
            .sample(x, y, detail_level, self.continentalness_strategy.as_ref())
    }

    /// Sample temperature at a world position.
    pub fn sample_temperature(
        &mut self,
        x: f64,
        y: f64,
        detail_level: rb_core::DetailLevel,
    ) -> f64 {
        self.temperature_hierarchy
            .sample(x, y, detail_level, self.temperature_strategy.as_ref())
    }

    /// Clear all caches.
    pub fn clear_caches(&mut self) {
        self.continentalness_hierarchy.clear();
        self.temperature_hierarchy.clear();
    }
}

impl Default for WorldChunks {
    fn default() -> Self {
        Self::new(42)
    }
}
