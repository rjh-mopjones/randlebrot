//! Artifact store managing `~/.randlebrot/` for layer and level persistence.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use image::{ImageBuffer, Rgba};
use rb_noise::{BiomeMap, RiverNetwork};
use rb_world::lifegen_data::LifeGenData;

use crate::error::ArtifactError;
use crate::manifest::{LayerManifest, LevelManifest};

// ─── File Names ─────────────────────────────────────────────────────────────

const MANIFEST_FILE: &str = "manifest.ron";
const MACRO_BIOME_FILE: &str = "macro_biome.bin";
const RIVER_NETWORK_FILE: &str = "river_network.bin";
const LIFEGEN_FILE: &str = "lifegen.bin";
const MICRO_BIOME_FILE: &str = "micro_biome.bin";
const IMAGES_DIR: &str = "images";

// ─── Artifact Kind ──────────────────────────────────────────────────────────

/// The kind of artifact (layers or levels).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtifactKind {
    /// Full layer artifact: macro BiomeMap + RiverNetwork + LifeGenData + images.
    Layers,
    /// Level artifact: micro BiomeMap for a specific chunk coordinate.
    Levels,
}

impl ArtifactKind {
    /// Directory name under `~/.randlebrot/`.
    fn dir_name(&self) -> &'static str {
        match self {
            Self::Layers => "layers",
            Self::Levels => "levels",
        }
    }

    /// Human-readable name for error messages.
    fn display_name(&self) -> &'static str {
        match self {
            Self::Layers => "layers",
            Self::Levels => "levels",
        }
    }
}

// ─── ArtifactStore ──────────────────────────────────────────────────────────

/// Manages the `~/.randlebrot/` artifact directory.
///
/// Provides save/load operations for layer artifacts (TerrainGen + LifeGen output)
/// and level artifacts (micro-level BiomeMap).
pub struct ArtifactStore {
    base_path: PathBuf,
}

impl ArtifactStore {
    /// Create a new artifact store at `~/.randlebrot/`.
    ///
    /// Creates the base directory and `layers/` and `levels/` subdirectories
    /// if they do not exist.
    pub fn new() -> Result<Self, ArtifactError> {
        let home = home_dir()?;
        let base_path = home.join(".randlebrot");
        Self::with_base_path(base_path)
    }

    /// Create an artifact store at a custom base path.
    ///
    /// Useful for testing with temporary directories.
    pub fn with_base_path(base_path: PathBuf) -> Result<Self, ArtifactError> {
        for kind in &[ArtifactKind::Layers, ArtifactKind::Levels] {
            let dir = base_path.join(kind.dir_name());
            fs::create_dir_all(&dir).map_err(|e| ArtifactError::Io {
                context: format!("creating directory {}", dir.display()),
                source: e,
            })?;
        }
        Ok(Self { base_path })
    }

    /// Base path of the artifact store (`~/.randlebrot/`).
    pub fn base_path(&self) -> &Path {
        &self.base_path
    }

    // ─── Layer Operations ───────────────────────────────────────────────

    /// Save a complete layer artifact.
    ///
    /// Writes the macro BiomeMap, RiverNetwork, and LifeGenData as bincode files,
    /// any provided layer images as PNGs, and a RON manifest.
    ///
    /// # Arguments
    /// * `tag` - Unique identifier for this artifact (alphanumeric + hyphens + underscores)
    /// * `biome_map` - The macro-level BiomeMap (1024x512)
    /// * `river_network` - The global river network (serialized separately from BiomeMap)
    /// * `lifegen` - The civilisation generation output
    /// * `images` - Map of layer name to RGBA image data (width, height, rgba_bytes)
    /// * `manifest` - Metadata about this generation run
    pub fn save_layers(
        &self,
        tag: &str,
        biome_map: &BiomeMap,
        river_network: &RiverNetwork,
        lifegen: &LifeGenData,
        images: &HashMap<String, (u32, u32, Vec<u8>)>,
        manifest: &LayerManifest,
    ) -> Result<(), ArtifactError> {
        validate_tag(tag)?;
        let dir = self.artifact_dir(ArtifactKind::Layers, tag);
        let images_dir = dir.join(IMAGES_DIR);
        fs::create_dir_all(&images_dir).map_err(|e| ArtifactError::Io {
            context: format!("creating layers directory {}", dir.display()),
            source: e,
        })?;

        // Write bincode files
        write_bincode(&dir.join(MACRO_BIOME_FILE), biome_map, "macro BiomeMap")?;
        write_bincode(
            &dir.join(RIVER_NETWORK_FILE),
            river_network,
            "RiverNetwork",
        )?;
        write_bincode(&dir.join(LIFEGEN_FILE), lifegen, "LifeGenData")?;

        // Write layer images
        for (name, (width, height, rgba_data)) in images {
            let path = images_dir.join(name);
            let img: ImageBuffer<Rgba<u8>, _> =
                ImageBuffer::from_raw(*width, *height, rgba_data.clone()).ok_or_else(|| {
                    ArtifactError::Io {
                        context: format!("image buffer size mismatch for {name}"),
                        source: std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "RGBA data length does not match width*height*4",
                        ),
                    }
                })?;
            img.save(&path).map_err(|e| ArtifactError::Image {
                context: format!("saving layer image {}", path.display()),
                source: e,
            })?;
        }

