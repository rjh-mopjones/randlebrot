//! Composited terrain renderer.
//!
//! Replaces flat biome colors with a multi-layer composited image using
//! heightmap, erosion, snowpack, volcanism, rivers, vegetation, and lighting data.

use rb_core::TileType;

use crate::biome_map::{BiomeMap, SEA_LEVEL};

/// Render a fully composited terrain image from all BiomeMap layers.
/// Returns RGBA bytes (width * height * 4).
pub fn render_terrain(map: &BiomeMap) -> Vec<u8> {
    let w = map.width;
    let h = map.height;
    let mut data = Vec::with_capacity(w * h * 4);

    // Precompute heightmap min/max for normalization (land only)
    let (land_min, land_max) = land_height_range(&map.heightmap, &map.continentalness);

    // Light direction for hillshading: northwest, steep angle
    let light_dir = normalize([1.0, -1.0, 2.0]);

    for y in 0..h {
        for x in 0..w {
            let idx = y * w + x;
            let cont = map.continentalness[idx];
            let biome = map.biomes[idx];

            let [r, g, b] = if is_water_biome(biome) && cont < SEA_LEVEL {
                // Ocean rendering
                render_ocean(cont, map.light_level[idx], map.temperature[idx])
            } else {
                // Land compositing pipeline
                let elevation_norm = if land_max > land_min {
                    ((map.heightmap[idx] - land_min) / (land_max - land_min)).clamp(0.0, 1.0)
                } else {
                    0.5
                };

                // 1. Base color from biome + elevation
                let mut pixel = biome_base_color(biome, elevation_norm);

                // 2. Slope-based rock exposure
                let slope = compute_slope(&map.heightmap, x, y, w, h);
                if slope > 0.15 {
                    let rock_alpha = ((slope - 0.15) / 0.3).clamp(0.0, 1.0);
                    let hardness_boost = map.rock_hardness[idx] * 0.3;
                    let alpha = (rock_alpha * (0.7 + hardness_boost)).clamp(0.0, 1.0);
                    pixel = lerp_rgb(pixel, [140, 130, 120], alpha);
                }

                // 3. Coastal fringing
                if !matches!(biome, TileType::Beach | TileType::Mangrove | TileType::RockyCoast | TileType::SeaCliff) {
                    let coast_factor = coastal_fringe(cont);
                    if coast_factor > 0.0 {
                        pixel = lerp_rgb(pixel, [210, 190, 150], coast_factor * 0.4);
                    }
                }

                // 4. River corridors
                let river = map.rivers[idx];
                if river > 0.02 {
                    let blend = river.sqrt() * 0.6;
                    pixel = lerp_rgb(pixel, [80, 130, 180], blend.min(0.9));
                }
                let rmoist = map.river_moisture[idx];
                if rmoist > 0.1 && river <= 0.02 {
                    let green_tint = ((rmoist - 0.1) * 0.15).clamp(0.0, 0.1);
                    pixel = lerp_rgb(pixel, [60, 160, 60], green_tint);
                }

                // 5. Snowpack overlay
                let snow = map.snowpack[idx];
                if snow > 0.01 {
                    let blend = snow.powf(0.7);
                    pixel = lerp_rgb(pixel, [245, 248, 255], blend);
                }

                // 6. Volcanism overlay (pre-hillshade for glow effect check)
                let volc = map.volcanism[idx];
                let is_emissive = volc > 0.85;
                if volc > 0.5 {
                    let factor = ((volc - 0.5) / 0.5).clamp(0.0, 1.0);
                    if is_emissive {
                        pixel = lerp_rgb(pixel, [255, 120, 30], factor);
                    } else {
                        pixel = lerp_rgb(pixel, [120, 40, 20], factor);
                    }
                }

                // 7. Vegetation tint
                let veg = map.vegetation_density[idx];
                if veg > 0.05 && !is_desert_biome(biome) {
                    let green_target = [40, (veg * 120.0 + 60.0).min(255.0) as u8, 30];
                    pixel = lerp_rgb(pixel, green_target, veg * 0.3);
                }

                // 8. Hillshading (skip for emissive volcanic)
                if !is_emissive {
                    let shade = compute_hillshade(&map.heightmap, x, y, w, h, light_dir);

                    // 9. Ambient occlusion
                    let ao = compute_ao(&map.heightmap, x, y, w, h);

                    let lighting = shade * ao;
                    pixel = [
                        (pixel[0] as f64 * lighting).clamp(0.0, 255.0) as u8,
                        (pixel[1] as f64 * lighting).clamp(0.0, 255.0) as u8,
                        (pixel[2] as f64 * lighting).clamp(0.0, 255.0) as u8,
                    ];
                }

                // 10. Aerial perspective
                let ll = map.light_level[idx];
                let haze_amount = (1.0 - ll) * 0.15;
                if haze_amount > 0.001 {
                    let haze_color = lerp_rgb([180, 200, 220], [220, 200, 170], ll);
                    pixel = lerp_rgb(pixel, haze_color, haze_amount);
                }

                pixel
            };

            data.extend_from_slice(&[r, g, b, 255]);
        }
    }

    data
}

