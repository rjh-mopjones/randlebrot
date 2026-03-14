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

    // A.2: Precompute slope grid from normalized heightmap
    let slope = crate::biome_map::compute_slope_grid(&norm_height, w, h);

    for y in 0..h {
        for x in 0..w {
            let idx = y * w + x;
            let cont = map.continentalness[idx];
            let biome = map.biomes[idx];

            let [r, g, b] = if is_water_biome(biome) && cont < SEA_LEVEL {
                render_ocean(biome, cont, map.light_level[idx], map.temperature[idx])
            } else {
                // 1. Start with biome base color — always use the biome's identity
                let mut pixel = biome.rgb();

                // 2. Sub-biome tinting for grey/brown highland biomes (A.8 expanded)
                if matches!(biome,
                    TileType::Mountain | TileType::Plateau | TileType::Badlands
                    | TileType::Hamada | TileType::ScorchedRock | TileType::AlpineMeadow
                ) {
                    let rock = map.rock_hardness[idx];
                    let eros = map.erosion[idx];
                    let temp = map.temperature[idx];
                    let soil = map.soil_type[idx];
                    let sl = slope[idx];
                    let hn = norm_height[idx];
                    let tect = map.tectonic[idx];

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

                    // A.8: Slope-based ridge highlighting — steep = darker rock
                    if sl > 0.02 {
                        let ridge_factor = ((sl - 0.02) / 0.08).clamp(0.0, 1.0);
                        pixel = lerp_rgb(pixel, [60, 55, 50], ridge_factor * 0.3);
                    }

                    // A.8: Altitudinal banding — high elevations lighter grey
                    if hn > 0.7 {
                        let alt_factor = ((hn - 0.7) / 0.3).clamp(0.0, 1.0);
                        pixel = lerp_rgb(pixel, [190, 195, 200], alt_factor * 0.2);
                    }

                    // A.8: Tectonic age — high stress (young) = reddish-brown, low stress (ancient) = cool grey
                    let stress = 1.0 - tect;
                    if stress > 0.5 {
                        let young_factor = ((stress - 0.5) / 0.5).clamp(0.0, 1.0);
                        pixel = lerp_rgb(pixel, [150, 100, 80], young_factor * 0.15);
                    } else if stress < 0.2 {
                        let ancient_factor = ((0.2 - stress) / 0.2).clamp(0.0, 1.0);
                        pixel = lerp_rgb(pixel, [140, 150, 165], ancient_factor * 0.15);
                    }
                }

                // 2b. Sub-biome tinting for desert biomes (A.5 expanded)
                if is_desert_biome(biome) {
                    let rock = map.rock_hardness[idx];
                    let temp = map.temperature[idx];
                    let eros = map.erosion[idx];
                    let arid = map.aridity[idx];

                    // Hard rock → darker reddish-brown, soft rock → lighter sandy
                    if rock > 0.6 {
                        pixel = lerp_rgb(pixel, [140, 90, 60], (rock - 0.6) * 0.5);
                    } else if rock < 0.4 {
                        pixel = lerp_rgb(pixel, [230, 210, 170], (0.4 - rock) * 0.4);
                    }

                    // Hotter → redder tint
                    if temp > 80.0 {
                        let heat = ((temp - 80.0) / 40.0).clamp(0.0, 1.0);
                        pixel = lerp_rgb(pixel, [200, 120, 60], heat * 0.2);
                    }

                    // A.5: Wind erosion — high wind → paler washed-out (sand-blasted)
                    if !map.wind_speed.is_empty() {
                        let wind = map.wind_speed[idx];
                        if wind > 0.5 {
                            let blast = ((wind - 0.5) / 0.5).clamp(0.0, 1.0);
                            pixel = lerp_rgb(pixel, [235, 225, 205], blast * 0.2);
                        }
                    }

                    // A.5: Erosion → rust/terracotta badlands pull
                    if eros > 0.4 {
                        let badlands = ((eros - 0.4) / 0.4).clamp(0.0, 1.0);
                        pixel = lerp_rgb(pixel, [185, 110, 75], badlands * 0.2);
                    }

                    // A.5: Sediment/drainage → pale alluvium (wadi lines)
                    if !map.drainage_area.is_empty() {
                        let drain = (map.drainage_area[idx] as f64 / 500.0).clamp(0.0, 1.0);
                        if drain > 0.1 {
                            pixel = lerp_rgb(pixel, [220, 210, 190], drain * 0.25);
                        }
                    }
                    if !map.sediment.is_empty() {
                        let sed = map.sediment[idx];
                        if sed > 0.1 {
                            pixel = lerp_rgb(pixel, [210, 195, 165], (sed * 0.3).min(0.2));
                        }
                    }

                    // A.5: Aridity gradient — moderate=golden, extreme=bleached, hyper-arid=dark scorched
                    if arid > 0.85 {
                        let scorched = ((arid - 0.85) / 0.15).clamp(0.0, 1.0);
                        pixel = lerp_rgb(pixel, [120, 80, 55], scorched * 0.2);
                    } else if arid > 0.6 {
                        let bleached = ((arid - 0.6) / 0.25).clamp(0.0, 1.0);
                        pixel = lerp_rgb(pixel, [240, 230, 200], bleached * 0.15);
                    } else if arid > 0.3 {
                        let golden = ((arid - 0.3) / 0.3).clamp(0.0, 1.0);
                        pixel = lerp_rgb(pixel, [210, 185, 120], golden * 0.15);
                    }
                }

                // 2c. Frozen biome tinting (A.6)
                if is_frozen_biome(biome) {
                    let rock = map.rock_hardness[idx];
                    let pv = map.peaks_valleys[idx];
                    let snow_depth = map.snowpack[idx];

                    // Rock showing through thin ice — peaks * rock_hardness
                    let rock_show = pv.abs() * rock;
                    if rock_show > 0.1 {
                        let rock_factor = ((rock_show - 0.1) / 0.4).clamp(0.0, 1.0);
                        pixel = lerp_rgb(pixel, [100, 105, 115], rock_factor * 0.25);
                    }

                    // Snowpack depth → blue ice (thin) vs white snow (deep)
                    if snow_depth < 0.3 {
                        let ice_blue = ((0.3 - snow_depth) / 0.3).clamp(0.0, 1.0);
                        pixel = lerp_rgb(pixel, [180, 210, 235], ice_blue * 0.2);
                    }

                    // Wind on frozen ocean → roughness texture
                    if !map.wind_speed.is_empty() {
                        let wind = map.wind_speed[idx];
                        if wind > 0.4 && matches!(biome, TileType::White | TileType::IceSheet) {
                            let rough = ((wind - 0.4) / 0.4).clamp(0.0, 1.0);
                            pixel = lerp_rgb(pixel, [200, 215, 230], rough * 0.15);
                        }
                    }
                }

                // 2d. Slope-based tinting for non-water, non-highland, non-desert, non-frozen (A.3)
                {
                    let sl = slope[idx];
                    if sl > 0.03 && !is_water_biome(biome) {
                        let cliff_factor = ((sl - 0.03) / 0.07).clamp(0.0, 1.0);
                        pixel = lerp_rgb(pixel, [90, 85, 80], cliff_factor * 0.2);
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

                // 3b. Coastal fringing (A.7 expanded)
                if !matches!(biome, TileType::Beach | TileType::Mangrove | TileType::RockyCoast | TileType::SeaCliff) {
                    let coast_factor = coastal_fringe(cont);
                    if coast_factor > 0.0 {
                        let temp = map.temperature[idx];
                        let rock = map.rock_hardness[idx];
                        let humid = map.humidity[idx];
                        let coast_color = if temp < 0.0 {
                            // A.7: Frozen coast → pale blue-white
                            [210, 220, 235]
                        } else if rock > 0.6 {
                            // A.7: Rocky headland → dark grey-brown
                            [130, 115, 100]
                        } else if humid > 0.6 && temp > 20.0 {
                            // A.7: Mangrove fringe → olive-green
                            [140, 160, 100]
                        } else {
                            // Existing sandy tan
                            [210, 190, 150]
                        };
                        pixel = lerp_rgb(pixel, coast_color, coast_factor * 0.4);
                    }
                }

                // 4. River corridors (A.9 expanded)
                let river = map.rivers[idx];
                if river > 0.02 {
                    let blend = river.sqrt() * 0.6;
                    pixel = lerp_rgb(pixel, [80, 130, 180], blend.min(0.9));
                }

                // A.9: Sediment deposits along rivers → dark brown alluvial tint
                if !map.sediment.is_empty() && river > 0.01 {
                    let sed = map.sediment[idx];
                    if sed > 0.2 {
                        let alluvial = ((sed - 0.2) / 0.5).clamp(0.0, 1.0);
                        pixel = lerp_rgb(pixel, [100, 80, 50], alluvial * 0.2);
                    }
                }

                let rmoist = map.water_table[idx];
                let temp_here = map.temperature[idx];
                if rmoist > 0.05 && river <= 0.02 && temp_here < 45.0 {
                    // A.9: Wider riparian green in arid zones
                    let base_strength = if is_desert_biome(biome) { 0.35 } else { 0.25 };
                    let green_tint = ((rmoist - 0.05) * base_strength).clamp(0.0, 0.2);
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

                // 7. Vegetation tint — category-aware (A.4 expanded)
                let veg = map.vegetation_density[idx];
                if veg > 0.05 && temp_here < 45.0 {
                    if is_forest_biome(biome) {
                        // A.4: Forests — humidity shifts dark green / olive-brown;
                        // rock_hardness shifts cool / warm green
                        let humid = map.humidity[idx];
                        let rock = map.rock_hardness[idx];
                        let base_g = (veg * 100.0 + 80.0).min(255.0) as u8;
                        let green_target = if humid > 0.6 {
                            [30, base_g, 20] // deep dark green
                        } else {
                            [60, (base_g as f64 * 0.85) as u8, 40] // olive-brown shift
                        };
                        let warm_shift = if rock > 0.5 {
                            [green_target[0], green_target[1], (green_target[2] as f64 * 0.8) as u8]
                        } else {
                            [green_target[0], green_target[1], (green_target[2] + 15).min(255)]
                        };
                        pixel = lerp_rgb(pixel, warm_shift, veg * 0.5);
                    } else if is_grassland_biome(biome) {
                        // A.4: Grasslands — aridity shifts green / golden-khaki;
                        // soil_type adds brown undertone
                        let arid = map.aridity[idx];
                        let soil = map.soil_type[idx];
                        let green_target = if arid > 0.4 {
                            let golden = ((arid - 0.4) / 0.4).clamp(0.0, 1.0);
                            lerp_rgb([80, 170, 50], [180, 170, 90], golden)
                        } else {
                            [80, (veg * 120.0 + 80.0).min(255.0) as u8, 50]
                        };
                        let with_soil = if soil > 0.3 {
                            lerp_rgb(green_target, [140, 130, 80], (soil - 0.3) * 0.3)
                        } else {
                            green_target
                        };
                        pixel = lerp_rgb(pixel, with_soil, veg * 0.45);
                    } else if is_wetland_biome(biome) {
                        // A.4: Wetlands — water_table shifts dark olive with blue-teal undertone
                        let wt = map.water_table[idx];
                        let teal_shift = ((wt - 0.2) / 0.6).clamp(0.0, 1.0);
                        let wet_target = lerp_rgb([50, 110, 40], [40, 100, 80], teal_shift);
                        pixel = lerp_rgb(pixel, wet_target, veg * 0.5);
                    } else {
                        // Default: preserve existing generic green tint
                        let green_target = [40, (veg * 120.0 + 60.0).min(255.0) as u8, 30];
                        let strength = if is_desert_biome(biome) { 0.3 } else { 0.5 };
                        pixel = lerp_rgb(pixel, green_target, veg * strength);
                    }
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

                // 9. Aerial perspective — reduced for frozen biomes to avoid gray wash
                let ll = map.light_level[idx];
                let haze_strength = if is_polar_ice(biome, ll) { 0.05 } else { 0.15 };
                let haze_amount = (1.0 - ll) * haze_strength;
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

/// Check if this pixel is in the polar ice cap zone (should render bright white).
fn is_polar_ice(b: TileType, light_level: f64) -> bool {
    // Deep polar zone: everything is ice
    if light_level < 0.12 {
        return matches!(b,
            TileType::White | TileType::IceSheet | TileType::Snow
            | TileType::Glacier | TileType::FrozenBog | TileType::Mountain
            | TileType::Tundra
        );
    }
    // Transition zone: only explicitly frozen biomes
    if light_level < 0.20 {
        return matches!(b,
            TileType::White | TileType::IceSheet | TileType::Snow | TileType::Glacier
        );
    }
    false
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

// A.1: Biome category helpers

fn is_forest_biome(b: TileType) -> bool {
    matches!(
        b,
        TileType::Forest
            | TileType::DeciduousForest
            | TileType::TemperateRainforest
            | TileType::SubtropicalForest
            | TileType::CloudForest
            | TileType::Jungle
            | TileType::Taiga
            | TileType::Woodland
            | TileType::DryWoodland
    )
}

fn is_grassland_biome(b: TileType) -> bool {
    matches!(
        b,
        TileType::Plains
            | TileType::Meadow
            | TileType::Steppe
            | TileType::Savanna
            | TileType::HighlandSavanna
            | TileType::Scrubland
            | TileType::Thornland
            | TileType::AlpineMeadow
    )
}

fn is_wetland_biome(b: TileType) -> bool {
    matches!(
        b,
        TileType::Marsh
            | TileType::FrozenBog
            | TileType::Mangrove
    )
}

fn is_frozen_biome(b: TileType) -> bool {
    matches!(
        b,
        TileType::Snow
            | TileType::IceSheet
            | TileType::Glacier
            | TileType::FrozenBog
            | TileType::Tundra
            | TileType::White
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
/// Edge pixels use clamped coordinates instead of flat defaults to avoid
/// visible grid seams when tiles are stitched together.
fn compute_hillshade(
    norm_height: &[f64],
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    light_dir: [f64; 3],
) -> f64 {
    let r = 2usize;
    let get = |xi: usize, yi: usize| norm_height[yi.min(h - 1) * w + xi.min(w - 1)];

    // High z_factor because normalized heightmap has full [0,1] range
    // and we want dramatic terrain relief
    let z_factor = 30.0;

    // Clamp at edges instead of returning a default — eliminates tile boundary seams
    let x_lo = x.saturating_sub(r);
    let x_hi = (x + r).min(w - 1);
    let y_lo = y.saturating_sub(r);
    let y_hi = (y + r).min(h - 1);
    let x_span = (x_hi - x_lo).max(1) as f64;
    let y_span = (y_hi - y_lo).max(1) as f64;

    let dx = (get(x_hi, y) - get(x_lo, y)) / x_span;
    let dy = (get(x, y_hi) - get(x, y_lo)) / y_span;

    let normal = normalize([-dx * z_factor, -dy * z_factor, 1.0]);
    let dot = normal[0] * light_dir[0] + normal[1] * light_dir[1] + normal[2] * light_dir[2];

    dot.clamp(0.25, 1.0)
}

/// Approximate ambient occlusion from heightmap curvature.
/// Edge pixels use clamped coordinates to avoid tile boundary seams.
fn compute_ao(norm_height: &[f64], x: usize, y: usize, w: usize, h: usize) -> f64 {
    let r = 2usize;
    let get = |xi: usize, yi: usize| norm_height[yi.min(h - 1) * w + xi.min(w - 1)];
    let center = get(x, y);
    let neighbors = get(x.saturating_sub(r), y)
        + get((x + r).min(w - 1), y)
        + get(x, y.saturating_sub(r))
        + get(x, (y + r).min(h - 1));
    let laplacian = neighbors / 4.0 - center;

    // Normalized heightmap so curvature values are larger → visible AO
    1.0 - (-laplacian * 8.0).clamp(0.0, 0.3)
}

/// Render an ocean pixel based on biome, depth, light, and temperature (A.10 expanded).
/// Uses the biome assignment (White vs Sea) for sharp ice boundaries rather than
/// a smooth temperature gradient, so the jagged ice edge from biome classification
/// is visually preserved.
fn render_ocean(biome: TileType, continentalness: f64, light_level: f64, temperature: f64) -> [u8; 3] {
    let depth = (SEA_LEVEL - continentalness).clamp(0.0, 0.5);
    let depth_norm = depth / 0.5;

    let is_frozen_biome = biome == TileType::White;

    // Frozen ocean biome: render as ice directly (sharp boundary)
    if is_frozen_biome {
        // Vary ice appearance by depth and light for visual interest
        let base_ice = lerp_rgb([220, 235, 250], [235, 245, 255], (1.0 - depth_norm).clamp(0.0, 1.0));
        let brightness = 0.85 + light_level * 0.15;
        return [
            (base_ice[0] as f64 * brightness) as u8,
            (base_ice[1] as f64 * brightness) as u8,
            (base_ice[2] as f64 * brightness) as u8,
        ];
    }

    let deep = [10u8, 30, 80];
    let shallow = [60u8, 140, 200];

    let mut pixel = lerp_rgb(shallow, deep, depth_norm);

    // A.10: Temperature-based shallow water colour shift
    if depth_norm < 0.3 {
        let shallow_factor = 1.0 - depth_norm / 0.3;
        if temperature > 25.0 {
            // Warm tropics → turquoise shift
            let warmth = ((temperature - 25.0) / 15.0).clamp(0.0, 1.0);
            pixel = lerp_rgb(pixel, [70, 190, 185], warmth * shallow_factor * 0.3);
        } else if temperature < 0.0 && temperature >= -10.0 {
            // Cold but not frozen → steel-blue shift
            let cold = ((-temperature) / 10.0).clamp(0.0, 1.0);
            pixel = lerp_rgb(pixel, [80, 110, 150], cold * shallow_factor * 0.3);
        }
    }

    // Slight ice fringe for non-frozen ocean near freezing point
    // (just a thin visual hint, not the full smooth gradient)
    if temperature < -10.0 {
        let ice_hint = ((-10.0 - temperature) / 15.0).clamp(0.0, 0.3);
        pixel = lerp_rgb(pixel, [200, 220, 240], ice_hint);
    }

    // Light-level modulation: darken on dark side, but less for near-frozen ocean
    let brightness = if temperature < -10.0 {
        0.7 + light_level * 0.3
    } else {
        0.5 + light_level * 0.5
    };
    pixel = [
        (pixel[0] as f64 * brightness) as u8,
        (pixel[1] as f64 * brightness) as u8,
        (pixel[2] as f64 * brightness) as u8,
    ];

    pixel
}