        // Write manifest (pretty-printed RON)
        write_ron_manifest(&dir.join(MANIFEST_FILE), manifest, "layer manifest")?;

        Ok(())
    }

    /// Load the binary data for a layer artifact.
    ///
    /// Returns the BiomeMap, RiverNetwork, and LifeGenData.
    /// Note: `BiomeMap.river_network` will be `None` after deserialization
    /// (it is `#[serde(skip)]` because of `Arc`). The caller should reconnect
    /// the returned `RiverNetwork` to the `BiomeMap` as needed.
    pub fn load_layers_data(
        &self,
        tag: &str,
    ) -> Result<(BiomeMap, RiverNetwork, LifeGenData), ArtifactError> {
        validate_tag(tag)?;
        let dir = self.artifact_dir(ArtifactKind::Layers, tag);
        if !dir.exists() {
            return Err(ArtifactError::NotFound {
                kind: ArtifactKind::Layers.display_name().to_string(),
                tag: tag.to_string(),
            });
        }

        let biome_map = read_bincode(&dir.join(MACRO_BIOME_FILE), "macro BiomeMap")?;
        let river_network = read_bincode(&dir.join(RIVER_NETWORK_FILE), "RiverNetwork")?;
        let lifegen = read_bincode(&dir.join(LIFEGEN_FILE), "LifeGenData")?;

        Ok((biome_map, river_network, lifegen))
    }

    /// Load only the manifest for a layer artifact (cheap, no binary data).
    pub fn load_layer_manifest(&self, tag: &str) -> Result<LayerManifest, ArtifactError> {
        validate_tag(tag)?;
        let dir = self.artifact_dir(ArtifactKind::Layers, tag);
        if !dir.exists() {
            return Err(ArtifactError::NotFound {
                kind: ArtifactKind::Layers.display_name().to_string(),
                tag: tag.to_string(),
            });
        }
        read_ron_manifest(&dir.join(MANIFEST_FILE), "layer manifest")
    }

    /// Resolve the path to a specific layer image PNG.
    pub fn layer_image_path(&self, tag: &str, layer_name: &str) -> PathBuf {
        self.artifact_dir(ArtifactKind::Layers, tag)
            .join(IMAGES_DIR)
            .join(layer_name)
    }

    // ─── Level Operations ───────────────────────────────────────────────

    /// Save a level artifact (micro-level BiomeMap + manifest).
    pub fn save_level(
        &self,
        tag: &str,
        micro_biome: &BiomeMap,
        manifest: &LevelManifest,
    ) -> Result<(), ArtifactError> {
        validate_tag(tag)?;
        let dir = self.artifact_dir(ArtifactKind::Levels, tag);
        fs::create_dir_all(&dir).map_err(|e| ArtifactError::Io {
            context: format!("creating levels directory {}", dir.display()),
            source: e,
        })?;

        write_bincode(&dir.join(MICRO_BIOME_FILE), micro_biome, "micro BiomeMap")?;
        write_ron_manifest(&dir.join(MANIFEST_FILE), manifest, "level manifest")?;

        Ok(())
    }

    /// Load a level artifact (micro BiomeMap + manifest).
    pub fn load_level(&self, tag: &str) -> Result<(BiomeMap, LevelManifest), ArtifactError> {
        validate_tag(tag)?;
        let dir = self.artifact_dir(ArtifactKind::Levels, tag);
        if !dir.exists() {
            return Err(ArtifactError::NotFound {
                kind: ArtifactKind::Levels.display_name().to_string(),
                tag: tag.to_string(),
            });
        }

        let micro_biome = read_bincode(&dir.join(MICRO_BIOME_FILE), "micro BiomeMap")?;
        let manifest = read_ron_manifest(&dir.join(MANIFEST_FILE), "level manifest")?;

        Ok((micro_biome, manifest))
    }

    // ─── Listing ────────────────────────────────────────────────────────

    /// List all layer artifacts with their manifests.
    ///
    /// Returns `(tag, manifest)` pairs for all valid layer artifacts.
    /// Artifacts with unreadable manifests are silently skipped.
    pub fn list_layers(&self) -> Result<Vec<(String, LayerManifest)>, ArtifactError> {
        self.list_artifacts(ArtifactKind::Layers)
    }

    /// List all level artifacts with their manifests.
    ///
    /// Returns `(tag, manifest)` pairs for all valid level artifacts.
    /// Artifacts with unreadable manifests are silently skipped.
    pub fn list_levels(&self) -> Result<Vec<(String, LevelManifest)>, ArtifactError> {
        self.list_artifacts(ArtifactKind::Levels)
    }

    // ─── Deletion & Existence ───────────────────────────────────────────

    /// Delete an artifact directory.
    pub fn delete(&self, kind: ArtifactKind, tag: &str) -> Result<(), ArtifactError> {
        validate_tag(tag)?;
        let dir = self.artifact_dir(kind, tag);
        if !dir.exists() {
            return Err(ArtifactError::NotFound {
                kind: kind.display_name().to_string(),
                tag: tag.to_string(),
            });
        }
        fs::remove_dir_all(&dir).map_err(|e| ArtifactError::Io {
            context: format!("deleting {} artifact '{}'", kind.display_name(), tag),
            source: e,
        })
    }

    /// Check whether an artifact with the given kind and tag exists.
    pub fn exists(&self, kind: ArtifactKind, tag: &str) -> bool {
        if validate_tag(tag).is_err() {
            return false;
        }
        self.artifact_dir(kind, tag).exists()
    }

    // ─── Internal Helpers ───────────────────────────────────────────────

    fn artifact_dir(&self, kind: ArtifactKind, tag: &str) -> PathBuf {
        self.base_path.join(kind.dir_name()).join(tag)
    }

    fn list_artifacts<M>(&self, kind: ArtifactKind) -> Result<Vec<(String, M)>, ArtifactError>
    where
        M: serde::de::DeserializeOwned,
    {
        let parent = self.base_path.join(kind.dir_name());
        let entries = fs::read_dir(&parent).map_err(|e| ArtifactError::Io {
            context: format!("reading {} directory", kind.display_name()),
            source: e,
        })?;

        let mut results = Vec::new();
        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let tag = match path.file_name().and_then(|n| n.to_str()) {
                Some(name) => name.to_string(),
                None => continue,
            };
            let manifest_path = path.join(MANIFEST_FILE);
            if !manifest_path.exists() {
                continue;
            }
            match read_ron_manifest::<M>(&manifest_path, "manifest") {
                Ok(manifest) => results.push((tag, manifest)),
                Err(_) => continue,
            }
        }

        results.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(results)
    }
}

