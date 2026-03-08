//! Saves all noise layers as PNG files to debug_layers/.
//!
//! Run with: cargo run -p rb_noise --example save_debug_layers

use rb_noise::BiomeMap;
use std::path::Path;

fn main() {
    let seed = std::env::args()
        .nth(1)
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(42);

    println!("Generating world with seed {seed}...");
    let map = BiomeMap::generate(seed, 1024, 512);

    let out = Path::new("debug_layers");
    map.save_debug_layers(out);
    println!("Done! Images saved to debug_layers/");
}
