use bevy::prelude::*;

/// Grid position of a chunk in chunk-space coordinates.
#[derive(Clone, Copy, Hash, Eq, PartialEq, Debug, Component)]
pub struct ChunkCoord {
    pub x: i32,
    pub y: i32,
}

impl ChunkCoord {
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
}

/// Global tile position in tile-space coordinates.
#[derive(Clone, Copy, Hash, Eq, PartialEq, Debug, Component)]
pub struct TileCoord {
    pub x: i32,
    pub y: i32,
}

impl TileCoord {
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
}

/// Continuous world-space position using f64 for precision.
#[derive(Clone, Copy, Debug, Default)]
pub struct WorldPos {
    pub x: f64,
    pub y: f64,
}

impl WorldPos {
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

/// Detail level for the fractal noise hierarchy.
/// Each level provides progressively finer detail.
///
/// - **Macro**: 64×64 samples — world overview tiles (64×64 world units)
/// - **Meso**: 128×128 samples — regional zoom (8×8 world units)
/// - **Micro**: 512×512 samples — playable tilemap (0.25×0.25 world units)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum DetailLevel {
    /// World overview: 64×64 samples
    #[default]
    Macro = 0,
    /// Regional detail: 128×128 samples
    Meso = 1,
    /// Playable street level: 512×512 samples
    Micro = 2,
}

impl DetailLevel {
    /// Returns the number of samples per side for this detail level.
    pub const fn samples_per_side(&self) -> usize {
        match self {
            DetailLevel::Macro => 64,
            DetailLevel::Meso => 128,
            DetailLevel::Micro => 512,
        }
    }

    /// Returns the detail level as a u32 for non-octave uses.
    pub const fn as_u32(&self) -> u32 {
        *self as u32
    }

    /// Returns the octave offset for noise generation.
    /// Each tier adds more octaves for finer detail.
    pub const fn octave_offset(&self) -> u32 {
        match self {
            DetailLevel::Macro => 1,
            DetailLevel::Meso => 2,
            DetailLevel::Micro => 3,
        }
    }
}
