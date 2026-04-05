//! Headless implementation of `generate layers <seed> <tag>`.
//!
//! Runs the full TerrainGen + LifeGen pipeline without Bevy and persists
//! everything via `rb_artifacts`. Replaces the old workflow of
//! "launch the GUI, click Generate, wait, then check `debug_layers/`".
//!
//! Pipeline phases:
//! 1. Macro `BiomeMap::generate_with_backend` (handles erosion + rivers internally)
//! 2. 128 macro tiles via rayon (16x8 grid, 512x512 each, detail_level=1)
//! 3. Global heightmap normalisation hints
//! 4. Stitch ~20 layer PNGs (4096x2048 each, downscaled 2x from 8192x4096)
//! 5. `MesoTerrainView::from_tile_map` → `rb_world::lifegen::generate`
//! 6. Persist macro BiomeMap, RiverNetwork, LifeGenData, and layer images
//!    via `rb_artifacts::ArtifactStore::save_layers`.

use std::collections::HashMap;
use std::sync::Arc;

use indicatif::{ProgressBar, ProgressStyle};
use rayon::prelude::*;

use rb_artifacts::{ArtifactKind, ArtifactStore, LayerManifest};
use rb_noise::{
    BiomeMap, MesoTerrainView, NoiseBackend, NoiseLayer, NormalizationHints, RiverNetwork,
};

// ─── Constants (mirror `src/main.rs`) ───────────────────────────────────────

/// World width in world units (full planet).
const WORLD_WIDTH: usize = 1024;
/// World height in world units.
const WORLD_HEIGHT: usize = 512;
/// Size of a macro chunk in world units (64x64 block).
const CHUNK_SIZE: f64 = 64.0;
/// Output resolution per macro tile (pixels per side).
const TILE_MAP_SIZE: usize = 512;
/// Macro tile grid (16 wide, 8 tall = 128 tiles covering the full world).
const TILES_X: usize = 16;
const TILES_Y: usize = 8;

// ─── Entry Point ────────────────────────────────────────────────────────────