// ---- Helper functions ----

fn lerp_rgb(a: [u8; 3], b: [u8; 3], t: f64) -> [u8; 3] {
    let t = t.clamp(0.0, 1.0);
    let inv = 1.0 - t;
    [
        (a[0] as f64 * inv + b[0] as f64 * t) as u8,
        (a[1] as f64 * inv + b[1] as f64 * t) as u8,
        (a[2] as f64 * inv + b[2] as f64 * t) as u8,
    ]
}

fn normalize(v: [f64; 3]) -> [f64; 3] {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if len < 1e-10 {
        return [0.0, 0.0, 1.0];
    }
    [v[0] / len, v[1] / len, v[2] / len]
}

fn is_water_biome(b: TileType) -> bool {
    matches!(
        b,
        TileType::Sea
            | TileType::ShallowSea
            | TileType::ContinentalShelf
            | TileType::DeepOcean
            | TileType::OceanTrench
            | TileType::OceanRidge
            | TileType::White
            | TileType::CoralReef
    )
}

fn is_desert_biome(b: TileType) -> bool {
    matches!(
        b,
        TileType::Desert
            | TileType::Sahara
            | TileType::Erg
            | TileType::Hamada
            | TileType::SaltFlat
            | TileType::Badlands
            | TileType::ScorchedRock
    )
}

/// Compute the land-only height range for normalization.
fn land_height_range(heightmap: &[f64], continentalness: &[f64]) -> (f64, f64) {
    let mut min = f64::MAX;
    let mut max = f64::MIN;
    for (i, &h) in heightmap.iter().enumerate() {
        if continentalness[i] >= SEA_LEVEL {
            if h < min { min = h; }
            if h > max { max = h; }
        }
    }
    if min > max {
        (0.0, 1.0)
    } else {
        (min, max)
    }
}

/// Biome base color modulated by elevation (2-stop gradient around biome rgb).
fn biome_base_color(biome: TileType, elevation_norm: f64) -> [u8; 3] {
    let base = biome.rgb();

    if is_desert_biome(biome) {
        // Desert: low = redder, high = yellower
        let low = [
            base[0].saturating_sub(15),
            base[1].saturating_sub(30),
            base[2].saturating_sub(10),
        ];
        let high = [
            base[0],
            base[1].saturating_add(15),
            base[2].saturating_add(20),
        ];
        lerp_rgb(low, high, elevation_norm)
    } else {
        // General: low = darker/warmer, high = lighter/cooler
        let low = [
            (base[0] as f64 * 0.80) as u8,
            (base[1] as f64 * 0.80) as u8,
            (base[2] as f64 * 0.82) as u8,
        ];
        let high = [
            ((base[0] as f64 * 1.20).min(255.0)) as u8,
            ((base[1] as f64 * 1.18).min(255.0)) as u8,
            ((base[2] as f64 * 1.20).min(255.0)) as u8,
        ];
        lerp_rgb(low, high, elevation_norm)
    }
}

