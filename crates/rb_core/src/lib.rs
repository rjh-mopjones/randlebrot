use bevy::prelude::*;

pub mod biome;
pub mod coords;
pub mod lifegen_query;
pub mod mode;
pub mod noise;
pub mod resource_type;
pub mod terrain_query;

pub use biome::{BiomeType, TileType};
pub use coords::{ChunkCoord, DetailLevel, TileCoord, WorldPos};
pub use lifegen_query::LifeGenQuery;
pub use mode::{AppMode, ModeTransitionEvent, handle_mode_shortcuts};
pub use noise::NoiseStrategy;
pub use resource_type::{ResourceType, TerrainBias};
pub use terrain_query::TerrainQuery;

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

/// Tracks which meso tile within a macro chunk has been selected.
#[derive(Resource, Debug, Clone)]
pub struct SelectedMesoTile {
    /// Position within the 8×8 meso grid (0-7, 0-7).
    pub meso_coord: (i32, i32),
    /// World-space origin of this meso tile (top-left).
    pub origin: WorldPos,
}

/// Tracks which micro tile within a meso tile has been selected.
#[derive(Resource, Debug, Clone)]
pub struct SelectedMicroTile {
    /// Position within the 32×32 micro grid (0-31, 0-31).
    pub micro_coord: (i32, i32),
    /// World-space origin of this micro tile (top-left).
    pub origin: WorldPos,
}

/// Core plugin providing foundational types for Randlebrot.
pub struct RbCorePlugin;

impl Plugin for RbCorePlugin {
    fn build(&self, _app: &mut App) {
        // Core types are used by other crates; no systems to register here.
    }
}
