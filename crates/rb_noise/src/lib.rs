use bevy::prelude::*;

pub mod biome_map;
pub mod chunk_hierarchy;
pub mod resource;
pub mod strategy;
pub mod tidally_locked;

pub use biome_map::{BiomeMap, SEA_LEVEL};
pub use chunk_hierarchy::{
    CacheConfig, CacheStats, ChunkHierarchy, MacroChunk, MesoChunk, MicroChunk,
};
pub use resource::WorldChunks;
pub use strategy::{ContinentalnessStrategy, TemperatureStrategy};
pub use tidally_locked::TidallyLockedTemperatureStrategy;

/// Noise generation plugin for Randlebrot.
/// Provides fractal noise hierarchy with LRU caching.
pub struct RbNoisePlugin;

impl Plugin for RbNoisePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<WorldChunks>();
    }
}
