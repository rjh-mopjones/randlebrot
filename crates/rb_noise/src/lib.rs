use bevy::prelude::*;

pub mod biome_map;
pub mod biome_splines;
pub mod chunk_hierarchy;
pub mod derived;
pub mod erosion_sim;
#[cfg(feature = "gpu")]
pub mod gpu;
pub mod progress;
pub mod resource;
pub mod resource_map;
pub mod rivers;
pub mod strategy;
pub mod terrain_render;
pub mod tidally_locked;
pub mod visualization;

pub use biome_map::{BiomeMap, NoiseBackend, SEA_LEVEL};
pub use erosion_sim::{ErosionParams, ErosionResult, simulate_erosion};
pub use biome_splines::BiomeSplines;
pub use chunk_hierarchy::{
    CacheConfig, CacheStats, ChunkHierarchy, MacroChunk, MesoChunk, MicroChunk,
};
pub use progress::{LayerId, LayerProgress};
pub use resource::WorldChunks;
pub use resource_map::ResourceMap;
pub use rivers::{
    Lake, RiverCharacter, RiverConstraint, RiverGenerator, RiverNetwork, RiverSegment,
    rasterize_from_network, LOD_THRESHOLD_MACRO, LOD_THRESHOLD_MESO, LOD_THRESHOLD_MICRO,
};
pub use strategy::{
    ContinentalnessStrategy, ErosionStrategy, HumidityStrategy, LightLevelStrategy,
    PeaksAndValleysStrategy, ResourceContext, ResourceNoiseStrategy, RockHardnessStrategy,
    TectonicPlatesStrategy, TemperatureStrategy,
};
pub use tidally_locked::{LatitudeTemperatureStrategy, TidallyLockedTemperatureStrategy};
pub use terrain_render::NormalizationHints;
pub use visualization::NoiseLayer;
pub use derived::{
    derive_aridity, derive_erosion, derive_heightmap, derive_peaks_valleys,
    derive_precipitation_type, derive_resource_richness, derive_twi, derive_water_table,
    derive_snowpack, derive_soil_type, derive_temperature, derive_vegetation_density,
};

#[cfg(feature = "gpu")]
pub use gpu::GpuNoiseContext;

/// Noise generation plugin for Randlebrot.
/// Provides fractal noise hierarchy with LRU caching.
pub struct RbNoisePlugin;

impl Plugin for RbNoisePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<WorldChunks>();
    }
}