/// Run the headless `generate layers` pipeline.
///
/// Returns `Ok(())` on success and a human-readable error string on failure.
/// The error is formatted for terminal output (no trailing newline).
pub fn run(
    seed: u32,
    tag: String,
    civ_seed: Option<u32>,
    backend: NoiseBackend,
    force: bool,
) -> Result<(), String> {
    let civ_seed = civ_seed.unwrap_or(seed);
    let backend_label = match backend {
        NoiseBackend::Gpu => "gpu",
        NoiseBackend::Cpu => "cpu",
    };

    println!(
        "generate layers: seed={seed}, tag={tag}, civ_seed={civ_seed}, backend={backend_label}, force={force}"
    );

    // ─── 0. Prepare artifact store ──────────────────────────────────────────
    let store = ArtifactStore::new()
        .map_err(|e| format!("failed to initialise artifact store at ~/.randlebrot: {e}"))?;

    if store.exists(ArtifactKind::Layers, &tag) {
        if !force {
            return Err(format!(
                "layer artifact '{tag}' already exists at {}\n\
                 pass --force to overwrite it.",
                store
                    .base_path()
                    .join("layers")
                    .join(&tag)
                    .display(),
            ));
        }
        println!("  --force: removing existing artifact '{tag}'");
        store
            .delete(ArtifactKind::Layers, &tag)
            .map_err(|e| format!("failed to delete existing artifact '{tag}': {e}"))?;
    }

    // ─── 1. Macro BiomeMap (erosion + rivers baked in) ──────────────────────
    let stage = stage_spinner("[1/6] Generating macro BiomeMap (1024x512, erosion + rivers)");
    let mut macro_biome =
        BiomeMap::generate_with_backend(seed, WORLD_WIDTH, WORLD_HEIGHT, backend);
    stage.finish_with_message("[1/6] Macro BiomeMap done");

    // Keep an Arc clone of the river network for the meso tile pass and LifeGen
    // (they read it through `&Arc<RiverNetwork>`); it will later be moved out of
    // the BiomeMap via `take()` for persistence.
    let river_network_arc: Option<Arc<RiverNetwork>> = macro_biome.river_network.clone();
    if let Some(ref net) = river_network_arc {
        println!(
            "       river network: {} segments, {} lakes",
            net.segment_count(),
            net.lakes.len()
        );
    } else {
        println!("       river network: (none; fallback flat grid)");
    }

    // ─── 2. 128 macro tiles (rayon, detail_level=1) ─────────────────────────
    let total_tiles = TILES_X * TILES_Y;
    let bar = tile_progress_bar(total_tiles as u64, "[2/6] Macro tiles");

    let river_ref_opt: Option<&Arc<RiverNetwork>> = river_network_arc.as_ref();

    // Build coordinate list once so rayon can split evenly.
    let coords: Vec<(usize, usize)> = (0..TILES_Y)
        .flat_map(|cy| (0..TILES_X).map(move |cx| (cx, cy)))
        .collect();

    let tiles: Vec<((i32, i32), BiomeMap)> = coords
        .par_iter()
        .map(|&(cx, cy)| {
            let world_x = cx as f64 * CHUNK_SIZE;
            let world_y = cy as f64 * CHUNK_SIZE;
            let tile = BiomeMap::generate_meso_full_with_backend(
                seed,
                world_x,
                world_y,
                CHUNK_SIZE,
                TILE_MAP_SIZE,
                WORLD_HEIGHT as f64,
                1, // detail_level = macro (octave_offset)
                None,
                backend,
                Some(&macro_biome),
                river_ref_opt,
            );
            bar.inc(1);
            ((cx as i32, cy as i32), tile)
        })
        .collect();
    bar.finish_with_message("[2/6] Macro tiles done");

    // ─── 3. Global heightmap normalisation hints ────────────────────────────
    let stage = stage_spinner("[3/6] Computing global normalisation hints");
    let mut hmin = f64::MAX;
    let mut hmax = f64::MIN;
    for (_, tile) in &tiles {
        for &v in &tile.heightmap {
            if v < hmin {
                hmin = v;
            }
            if v > hmax {
                hmax = v;
            }
        }
    }
    let norm_hints = NormalizationHints {
        heightmap_min: if hmin < hmax { hmin } else { 0.0 },
        heightmap_max: if hmin < hmax { hmax } else { 1.0 },
    };
    stage.finish_with_message(format!(
        "[3/6] Heightmap range [{:.4}, {:.4}]",
        norm_hints.heightmap_min, norm_hints.heightmap_max
    ));

    // ─── 4. Stitch layer PNGs (8192x4096 → 4096x2048) ───────────────────────
    let layer_count = NoiseLayer::all().len();
    let layer_bar = tile_progress_bar(layer_count as u64, "[4/6] Stitching layer PNGs");
    let (images, layer_image_names) = stitch_layer_images(&tiles, &norm_hints, &layer_bar);
    layer_bar.finish_with_message(format!("[4/6] Stitched {} layer PNGs", layer_image_names.len()));

    // ─── 5. Build MesoTerrainView and run LifeGen ───────────────────────────
    let stage = stage_spinner(
        "[5/6] LifeGen (building 8192x4096 MesoTerrainView + civilisation pipeline)",
    );
    let tile_biome_maps: HashMap<(i32, i32), Arc<BiomeMap>> = tiles
        .into_iter()
        .map(|(coord, bm)| (coord, Arc::new(bm)))
        .collect();
    let terrain_view =
        MesoTerrainView::from_tile_map(&tile_biome_maps, TILES_X, TILES_Y, TILE_MAP_SIZE);
    let lifegen = rb_world::lifegen::generate(&terrain_view, civ_seed);
    stage.finish_with_message(format!(
        "[5/6] LifeGen done: {} provinces, {} factions, {} settlements, {} roads",
        lifegen.provinces.len(),
        lifegen.factions.len(),
        lifegen.settlement_seeds.len(),
        lifegen.road_segments.len(),
    ));

    // ─── 6. Persist artifact ────────────────────────────────────────────────
    let stage = stage_spinner("[6/6] Saving artifact (bincode + PNGs + manifest)");

    // Drop the second Arc we were holding so the only remaining reference lives
    // inside the BiomeMap — this lets `Arc::try_unwrap` succeed without needing
    // a deep clone. If the strong count is still >1 (unexpected), fall back to
    // a serialize round-trip so we never panic.
    drop(river_network_arc);
    let river_network_owned: RiverNetwork = match macro_biome.river_network.take() {
        Some(arc) => match Arc::try_unwrap(arc) {
            Ok(net) => net,
            Err(arc) => {
                // Unexpected outstanding reference — fall back to serde roundtrip.
                // `RiverNetwork` doesn't derive `Clone` (the spatial index is skipped),
                // so this is the supported deep-copy path.
                let bytes = bincode::serialize(&*arc)
                    .map_err(|e| format!("failed to clone RiverNetwork for saving: {e}"))?;
                bincode::deserialize(&bytes)
                    .map_err(|e| format!("failed to clone RiverNetwork for saving: {e}"))?
            }
        },
        None => ron::de::from_str("(segments: [], lakes: [])")
            .map_err(|e| format!("failed to build empty RiverNetwork: {e}"))?,
    };

    let manifest = LayerManifest {
        seed,
        civ_seed,
        created: chrono::Utc::now().to_rfc3339(),
        world_width: WORLD_WIDTH as u32,
        world_height: WORLD_HEIGHT as u32,
        backend: backend_label.to_string(),
        layer_images: layer_image_names,
    };

    store
        .save_layers(
            &tag,
            &macro_biome,
            &river_network_owned,
            &lifegen,
            &images,
            &manifest,
        )
        .map_err(|e| format!("failed to save layer artifact '{tag}': {e}"))?;

    stage.finish_with_message("[6/6] Artifact saved");

    let artifact_dir = store.base_path().join("layers").join(&tag);
    println!("Done. Saved to {}", artifact_dir.display());

    Ok(())
}

