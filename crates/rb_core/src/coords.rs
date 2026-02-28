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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum DetailLevel {
    /// Coarsest view: 32×32 samples
    #[default]
    Macro = 0,
    /// Medium detail: 64×64 samples
    Meso = 1,
    /// Highest detail (street level): 128×128 samples
    Micro = 2,
}

impl DetailLevel {
    /// Returns the number of samples per side for this detail level.
    pub const fn samples_per_side(&self) -> usize {
        match self {
            DetailLevel::Macro => 32,
            DetailLevel::Meso => 64,
            DetailLevel::Micro => 128,
        }
    }

    /// Returns the detail level as a u32 for noise generation.
    pub const fn as_u32(&self) -> u32 {
        *self as u32
    }
}
