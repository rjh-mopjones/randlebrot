//! Manifest types for layer and level artifacts.
//!
//! Manifests are stored as pretty-printed RON files for human readability.
//! They contain metadata about the artifact without the heavy binary data.

use serde::{Deserialize, Serialize};

/// Metadata for a layer artifact (TerrainGen + LifeGen output).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LayerManifest {
    /// Terrain generation seed.
    pub seed: u32,
    /// Civilisation generation seed.
    pub civ_seed: u32,
    /// ISO 8601 timestamp of creation.
    pub created: String,
    /// World width in world units.
    pub world_width: u32,
    /// World height in world units.
    pub world_height: u32,
    /// Noise backend used for generation ("cpu" or "gpu").
    pub backend: String,
    /// List of available layer image filenames (e.g. ["biome.png", "heightmap.png"]).
    pub layer_images: Vec<String>,
}

/// Metadata for a level artifact (micro-level BiomeMap).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LevelManifest {
    /// Tag of the parent layers artifact this level was generated from, if any.
    pub parent_layers_tag: Option<String>,
    /// Terrain generation seed.
    pub seed: u32,
    /// Civilisation generation seed.
    pub civ_seed: u32,
    /// Micro-level chunk coordinate (x, y).
    pub micro_coord: (i32, i32),
    /// ISO 8601 timestamp of creation.
    pub created: String,
}
