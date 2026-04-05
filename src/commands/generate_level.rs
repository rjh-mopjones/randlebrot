//! Headless implementation of `generate level <layers-tag|--seed N> <x,y> <tag>`.
//!
//! Generates a micro-level `BiomeMap` at a specific global micro coordinate
//! and saves it as a level artifact. Two code paths:
//!
//! 1. **Layers path** — loads a previously generated layers artifact (macro
//!    `BiomeMap` + `RiverNetwork`) and samples a micro tile from it. This is
//!    the fast, LifeGen-aware path.
//! 2. **Seed path** — generates the macro `BiomeMap` (with erosion + rivers)
//!    in memory from a raw seed and samples a micro tile. Slower, terrain-only.
//!
//! Both paths invoke `BiomeMap::generate_meso_full_with_backend` at
//! `detail_level = 3` (micro) with the same world position, so identical
//! `seed + coord` inputs yield identical terrain.
//!
//! ## CLI micro coordinate convention
//!
//! A CLI `micro_coord` is a **global** `(mx, my)` pair where
//! `mx ∈ [0, MICRO_GRID_WIDTH)` and `my ∈ [0, MICRO_GRID_HEIGHT)`. The
//! mapping to world position is:
//!
//! ```text
//! world_x = mx * MICRO_WORLD_SIZE
//! world_y = my * MICRO_WORLD_SIZE
//! ```
//!
//! With the current constants (`MICRO_WORLD_SIZE = 1.0`, world 1024×512),
//! there are `1024 × 512 = 524,288` global micro tiles, and the global
//! coordinate equals the integer world position of the tile's top-left
//! corner.

use std::sync::Arc;

use indicatif::{ProgressBar, ProgressStyle};

use rb_artifacts::{ArtifactKind, ArtifactStore, LevelManifest};
use rb_noise::{BiomeMap, NoiseBackend, RiverNetwork};

use crate::cli::coords::{
    micro_coord_to_world_pos, validate_micro_coord, MICRO_OUTPUT_SIZE, MICRO_WORLD_SIZE,
    WORLD_HEIGHT, WORLD_WIDTH,
};

// ─── Entry Points ───────────────────────────────────────────────────────────

/// Run `generate level <layers-tag> <x,y> <level-tag>`.
///
/// Loads the cached macro `BiomeMap` + `RiverNetwork` from the layers
/// artifact, generates a single micro-level tile at `micro_coord`, and saves
/// the result as a level artifact tagged `level_tag`.
pub fn run_from_layers(
    layers_tag: String,
    micro_coord: (i32, i32),
    level_tag: String,
    backend: NoiseBackend,
    force: bool,
) -> Result<(), String> {
    let backend_label = backend_label(backend);
    println!(
        "generate level: layers_tag={layers_tag}, coord=({},{}), \
         level_tag={level_tag}, backend={backend_label}, force={force}",
        micro_coord.0, micro_coord.1,
    );

    // ─── 0. Prepare store + validate inputs ─────────────────────────────────
    let store = ArtifactStore::new()
        .map_err(|e| format!("failed to initialise artifact store at ~/.randlebrot: {e}"))?;

    // Validate the coordinate *before* touching the filesystem. Otherwise a
    // typo combined with `--force` would delete the existing level artifact
    // inside `check_level_tag_available` before returning the coord error.
    validate_micro_coord(micro_coord).map_err(|e| e.to_string())?;
    check_level_tag_available(&store, &level_tag, force)?;

    // ─── 1. Load the layers artifact ────────────────────────────────────────
    let stage = stage_spinner(&format!("[1/3] Loading layers artifact '{layers_tag}'"));
    let layers_manifest = store
        .load_layer_manifest(&layers_tag)
        .map_err(|e| format!("failed to load layer manifest '{layers_tag}': {e}"))?;
    let (macro_biome, river_network, _lifegen) = store
        .load_layers_data(&layers_tag)
        .map_err(|e| format!("failed to load layer data '{layers_tag}': {e}"))?;
    stage.finish_with_message(format!(
        "[1/3] Loaded '{layers_tag}' (seed={}, civ_seed={})",
        layers_manifest.seed, layers_manifest.civ_seed
    ));

    // Wrap river network in an Arc so we can hand a reference to the
    // `generate_meso_full_with_backend` call (it takes `Option<&Arc<RiverNetwork>>`).
    let river_network_arc: Arc<RiverNetwork> = Arc::new(river_network);

    // ─── 2. Generate the micro tile ─────────────────────────────────────────
    let (world_x, world_y) = micro_coord_to_world_pos(micro_coord);
    let stage = stage_spinner(&format!(
        "[2/3] Generating micro tile at world ({world_x:.2}, {world_y:.2})"
    ));
    let micro_biome = BiomeMap::generate_meso_full_with_backend(
        layers_manifest.seed,
        world_x,
        world_y,
        MICRO_WORLD_SIZE,
        MICRO_OUTPUT_SIZE,
        layers_manifest.world_height as f64,
        3, // detail_level = 3 (micro)
        None,
        backend,
        Some(&macro_biome),
        Some(&river_network_arc),
    );
    stage.finish_with_message(format!(
        "[2/3] Micro tile done ({}x{} pixels)",
        micro_biome.width, micro_biome.height
    ));

    // ─── 3. Save the level artifact ─────────────────────────────────────────
    let stage = stage_spinner("[3/3] Saving level artifact");
    let level_manifest = LevelManifest {
        parent_layers_tag: Some(layers_tag.clone()),
        seed: layers_manifest.seed,
        civ_seed: layers_manifest.civ_seed,
        micro_coord,
        created: chrono::Utc::now().to_rfc3339(),
    };
    store
        .save_level(&level_tag, &micro_biome, &level_manifest)
        .map_err(|e| format!("failed to save level artifact '{level_tag}': {e}"))?;
    stage.finish_with_message("[3/3] Level artifact saved");

    let level_dir = store.base_path().join("levels").join(&level_tag);
    println!("Done. Saved to {}", level_dir.display());
    Ok(())
}

