//! Artifact storage for Randlebrot.
//!
//! Manages the `~/.randlebrot/` directory for persisting layer and level artifacts.
//! Layer artifacts contain the full TerrainGen + LifeGen output (BiomeMap, RiverNetwork,
//! LifeGenData, and debug PNG images). Level artifacts contain micro-level BiomeMap data.
//!
//! ## On-Disk Layout
//!
//! ```text
//! ~/.randlebrot/
//! +-- layers/
//! |   +-- <tag>/
//! |       +-- manifest.ron           # LayerManifest
//! |       +-- macro_biome.bin        # bincode: BiomeMap
//! |       +-- river_network.bin      # bincode: RiverNetwork
//! |       +-- lifegen.bin            # bincode: LifeGenData
//! |       +-- images/                # layer PNGs
//! |           +-- biome.png
//! |           +-- heightmap.png
//! |           +-- ...
//! +-- levels/
//!     +-- <tag>/
//!         +-- manifest.ron           # LevelManifest
//!         +-- micro_biome.bin        # bincode: micro-level BiomeMap
//! ```

mod error;
mod manifest;
mod store;

pub use error::ArtifactError;
pub use manifest::{LayerManifest, LevelManifest};
pub use store::{ArtifactKind, ArtifactStore};
