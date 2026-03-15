//! Generates terrain meso tiles, runs LifeGen, and saves all debug PNGs.
//!
//! Run with: cargo run --release -p rb_world --example save_lifegen_layers
//!
//! Optional args:
//!   cargo run --release -p rb_world --example save_lifegen_layers -- [terrain_seed] [civ_seed]
//!
//! Defaults: terrain_seed=42, civ_seed=137

use rb_core::TerrainQuery;
use rb_noise::{BiomeMap, LayerProgress, MesoTerrainView, NoiseBackend};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let terrain_seed = args.get(1).and_then(|s| s.parse::<u32>().ok()).unwrap_or(42);
    let civ_seed = args.get(2).and_then(|s| s.parse::<u32>().ok()).unwrap_or(137);

    let world_width = 1024.0_f64;
    let world_height = 512.0_f64;
    let chunk_size = 64.0_f64;
    let tile_px = 512_usize;

    let chunks_x = (world_width / chunk_size) as usize; // 16
    let chunks_y = (world_height / chunk_size) as usize; // 8
    let total_tiles = chunks_x * chunks_y; // 128

    // --- Phase 1: Generate macro map ---
    println!("=== Terrain Generation ===");
    println!("Generating macro world with seed {terrain_seed}...");
    let macro_map = BiomeMap::generate(terrain_seed, world_width as usize, world_height as usize);
    println!("Macro map done.");

    // --- Phase 2: Generate meso tiles (matching main.rs macro pregen) ---
    println!(
        "Generating {total_tiles} macro tiles ({chunks_x}x{chunks_y}, {chunk_size}x{chunk_size} world units each at {tile_px}px)..."
    );

    let mut tile_map: HashMap<(i32, i32), Arc<BiomeMap>> = HashMap::new();

    for cy in 0..chunks_y {
        print!("\r  Row {}/{chunks_y}...", cy + 1);
        for cx in 0..chunks_x {
            let wx = cx as f64 * chunk_size;
            let wy = cy as f64 * chunk_size;

            let progress = Arc::new(LayerProgress::new(tile_px * tile_px));
            let tile = BiomeMap::generate_meso_full_with_backend(
                terrain_seed,
                wx,
                wy,
                chunk_size,
                tile_px,
                world_height,
                1, // macro detail level (matches main.rs)
                Some(&progress),
                NoiseBackend::Gpu,
                Some(&macro_map),
                macro_map.river_network.as_ref(),
            );
            tile_map.insert((cx as i32, cy as i32), Arc::new(tile));
        }
    }
    println!("\r  All {total_tiles} macro tiles generated.          ");

    // --- Phase 3: Build MesoTerrainView ---
    let terrain_view = MesoTerrainView::from_tile_map(&tile_map, chunks_x, chunks_y, tile_px);
    println!(
        "MesoTerrainView constructed: {}x{}",
        terrain_view.width(),
        terrain_view.height()
    );

    // --- Phase 4: Run LifeGen ---
    println!("\n=== LifeGen (civ_seed={civ_seed}) ===");
    let lifegen = rb_world::lifegen::generate(&terrain_view, civ_seed);

    // --- Phase 5: Save debug layers ---
    let out_dir = Path::new("debug_layers");
    std::fs::create_dir_all(out_dir).ok();
    lifegen.save_debug_layers(out_dir);

    println!("\nAll lifegen debug layers saved to debug_layers/lifegen/");
}