// ─── Layer Stitching ────────────────────────────────────────────────────────

/// Stitch per-tile layer bytes into full-world PNGs and return them as
/// `(layer_name, (width, height, rgba_bytes))` pairs ready for
/// `ArtifactStore::save_layers`.
///
/// Images are stitched at `TILES_X * TILE_MAP_SIZE` x `TILES_Y * TILE_MAP_SIZE`
/// (8192x4096 by default) and then downscaled 2x to `4096x2048` so each PNG
/// stays under ~30 MB. This matches the output of
/// `crates/rb_noise/examples/save_debug_layers.rs` and the in-app
/// `save_stitched_debug_layers` helper.
///
/// Layers are stitched in parallel via rayon — each worker owns its own
/// 128 MB scratch buffer. Peak memory scales with thread count rather than
/// being sequential (~128 MB × threads instead of sequential 128 MB × 20).
fn stitch_layer_images(
    tiles: &[((i32, i32), BiomeMap)],
    norm_hints: &NormalizationHints,
    progress: &ProgressBar,
) -> (HashMap<String, (u32, u32, Vec<u8>)>, Vec<String>) {
    // Index tiles by (cx, cy) for O(1) lookup during stitching.
    let mut tile_grid: Vec<Vec<Option<&BiomeMap>>> = vec![vec![None; TILES_X]; TILES_Y];
    for (coord, bm) in tiles {
        let cx = coord.0 as usize;
        let cy = coord.1 as usize;
        if cx < TILES_X && cy < TILES_Y {
            tile_grid[cy][cx] = Some(bm);
        }
    }

    // Stitch each layer in parallel. Each worker owns its own full-resolution
    // scratch buffer, so peak memory is ~128 MB * rayon_thread_count instead
    // of the sequential 128 MB allocated 20 times.
    let results: Vec<(String, (u32, u32, Vec<u8>))> = NoiseLayer::all()
        .par_iter()
        .map(|layer| {
            let file_name = layer_file_name(*layer);
            let (half_w, half_h, small) = stitch_single_layer(*layer, &tile_grid, norm_hints);
            progress.inc(1);
            (file_name, (half_w, half_h, small))
        })
        .collect();

    // Rebuild the output map + ordered name list in `NoiseLayer::all()` order
    // so the manifest's `layer_images` field matches the canonical noise layer
    // ordering — makes diffs between runs easier to scan.
    let mut images: HashMap<String, (u32, u32, Vec<u8>)> =
        HashMap::with_capacity(results.len());
    let mut names: Vec<String> = Vec::with_capacity(results.len());
    for (name, data) in results {
        names.push(name.clone());
        images.insert(name, data);
    }
    (images, names)
}

