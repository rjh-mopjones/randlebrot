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
    let (mut images, mut layer_image_names) = stitch_layer_images(&tiles, &norm_hints, &layer_bar);
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

    // ─── 5b. Render LifeGen layer PNGs and add to images ─────────────────────
    let lifegen_layers = [
        "habitability", "navigation_cost", "resource_desirability",
        "factions", "settlements", "roads", "composite",
    ];
    for layer_name in &lifegen_layers {
        let rgba = lifegen.to_layer_image(layer_name);
        if !rgba.is_empty() {
            let filename = format!("lifegen_{layer_name}.png");
            images.insert(filename.clone(), (lifegen.width as u32, lifegen.height as u32, rgba));
            layer_image_names.push(filename);
        }
    }

    // ─── 6. Persist artifact ────────────────────────────────────────────────
    let stage = stage_spinner("[6/6] Saving artifact (bincode + PNGs + manifest)");

    // Drop the second Arc we were holding so the only remaining reference lives
    // inside the BiomeMap — this lets `Arc::try_unwrap` succeed without needing
    // a deep clone. If the strong count is still >1 (unexpected), fall back to
    // a serialize round-trip so we never panic.
    drop(river_network_arc);
    let river_network_owned: RiverNetwork =
        extract_river_network(macro_biome.river_network.take())?;

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
pub(crate) fn stitch_layer_images(
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
pub(crate) fn stitch_single_layer(
    layer: NoiseLayer,
    tile_grid: &[Vec<Option<&BiomeMap>>],
    norm_hints: &NormalizationHints,
) -> (u32, u32, Vec<u8>) {
    let tiles_x = tile_grid.first().map_or(0, |r| r.len());
    let tiles_y = tile_grid.len();
    // Infer tile pixel size from the first non-None BiomeMap in the grid.
    let tile_px = tile_grid
        .iter()
        .flatten()
        .find_map(|opt| opt.map(|bm| bm.width))
        .unwrap_or(TILE_MAP_SIZE);
    stitch_rgba_grid(layer, tile_grid, norm_hints, tiles_x, tiles_y, tile_px)
}

/// Core stitching + 2x box-filter downscale, parameterised by grid and tile
/// dimensions so unit tests can exercise the same logic on tiny grids without
/// allocating 128 MB buffers.
///
/// Returns `(half_w, half_h, rgba_bytes)`.
pub(crate) fn stitch_rgba_grid(
    layer: NoiseLayer,
    tile_grid: &[Vec<Option<&BiomeMap>>],
    norm_hints: &NormalizationHints,
    tiles_x: usize,
    tiles_y: usize,
    tile_px: usize,
) -> (u32, u32, Vec<u8>) {
    let full_w = (tiles_x * tile_px) as u32;
    let full_h = (tiles_y * tile_px) as u32;

    // Full-resolution buffer (tight RGBA row-major).
    let stride_px = full_w as usize;
    let mut full: Vec<u8> = vec![0u8; (full_w as usize) * (full_h as usize) * 4];

    for cy in 0..tiles_y {
        for cx in 0..tiles_x {
            let Some(bm) = tile_grid[cy][cx] else { continue };
            let rgba = bm.to_layer_image_with_hints(layer, Some(norm_hints));
            if rgba.len() != tile_px * tile_px * 4 {
                // Skip malformed tiles rather than panicking — this matches
                // the behaviour of `save_stitched_debug_layers` in main.rs.
                continue;
            }
            let ox = cx * tile_px;
            let oy = cy * tile_px;
            for py in 0..tile_px {
                let src_row = py * tile_px * 4;
                let dst_row = ((oy + py) * stride_px + ox) * 4;
                let row_bytes = tile_px * 4;
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
pub(crate) fn layer_file_name(layer: NoiseLayer) -> String {
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

// ─── Arc Extraction ─────────────────────────────────────────────────────────

/// Extract an owned `RiverNetwork` from an `Option<Arc<RiverNetwork>>`.
///
/// Happy path: the Arc has a strong count of 1, so `Arc::try_unwrap` succeeds
/// and we avoid any copies.
///
/// Fallback: if there are outstanding references (unexpected), we fall back to
/// a bincode serialize → deserialize round-trip. `RiverNetwork` doesn't derive
/// `Clone` (the spatial index is `#[serde(skip)]`), so bincode is the supported
/// deep-copy path.
///
/// If `None`, returns an empty RiverNetwork via RON deserialization.
pub(crate) fn extract_river_network(
    arc_opt: Option<Arc<RiverNetwork>>,
) -> Result<RiverNetwork, String> {
    match arc_opt {
        Some(arc) => match Arc::try_unwrap(arc) {
            Ok(net) => Ok(net),
            Err(arc) => {
                let bytes = bincode::serialize(&*arc)
                    .map_err(|e| format!("failed to clone RiverNetwork for saving: {e}"))?;
                bincode::deserialize(&bytes)
                    .map_err(|e| format!("failed to clone RiverNetwork for saving: {e}"))
            }
        },
        None => ron::de::from_str("(segments: [], lakes: [])")
            .map_err(|e| format!("failed to build empty RiverNetwork: {e}")),
    }
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

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use rb_core::biome::TileType;

    /// Build a tiny BiomeMap of `size x size` pixels with every f64 layer
    /// filled with `fill_value` and biomes set to `TileType::Sea`.
    ///
    /// This is NOT a valid simulation output — it's the minimum structure
    /// needed to exercise `to_layer_image_with_hints` for the Continentalness
    /// layer (which maps the `continentalness` vector via `grayscale_to_rgba`).
    fn make_tiny_biome(size: usize, fill_value: f64) -> BiomeMap {
        let n = size * size;
        BiomeMap {
            width: size,
            height: size,
            continentalness: vec![fill_value; n],
            tectonic: vec![0.0; n],
            tectonic_plate_ids: vec![0.0; n],
            humidity: vec![0.0; n],
            rock_hardness: vec![0.0; n],
            light_level: vec![0.0; n],
            peaks_valleys: vec![0.0; n],
            volcanism: vec![0.0; n],
            heightmap: vec![0.0; n],
            temperature: vec![0.0; n],
            erosion: vec![0.0; n],
            rivers: vec![0.0; n],
            aridity: vec![0.0; n],
            precipitation_type: vec![0.0; n],
            water_table: vec![0.0; n],
            wind_speed: vec![0.0; n],
            resource_richness: vec![0.0; n],
            snowpack: vec![0.0; n],
            biomes: vec![TileType::Sea; n],
            vegetation_density: vec![0.0; n],
            soil_type: vec![0.0; n],
            resource_map: None,
            river_network: None,
            drainage_area: vec![0; n],
            sediment: vec![0.0; n],
            world_width: 1024.0,
            world_height: 512.0,
        }
    }

    // ── Stitching tests ─────────────────────────────────────────────────────

    /// Place two tiles with distinct fill values in a 2x2 grid (4px tile),
    /// stitch them, and verify:
    ///   - the full-res pixels land at the correct grid position
    ///   - the 2x box-filter downscale averages correctly
    ///   - empty slots remain zero (black transparent)
    #[test]
    fn stitch_places_tiles_and_downscales() {
        let tile_px = 4;

        // Tile at (0,0): continentalness = 1.0 → grayscale 255
        let top_left = make_tiny_biome(tile_px, 1.0);
        // Tile at (1,1): continentalness = -1.0 → grayscale 0
        let bot_right = make_tiny_biome(tile_px, -1.0);

        // Build a 2x2 tile grid: top_left at (0,0), bot_right at (1,1),
        // (0,1) and (1,0) empty.
        let tile_grid: Vec<Vec<Option<&BiomeMap>>> = vec![
            vec![Some(&top_left), None],
            vec![None, Some(&bot_right)],
        ];
        let hints = NormalizationHints {
            heightmap_min: 0.0,
            heightmap_max: 1.0,
        };

        let (half_w, half_h, small) = stitch_rgba_grid(
            NoiseLayer::Continentalness,
            &tile_grid,
            &hints,
            2,  // tiles_x
            2,  // tiles_y
            tile_px,
        );

        // Full image: 8x8 → downscaled 4x4
        assert_eq!(half_w, 4);
        assert_eq!(half_h, 4);
        assert_eq!(small.len(), 4 * 4 * 4); // 4x4 RGBA

        // Helper: read pixel (px, py) from the downscaled image.
        let pixel = |px: usize, py: usize| -> [u8; 4] {
            let base = (py * half_w as usize + px) * 4;
            [small[base], small[base + 1], small[base + 2], small[base + 3]]
        };

        // Top-left quadrant (0..2, 0..2 in downscaled) came from the tile
        // at grid (0,0) which had continentalness = 1.0.
        // grayscale_to_rgba(1.0, -1.0, 1.0) → normalized 1.0 → gray 255
        // Box filter of four identical pixels = same value.
        let tl = pixel(0, 0);
        assert_eq!(tl, [255, 255, 255, 255], "top-left should be white (cont=1.0)");
        let tl2 = pixel(1, 1);
        assert_eq!(tl2, [255, 255, 255, 255], "still in top-left tile quadrant");

        // Bottom-right quadrant (2..4, 2..4 in downscaled) came from the tile
        // at grid (1,1) which had continentalness = -1.0.
        // grayscale_to_rgba(-1.0, -1.0, 1.0) → normalized 0.0 → gray 0
        let br = pixel(2, 2);
        assert_eq!(br, [0, 0, 0, 255], "bottom-right should be black (cont=-1.0)");
        let br2 = pixel(3, 3);
        assert_eq!(br2, [0, 0, 0, 255], "still in bottom-right tile quadrant");

        // Top-right quadrant (2..4, 0..2) had no tile → all zeros (black transparent).
        let empty = pixel(2, 0);
        assert_eq!(empty, [0, 0, 0, 0], "empty tile slot should be transparent black");
    }

    /// A completely empty grid (all None) produces a fully black-transparent
    /// downscaled image.
    #[test]
    fn stitch_empty_grid_is_transparent() {
        let tile_px = 4;
        let tile_grid: Vec<Vec<Option<&BiomeMap>>> = vec![vec![None; 2]; 2];
        let hints = NormalizationHints {
            heightmap_min: 0.0,
            heightmap_max: 1.0,
        };

        let (half_w, half_h, small) = stitch_rgba_grid(
            NoiseLayer::Continentalness,
            &tile_grid,
            &hints,
            2,
            2,
            tile_px,
        );

        assert_eq!(half_w, 4);
        assert_eq!(half_h, 4);
        assert!(small.iter().all(|&b| b == 0), "all bytes should be zero for empty grid");
    }

    /// A single tile filling the entire 1x1 grid still round-trips through
    /// stitch + downscale correctly.
    #[test]
    fn stitch_single_tile_grid() {
        let tile_px = 4;
        // Fill with a mid-range value: continentalness 0.0 → grayscale 127
        let tile = make_tiny_biome(tile_px, 0.0);
        let tile_grid: Vec<Vec<Option<&BiomeMap>>> = vec![vec![Some(&tile)]];
        let hints = NormalizationHints {
            heightmap_min: 0.0,
            heightmap_max: 1.0,
        };

        let (half_w, half_h, small) = stitch_rgba_grid(
            NoiseLayer::Continentalness,
            &tile_grid,
            &hints,
            1,
            1,
            tile_px,
        );

        // 4x4 → downscaled 2x2
        assert_eq!(half_w, 2);
        assert_eq!(half_h, 2);

        // Every downscaled pixel should be the same gray value.
        // grayscale_to_rgba(0.0, -1.0, 1.0) → normalized 0.5 → gray 127
        for py in 0..2 {
            for px in 0..2 {
                let base = (py * 2 + px) * 4;
                assert_eq!(small[base], 127, "R at ({px},{py})");
                assert_eq!(small[base + 1], 127, "G at ({px},{py})");
                assert_eq!(small[base + 2], 127, "B at ({px},{py})");
                assert_eq!(small[base + 3], 255, "A at ({px},{py})");
            }
        }
    }

    /// Verify the box-filter averages two different adjacent tiles correctly.
    /// Tile (0,0) = gray 255, tile (1,0) = gray 0. Along the boundary, the
    /// downscale should produce a clean average where tiles meet (within a
    /// single tile the average is just the fill value).
    #[test]
    fn stitch_downscale_averages_correctly() {
        let tile_px = 2; // minimum even size for a 2x downscale

        let bright = make_tiny_biome(tile_px, 1.0);  // gray 255
        let dark = make_tiny_biome(tile_px, -1.0);    // gray 0

        // 2x1 grid: [bright, dark]
        let tile_grid: Vec<Vec<Option<&BiomeMap>>> = vec![
            vec![Some(&bright), Some(&dark)],
        ];
        let hints = NormalizationHints {
            heightmap_min: 0.0,
            heightmap_max: 1.0,
        };

        let (half_w, half_h, small) = stitch_rgba_grid(
            NoiseLayer::Continentalness,
            &tile_grid,
            &hints,
            2,
            1,
            tile_px,
        );

        // Full: 4x2, downscaled: 2x1
        assert_eq!(half_w, 2);
        assert_eq!(half_h, 1);

        // Pixel (0,0): average of four full-res pixels in the bright tile.
        // All four are 255 → avg = 255.
        assert_eq!(small[0], 255, "bright tile downscale R");
        assert_eq!(small[3], 255, "bright tile downscale A");

        // Pixel (1,0): average of four full-res pixels in the dark tile.
        // All four are 0 → avg = 0.
        assert_eq!(small[4], 0, "dark tile downscale R");
        assert_eq!(small[7], 255, "dark tile downscale A");
    }

    // ── Arc extraction tests ────────────────────────────────────────────────

    /// Happy path: the Arc has a strong count of 1, so `try_unwrap` succeeds
    /// and returns the RiverNetwork without any serialization overhead.
    #[test]
    fn arc_extraction_unique_ref_succeeds() {
        let net: RiverNetwork = ron::de::from_str("(segments: [], lakes: [])").unwrap();
        let arc = Arc::new(net);
        // Strong count is exactly 1 — try_unwrap should succeed.
        assert_eq!(Arc::strong_count(&arc), 1);

        let result = extract_river_network(Some(arc));
        assert!(result.is_ok(), "unique Arc should unwrap cleanly");
        let owned = result.unwrap();
        assert_eq!(owned.segments.len(), 0);
        assert_eq!(owned.lakes.len(), 0);
    }

    /// Fallback path: an outstanding Arc reference prevents `try_unwrap`, so
    /// the function falls back to a bincode serialize → deserialize roundtrip.
    /// The result must still be a valid RiverNetwork with the same data.
    #[test]
    fn arc_extraction_fallback_roundtrip() {
        let net: RiverNetwork = ron::de::from_str("(segments: [], lakes: [])").unwrap();
        let arc = Arc::new(net);
        // Create a second reference so try_unwrap will fail.
        let _extra_ref = Arc::clone(&arc);
        assert_eq!(Arc::strong_count(&arc), 2);

        let result = extract_river_network(Some(arc));
        assert!(result.is_ok(), "fallback bincode roundtrip should succeed");
        let owned = result.unwrap();
        assert_eq!(owned.segments.len(), 0);
        assert_eq!(owned.lakes.len(), 0);
    }

    /// Fallback path with non-trivial data: a RiverNetwork with real segments
    /// survives the bincode roundtrip.
    #[test]
    fn arc_extraction_fallback_preserves_segments() {
        let ron_str = r#"(
            segments: [(
                id: 42,
                path: [(10.0, 20.0), (30.0, 40.0)],
                drainage_area: 500,
                downstream: None,
                upstream: [],
                character: Permanent,
                meander_offsets: [0.1, -0.2],
                strahler_order: 3,
            )],
            lakes: [],
        )"#;
        let net: RiverNetwork = ron::de::from_str(ron_str).unwrap();
        // Force the fallback by holding two Arcs.
        let arc = Arc::new(net);
        let _extra = Arc::clone(&arc);

        let result = extract_river_network(Some(arc));
        assert!(result.is_ok());
        let owned = result.unwrap();
        assert_eq!(owned.segments.len(), 1);
        assert_eq!(owned.segments[0].id, 42);
        assert_eq!(owned.segments[0].drainage_area, 500);
        assert_eq!(owned.segments[0].path.len(), 2);
        assert_eq!(owned.segments[0].strahler_order, 3);
    }

    /// When `None` is passed (no river network was generated), the function
    /// returns a valid empty RiverNetwork via RON deserialization.
    #[test]
    fn arc_extraction_none_returns_empty() {
        let result = extract_river_network(None);
        assert!(result.is_ok(), "None should produce empty RiverNetwork");
        let owned = result.unwrap();
        assert_eq!(owned.segments.len(), 0);
        assert_eq!(owned.lakes.len(), 0);
    }

    // ── layer_file_name tests ───────────────────────────────────────────────

    /// Every noise layer maps to a unique `.png` filename.
    #[test]
    fn layer_file_names_are_unique_and_end_with_png() {
        let mut seen = std::collections::HashSet::new();
        for layer in NoiseLayer::all() {
            let name = layer_file_name(*layer);
            assert!(name.ends_with(".png"), "{name} should end with .png");
            assert!(
                seen.insert(name.clone()),
                "duplicate filename: {name}"
            );
        }
        // Should cover all 20 layers.
        assert_eq!(seen.len(), NoiseLayer::all().len());
    }
}
