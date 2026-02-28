use bevy::prelude::*;
use bitflags::bitflags;

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

/// Tilemap plugin for Randlebrot.
/// Manages tile storage, collision layers, tileset registry, and chunk rendering.
pub struct RbTilemapPlugin;

impl Plugin for RbTilemapPlugin {
    fn build(&self, _app: &mut App) {
        // Tilemap systems will be added in future iterations.
    }
}