/// Stitch a single noise layer into an 8192x4096 full-resolution buffer,
/// then downscale 2x with a box filter to 4096x2048.
///
/// Returns `(half_w, half_h, rgba_bytes)` for the downscaled image.
fn stitch_single_layer(
    layer: NoiseLayer,
    tile_grid: &[Vec<Option<&BiomeMap>>],
    norm_hints: &NormalizationHints,
) -> (u32, u32, Vec<u8>) {
    let full_w = (TILES_X * TILE_MAP_SIZE) as u32;
    let full_h = (TILES_Y * TILE_MAP_SIZE) as u32;
    let tile_px = TILE_MAP_SIZE as u32;

    // Full-resolution buffer (tight RGBA row-major).
    let stride_px = full_w as usize;
    let mut full: Vec<u8> = vec![0u8; (full_w as usize) * (full_h as usize) * 4];

    for cy in 0..TILES_Y {
        for cx in 0..TILES_X {
            let Some(bm) = tile_grid[cy][cx] else { continue };
            let rgba = bm.to_layer_image_with_hints(layer, Some(norm_hints));
            if rgba.len() != (tile_px as usize) * (tile_px as usize) * 4 {
                // Skip malformed tiles rather than panicking — this matches
                // the behaviour of `save_stitched_debug_layers` in main.rs.
                continue;
            }
            let ox = cx * TILE_MAP_SIZE;
            let oy = cy * TILE_MAP_SIZE;
            for py in 0..TILE_MAP_SIZE {
                let src_row = py * TILE_MAP_SIZE * 4;
                let dst_row = ((oy + py) * stride_px + ox) * 4;
                let row_bytes = TILE_MAP_SIZE * 4;
                full[dst_row..dst_row + row_bytes]
                    .copy_from_slice(&rgba[src_row..src_row + row_bytes]);
            }
        }
    }

    // Downscale 2x (box filter) to keep PNGs under ~30 MB.
    let half_w = full_w / 2;
    let half_h = full_h / 2;
    let mut small: Vec<u8> = vec![0u8; (half_w as usize) * (half_h as usize) * 4];
    for sy in 0..half_h as usize {
        for sx in 0..half_w as usize {
            let x0 = sx * 2;
            let y0 = sy * 2;
            let idx = |x: usize, y: usize| (y * stride_px + x) * 4;
            let p0 = idx(x0, y0);
            let p1 = idx(x0 + 1, y0);
            let p2 = idx(x0, y0 + 1);
            let p3 = idx(x0 + 1, y0 + 1);
            let avg = |a: u8, b: u8, c: u8, d: u8| -> u8 {
                ((a as u16 + b as u16 + c as u16 + d as u16) / 4) as u8
            };
            let dst = (sy * half_w as usize + sx) * 4;
            small[dst] = avg(full[p0], full[p1], full[p2], full[p3]);
            small[dst + 1] = avg(full[p0 + 1], full[p1 + 1], full[p2 + 1], full[p3 + 1]);
            small[dst + 2] = avg(full[p0 + 2], full[p1 + 2], full[p2 + 2], full[p3 + 2]);
            small[dst + 3] = avg(full[p0 + 3], full[p1 + 3], full[p2 + 3], full[p3 + 3]);
        }
    }

    (half_w, half_h, small)
}

/// Stable filesystem-friendly filename for a noise layer.
fn layer_file_name(layer: NoiseLayer) -> String {
    let base = match layer {
        NoiseLayer::Biome => "biome",
        NoiseLayer::Continentalness => "continentalness",
        NoiseLayer::Tectonic => "tectonic",
        NoiseLayer::Humidity => "humidity",
        NoiseLayer::RockHardness => "rock_hardness",
        NoiseLayer::LightLevel => "light_level",
        NoiseLayer::PeaksValleys => "peaks_valleys",
        NoiseLayer::Volcanism => "volcanism",
        NoiseLayer::Heightmap => "heightmap",
        NoiseLayer::Temperature => "temperature",
        NoiseLayer::Erosion => "erosion",
        NoiseLayer::RiverFlow => "river_flow",
        NoiseLayer::Aridity => "aridity",
        NoiseLayer::PrecipitationType => "precipitation_type",
        NoiseLayer::WaterTable => "water_table",
        NoiseLayer::Wind => "wind",
        NoiseLayer::Resources => "resources",
        NoiseLayer::Snowpack => "snowpack",
        NoiseLayer::VegetationDensity => "vegetation_density",
        NoiseLayer::SoilType => "soil_type",
    };
    format!("{base}.png")
}

// ─── Progress Helpers ───────────────────────────────────────────────────────

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

fn tile_progress_bar(total: u64, prefix: &str) -> ProgressBar {
    let pb = ProgressBar::new(total);
    pb.set_style(
        ProgressStyle::with_template(
            "{prefix:.cyan} [{bar:40.cyan/blue}] {pos}/{len} ({eta})",
        )
        .unwrap_or_else(|_| ProgressStyle::default_bar())
        .progress_chars("=>-"),
    );
    pb.set_prefix(prefix.to_string());
    pb
}
