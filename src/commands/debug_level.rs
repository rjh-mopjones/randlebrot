//! Debug diagnostic: load a level artifact and print statistics about all layers.

use rb_artifacts::ArtifactStore;

pub fn run(level_tag: &str) -> Result<(), String> {
    let store = ArtifactStore::new()
        .map_err(|e| format!("artifact store: {e}"))?;

    let (biome_map, manifest) = store.load_level(level_tag)
        .map_err(|e| format!("load level: {e}"))?;

    println!("Level: {level_tag}");
    println!("Coord: ({}, {})", manifest.chunk_coord.0, manifest.chunk_coord.1);
    println!("Size: {}x{}", biome_map.width, biome_map.height);
    println!();

    fn stats(name: &str, data: &[f64]) {
        if data.is_empty() {
            println!("  {name}: EMPTY");
            return;
        }
        let min = data.iter().cloned().fold(f64::INFINITY, f64::min);
        let max = data.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        let mean = data.iter().sum::<f64>() / data.len() as f64;
        let variance = data.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / data.len() as f64;
        let stddev = variance.sqrt();
        let range = max - min;
        // Count unique values when quantized to 1.0 blocks at height_scale=5000
        let unique_blocks: std::collections::HashSet<i64> = data.iter()
            .map(|v| (v * 5000.0).floor() as i64)
            .collect();
        println!("  {name}:");
        println!("    range: [{min:.6}, {max:.6}]  span: {range:.6}");
        println!("    mean: {mean:.6}  stddev: {stddev:.6}");
        println!("    unique blocks (@5000x): {}", unique_blocks.len());
    }

    stats("heightmap", &biome_map.heightmap);
    stats("continentalness", &biome_map.continentalness);
    stats("peaks_valleys", &biome_map.peaks_valleys);
    stats("tectonic", &biome_map.tectonic);
    stats("erosion", &biome_map.erosion);
    stats("humidity", &biome_map.humidity);
    stats("temperature", &biome_map.temperature);
    stats("volcanism", &biome_map.volcanism);
    stats("rock_hardness", &biome_map.rock_hardness);
    stats("vegetation_density", &biome_map.vegetation_density);
    stats("rivers", &biome_map.rivers);
    stats("snowpack", &biome_map.snowpack);
    stats("aridity", &biome_map.aridity);
    stats("sediment", &biome_map.sediment);

    // Check biome distribution
    let mut biome_counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for b in &biome_map.biomes {
        *biome_counts.entry(format!("{:?}", b)).or_insert(0) += 1;
    }
    println!("\n  biomes:");
    let mut sorted: Vec<_> = biome_counts.into_iter().collect();
    sorted.sort_by_key(|b| std::cmp::Reverse(b.1));
    for (biome, count) in &sorted {
        let pct = *count as f64 / biome_map.biomes.len() as f64 * 100.0;
        println!("    {biome}: {count} ({pct:.1}%)");
    }

    Ok(())
}