/// Run `generate level --seed N <x,y> <level-tag>`.
///
/// Generates the macro `BiomeMap` in memory from `seed` (including erosion
/// and river network), then samples a single micro-level tile at
/// `micro_coord`. No LifeGen pipeline runs — the level artifact is pure
/// terrain with `parent_layers_tag = None`.
pub fn run_from_seed(
    seed: u32,
    micro_coord: (i32, i32),
    level_tag: String,
    backend: NoiseBackend,
    force: bool,
) -> Result<(), String> {
    let backend_label = backend_label(backend);
    println!(
        "generate level: seed={seed}, coord=({},{}), level_tag={level_tag}, \
         backend={backend_label}, force={force}",
        micro_coord.0, micro_coord.1,
    );

    // ─── 0. Prepare store + validate inputs ─────────────────────────────────
    let store = ArtifactStore::new()
        .map_err(|e| format!("failed to initialise artifact store at ~/.randlebrot: {e}"))?;

    // Validate the coordinate *before* touching the filesystem. Otherwise a
    // typo combined with `--force` would delete the existing level artifact
    // inside `check_level_tag_available` before returning the coord error.
    validate_micro_coord(micro_coord).map_err(|e| e.to_string())?;
    check_level_tag_available(&store, &level_tag, force)?;

    // ─── 1. Generate macro BiomeMap in memory (erosion + rivers) ────────────
    let stage = stage_spinner(
        "[1/3] Generating macro BiomeMap in memory (1024x512, erosion + rivers)",
    );
    let macro_biome =
        BiomeMap::generate_with_backend(seed, WORLD_WIDTH, WORLD_HEIGHT, backend);
    let river_network_arc: Option<Arc<RiverNetwork>> = macro_biome.river_network.clone();
    let river_msg = match river_network_arc.as_ref() {
        Some(net) => format!(
            "{} segments, {} lakes",
            net.segment_count(),
            net.lakes.len()
        ),
        None => "none (fallback flat grid)".to_string(),
    };
    stage.finish_with_message(format!("[1/3] Macro BiomeMap done — river network: {river_msg}"));

    // ─── 2. Generate the micro tile ─────────────────────────────────────────
    let (world_x, world_y) = micro_coord_to_world_pos(micro_coord);
    let stage = stage_spinner(&format!(
        "[2/3] Generating micro tile at world ({world_x:.2}, {world_y:.2})"
    ));
    let micro_biome = BiomeMap::generate_meso_full_with_backend(
        seed,
        world_x,
        world_y,
        MICRO_WORLD_SIZE,
        MICRO_OUTPUT_SIZE,
        WORLD_HEIGHT as f64,
        3, // detail_level = 3 (micro)
        None,
        backend,
        Some(&macro_biome),
        river_network_arc.as_ref(),
    );
    stage.finish_with_message(format!(
        "[2/3] Micro tile done ({}x{} pixels)",
        micro_biome.width, micro_biome.height
    ));

    // ─── 3. Save the level artifact ─────────────────────────────────────────
    let stage = stage_spinner("[3/3] Saving level artifact");
    let level_manifest = LevelManifest {
        parent_layers_tag: None,
        seed,
        // No civ pipeline ran — reuse the terrain seed as a placeholder so
        // downstream tooling always has a concrete u32.
        civ_seed: seed,
        micro_coord,
        created: chrono::Utc::now().to_rfc3339(),
    };
    store
        .save_level(&level_tag, &micro_biome, &level_manifest)
        .map_err(|e| format!("failed to save level artifact '{level_tag}': {e}"))?;
    stage.finish_with_message("[3/3] Level artifact saved");

    let level_dir = store.base_path().join("levels").join(&level_tag);
    println!("Done. Saved to {}", level_dir.display());
    Ok(())
}

