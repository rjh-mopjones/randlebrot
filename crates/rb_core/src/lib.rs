use bevy::prelude::*;

pub mod biome;
pub mod coords;
pub mod mode;
pub mod noise;
pub mod resource_type;

pub use biome::{BiomeType, TileType};
pub use coords::{ChunkCoord, DetailLevel, TileCoord, WorldPos};
pub use mode::{AppMode, ModeTransitionEvent, handle_mode_shortcuts};
pub use noise::NoiseStrategy;
pub use resource_type::{ResourceType, TerrainBias};

/// Active playable level state. Inserted when entering play mode, removed on exit.
#[derive(Resource)]
pub struct PlayableLevel {
    /// World-space origin of the level (top-left of the selected macro chunk).
    pub origin: WorldPos,
    /// The macro chunk coordinate the level is centered on.
    pub chunk_coord: (i32, i32),
    /// World seed.
    pub seed: u32,
    /// World height for BiomeMap generation.
    pub world_height: f64,
}

/// Tracks which chunk has been selected on the world map for the level launcher.
/// Set by clicking on the world map; consumed by the level launcher to start play.
#[derive(Resource, Debug, Clone)]
pub struct SelectedChunk {
    /// The macro chunk coordinate.
    pub chunk_coord: (i32, i32),
    /// World-space origin (top-left of the macro chunk).
    pub origin: WorldPos,
}

/// Core plugin providing foundational types for Randlebrot.
pub struct RbCorePlugin;

impl Plugin for RbCorePlugin {
    fn build(&self, _app: &mut App) {
        // Core types are used by other crates; no systems to register here.
    }
}