// ─── Tag Validation ─────────────────────────────────────────────────────────

/// Validate an artifact tag.
///
/// Tags must be non-empty and contain only alphanumeric characters, hyphens, and underscores.
fn validate_tag(tag: &str) -> Result<(), ArtifactError> {
    if tag.is_empty() {
        return Err(ArtifactError::InvalidTag {
            tag: tag.to_string(),
            reason: "tag must not be empty",
        });
    }
    if !tag
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(ArtifactError::InvalidTag {
            tag: tag.to_string(),
            reason: "tag must contain only alphanumeric characters, hyphens, and underscores",
        });
    }
    Ok(())
}

// ─── Serialization Helpers ──────────────────────────────────────────────────

fn write_bincode<T: serde::Serialize>(
    path: &Path,
    value: &T,
    context: &str,
) -> Result<(), ArtifactError> {
    let data = bincode::serialize(value).map_err(|e| ArtifactError::Bincode {
        context: format!("serializing {context}"),
        source: e,
    })?;
    fs::write(path, &data).map_err(|e| ArtifactError::Io {
        context: format!("writing {context} to {}", path.display()),
        source: e,
    })
}

fn read_bincode<T: serde::de::DeserializeOwned>(
    path: &Path,
    context: &str,
) -> Result<T, ArtifactError> {
    if !path.exists() {
        return Err(ArtifactError::FileNotFound {
            path: path.to_path_buf(),
        });
    }
    let data = fs::read(path).map_err(|e| ArtifactError::Io {
        context: format!("reading {context} from {}", path.display()),
        source: e,
    })?;
    bincode::deserialize(&data).map_err(|e| ArtifactError::Bincode {
        context: format!("deserializing {context} from {}", path.display()),
        source: e,
    })
}

