//! Saves all noise layers as PNG files to debug_layers/.
//!
//! Run with: cargo run --release -p rb_noise --example save_debug_layers
//!
//! Tiles the full world (1024×512) with 8×8 world-unit meso tiles at
//! detail_level=2, matching what the in-app world map renders.
//! Each tile is generated at 64px and stitched into 8192×4096 images.

use image::{ImageBuffer, Rgba, RgbaImage};
use rb_noise::{BiomeMap, LayerProgress, NoiseBackend, NoiseLayer};
use std::path::Path;
use std::sync::Arc;

fn main() {
    let seed = std::env::args()
        .nth(1)
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(42);

    let world_width = 1024.0_f64;
    let world_height = 512.0_f64;
    let meso_world_size = 8.0_f64; // matches MESO_WORLD_SIZE in main.rs
    let tile_px = 64_usize; // pixels per meso tile in output

    let tiles_x = (world_width / meso_world_size) as usize; // 128
    let tiles_y = (world_height / meso_world_size) as usize; // 64
    let total_tiles = tiles_x * tiles_y; // 8192
    let full_w = tiles_x * tile_px; // 8192
    let full_h = tiles_y * tile_px; // 4096

    // Generate macro map first (needed as river seed for meso tiles)
    println!("Generating macro world with seed {seed}...");
    let macro_map = BiomeMap::generate(seed, world_width as usize, world_height as usize);
    println!("Macro map done.");

    // Generate all meso tiles
    println!(
        "Generating {total_tiles} meso tiles ({tiles_x}x{tiles_y}, {meso_world_size}x{meso_world_size} world units each at {tile_px}px) -> {full_w}x{full_h}..."
    );
    println!("  (Use --release for ~10x faster generation)");

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
                seed,
                wx,
                wy,
                meso_world_size,
                tile_px,
                world_height,
                2, // meso detail level
                Some(&progress),
                NoiseBackend::Gpu,
                Some(&macro_map),
            );
            meso_tiles.push(tile);
        }
    }
    println!("\r  All {total_tiles} meso tiles generated.                              ");

    // Stitch and save each layer
    let out_dir = Path::new("debug_layers");
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
                let rgba_data = tile.to_layer_image(*layer);
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

    println!("Done!");
}
