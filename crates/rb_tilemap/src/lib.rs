use bevy::prelude::*;
use bitflags::bitflags;
use std::collections::HashMap;
use rb_core::TileType;

bitflags! {
    /// Collision flags for tiles.
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub struct CollisionFlags: u8 {
        const PASSABLE = 0b0000_0001;
        const BLOCKED = 0b0000_0010;
        const WATER = 0b0000_0100;
    }
}

/// Tileset identifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TilesetId(pub u32);

/// Marker for a playable-level chunk entity.
#[derive(Component)]
pub struct LevelChunk {
    pub coord: (i32, i32),
}

/// Tile data for a loaded chunk (64x64).
#[derive(Component)]
pub struct ChunkTiles {
    pub tiles: Vec<TileType>,
    pub collision: Vec<CollisionFlags>,
    pub width: usize,
    pub height: usize,
}

/// Tracks loaded level chunks.
#[derive(Resource, Default)]
pub struct LoadedChunks {
    pub chunks: HashMap<(i32, i32), Entity>,
}

/// Tilemap plugin for Randlebrot.
/// Manages tile storage, collision layers, tileset registry, and chunk rendering.
pub struct RbTilemapPlugin;

impl Plugin for RbTilemapPlugin {
    fn build(&self, _app: &mut App) {
        // Tilemap systems will be added in future iterations.
    }
}