// ─── Helpers ────────────────────────────────────────────────────────────────

fn backend_label(backend: NoiseBackend) -> &'static str {
    match backend {
        NoiseBackend::Gpu => "gpu",
        NoiseBackend::Cpu => "cpu",
    }
}

/// Ensure the target level tag either doesn't exist or `--force` is set.
/// If `--force` is set and the tag exists, delete it so `save_level` starts
/// from a clean directory.
fn check_level_tag_available(
    store: &ArtifactStore,
    level_tag: &str,
    force: bool,
) -> Result<(), String> {
    if store.exists(ArtifactKind::Levels, level_tag) {
        if !force {
            return Err(format!(
                "level artifact '{level_tag}' already exists at {}\n\
                 pass --force to overwrite it.",
                store.base_path().join("levels").join(level_tag).display(),
            ));
        }
        println!("  --force: removing existing level artifact '{level_tag}'");
        store
            .delete(ArtifactKind::Levels, level_tag)
            .map_err(|e| format!("failed to delete existing level artifact '{level_tag}': {e}"))?;
    }
    Ok(())
}

fn stage_spinner(msg: &str) -> ProgressBar {
    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::with_template("{spinner:.cyan} {msg}")
            .unwrap_or_else(|_| ProgressStyle::default_spinner()),
    );
    pb.enable_steady_tick(std::time::Duration::from_millis(120));
    pb.set_message(msg.to_string());
    pb
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::coords::{
        micro_coord_to_world_pos, validate_micro_coord, MICRO_GRID_HEIGHT, MICRO_GRID_WIDTH,
    };

    #[test]
    fn micro_coord_maps_to_world_position() {
        // Global (0, 0) is the top-left corner of the world.
        assert_eq!(micro_coord_to_world_pos((0, 0)), (0.0, 0.0));
        // Global (512, 256) is the middle of the 1024×512 world.
        assert_eq!(micro_coord_to_world_pos((512, 256)), (512.0, 256.0));
        // Global (1023, 511) is the bottom-right interior pixel.
        assert_eq!(micro_coord_to_world_pos((1023, 511)), (1023.0, 511.0));
    }

    #[test]
    fn validate_micro_coord_accepts_in_range() {
        assert!(validate_micro_coord((0, 0)).is_ok());
        assert!(validate_micro_coord((MICRO_GRID_WIDTH - 1, MICRO_GRID_HEIGHT - 1)).is_ok());
        assert!(validate_micro_coord((512, 256)).is_ok());
    }

    #[test]
    fn validate_micro_coord_rejects_negative() {
        assert!(validate_micro_coord((-1, 0)).is_err());
        assert!(validate_micro_coord((0, -1)).is_err());
    }

    #[test]
    fn validate_micro_coord_rejects_out_of_bounds() {
        assert!(validate_micro_coord((MICRO_GRID_WIDTH, 0)).is_err());
        assert!(validate_micro_coord((0, MICRO_GRID_HEIGHT)).is_err());
        assert!(validate_micro_coord((100_000, 100_000)).is_err());
    }
}