fn write_ron_manifest<T: serde::Serialize>(
    path: &Path,
    value: &T,
    context: &str,
) -> Result<(), ArtifactError> {
    let pretty = ron::ser::PrettyConfig::default();
    let text =
        ron::ser::to_string_pretty(value, pretty).map_err(|e| ArtifactError::RonSerialize {
            context: format!("serializing {context}"),
            source: e,
        })?;
    fs::write(path, text.as_bytes()).map_err(|e| ArtifactError::Io {
        context: format!("writing {context} to {}", path.display()),
        source: e,
    })
}

fn read_ron_manifest<T: serde::de::DeserializeOwned>(
    path: &Path,
    context: &str,
) -> Result<T, ArtifactError> {
    if !path.exists() {
        return Err(ArtifactError::FileNotFound {
            path: path.to_path_buf(),
        });
    }
    let text = fs::read_to_string(path).map_err(|e| ArtifactError::Io {
        context: format!("reading {context} from {}", path.display()),
        source: e,
    })?;
    ron::de::from_str(&text).map_err(|e| ArtifactError::RonDeserialize {
        context: format!("deserializing {context} from {}", path.display()),
        source: e,
    })
}

// ─── Home Directory ─────────────────────────────────────────────────────────

fn home_dir() -> Result<PathBuf, ArtifactError> {
    std::env::var("HOME")
        .map(PathBuf::from)
        .map_err(|_| ArtifactError::NoHomeDirectory)
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{LayerManifest, LevelManifest};
    use rb_core::TileType;
    use rb_noise::RiverNetwork;

    /// Create a minimal BiomeMap for testing.
    fn make_test_biome_map() -> BiomeMap {
        let w = 4;
        let h = 4;
        let n = w * h;
        BiomeMap {
            width: w,
            height: h,
            continentalness: vec![0.0; n],
            tectonic: vec![0.0; n],
            tectonic_plate_ids: vec![0.0; n],
            humidity: vec![0.5; n],
            rock_hardness: vec![0.5; n],
            light_level: vec![0.5; n],
            peaks_valleys: vec![0.0; n],
            volcanism: vec![0.0; n],
            heightmap: vec![0.1; n],
            temperature: vec![20.0; n],
            erosion: vec![0.0; n],
            rivers: vec![0.0; n],
            aridity: vec![0.3; n],
            precipitation_type: vec![0.0; n],
            water_table: vec![0.2; n],
            wind_speed: vec![0.1; n],
            resource_richness: vec![0.0; n],
            snowpack: vec![0.0; n],
            biomes: vec![TileType::Plains; n],
            vegetation_density: vec![0.5; n],
            soil_type: vec![0.5; n],
            resource_map: None,
            river_network: None,
            drainage_area: vec![0; n],
            sediment: vec![0.0; n],
            world_width: 4.0,
            world_height: 4.0,
        }
    }

    /// Create a minimal RiverNetwork for testing via serde round-trip.
    fn make_test_river_network() -> RiverNetwork {
        // RiverNetwork has a private spatial_index field with #[serde(skip)],
        // so we construct an empty one via serde deserialization.
        let ron_str = "(segments: [], lakes: [])";
        ron::de::from_str(ron_str).expect("failed to deserialize empty RiverNetwork")
    }

    /// Create a minimal LifeGenData for testing.
    fn make_test_lifegen_data() -> LifeGenData {
        let w = 8;
        let h = 4;
        let n = w * h;
        LifeGenData {
            width: w,
            height: h,
            habitability: vec![0.5; n],
            navigation_cost: vec![1.0; n],
            resource_desirability: vec![0.3; n],
            province_ids: vec![0u16; n],
            provinces: vec![],
            faction_ids: vec![0u32; n],
            factions: vec![],
            settlement_seeds: vec![],
            road_segments: vec![],
            trade_edges: vec![],
        }
    }

    fn make_layer_manifest() -> LayerManifest {
        LayerManifest {
            seed: 42,
            civ_seed: 99,
            created: "2026-01-15T12:00:00Z".to_string(),
            world_width: 1024,
            world_height: 512,
            backend: "cpu".to_string(),
            layer_images: vec!["biome.png".to_string()],
        }
    }

    fn make_level_manifest() -> LevelManifest {
        LevelManifest {
            parent_layers_tag: Some("test-layers".to_string()),
            seed: 42,
            civ_seed: 99,
            chunk_coord: (4, 3),
            created: "2026-01-15T12:30:00Z".to_string(),
        }
    }

    #[test]
    fn tag_validation_accepts_valid_tags() {
        assert!(validate_tag("my-layers-42").is_ok());
        assert!(validate_tag("test_tag").is_ok());
        assert!(validate_tag("CamelCase123").is_ok());
        assert!(validate_tag("a").is_ok());
    }

    #[test]
    fn tag_validation_rejects_empty() {
        assert!(validate_tag("").is_err());
    }

    #[test]
    fn tag_validation_rejects_spaces() {
        assert!(validate_tag("has space").is_err());
    }

    #[test]
    fn tag_validation_rejects_slashes() {
        assert!(validate_tag("path/traversal").is_err());
        assert!(validate_tag("../escape").is_err());
    }

    #[test]
    fn tag_validation_rejects_dots() {
        assert!(validate_tag("has.dot").is_err());
    }

    #[test]
    fn round_trip_layers() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ArtifactStore::with_base_path(tmp.path().to_path_buf()).unwrap();

        let biome_map = make_test_biome_map();
        let river_network = make_test_river_network();
        let lifegen = make_test_lifegen_data();
        let manifest = make_layer_manifest();

        // Create a small test image (2x2 RGBA)
        let mut images = HashMap::new();
        let rgba_data = vec![255u8; 2 * 2 * 4];
        images.insert("biome.png".to_string(), (2, 2, rgba_data));

        store
            .save_layers("test-layers", &biome_map, &river_network, &lifegen, &images, &manifest)
            .unwrap();

        // Verify manifest round-trip
        let loaded_manifest = store.load_layer_manifest("test-layers").unwrap();
        assert_eq!(loaded_manifest, manifest);

        // Verify binary data round-trip
        let (loaded_bm, loaded_rn, loaded_lg) = store.load_layers_data("test-layers").unwrap();
        assert_eq!(loaded_bm.width, biome_map.width);
        assert_eq!(loaded_bm.height, biome_map.height);
        assert_eq!(loaded_bm.continentalness, biome_map.continentalness);
        assert_eq!(loaded_bm.temperature, biome_map.temperature);
        assert_eq!(loaded_bm.biomes, biome_map.biomes);
        assert_eq!(loaded_rn.segments.len(), river_network.segments.len());
        assert_eq!(loaded_lg.width, lifegen.width);
        assert_eq!(loaded_lg.height, lifegen.height);
        assert_eq!(loaded_lg.habitability, lifegen.habitability);

        // BiomeMap.river_network should be None (serde skip)
        assert!(loaded_bm.river_network.is_none());

        // Image file should exist
        let img_path = store.layer_image_path("test-layers", "biome.png");
        assert!(img_path.exists());
    }

    #[test]
    fn round_trip_level() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ArtifactStore::with_base_path(tmp.path().to_path_buf()).unwrap();

        let micro_biome = make_test_biome_map();
        let manifest = make_level_manifest();

        store
            .save_level("test-level", &micro_biome, &manifest)
            .unwrap();

        let (loaded_bm, loaded_manifest) = store.load_level("test-level").unwrap();
        assert_eq!(loaded_manifest, manifest);
        assert_eq!(loaded_bm.width, micro_biome.width);
        assert_eq!(loaded_bm.biomes, micro_biome.biomes);
    }

    #[test]
    fn list_layers_returns_saved_artifacts() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ArtifactStore::with_base_path(tmp.path().to_path_buf()).unwrap();

        let biome_map = make_test_biome_map();
        let river_network = make_test_river_network();
        let lifegen = make_test_lifegen_data();
        let images = HashMap::new();

        let manifest_a = LayerManifest {
            seed: 1,
            ..make_layer_manifest()
        };
        let manifest_b = LayerManifest {
            seed: 2,
            ..make_layer_manifest()
        };

        store
            .save_layers("alpha", &biome_map, &river_network, &lifegen, &images, &manifest_a)
            .unwrap();
        store
            .save_layers("beta", &biome_map, &river_network, &lifegen, &images, &manifest_b)
            .unwrap();

        let listed = store.list_layers().unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].0, "alpha");
        assert_eq!(listed[0].1.seed, 1);
        assert_eq!(listed[1].0, "beta");
        assert_eq!(listed[1].1.seed, 2);
    }

    #[test]
    fn list_levels_returns_saved_artifacts() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ArtifactStore::with_base_path(tmp.path().to_path_buf()).unwrap();

        let micro_biome = make_test_biome_map();
        let manifest = make_level_manifest();

        store
            .save_level("level-one", &micro_biome, &manifest)
            .unwrap();

        let listed = store.list_levels().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].0, "level-one");
        assert_eq!(listed[0].1.chunk_coord, (4, 3));
    }

    #[test]
    fn exists_returns_correct_values() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ArtifactStore::with_base_path(tmp.path().to_path_buf()).unwrap();

        assert!(!store.exists(ArtifactKind::Layers, "nonexistent"));

        let biome_map = make_test_biome_map();
        let river_network = make_test_river_network();
        let lifegen = make_test_lifegen_data();
        let images = HashMap::new();
        let manifest = make_layer_manifest();

        store
            .save_layers("exists-test", &biome_map, &river_network, &lifegen, &images, &manifest)
            .unwrap();

        assert!(store.exists(ArtifactKind::Layers, "exists-test"));
        assert!(!store.exists(ArtifactKind::Levels, "exists-test"));
    }

    #[test]
    fn delete_removes_artifact() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ArtifactStore::with_base_path(tmp.path().to_path_buf()).unwrap();

        let biome_map = make_test_biome_map();
        let river_network = make_test_river_network();
        let lifegen = make_test_lifegen_data();
        let images = HashMap::new();
        let manifest = make_layer_manifest();

        store
            .save_layers("to-delete", &biome_map, &river_network, &lifegen, &images, &manifest)
            .unwrap();
        assert!(store.exists(ArtifactKind::Layers, "to-delete"));

        store.delete(ArtifactKind::Layers, "to-delete").unwrap();
        assert!(!store.exists(ArtifactKind::Layers, "to-delete"));
    }

    #[test]
    fn delete_nonexistent_returns_error() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ArtifactStore::with_base_path(tmp.path().to_path_buf()).unwrap();

        let result = store.delete(ArtifactKind::Layers, "ghost");
        assert!(result.is_err());
    }

    #[test]
    fn load_nonexistent_returns_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ArtifactStore::with_base_path(tmp.path().to_path_buf()).unwrap();

        let result = store.load_layers_data("missing");
        assert!(result.is_err());

        let result = store.load_level("missing");
        assert!(result.is_err());
    }

    #[test]
    fn manifest_is_pretty_printed_ron() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ArtifactStore::with_base_path(tmp.path().to_path_buf()).unwrap();

        let biome_map = make_test_biome_map();
        let river_network = make_test_river_network();
        let lifegen = make_test_lifegen_data();
        let images = HashMap::new();
        let manifest = make_layer_manifest();

        store
            .save_layers("pretty-test", &biome_map, &river_network, &lifegen, &images, &manifest)
            .unwrap();

        let manifest_path = store
            .artifact_dir(ArtifactKind::Layers, "pretty-test")
            .join(MANIFEST_FILE);
        let text = fs::read_to_string(manifest_path).unwrap();

        // Pretty-printed RON should contain newlines and indentation
        assert!(text.contains('\n'));
        assert!(text.contains("seed"));
        assert!(text.contains("42"));
    }

    #[test]
    fn with_base_path_creates_directories() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().join("custom-root");
        let _store = ArtifactStore::with_base_path(base.clone()).unwrap();

        assert!(base.join("layers").exists());
        assert!(base.join("levels").exists());
    }
}
