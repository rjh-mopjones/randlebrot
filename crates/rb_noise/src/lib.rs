use bevy::prelude::*;

pub mod biome_map;
pub mod biome_splines;
pub mod chunk_hierarchy;
pub mod derived;
pub mod progress;
pub mod resource;
pub mod resource_map;
pub mod strategy;
pub mod tidally_locked;
pub mod visualization;

pub use biome_map::{BiomeMap, SEA_LEVEL};
pub use biome_splines::BiomeSplines;
pub use chunk_hierarchy::{
    CacheConfig, CacheStats, ChunkHierarchy, MacroChunk, MesoChunk, MicroChunk,
};
pub use derived::{calculate_political_score, calculate_trade_cost};
pub use progress::{LayerId, LayerProgress};
pub use resource::WorldChunks;
pub use resource_map::ResourceMap;
pub use strategy::{
    ContinentalnessStrategy, ErosionStrategy, HumidityStrategy, PeaksAndValleysStrategy,
    ResourceContext, ResourceNoiseStrategy, TectonicPlatesStrategy, TemperatureStrategy,
};
pub use tidally_locked::{LatitudeTemperatureStrategy, TidallyLockedTemperatureStrategy};
pub use visualization::NoiseLayer;

/// Noise generation plugin for Randlebrot.
/// Provides fractal noise hierarchy with LRU caching.
pub struct RbNoisePlugin;

impl Plugin for RbNoisePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<WorldChunks>();
    }
}
