//! Saves all terrain + lifegen layers as PNG files to debug_layers/.
//!
//! Run with: cargo run --release --example save_all_debug_layers
//!
//! Generates terrain meso tiles (8192x4096), then runs lifegen and saves
//! both terrain and civilisation debug layer PNGs.

use image::{ImageBuffer, Rgba, RgbaImage};
use rb_noise::{BiomeMap, LayerProgress, MesoTerrainView, NormalizationHints, NoiseBackend, NoiseLayer};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

fn main() {
    let seed = std::env::args()
        .nth(1)
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(42);

    let civ_seed = std::env::args()
        .nth(2)
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(42);

    let world_width = 1024.0_f64;
    let world_height = 512.0_f64;
    let meso_world_size = 8.0_f64;
    let tile_px = 32_usize;

    let tiles_x = (world_width / meso_world_size) as usize; // 128
    let tiles_y = (world_height / meso_world_size) as usize; // 64
    let total_tiles = tiles_x * tiles_y;
    let full_w = tiles_x * tile_px;
    let full_h = tiles_y * tile_px;

    // ── Terrain ──────────────────────────────────────────────────────────────

    println!("Generating macro world with seed {seed}...");
    let macro_map = BiomeMap::generate(seed, world_width as usize, world_height as usize);
    println!("Macro map done.");

    let out_dir = Path::new("debug_layers");
    std::fs::create_dir_all(out_dir).ok();

    println!(
        "Generating {total_tiles} meso tiles ({tiles_x}x{tiles_y}, \
         {meso_world_size}x{meso_world_size} world units each at {tile_px}px) \
         -> {full_w}x{full_h}..."
    );

    let mut meso_tiles: Vec<BiomeMap> = Vec::with_capacity(total_tiles);
    for ty in 0..tiles_y {
        let row_start = ty * tiles_x + 1;
        let row_end = row_start + tiles_x - 1;
        print!("\r  Row {}/{tiles_y} (tiles {row_start}-{row_end}/{total_tiles})...    ", ty + 1);

        for tx in 0..tiles_x {
            let wx = tx as f64 * meso_world_size;
            let wy = ty as f64 * meso_world_size;

            let progress = Arc::new(LayerProgress::new(tile_px * tile_px));
            let tile = BiomeMap::generate_meso_full_with_backend(
                seed, wx, wy, meso_world_size, tile_px, world_height, 2,
                Some(&progress), NoiseBackend::Gpu,
                Some(&macro_map), macro_map.river_network.as_ref(),
            );
            meso_tiles.push(tile);
        }
    }
    println!("\r  All {total_tiles} meso tiles generated.                              ");

    // Compute global heightmap range
    let norm_hints = {
        let mut hmin = f64::MAX;
        let mut hmax = f64::MIN;
        for tile in &meso_tiles {
            for &v in &tile.heightmap {
                if v < hmin { hmin = v; }
                if v > hmax { hmax = v; }
            }
        }
        NormalizationHints {
            heightmap_min: if hmin < hmax { hmin } else { 0.0 },
            heightmap_max: if hmin < hmax { hmax } else { 1.0 },
        }
    };
    println!("  Global heightmap range: [{:.4}, {:.4}]", norm_hints.heightmap_min, norm_hints.heightmap_max);

    // Stitch and save terrain layers
    let base_dir = out_dir.join("base");
    let derived_dir = out_dir.join("derived");
    for dir in [out_dir, &base_dir, &derived_dir] {
        std::fs::create_dir_all(dir).expect("failed to create output dir");
    }

    for layer in NoiseLayer::all() {
        let name = layer.name();
        print!("  Stitching {name}...");

        let mut full_img: RgbaImage = ImageBuffer::new(full_w as u32, full_h as u32);
        for ty in 0..tiles_y {
            for tx in 0..tiles_x {
                let tile = &meso_tiles[ty * tiles_x + tx];
                let rgba_data = tile.to_layer_image_with_hints(*layer, Some(&norm_hints));
                let tile_img: ImageBuffer<Rgba<u8>, _> =
                    ImageBuffer::from_raw(tile_px as u32, tile_px as u32, rgba_data)
                        .expect("tile image size mismatch");

                let ox = (tx * tile_px) as u32;
                let oy = (ty * tile_px) as u32;
                for py in 0..tile_px as u32 {
                    for px in 0..tile_px as u32 {
                        full_img.put_pixel(ox + px, oy + py, *tile_img.get_pixel(px, py));
                    }
                }
            }
        }

        let path = match layer {
            NoiseLayer::Biome => out_dir.join("biome.png"),
            NoiseLayer::Continentalness
            | NoiseLayer::Tectonic
            | NoiseLayer::Humidity
            | NoiseLayer::RockHardness
            | NoiseLayer::LightLevel => base_dir.join(format!("{name}.png")),
            _ => derived_dir.join(format!("{name}.png")),
        };

        full_img.save(&path).expect("failed to save image");
        println!(" saved {}", path.display());
    }

    // ── LifeGen ──────────────────────────────────────────────────────────────

    println!("\nBuilding MesoTerrainView for lifegen...");

    // Build tile map: each meso tile (32x32 px) indexed by its grid position
    let mut tile_map: HashMap<(i32, i32), Arc<BiomeMap>> = HashMap::with_capacity(total_tiles);
    for (i, tile) in meso_tiles.into_iter().enumerate() {
        let tx = (i % tiles_x) as i32;
        let ty = (i / tiles_x) as i32;
        tile_map.insert((tx, ty), Arc::new(tile));
    }

    let terrain_view = MesoTerrainView::from_tile_map(
        &tile_map, tiles_x, tiles_y, tile_px,
    );

    println!("Running lifegen with civ_seed {civ_seed}...");
    let lifegen = rb_world::lifegen::generate(&terrain_view, civ_seed);
    println!(
        "  LifeGen complete: {} provinces, {} factions, {} settlements, {} road segments",
        lifegen.provinces.len(),
        lifegen.factions.len(),
        lifegen.settlement_seeds.len(),
        lifegen.road_segments.len(),
    );

    lifegen.save_debug_layers(out_dir);
    println!("Done! All debug layers saved to debug_layers/");
}
