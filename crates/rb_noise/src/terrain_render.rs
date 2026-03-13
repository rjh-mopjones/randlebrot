//! Composited terrain renderer.
//!
//! Replaces flat biome colors with a multi-layer composited image using
//! heightmap, erosion, snowpack, volcanism, rivers, vegetation, and lighting data.
//!
//! Key insight: the heightmap raw values vary over a tiny range per tile (e.g. 0.03 to 0.33),
//! so we precompute a contrast-stretched heightmap normalized to [0,1] per tile.
//! All terrain shading (hillshade, AO, slope) operates on this normalized version
//! so derivatives are meaningful.

use rb_core::TileType;

use crate::biome_map::{BiomeMap, SEA_LEVEL};

/// Global heightmap statistics for consistent normalization across tiles.
/// Without this, each tile normalizes independently causing visible tile boundary seams.
#[derive(Clone, Debug)]
pub struct NormalizationHints {
    pub heightmap_min: f64,
    pub heightmap_max: f64,
}

/// Render a fully composited terrain image from all BiomeMap layers.
/// Returns RGBA bytes (width * height * 4).
///
/// If `hints` is provided, uses global heightmap range for normalization
/// so all tiles produce consistent shading. Without hints, falls back to
/// per-tile normalization (which causes tile boundary artifacts).
pub fn render_terrain(map: &BiomeMap, hints: Option<&NormalizationHints>) -> Vec<u8> {
    let w = map.width;
    let h = map.height;
    let mut data = Vec::with_capacity(w * h * 4);

    // Light direction for hillshading: northwest, steep angle
    let light_dir = normalize([1.0, -1.0, 2.0]);

    // Contrast-stretched heightmap normalized to [0,1].
    // Uses global range from hints if available, otherwise per-tile range.
    let norm_height = {
        let (hmin, hmax) = if let Some(h) = hints {
            (h.heightmap_min, h.heightmap_max)
        } else {
            let mut hmin = f64::MAX;
            let mut hmax = f64::MIN;
            for &v in &map.heightmap {
                if v < hmin { hmin = v; }
                if v > hmax { hmax = v; }
            }
            (hmin, hmax)
        };
        let range = if hmax > hmin { hmax - hmin } else { 1.0 };
        map.heightmap.iter().map(|&v| ((v - hmin) / range).clamp(0.0, 1.0)).collect::<Vec<f64>>()
    };

    for y in 0..h {
        for x in 0..w {
            let idx = y * w + x;
            let cont = map.continentalness[idx];
            let biome = map.biomes[idx];

            let [r, g, b] = if is_water_biome(biome) && cont < SEA_LEVEL {
                render_ocean(cont, map.light_level[idx], map.temperature[idx])
            } else {
                // 1. Start with biome base color — always use the biome's identity
                let mut pixel = biome.rgb();

                // 2. Sub-biome tinting for grey/brown highland biomes
                //    Uses rock hardness, erosion, temperature, soil to break up monotony
                if matches!(biome,
                    TileType::Mountain | TileType::Plateau | TileType::Badlands
                    | TileType::Hamada | TileType::ScorchedRock | TileType::AlpineMeadow
                ) {
                    let rock = map.rock_hardness[idx];
                    let eros = map.erosion[idx];
                    let temp = map.temperature[idx];
                    let soil = map.soil_type[idx];

                    // Hard rock → blue-grey, soft rock → warm brown
                    pixel = lerp_rgb(pixel, [160, 130, 100], (1.0 - rock) * 0.25);

                    // Erosion reveals warm sedimentary strata
                    if eros > 0.1 {
                        pixel = lerp_rgb(pixel, [180, 150, 110], (eros * 0.4).min(0.3));
                    }

                    // Temperature: hot = reddish, cold = blue-grey
                    if temp > 50.0 {
                        let heat = ((temp - 50.0) / 40.0).clamp(0.0, 1.0);
                        pixel = lerp_rgb(pixel, [170, 100, 70], heat * 0.25);
                    } else if temp < 10.0 {
                        let cold = ((10.0 - temp) / 30.0).clamp(0.0, 1.0);
                        pixel = lerp_rgb(pixel, [130, 140, 160], cold * 0.2);
                    }

                    // Soil in low-slope areas → greenish-brown tint
                    if soil > 0.2 {
                        pixel = lerp_rgb(pixel, [120, 140, 80], (soil - 0.2) * 0.3);
                    }
                }

                // 3. Modulate brightness by normalized height (±15% around base)
                let hn = norm_height[idx];
                let brightness = 0.85 + hn * 0.30;
                pixel = [
                    (pixel[0] as f64 * brightness).clamp(0.0, 255.0) as u8,
                    (pixel[1] as f64 * brightness).clamp(0.0, 255.0) as u8,
                    (pixel[2] as f64 * brightness).clamp(0.0, 255.0) as u8,
                ];

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
                let rmoist = map.water_table[idx];
                if rmoist > 0.1 && river <= 0.02 {
                    let green_tint = ((rmoist - 0.1) * 0.15).clamp(0.0, 0.1);
                    pixel = lerp_rgb(pixel, [60, 160, 60], green_tint);
                }

                // 5. Snowpack overlay
                let snow = map.snowpack[idx];
                if snow > 0.01 {
                    pixel = lerp_rgb(pixel, [245, 248, 255], snow.powf(0.7));
                }

                // 6. Volcanism overlay
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

                // 7. Vegetation tint — strong enough to show through everywhere
                let veg = map.vegetation_density[idx];
                if veg > 0.05 {
                    let green_target = [40, (veg * 120.0 + 60.0).min(255.0) as u8, 30];
                    // Weaker in desert (oases), stronger elsewhere
                    let strength = if is_desert_biome(biome) { 0.3 } else { 0.5 };
                    pixel = lerp_rgb(pixel, green_target, veg * strength);
                }

                // 8. Hillshading on normalized heightmap (skip for emissive volcanic)
                if !is_emissive {
                    let shade = compute_hillshade(&norm_height, x, y, w, h, light_dir);
                    let ao = compute_ao(&norm_height, x, y, w, h);
                    let lighting = shade * ao;
                    pixel = [
                        (pixel[0] as f64 * lighting).clamp(0.0, 255.0) as u8,
                        (pixel[1] as f64 * lighting).clamp(0.0, 255.0) as u8,
                        (pixel[2] as f64 * lighting).clamp(0.0, 255.0) as u8,
                    ];
                }

                // 9. Aerial perspective
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

/// Coastal fringe blend factor for land pixels near sea level.
fn coastal_fringe(continentalness: f64) -> f64 {
    if continentalness < SEA_LEVEL {
        return 0.0;
    }
    1.0 - ((continentalness - SEA_LEVEL) / 0.03).clamp(0.0, 1.0)
}

/// Lambertian hillshading on contrast-stretched heightmap.
/// Since the input is normalized [0,1], derivatives are meaningful and
/// z_factor controls how dramatic the relief shading appears.
fn compute_hillshade(
    norm_height: &[f64],
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    light_dir: [f64; 3],
) -> f64 {
    let r = 2usize;
    if x < r || x + r >= w || y < r || y + r >= h {
        return 0.8;
    }
    let get = |xi: usize, yi: usize| norm_height[yi * w + xi];

    // High z_factor because normalized heightmap has full [0,1] range
    // and we want dramatic terrain relief
    let z_factor = 30.0;

    let dx = (get(x + r, y) - get(x - r, y)) / (2 * r) as f64;
    let dy = (get(x, y + r) - get(x, y - r)) / (2 * r) as f64;

    let normal = normalize([-dx * z_factor, -dy * z_factor, 1.0]);
    let dot = normal[0] * light_dir[0] + normal[1] * light_dir[1] + normal[2] * light_dir[2];

    dot.clamp(0.25, 1.0)
}

/// Approximate ambient occlusion from heightmap curvature.
fn compute_ao(norm_height: &[f64], x: usize, y: usize, w: usize, h: usize) -> f64 {
    let r = 2usize;
    if x < r || x + r >= w || y < r || y + r >= h {
        return 1.0;
    }
    let get = |xi: usize, yi: usize| norm_height[yi * w + xi];
    let center = get(x, y);
    let neighbors = get(x - r, y) + get(x + r, y) + get(x, y - r) + get(x, y + r);
    let laplacian = neighbors / 4.0 - center;

    // Normalized heightmap so curvature values are larger → visible AO
    1.0 - (-laplacian * 8.0).clamp(0.0, 0.3)
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
