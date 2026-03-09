//! Saves all noise layers as PNG files to debug_layers/.
//!
//! Run with: cargo run -p rb_noise --example save_debug_layers
//!
//! Generates both macro (full world 1024×512) and meso (64×64 chunk at 512×512)
//! debug layers. A chunk with a visible macro river is automatically selected
//! for the meso view.

use rb_noise::{BiomeMap, LayerProgress, NoiseBackend};
use std::path::Path;
use std::sync::Arc;

fn main() {
    let seed = std::env::args()
        .nth(1)
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(42);

    let macro_width = 1024;
    let macro_height = 512;
    let chunk_size = 64.0;

    // ── Macro ───────────────────────────────────────────────────────────────
    println!("Generating macro world with seed {seed}...");
    let macro_map = BiomeMap::generate(seed, macro_width, macro_height);

    let macro_dir = Path::new("debug_layers/macro");
    macro_map.save_debug_layers(macro_dir);
    println!("Macro layers saved to debug_layers/macro/");

    // ── Find a chunk with a river for the meso view ─────────────────────────
    let chunks_x = macro_width as f64 / chunk_size;
    let chunks_y = macro_height as f64 / chunk_size;

    let mut best_chunk = (4.0 * chunk_size, 4.0 * chunk_size); // fallback: chunk (4,4)
    let mut best_flow = 0.0_f64;

    for cy in 0..chunks_y as usize {
        for cx in 0..chunks_x as usize {
            let wx = cx as f64 * chunk_size;
            let wy = cy as f64 * chunk_size;

            // Sum river flow in this chunk
            let mut total_flow = 0.0;
            for py in wy as usize..(wy + chunk_size) as usize {
                for px in wx as usize..(wx + chunk_size) as usize {
                    if px < macro_width && py < macro_height {
                        total_flow += macro_map.rivers[py * macro_width + px];
                    }
                }
            }

            if total_flow > best_flow {
                best_flow = total_flow;
                best_chunk = (wx, wy);
            }
        }
    }

    let (world_x, world_y) = best_chunk;
    println!(
        "Selected meso chunk at world ({world_x}, {world_y}) with total river flow {best_flow:.2}"
    );

    // ── Meso ────────────────────────────────────────────────────────────────
    println!("Generating meso view...");
    let progress = Arc::new(LayerProgress::new(512 * 512));
    let meso_map = BiomeMap::generate_meso_full_with_backend(
        seed,
        world_x,
        world_y,
        chunk_size,
        512,
        macro_height as f64,
        1, // detail_level = meso
        &progress,
        NoiseBackend::Gpu,
        Some(&macro_map),
    );

    let meso_dir = Path::new("debug_layers/meso");
    meso_map.save_debug_layers(meso_dir);
    println!("Meso layers saved to debug_layers/meso/");

    println!("Done!");
}