/// Sobel-based slope magnitude from heightmap.
fn compute_slope(heightmap: &[f64], x: usize, y: usize, w: usize, h: usize) -> f64 {
    if x == 0 || x >= w - 1 || y == 0 || y >= h - 1 {
        return 0.0;
    }
    let get = |xi: usize, yi: usize| heightmap[yi * w + xi];

    let dx = (get(x + 1, y - 1) + 2.0 * get(x + 1, y) + get(x + 1, y + 1))
           - (get(x - 1, y - 1) + 2.0 * get(x - 1, y) + get(x - 1, y + 1));
    let dy = (get(x - 1, y + 1) + 2.0 * get(x, y + 1) + get(x + 1, y + 1))
           - (get(x - 1, y - 1) + 2.0 * get(x, y - 1) + get(x + 1, y - 1));

    (dx * dx + dy * dy).sqrt()
}

/// Lambertian hillshading from heightmap normals.
fn compute_hillshade(
    heightmap: &[f64],
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    light_dir: [f64; 3],
) -> f64 {
    if x == 0 || x >= w - 1 || y == 0 || y >= h - 1 {
        return 0.8;
    }
    let get = |xi: usize, yi: usize| heightmap[yi * w + xi];

    let z_factor = 8.0;

    let dx = (get(x + 1, y - 1) + 2.0 * get(x + 1, y) + get(x + 1, y + 1))
           - (get(x - 1, y - 1) + 2.0 * get(x - 1, y) + get(x - 1, y + 1));
    let dy = (get(x - 1, y + 1) + 2.0 * get(x, y + 1) + get(x + 1, y + 1))
           - (get(x - 1, y - 1) + 2.0 * get(x, y - 1) + get(x + 1, y - 1));

    let normal = normalize([-dx * z_factor, -dy * z_factor, 1.0]);
    let dot = normal[0] * light_dir[0] + normal[1] * light_dir[1] + normal[2] * light_dir[2];

    dot.clamp(0.2, 1.0)
}

/// Approximate ambient occlusion from heightmap Laplacian (curvature).
fn compute_ao(heightmap: &[f64], x: usize, y: usize, w: usize, h: usize) -> f64 {
    if x == 0 || x >= w - 1 || y == 0 || y >= h - 1 {
        return 1.0;
    }
    let get = |xi: usize, yi: usize| heightmap[yi * w + xi];
    let center = get(x, y);
    let neighbors = get(x - 1, y) + get(x + 1, y) + get(x, y - 1) + get(x, y + 1);
    let laplacian = neighbors / 4.0 - center;

    // Negative laplacian = valley → darken
    1.0 - (-laplacian * 15.0).clamp(0.0, 0.3)
}

/// Coastal fringe blend factor for land pixels near sea level.
fn coastal_fringe(continentalness: f64) -> f64 {
    if continentalness < SEA_LEVEL {
        return 0.0;
    }
    1.0 - ((continentalness - SEA_LEVEL) / 0.03).clamp(0.0, 1.0)
}

/// Render an ocean pixel based on depth, light, and temperature.
fn render_ocean(continentalness: f64, light_level: f64, temperature: f64) -> [u8; 3] {
    let depth = (SEA_LEVEL - continentalness).clamp(0.0, 0.5);
    let depth_norm = depth / 0.5;

    let deep = [10u8, 30, 80];
    let shallow = [60u8, 140, 200];

    let mut pixel = lerp_rgb(shallow, deep, depth_norm);

    // Frozen ocean
    if temperature < -15.0 {
        let ice_factor = ((-15.0 - temperature) / 20.0).clamp(0.0, 1.0);
        pixel = lerp_rgb(pixel, [220, 235, 255], ice_factor);
    }

    // Light-level modulation: darken on dark side
    let brightness = 0.5 + light_level * 0.5;
    pixel = [
        (pixel[0] as f64 * brightness) as u8,
        (pixel[1] as f64 * brightness) as u8,
        (pixel[2] as f64 * brightness) as u8,
    ];

    pixel
}
