use bevy::prelude::*;

pub mod biome;
pub mod coords;
pub mod mode;
pub mod noise;

pub use biome::{BiomeType, TileType};
pub use coords::{ChunkCoord, DetailLevel, TileCoord, WorldPos};
pub use mode::{AppMode, ModeTransitionEvent, handle_mode_shortcuts};
pub use noise::NoiseStrategy;

/// Core plugin providing foundational types for Randlebrot.
pub struct RbCorePlugin;

impl Plugin for RbCorePlugin {
    fn build(&self, _app: &mut App) {
        // Core types are used by other crates; no systems to register here.
    }
}
