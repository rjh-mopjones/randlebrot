use noise::OpenSimplex;
use rayon::prelude::*;
use rb_core::{NoiseStrategy, TileType};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::biome_splines::BiomeSplines;
use crate::erosion_sim::{ErosionParams, simulate_erosion};
use crate::progress::{LayerId, LayerProgress};
use crate::resource_map::ResourceMap;
use crate::rivers::{RiverGenerator, RiverNetwork};
use crate::derived;
use crate::strategy::{
    ContinentalnessStrategy, HumidityStrategy, LightLevelStrategy,
    PeaksAndValleysStrategy, RockHardnessStrategy,
    TectonicPlatesStrategy,
    wind::{WindField, advect_moisture},
};
use crate::visualization::{
    aridity_to_rgba, grayscale_to_rgba, heightmap_to_rgba, humidity_to_rgba,
    light_level_to_rgba, peaks_to_rgba, precipitation_type_to_rgba, resources_to_rgba,
    river_to_rgba, rock_hardness_to_rgba, snowpack_to_rgba,
    soil_type_to_rgba, tectonic_to_rgba, temperature_to_rgba, vegetation_to_rgba,
    volcanism_to_rgba, water_table_to_rgba, wind_speed_to_rgba, NoiseLayer,
};

/// Sea level threshold for continentalness.
/// Values below this are ocean, values above are land.
pub const SEA_LEVEL: f64 = -0.01;

/// Backend selection for noise generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum NoiseBackend {
    /// CPU-based parallel noise generation using Rayon.
    Cpu,
    /// GPU-accelerated noise generation using wgpu compute shaders.
    /// Falls back to CPU if GPU is unavailable.
    #[default]
    Gpu,
}

impl NoiseBackend {
    /// Check if GPU is available for noise generation.
    #[cfg(feature = "gpu")]
    pub fn gpu_available() -> bool {
        crate::gpu::GpuNoiseContext::is_available()
    }

    /// Check if GPU is available for noise generation.
    #[cfg(not(feature = "gpu"))]
    pub fn gpu_available() -> bool {
        false
    }
}

/// A complete biome map storing noise values and computed biomes.
///
/// This struct holds all the data needed to render different visualization
/// layers (biome colors, temperature heatmap, etc.).
#[derive(Serialize, Deserialize)]
pub struct BiomeMap {
    pub width: usize,
    pub height: usize,

    // Base layers
    pub continentalness: Vec<f64>,
    pub tectonic: Vec<f64>,
    pub tectonic_plate_ids: Vec<f64>,
    pub humidity: Vec<f64>,
    pub rock_hardness: Vec<f64>,
    pub light_level: Vec<f64>,

    // Derived layers
    pub peaks_valleys: Vec<f64>,
    pub volcanism: Vec<f64>,
    pub heightmap: Vec<f64>,
    pub temperature: Vec<f64>,
    pub erosion: Vec<f64>,
    pub rivers: Vec<f64>,
    pub aridity: Vec<f64>,
    pub precipitation_type: Vec<f64>,
    pub water_table: Vec<f64>,
    pub wind_speed: Vec<f64>,
    pub resource_richness: Vec<f64>,
    pub snowpack: Vec<f64>,
    pub biomes: Vec<TileType>,
    pub vegetation_density: Vec<f64>,
    pub soil_type: Vec<f64>,
    pub resource_map: Option<ResourceMap>,

    /// Global river network (only set on the macro-level 1024×512 map).
    /// Skipped during serialization — rebuild from terrain data after deserialization.
    #[serde(skip)]
    pub river_network: Option<Arc<RiverNetwork>>,

    /// Drainage area from erosion sim (macro-level, sampled at other levels).
    pub drainage_area: Vec<u32>,
    /// Sediment accumulation from erosion sim (macro-level, sampled at other levels).
    pub sediment: Vec<f64>,
    /// World dimensions for coordinate mapping when sampling macro fields.
    pub world_width: f64,
    pub world_height: f64,
}

/// Apply the polar ice cap override with jagged edges.
///
/// The deep core (light < 0.05) is solid White. The outer margin uses
/// peaks_valleys + rock_hardness noise to perturb the 0.12 light threshold,
/// creating irregular ice sheet coastlines with fjords and peninsulas.
fn apply_polar_ice_cap(
    biomes: &mut [TileType],
    light_level: &[f64],
    continentalness: &[f64],
    peaks_valleys: &[f64],
    rock_hardness: &[f64],
    temperature: &[f64],
    sea_level: f64,
) {
    for idx in 0..biomes.len() {
        let light = light_level[idx];
        let cont = continentalness[idx];
        let rock = rock_hardness[idx];
        let pv = peaks_valleys[idx];

        if light < 0.05 {
            // Deep polar core: always solid White
            biomes[idx] = TileType::White;
            continue;
        }

        // Phase 1: Perturbed light-level threshold for polar cap edge
        // peaks_valleys: high-frequency jaggedness (fjords/peninsulas)
        // rock_hardness: low-frequency lobes
        // continentalness: ice extends further over land than water
        let land_bonus = if cont >= sea_level { 0.02 } else { -0.02 };
        let light_perturb = pv * 0.06 + (rock - 0.5) * 0.06 + land_bonus;
        let light_threshold = 0.12 + light_perturb;
        if light < light_threshold {
            if cont < sea_level {
                biomes[idx] = TileType::White;
            } else {
                biomes[idx] = TileType::IceSheet;
            }
            continue;
        }

        // Phase 2: Jagged frozen ocean edge
        // The -15°C isotherm in ocean_biome() creates a smooth ice front because
        // peaks_valleys has low amplitude over deep ocean. Use rock_hardness and
        // continentalness as noise sources to perturb the frozen ocean boundary.
        if cont < sea_level {
            let temp = temperature[idx];
            // Only perturb near the freezing boundary (-25°C to 5°C)
            if temp > -25.0 && temp < 5.0 {
                // rock_hardness has meaningful variation everywhere (independent fBm)
                // continentalness provides depth-based variation (shallow water freezes first)
                let depth = (sea_level - cont).clamp(0.0, 0.5);
                let depth_factor = 1.0 - depth * 3.0; // shallow → freezes easily (+), deep → resists (-)
                let rock_perturb = (rock - 0.5) * 15.0; // ±7.5°C
                let cont_perturb = depth_factor * 5.0;   // shallow: +5°C (colder), deep: -5°C (warmer)
                let pv_perturb = pv * 5.0;               // ±5°C (small over ocean but still helps)
                let perturbed_threshold = -15.0 + rock_perturb + cont_perturb + pv_perturb;
                if temp < perturbed_threshold {
                    biomes[idx] = TileType::White;
                } else if biomes[idx] == TileType::White {
                    // Perturbation opened water where it was frozen — reclassify
                    biomes[idx] = TileType::Sea;
                }
            }
        }
    }
}

/// Compute slope grid from heightmap using 3x3 finite differences.
/// Returns gradient magnitude per cell.
pub(crate) fn compute_slope_grid(heightmap: &[f64], width: usize, height: usize) -> Vec<f64> {
    let total = width * height;
    let mut slope = vec![0.0f64; total];

    for y in 1..(height - 1) {
        for x in 1..(width - 1) {
            let idx = y * width + x;
            // Sobel-like gradient
            let dzdx = (heightmap[idx + 1] - heightmap[idx - 1]) * 0.5;
            let dzdy = (heightmap[idx + width] - heightmap[idx - width]) * 0.5;
            slope[idx] = (dzdx * dzdx + dzdy * dzdy).sqrt();
        }
    }
    // Edge cells: copy nearest interior
    for x in 0..width {
        slope[x] = slope[width + x.clamp(1, width - 2)];
        slope[(height - 1) * width + x] = slope[(height - 2) * width + x.clamp(1, width - 2)];
    }
    for y in 0..height {
        slope[y * width] = slope[y * width + 1];
        slope[y * width + width - 1] = slope[y * width + width - 2];
    }

    slope
}

/// Bilinear sample a f64 grid at fractional pixel coordinates.
/// x wraps horizontally for cylindrical world.
fn sample_bilinear(data: &[f64], width: usize, height: usize, fx: f64, fy: f64) -> f64 {
    let x0 = (fx.floor() as usize).min(width - 1);
    let y0 = (fy.floor() as usize).min(height - 1);
    let x1 = (x0 + 1) % width; // wrap x horizontally
    let y1 = (y0 + 1).min(height - 1);
    let tx = fx - fx.floor();
    let ty = fy - fy.floor();
    let v00 = data[y0 * width + x0];
    let v10 = data[y0 * width + x1];
    let v01 = data[y1 * width + x0];
    let v11 = data[y1 * width + x1];
    let top = v00 * (1.0 - tx) + v10 * tx;
    let bot = v01 * (1.0 - tx) + v11 * tx;
    top * (1.0 - ty) + bot * ty
}

/// Bilinear sample a u32 grid (returns f64 for interpolation).
/// x wraps horizontally for cylindrical world.
fn sample_bilinear_u32(data: &[u32], width: usize, height: usize, fx: f64, fy: f64) -> f64 {
    let x0 = (fx.floor() as usize).min(width - 1);
    let y0 = (fy.floor() as usize).min(height - 1);
    let x1 = (x0 + 1) % width; // wrap x horizontally
    let y1 = (y0 + 1).min(height - 1);
    let tx = fx - fx.floor();
    let ty = fy - fy.floor();
    let v00 = data[y0 * width + x0] as f64;
    let v10 = data[y0 * width + x1] as f64;
    let v01 = data[y1 * width + x0] as f64;
    let v11 = data[y1 * width + x1] as f64;
    let top = v00 * (1.0 - tx) + v10 * tx;
    let bot = v01 * (1.0 - tx) + v11 * tx;
    top * (1.0 - ty) + bot * ty
}

impl BiomeMap {
    /// Sample a macro-level field at arbitrary world coordinates.
    /// Uses nearest-neighbor to preserve sharp ridge/valley edges from erosion.
    /// X wraps horizontally for cylindrical world.
    pub fn sample_heightmap_at(&self, wx: f64, wy: f64) -> f64 {
        if self.heightmap.is_empty() { return 0.0; }
        let wrapped_x = crate::wrap::wrap_x(wx, self.world_width);
        let x = (wrapped_x.round() as usize).min(self.width - 1);
        let y = (wy.clamp(0.0, self.world_height - 1.0).round() as usize).min(self.height - 1);
        self.heightmap[y * self.width + x]
    }

    pub fn sample_drainage_at(&self, wx: f64, wy: f64) -> f64 {
        if self.drainage_area.is_empty() { return 1.0; }
        let fx = crate::wrap::wrap_x(wx, self.world_width);
        let fy = wy.clamp(0.0, self.world_height - 1.0);
        sample_bilinear_u32(&self.drainage_area, self.width, self.height, fx, fy)
    }

    pub fn sample_sediment_at(&self, wx: f64, wy: f64) -> f64 {
        if self.sediment.is_empty() { return 0.0; }
        let fx = crate::wrap::wrap_x(wx, self.world_width);
        let fy = wy.clamp(0.0, self.world_height - 1.0);
        sample_bilinear(&self.sediment, self.width, self.height, fx, fy)
    }

    /// Generate a biome map with all terrain layers using the specified backend.
    pub fn generate_with_backend(
        seed: u32,
        width: usize,
        height: usize,
        _backend: NoiseBackend,
    ) -> Self {
        // Force CPU backend: GPU shaders use 2D noise which doesn't wrap horizontally.
        // TODO: implement 3D OpenSimplex in WGSL, then restore GPU path.
        Self::generate(seed, width, height)
    }

    /// Generate a biome map with all terrain layers using parallel processing.
    pub fn generate(seed: u32, width: usize, height: usize) -> Self {
        Self::generate_with_sub_stellar(seed, width, height, 0.5, 1.0)
    }

    /// Generate a biome map with a custom sub-stellar point.
    pub fn generate_with_sub_stellar(
        seed: u32,
        width: usize,
        height: usize,
        sub_stellar_x: f64,
        sub_stellar_y: f64,
    ) -> Self {
        // Create all strategies
        let cont_strategy = ContinentalnessStrategy::new_wrapping(seed, width as f64);
        let tectonic_strategy = TectonicPlatesStrategy::new_wrapping(seed.wrapping_add(2), width as f64);
        let raw_peaks_strategy = PeaksAndValleysStrategy::new_wrapping(seed.wrapping_add(4), width as f64);
        let humidity_strategy = HumidityStrategy::new_wrapping(seed.wrapping_add(5), width as f64);
        let light_level_strategy = LightLevelStrategy::new(
            seed.wrapping_add(6),
            sub_stellar_x,
            sub_stellar_y,
            width as f64,
            height as f64,
        );
        let rock_hardness_strategy = RockHardnessStrategy::new_wrapping(seed.wrapping_add(7), width as f64);

        let total_pixels = width * height;

        // Generate pixel indices
        let indices: Vec<(usize, usize)> = (0..height)
            .flat_map(|y| (0..width).map(move |x| (x, y)))
            .collect();

        // Phase 1: Generate all independent base layers in parallel
        let base_data: Vec<_> = indices
            .par_iter()
            .map(|&(x, y)| {
                let fx = x as f64;
                let fy = y as f64;

                let cont = cont_strategy.generate(fx, fy, 0);
                let tect_sample = tectonic_strategy.generate_full(fx, fy);
                let raw_peaks = raw_peaks_strategy.ridged_aligned(fx, fy, 0, tect_sample.boundary_tangent, tect_sample.stress);
                let light = light_level_strategy.generate(fx, fy, 0);
                let rock = rock_hardness_strategy.generate(fx, fy, 0);
                let humid = humidity_strategy.generate_terminator_model(fx, fy, 0, cont, light);

                (cont, tect_sample.boundary_distance, tect_sample.plate_id, raw_peaks, light, rock, humid, tect_sample.volcanism)
            })
            .collect();

        // Unpack base data into separate vectors
        let mut continentalness = Vec::with_capacity(total_pixels);
        let mut tectonic = Vec::with_capacity(total_pixels);
        let mut tectonic_plate_ids = Vec::with_capacity(total_pixels);
        let mut humidity = Vec::with_capacity(total_pixels);
        let mut rock_hardness = Vec::with_capacity(total_pixels);
        let mut light_level = Vec::with_capacity(total_pixels);
        let mut raw_peaks_vec = Vec::with_capacity(total_pixels);
        let mut volcanism = Vec::with_capacity(total_pixels);

        for &(cont, tect, pid, raw_peaks, light, rock, humid, volc) in &base_data {
            continentalness.push(cont);
            tectonic.push(tect);
            tectonic_plate_ids.push(pid);
            humidity.push(humid);
            rock_hardness.push(rock);
            light_level.push(light);
            raw_peaks_vec.push(raw_peaks);
            volcanism.push(volc);
        }

        // Phase 1.5a: Compute quick heightmap for erosion sim
        let quick_heightmap: Vec<f64> = (0..total_pixels).map(|i| {
            let peaks = derived::derive_peaks_valleys(raw_peaks_vec[i], tectonic[i], rock_hardness[i]);
            derived::derive_heightmap(continentalness[i], tectonic[i], peaks)
        }).collect();

        // Phase 1.5b: Run erosion simulation on quick heightmap
        let tectonic_stress_for_erosion: Vec<f64> = tectonic.iter().map(|&t| 1.0 - t).collect();
        let erosion_result = simulate_erosion(
            &quick_heightmap, &rock_hardness, &tectonic_stress_for_erosion,
            &continentalness, width, height, &ErosionParams::default(),
        );
        let eroded_heightmap = erosion_result.heightmap;

        // Phase 1.5c: Wind-driven moisture advection using eroded heightmap
        let wind = WindField::generate(
            &light_level, &eroded_heightmap, width, height,
            (sub_stellar_x, sub_stellar_y),
        );
        advect_moisture(
            &mut humidity, &wind, &eroded_heightmap, &continentalness,
            &light_level, width, height, SEA_LEVEL, 4,
        );

        // Phase 2: Derive all per-pixel layers (using eroded heightmap and wind-modified humidity)
        let splines = BiomeSplines::new(SEA_LEVEL);

        let mut peaks_valleys = Vec::with_capacity(total_pixels);
        let mut heightmap_vec = Vec::with_capacity(total_pixels);
        let mut temperature = Vec::with_capacity(total_pixels);
        let mut erosion = Vec::with_capacity(total_pixels);
        let mut aridity = Vec::with_capacity(total_pixels);
        let mut precipitation_type = Vec::with_capacity(total_pixels);
        let mut resource_richness = Vec::with_capacity(total_pixels);
        let mut snowpack = Vec::with_capacity(total_pixels);
        let mut biomes = Vec::with_capacity(total_pixels);

        for idx in 0..total_pixels {
            let cont = continentalness[idx];
            let tect = tectonic[idx];
            let light = light_level[idx];
            let rock = rock_hardness[idx];
            let humid = humidity[idx];
            let px = idx % width;
            let py = idx / width;

            let peaks = derived::derive_peaks_valleys(raw_peaks_vec[idx], tect, rock);
            let hm = eroded_heightmap[idx]; // Use eroded heightmap directly
            let temp = derived::derive_temperature(light, hm, humid, cont);
            let eros = derived::derive_erosion(hm, rock, humid);
            let arid = derived::derive_aridity(temp, humid);
            let precip = derived::derive_precipitation_type(temp, humid, hm);
            let res = derived::derive_resource_richness(tect, rock, eros);
            let snow = derived::derive_snowpack(precip, temp, hm, light);
            let biome = splines.evaluate_dithered_with_light(cont, temp, tect, eros, peaks, humid, arid, rock, px, py, light);

            peaks_valleys.push(peaks);
            heightmap_vec.push(hm);
            temperature.push(temp);
            erosion.push(eros);
            aridity.push(arid);
            precipitation_type.push(precip);
            resource_richness.push(res);
            snowpack.push(snow);
            biomes.push(biome);
        }

        // Phase 3: Generate rivers
        // Use geology-aware RiverNetwork for large maps, fast RiverGenerator for small ones
        let (rivers, river_network) = if total_pixels >= 256 * 128 {
            let tectonic_stress: Vec<f64> = tectonic.iter().map(|&t| 1.0 - t).collect();
            let river_network = RiverNetwork::generate(
                &heightmap_vec, &rock_hardness, &tectonic_stress, &continentalness,
                &light_level, &humidity, &temperature, &peaks_valleys,
                width, height, SEA_LEVEL,
            );
            let grid = river_network.to_flow_grid(width, height);
            (grid, Some(Arc::new(river_network)))
        } else {
            let river_gen = RiverGenerator::for_map_size(SEA_LEVEL, width, height);
            (river_gen.generate_climate_aware(&heightmap_vec, &light_level, &humidity, width, height), None)
        };

        // Erosion feedback: carve terrain along rivers and eroded areas
        for idx in 0..total_pixels {
            if continentalness[idx] < SEA_LEVEL { continue; }
            heightmap_vec[idx] -= erosion[idx] * 0.03;
            if rivers[idx] > 0.0 {
                heightmap_vec[idx] -= rivers[idx] * 0.02;
            }
        }
        // Recompute temperature with modified heightmap
        for idx in 0..total_pixels {
            temperature[idx] = derived::derive_temperature(
                light_level[idx], heightmap_vec[idx], humidity[idx], continentalness[idx],
            );
        }

        // Override biomes where rivers flow - only in habitable climate zones
        for idx in 0..total_pixels {
            if rivers[idx] > 0.05
                && continentalness[idx] >= SEA_LEVEL
                && temperature[idx] > -10.0
                && temperature[idx] < 45.0
            {
                biomes[idx] = TileType::River;
            }
        }

        // Post-river derivation: TWI-based water_table, vegetation, soil
        let slope_grid = compute_slope_grid(&heightmap_vec, width, height);
        let water_table: Vec<f64> = (0..total_pixels).map(|idx| {
            let twi = derived::derive_twi(erosion_result.drainage_area[idx] as f64, slope_grid[idx]);
            // Blend TWI with river flow for riparian boost
            let river_boost = (rivers[idx] * 4.0).min(1.0) * 0.45;
            (twi + river_boost).clamp(0.0, 1.0)
        }).collect();

        // Oasis override: desert biomes near rivers become oases (rare, small)
        for idx in 0..total_pixels {
            if water_table[idx] > 0.45 && continentalness[idx] >= SEA_LEVEL && temperature[idx] < 80.0 {
                match biomes[idx] {
                    TileType::Desert | TileType::Sahara | TileType::Erg | TileType::Hamada => {
                        biomes[idx] = TileType::Oasis;
                    }
                    TileType::Steppe | TileType::Scrubland => {
                        biomes[idx] = TileType::Meadow;
                    }
                    _ => {}
                }
            }
        }

        // Polar ice cap override: jagged ice edge using noise perturbation
        apply_polar_ice_cap(&mut biomes, &light_level, &continentalness, &peaks_valleys, &rock_hardness, &temperature, SEA_LEVEL);

        let vegetation_density: Vec<f64> = biomes.iter().zip(water_table.iter())
            .map(|(&b, &wt)| derived::derive_vegetation_density(b, wt)).collect();
        let soil_type: Vec<f64> = biomes.iter().zip(erosion.iter()).zip(rock_hardness.iter())
            .zip(erosion_result.sediment.iter())
            .map(|(((&b, &e), &r), &sed)| {
                // Task 6: Sediment tracking — high sediment = alluvial = fertile
                let base_soil = derived::derive_soil_type(b, e, r);
                (base_soil + sed.min(1.0) * 0.2).clamp(0.0, 1.0)
            }).collect();

        // Phase 4: Generate per-type resource distribution
        let resource_map = Self::generate_resource_map(
            seed, width, height, &continentalness, &tectonic, &biomes,
        );

        Self {
            width,
            height,
            continentalness,
            tectonic,
            tectonic_plate_ids,
            humidity,
            rock_hardness,
            light_level,
            peaks_valleys,
            volcanism,
            heightmap: heightmap_vec,
            temperature,
            erosion,
            rivers,
            aridity,
            precipitation_type,
            water_table,
            wind_speed: wind.speed,
            resource_richness,
            snowpack,
            biomes,
            vegetation_density,
            soil_type,
            resource_map: Some(resource_map),
            river_network,
            drainage_area: erosion_result.drainage_area,
            sediment: erosion_result.sediment,
            world_width: width as f64,
            world_height: height as f64,
        }
    }

    /// Generate a biome map using GPU-accelerated noise generation.
    /// Falls back to CPU if GPU is unavailable.
    #[cfg(feature = "gpu")]
    fn generate_gpu(seed: u32, width: usize, height: usize) -> Self {
        use crate::gpu::GpuNoiseContext;

        let Some(gpu) = GpuNoiseContext::global() else {
            return Self::generate(seed, width, height);
        };

        let total_pixels = width * height;

        // Generate base noise layers on GPU
        let layers = gpu.generate_layers(
            seed,
            width,
            height,
            0.0,
            0.0,
            1.0,
            height as f64,
            0,
        );

        // Convert f32 GPU results to f64
        let continentalness: Vec<f64> = layers.continentalness.iter().map(|&v| v as f64).collect();
        let gpu_light_level: Vec<f64> = layers.light_level.iter().map(|&v| v as f64).collect();
        let gpu_rock_hardness: Vec<f64> = layers.rock_hardness.iter().map(|&v| v as f64).collect();

        // Tectonic, peaks, and humidity computed on CPU for boundary alignment and terminator model
        let tectonic_strategy = TectonicPlatesStrategy::new_wrapping(seed.wrapping_add(2), width as f64);
        let raw_peaks_strategy = PeaksAndValleysStrategy::new_wrapping(seed.wrapping_add(4), width as f64);
        let humidity_strategy = HumidityStrategy::new_wrapping(seed.wrapping_add(5), width as f64);
        let cpu_data: Vec<_> = (0..total_pixels)
            .into_par_iter()
            .map(|idx| {
                let x = (idx % width) as f64;
                let y = (idx / width) as f64;
                let tect_sample = tectonic_strategy.generate_full(x, y);
                let raw_peaks = raw_peaks_strategy.ridged_aligned(x, y, 0, tect_sample.boundary_tangent, tect_sample.stress);
                let cont = continentalness[idx];
                let light = gpu_light_level[idx];
                let humid = humidity_strategy.generate_terminator_model(x, y, 0, cont, light);
                (tect_sample, raw_peaks, humid)
            })
            .collect();

        let gpu_tectonic: Vec<f64> = cpu_data.iter().map(|(s, _, _)| s.boundary_distance).collect();
        let tectonic_plate_ids: Vec<f64> = cpu_data.iter().map(|(s, _, _)| s.plate_id).collect();
        let tectonic_volcanism: Vec<f64> = cpu_data.iter().map(|(s, _, _)| s.volcanism).collect();
        let raw_peaks: Vec<f64> = cpu_data.iter().map(|(_, p, _)| *p).collect();
        let mut gpu_humidity: Vec<f64> = cpu_data.iter().map(|(_, _, h)| *h).collect();

        // Erosion simulation + wind-driven moisture advection
        let quick_heightmap: Vec<f64> = (0..total_pixels).map(|i| {
            let peaks = derived::derive_peaks_valleys(raw_peaks[i], gpu_tectonic[i], gpu_rock_hardness[i]);
            derived::derive_heightmap(continentalness[i], gpu_tectonic[i], peaks)
        }).collect();

        let gpu_tectonic_stress: Vec<f64> = gpu_tectonic.iter().map(|&t| 1.0 - t).collect();
        let gpu_erosion_result = simulate_erosion(
            &quick_heightmap, &gpu_rock_hardness, &gpu_tectonic_stress,
            &continentalness, width, height, &ErosionParams::default(),
        );
        let gpu_eroded_heightmap = gpu_erosion_result.heightmap;

        let wind = WindField::generate(&gpu_light_level, &gpu_eroded_heightmap, width, height, (0.5, 1.0));
        advect_moisture(&mut gpu_humidity, &wind, &gpu_eroded_heightmap, &continentalness, &gpu_light_level, width, height, SEA_LEVEL, 4);

        // Derive all layers on CPU in parallel
        let splines = BiomeSplines::new(SEA_LEVEL);
        let derived_data: Vec<_> = (0..total_pixels)
            .into_par_iter()
            .map(|idx| {
                let cont = continentalness[idx];
                let tect = gpu_tectonic[idx];
                let light = gpu_light_level[idx];
                let rock = gpu_rock_hardness[idx];
                let humid = gpu_humidity[idx];

                let px = idx % width;
                let py = idx / width;
                let peaks = derived::derive_peaks_valleys(raw_peaks[idx], tect, rock);
                let volc = tectonic_volcanism[idx];
                let hm = gpu_eroded_heightmap[idx]; // Use eroded heightmap
                let temp = derived::derive_temperature(light, hm, humid, cont);
                let eros = derived::derive_erosion(hm, rock, humid);
                let arid = derived::derive_aridity(temp, humid);
                let precip = derived::derive_precipitation_type(temp, humid, hm);
                let res = derived::derive_resource_richness(tect, rock, eros);
                let snow = derived::derive_snowpack(precip, temp, hm, light);
                let biome = splines.evaluate_dithered_with_light(cont, temp, tect, eros, peaks, humid, arid, rock, px, py, light);

                (peaks, volc, hm, temp, eros, arid, precip, res, snow, biome)
            })
            .collect();

        let mut peaks_valleys = Vec::with_capacity(total_pixels);
        let mut volcanism = Vec::with_capacity(total_pixels);
        let mut heightmap_vec = Vec::with_capacity(total_pixels);
        let mut temperature = Vec::with_capacity(total_pixels);
        let mut erosion = Vec::with_capacity(total_pixels);
        let mut aridity = Vec::with_capacity(total_pixels);
        let mut precipitation_type = Vec::with_capacity(total_pixels);
        let mut resource_richness = Vec::with_capacity(total_pixels);
        let mut snowpack = Vec::with_capacity(total_pixels);
        let mut biomes = Vec::with_capacity(total_pixels);

        for (peaks, volc, hm, temp, eros, arid, precip, res, snow, biome) in derived_data {
            peaks_valleys.push(peaks);
            volcanism.push(volc);
            heightmap_vec.push(hm);
            temperature.push(temp);
            erosion.push(eros);
            aridity.push(arid);
            precipitation_type.push(precip);
            resource_richness.push(res);
            snowpack.push(snow);
            biomes.push(biome);
        }

        // Rivers: geology-aware RiverNetwork for large maps, fast fallback for small
        let (rivers, river_network) = if total_pixels >= 256 * 128 {
            let tectonic_stress: Vec<f64> = gpu_tectonic.iter().map(|&t| 1.0 - t).collect();
            let river_network = RiverNetwork::generate(
                &heightmap_vec, &gpu_rock_hardness, &tectonic_stress, &continentalness,
                &gpu_light_level, &gpu_humidity, &temperature, &peaks_valleys,
                width, height, SEA_LEVEL,
            );
            let grid = river_network.to_flow_grid(width, height);
            (grid, Some(Arc::new(river_network)))
        } else {
            let river_gen = RiverGenerator::for_map_size(SEA_LEVEL, width, height);
            (river_gen.generate_climate_aware(&heightmap_vec, &gpu_light_level, &gpu_humidity, width, height), None)
        };

        // Erosion feedback: carve terrain along rivers and eroded areas
        for idx in 0..total_pixels {
            if continentalness[idx] < SEA_LEVEL { continue; }
            heightmap_vec[idx] -= erosion[idx] * 0.03;
            if rivers[idx] > 0.0 {
                heightmap_vec[idx] -= rivers[idx] * 0.02;
            }
        }
        for idx in 0..total_pixels {
            temperature[idx] = derived::derive_temperature(
                gpu_light_level[idx], heightmap_vec[idx], gpu_humidity[idx], continentalness[idx],
            );
        }

        for idx in 0..total_pixels {
            if rivers[idx] > 0.05
                && continentalness[idx] >= SEA_LEVEL
                && temperature[idx] > -10.0
                && temperature[idx] < 45.0
            {
                biomes[idx] = TileType::River;
            }
        }

        // Polar ice cap override: jagged ice edge using noise perturbation
        apply_polar_ice_cap(&mut biomes, &gpu_light_level, &continentalness, &peaks_valleys, &gpu_rock_hardness, &temperature, SEA_LEVEL);

        let gpu_slope_grid = compute_slope_grid(&heightmap_vec, width, height);
        let water_table: Vec<f64> = (0..total_pixels).map(|idx| {
            let twi = derived::derive_twi(gpu_erosion_result.drainage_area[idx] as f64, gpu_slope_grid[idx]);
            let river_boost = (rivers[idx] * 4.0).min(1.0) * 0.45;
            (twi + river_boost).clamp(0.0, 1.0)
        }).collect();
        let vegetation_density: Vec<f64> = biomes.iter().zip(water_table.iter())
            .map(|(&b, &wt)| derived::derive_vegetation_density(b, wt)).collect();
        let soil_type: Vec<f64> = biomes.iter().zip(erosion.iter()).zip(gpu_rock_hardness.iter())
            .zip(gpu_erosion_result.sediment.iter())
            .map(|(((&b, &e), &r), &sed)| {
                let base_soil = derived::derive_soil_type(b, e, r);
                (base_soil + sed.min(1.0) * 0.2).clamp(0.0, 1.0)
            }).collect();

        Self {
            width,
            height,
            continentalness,
            tectonic: gpu_tectonic,
            tectonic_plate_ids,
            humidity: gpu_humidity,
            rock_hardness: gpu_rock_hardness,
            light_level: gpu_light_level,
            peaks_valleys,
            volcanism,
            heightmap: heightmap_vec,
            temperature,
            erosion,
            rivers,
            aridity,
            precipitation_type,
            water_table,
            wind_speed: wind.speed,
            resource_richness,
            snowpack,
            biomes,
            vegetation_density,
            soil_type,
            resource_map: None,
            river_network,
            drainage_area: gpu_erosion_result.drainage_area,
            sediment: gpu_erosion_result.sediment,
            world_width: width as f64,
            world_height: height as f64,
        }
    }

    /// GPU generation stub when gpu feature is disabled.
    #[cfg(not(feature = "gpu"))]
    fn generate_gpu(seed: u32, width: usize, height: usize) -> Self {
        Self::generate(seed, width, height)
    }

    /// Drop all float layers to free memory. Only `biomes` and dimensions are retained.
    /// After calling this, `to_layer_image()` will only work for the Biome layer.
    pub fn shrink(&mut self) {
        self.continentalness = Vec::new();
        self.tectonic = Vec::new();
        self.tectonic_plate_ids = Vec::new();
        self.humidity = Vec::new();
        self.rock_hardness = Vec::new();
        self.light_level = Vec::new();
        self.peaks_valleys = Vec::new();
        self.volcanism = Vec::new();
        self.heightmap = Vec::new();
        self.temperature = Vec::new();
        self.erosion = Vec::new();
        self.rivers = Vec::new();
        self.aridity = Vec::new();
        self.precipitation_type = Vec::new();
        self.water_table = Vec::new();
        self.wind_speed = Vec::new();
        self.resource_richness = Vec::new();
        self.snowpack = Vec::new();
        self.vegetation_density = Vec::new();
        self.soil_type = Vec::new();
        self.resource_map = None;
        self.river_network = None;
        self.drainage_area = Vec::new();
        self.sediment = Vec::new();
    }

    /// Returns true if this BiomeMap has been shrunk (float layers dropped).
    pub fn is_shrunk(&self) -> bool {
        self.continentalness.is_empty()
    }

    /// Convert any layer to RGBA image bytes with optional global normalization hints.
    pub fn to_layer_image_with_hints(
        &self,
        layer: NoiseLayer,
        hints: Option<&crate::terrain_render::NormalizationHints>,
    ) -> Vec<u8> {
        // Composited terrain render for Biome layer when full data is available
        if layer == NoiseLayer::Biome && !self.is_shrunk() {
            return crate::terrain_render::render_terrain(self, hints);
        }
        self.to_layer_image_inner(layer)
    }

    /// Convert any layer to RGBA image bytes.
    pub fn to_layer_image(&self, layer: NoiseLayer) -> Vec<u8> {
        // Composited terrain render for Biome layer when full data is available
        if layer == NoiseLayer::Biome && !self.is_shrunk() {
            return crate::terrain_render::render_terrain(self, None);
        }
        self.to_layer_image_inner(layer)
    }

    fn to_layer_image_inner(&self, layer: NoiseLayer) -> Vec<u8> {

        let mut data = Vec::with_capacity(self.width * self.height * 4);

        for y in 0..self.height {
            for x in 0..self.width {
                let idx = y * self.width + x;
                let color = match layer {
                    NoiseLayer::Biome => self.biomes[idx].color(),
                    NoiseLayer::Continentalness => {
                        grayscale_to_rgba(self.continentalness[idx], -1.0, 1.0)
                    }
                    NoiseLayer::Tectonic => tectonic_to_rgba(self.tectonic_plate_ids[idx], self.tectonic[idx]),
                    NoiseLayer::Humidity => humidity_to_rgba(self.humidity[idx]),
                    NoiseLayer::RockHardness => rock_hardness_to_rgba(self.rock_hardness[idx]),
                    NoiseLayer::LightLevel => light_level_to_rgba(self.light_level[idx]),
                    NoiseLayer::PeaksValleys => peaks_to_rgba(self.peaks_valleys[idx]),
                    NoiseLayer::Volcanism => volcanism_to_rgba(self.volcanism[idx]),
                    NoiseLayer::Heightmap => heightmap_to_rgba(self.heightmap[idx]),
                    NoiseLayer::Temperature => temperature_to_rgba(self.temperature[idx]),
                    NoiseLayer::Erosion => grayscale_to_rgba(self.erosion[idx], 0.0, 1.0),
                    NoiseLayer::RiverFlow => river_to_rgba(self.rivers[idx]),
                    NoiseLayer::Aridity => aridity_to_rgba(self.aridity[idx]),
                    NoiseLayer::PrecipitationType => precipitation_type_to_rgba(self.precipitation_type[idx]),
                    NoiseLayer::WaterTable => water_table_to_rgba(self.water_table[idx]),
                    NoiseLayer::Wind => if self.wind_speed.is_empty() {
                        [0, 0, 0, 255]
                    } else {
                        wind_speed_to_rgba(self.wind_speed[idx])
                    },
                    NoiseLayer::Resources => resources_to_rgba(self.resource_richness[idx]),
                    NoiseLayer::Snowpack => snowpack_to_rgba(self.snowpack[idx]),
                    NoiseLayer::VegetationDensity => vegetation_to_rgba(self.vegetation_density[idx]),
                    NoiseLayer::SoilType => soil_type_to_rgba(self.soil_type[idx]),
                };
                data.extend_from_slice(&color);
            }
        }

        data
    }

    /// Convert biome data to RGBA image bytes.
    pub fn to_biome_image(&self) -> Vec<u8> {
        self.to_layer_image(NoiseLayer::Biome)
    }

    /// Convert temperature data to RGBA image bytes (blue-to-red gradient).
    pub fn to_temperature_image(&self) -> Vec<u8> {
        self.to_layer_image(NoiseLayer::Temperature)
    }

    /// Convert continentalness data to RGBA image bytes (grayscale).
    pub fn to_continentalness_image(&self) -> Vec<u8> {
        self.to_layer_image(NoiseLayer::Continentalness)
    }

    /// Save all noise layers as PNG files to `debug_layers/` directory.
    pub fn save_debug_layers(&self, base_path: &std::path::Path) {
        use image::{ImageBuffer, Rgba};

        let base_dir = base_path.join("base");
        let derived_dir = base_path.join("derived");

        for dir in [base_path, &base_dir, &derived_dir] {
            if let Err(e) = std::fs::create_dir_all(dir) {
                eprintln!("Failed to create dir {}: {e}", dir.display());
                return;
            }
        }

        let save = |layer: NoiseLayer, path: &std::path::Path| {
            let rgba_data = self.to_layer_image(layer);
            let img: ImageBuffer<Rgba<u8>, _> = ImageBuffer::from_raw(
                self.width as u32,
                self.height as u32,
                rgba_data,
            )
            .expect("image buffer size mismatch");

            if let Err(e) = img.save(path) {
                eprintln!("Failed to save {}: {e}", path.display());
            } else {
                println!("  Saved debug layer: {}", path.display());
            }
        };

        // Biome at root
        save(NoiseLayer::Biome, &base_path.join("biome.png"));

        // Base layers
        let base_layers = [
            (NoiseLayer::Continentalness, "continentalness"),
            (NoiseLayer::Tectonic, "tectonic"),
            (NoiseLayer::Humidity, "humidity"),
            (NoiseLayer::RockHardness, "rock_hardness"),
            (NoiseLayer::LightLevel, "light_level"),
        ];
        for (layer, name) in &base_layers {
            save(*layer, &base_dir.join(format!("{name}.png")));
        }

        // Derived layers
        let derived_layers = [
            (NoiseLayer::PeaksValleys, "peaks_valleys"),
            (NoiseLayer::Volcanism, "volcanism"),
            (NoiseLayer::Heightmap, "heightmap"),
            (NoiseLayer::Temperature, "temperature"),
            (NoiseLayer::Erosion, "erosion"),
            (NoiseLayer::RiverFlow, "river_flow"),
            (NoiseLayer::Aridity, "aridity"),
            (NoiseLayer::PrecipitationType, "precipitation_type"),
            (NoiseLayer::WaterTable, "water_table"),
            (NoiseLayer::Wind, "wind_speed"),
            (NoiseLayer::Resources, "resources"),
            (NoiseLayer::Snowpack, "snowpack"),
            (NoiseLayer::VegetationDensity, "vegetation_density"),
            (NoiseLayer::SoilType, "soil_type"),
        ];
        for (layer, name) in &derived_layers {
            save(*layer, &derived_dir.join(format!("{name}.png")));
        }
    }

    /// Get biome at specific coordinates.
    pub fn get_biome(&self, x: usize, y: usize) -> Option<TileType> {
        if x < self.width && y < self.height {
            Some(self.biomes[y * self.width + x])
        } else {
            None
        }
    }

    /// Get temperature at specific coordinates.
    pub fn get_temperature(&self, x: usize, y: usize) -> Option<f64> {
        if x < self.width && y < self.height {
            Some(self.temperature[y * self.width + x])
        } else {
            None
        }
    }

    /// Get continentalness at specific coordinates.
    pub fn get_continentalness(&self, x: usize, y: usize) -> Option<f64> {
        if x < self.width && y < self.height {
            Some(self.continentalness[y * self.width + x])
        } else {
            None
        }
    }

    /// Get tectonic boundary distance at specific coordinates.
    pub fn get_tectonic(&self, x: usize, y: usize) -> Option<f64> {
        if x < self.width && y < self.height {
            Some(self.tectonic[y * self.width + x])
        } else {
            None
        }
    }

    /// Get erosion at specific coordinates.
    pub fn get_erosion(&self, x: usize, y: usize) -> Option<f64> {
        if x < self.width && y < self.height {
            Some(self.erosion[y * self.width + x])
        } else {
            None
        }
    }

    /// Get peaks/valleys value at specific coordinates.
    pub fn get_peaks_valleys(&self, x: usize, y: usize) -> Option<f64> {
        if x < self.width && y < self.height {
            Some(self.peaks_valleys[y * self.width + x])
        } else {
            None
        }
    }

    /// Get humidity at specific coordinates.
    pub fn get_humidity(&self, x: usize, y: usize) -> Option<f64> {
        if x < self.width && y < self.height {
            Some(self.humidity[y * self.width + x])
        } else {
            None
        }
    }

    /// Get river flow at specific coordinates.
    pub fn get_river(&self, x: usize, y: usize) -> Option<f64> {
        if x < self.width && y < self.height {
            Some(self.rivers[y * self.width + x])
        } else {
            None
        }
    }

    /// Generate per-type resource map using geology-aware strategies.
    fn generate_resource_map(
        seed: u32,
        width: usize,
        height: usize,
        continentalness: &[f64],
        tectonic: &[f64],
        biomes: &[TileType],
    ) -> ResourceMap {
        use crate::strategy::{ResourceNoiseStrategy, ResourceContext};
        use rb_core::ResourceType;

        let mut resource_map = ResourceMap::new(width, height);

        for &resource_type in ResourceType::all() {
            let strategy = ResourceNoiseStrategy::new(seed, resource_type);

            for y in 0..height {
                for x in 0..width {
                    let idx = y * width + x;
                    let cont = continentalness[idx];
                    let tect = tectonic[idx];
                    // Approximate water distance from continentalness
                    let water_dist = ((cont + 0.025).max(0.0) * 10.0).min(1.0);

                    let context = ResourceContext {
                        continentalness: cont,
                        tectonic_boundary_distance: tect,
                        water_distance: water_dist,
                        biome: biomes[idx],
                    };

                    let abundance = strategy.generate_with_context(x as f64, y as f64, 0, &context);
                    resource_map.set(x, y, resource_type, abundance as f32);
                }
            }
        }

        resource_map
    }

    /// Generate a meso-level (zoomed in) biome map for a specific world region.
    pub fn generate_region(
        seed: u32,
        world_x: f64,
        world_y: f64,
        world_size: f64,
        output_size: usize,
        world_height: f64,
        detail_level: u32,
    ) -> Self {
        Self::generate_region_with_sub_stellar(
            seed, world_x, world_y, world_size, output_size, world_height, detail_level, 0.5, 1.0,
        )
    }

    /// Generate a meso-level biome map with custom sub-stellar point.
    pub fn generate_region_with_sub_stellar(
        seed: u32,
        world_x: f64,
        world_y: f64,
        world_size: f64,
        output_size: usize,
        world_height: f64,
        detail_level: u32,
        sub_stellar_x: f64,
        sub_stellar_y: f64,
    ) -> Self {
        let world_width = world_height * 2.0;
        let cont_strategy = ContinentalnessStrategy::new_wrapping(seed, world_width);
        let tectonic_strategy = TectonicPlatesStrategy::new_wrapping(seed.wrapping_add(2), world_width);
        let raw_peaks_strategy = PeaksAndValleysStrategy::new_wrapping(seed.wrapping_add(4), world_width);
        let humidity_strategy = HumidityStrategy::new_wrapping(seed.wrapping_add(5), world_width);
        let light_level_strategy = LightLevelStrategy::new(
            seed.wrapping_add(6), sub_stellar_x, sub_stellar_y, world_width, world_height,
        );
        let rock_hardness_strategy = RockHardnessStrategy::new_wrapping(seed.wrapping_add(7), world_width);
        let splines = BiomeSplines::new(SEA_LEVEL);
        let detail_noise = OpenSimplex::new(seed.wrapping_add(50));

        let total_pixels = output_size * output_size;
        let scale = world_size / output_size as f64;

        let mut continentalness = Vec::with_capacity(total_pixels);
        let mut tectonic = Vec::with_capacity(total_pixels);
        let mut tectonic_plate_ids = Vec::with_capacity(total_pixels);
        let mut humidity = Vec::with_capacity(total_pixels);
        let mut rock_hardness = Vec::with_capacity(total_pixels);
        let mut light_level = Vec::with_capacity(total_pixels);
        let mut peaks_valleys = Vec::with_capacity(total_pixels);
        let mut volcanism = Vec::with_capacity(total_pixels);
        let mut heightmap_vec = Vec::with_capacity(total_pixels);
        let mut temperature = Vec::with_capacity(total_pixels);
        let mut erosion = Vec::with_capacity(total_pixels);
        let mut aridity = Vec::with_capacity(total_pixels);
        let mut precipitation_type = Vec::with_capacity(total_pixels);
        let mut resource_richness = Vec::with_capacity(total_pixels);
        let mut snowpack = Vec::with_capacity(total_pixels);
        let mut biomes = Vec::with_capacity(total_pixels);

        for py in 0..output_size {
            for px in 0..output_size {
                let wx = world_x + (px as f64 * scale);
                let wy = world_y + (py as f64 * scale);

                let cont = cont_strategy.generate(wx, wy, detail_level);
                let tect_sample = tectonic_strategy.generate_full(wx, wy);
                let tect = tect_sample.boundary_distance;
                let pid = tect_sample.plate_id;
                let raw_peaks = raw_peaks_strategy.ridged_aligned(wx, wy, detail_level, tect_sample.boundary_tangent, tect_sample.stress);
                let light = light_level_strategy.generate(wx, wy, detail_level);
                let rock = rock_hardness_strategy.generate(wx, wy, detail_level);
                let humid = humidity_strategy.generate_terminator_model(wx, wy, detail_level, cont, light);

                let peaks = derived::derive_peaks_valleys(raw_peaks, tect, rock);
                let volc = tect_sample.volcanism;
                let hm = derived::derive_heightmap(cont, tect, peaks);
                let hm = if detail_level >= 3 {
                    derived::derive_micro_heightmap(hm, wx, wy, &detail_noise)
                } else {
                    hm
                };
                let temp = derived::derive_temperature(light, hm, humid, cont);
                let eros = derived::derive_erosion(hm, rock, humid);
                let arid = derived::derive_aridity(temp, humid);
                let precip = derived::derive_precipitation_type(temp, humid, hm);
                let res = derived::derive_resource_richness(tect, rock, eros);
                let snow = derived::derive_snowpack(precip, temp, hm, light);
                let biome = splines.evaluate_dithered_with_light(cont, temp, tect, eros, peaks, humid, arid, rock, px, py, light);

                continentalness.push(cont);
                tectonic.push(tect);
                tectonic_plate_ids.push(pid);
                humidity.push(humid);
                rock_hardness.push(rock);
                light_level.push(light);
                peaks_valleys.push(peaks);
                volcanism.push(volc);
                heightmap_vec.push(hm);
                temperature.push(temp);
                erosion.push(eros);
                aridity.push(arid);
                precipitation_type.push(precip);
                resource_richness.push(res);
                snowpack.push(snow);
                biomes.push(biome);
            }
        }

        // Rivers
        let river_gen = RiverGenerator::for_map_size_with_detail(SEA_LEVEL, output_size, output_size, detail_level);
        let rivers = river_gen.generate(&heightmap_vec, output_size, output_size);

        for idx in 0..total_pixels {
            if rivers[idx] > 0.05
                && continentalness[idx] >= SEA_LEVEL
                && temperature[idx] > -10.0
                && temperature[idx] < 45.0
            {
                biomes[idx] = TileType::River;
            }
        }

        let water_table: Vec<f64> = (0..total_pixels).map(|idx| {
            derived::derive_water_table(rivers[idx], humidity[idx], heightmap_vec[idx], precipitation_type[idx], continentalness[idx])
        }).collect();

        // Oasis override: desert biomes near rivers become oases (rare, small)
        for idx in 0..total_pixels {
            if water_table[idx] > 0.45 && continentalness[idx] >= SEA_LEVEL && temperature[idx] < 80.0 {
                match biomes[idx] {
                    TileType::Desert | TileType::Sahara | TileType::Erg | TileType::Hamada => {
                        biomes[idx] = TileType::Oasis;
                    }
                    TileType::Steppe | TileType::Scrubland => {
                        biomes[idx] = TileType::Meadow;
                    }
                    _ => {}
                }
            }
        }

        // Polar ice cap override: jagged ice edge using noise perturbation
        apply_polar_ice_cap(&mut biomes, &light_level, &continentalness, &peaks_valleys, &rock_hardness, &temperature, SEA_LEVEL);

        let vegetation_density: Vec<f64> = biomes.iter().zip(water_table.iter())
            .map(|(&b, &wt)| derived::derive_vegetation_density(b, wt)).collect();
        let soil_type: Vec<f64> = biomes.iter().zip(erosion.iter()).zip(rock_hardness.iter())
            .map(|((&b, &e), &r)| derived::derive_soil_type(b, e, r)).collect();


        // Compute wind field
        let sub_x_pixel = (sub_stellar_x * world_width - world_x) / scale;
        let sub_y_pixel = (sub_stellar_y * world_height - world_y) / scale;
        let sub_stellar_norm = (sub_x_pixel / output_size as f64, sub_y_pixel / output_size as f64);
        let region_wind = WindField::generate(
            &light_level, &heightmap_vec, output_size, output_size,
            sub_stellar_norm,
        );

        Self {
            width: output_size,
            height: output_size,
            continentalness,
            tectonic,
            tectonic_plate_ids,
            humidity,
            rock_hardness,
            light_level,
            peaks_valleys,
            volcanism,
            heightmap: heightmap_vec,
            temperature,
            erosion,
            rivers,
            aridity,
            precipitation_type,
            water_table,
            wind_speed: region_wind.speed,
            resource_richness,
            snowpack,
            biomes,
            vegetation_density,
            soil_type,
            resource_map: None,
            river_network: None,
            drainage_area: Vec::new(),
            sediment: Vec::new(),
            world_width: world_size,
            world_height: world_size,
        }
    }

    /// Fast biome-only generation for meso tiles.
    /// Only computes continentalness, temperature, and biome - skips all other layers.
    pub fn generate_biome_only(
        seed: u32,
        world_x: f64,
        world_y: f64,
        world_size: f64,
        output_size: usize,
        world_height: f64,
        detail_level: u32,
    ) -> Vec<u8> {
        let world_width = world_height * 2.0;
        let cont_strategy = ContinentalnessStrategy::new_wrapping(seed, world_width);
        let light_strategy = LightLevelStrategy::new(
            seed.wrapping_add(6), 0.5, 1.0, world_width, world_height,
        );

        let total_pixels = output_size * output_size;
        let scale = world_size / output_size as f64;
        let mut image_data = Vec::with_capacity(total_pixels * 4);

        for py in 0..output_size {
            for px in 0..output_size {
                let wx = world_x + (px as f64 * scale);
                let wy = world_y + (py as f64 * scale);

                let cont = cont_strategy.generate(wx, wy, detail_level);
                let light = light_strategy.generate(wx, wy, detail_level);
                let temp = derived::derive_temperature(light, cont.max(0.0), 0.3, cont);
                let biome = TileType::from_climate(cont, temp, SEA_LEVEL);

                image_data.extend_from_slice(&biome.color());
            }
        }

        image_data
    }

    /// Generate full meso BiomeMap with all layers in parallel + progress tracking.
    pub fn generate_meso_full(
        seed: u32,
        world_x: f64,
        world_y: f64,
        world_size: f64,
        output_size: usize,
        world_height: f64,
        detail_level: u32,
        progress: Option<&Arc<LayerProgress>>,
        macro_map: Option<&BiomeMap>,
        river_network: Option<&Arc<RiverNetwork>>,
    ) -> Self {
        let world_width = world_height * 2.0;
        let cont_strategy = ContinentalnessStrategy::new_wrapping(seed, world_width);
        let tectonic_strategy = TectonicPlatesStrategy::new_wrapping(seed.wrapping_add(2), world_width);
        let raw_peaks_strategy = PeaksAndValleysStrategy::new_wrapping(seed.wrapping_add(4), world_width);
        let humidity_strategy = HumidityStrategy::new_wrapping(seed.wrapping_add(5), world_width);
        let light_level_strategy = LightLevelStrategy::new(
            seed.wrapping_add(6), 0.5, 1.0, world_width, world_height,
        );
        let rock_hardness_strategy = RockHardnessStrategy::new_wrapping(seed.wrapping_add(7), world_width);
        let splines = BiomeSplines::new(SEA_LEVEL);
        let detail_noise = OpenSimplex::new(seed.wrapping_add(50));

        let total_pixels = output_size * output_size;
        let scale = world_size / output_size as f64;
        let progress_chunk = (total_pixels / 100).max(256);

        let indices: Vec<usize> = (0..total_pixels).collect();

        // Phase 1+2: Generate base layers and derive dependent layers in parallel
        let all_data: Vec<_> = indices
            .par_chunks(progress_chunk)
            .flat_map_iter(|chunk| {
                let mut results = Vec::with_capacity(chunk.len());

                for &idx in chunk {
                    let py = idx / output_size;
                    let px = idx % output_size;
                    let wx = world_x + (px as f64 * scale);
                    let wy = world_y + (py as f64 * scale);

                    // Phase 1: Base layers
                    let cont = cont_strategy.generate(wx, wy, detail_level);
                    let tect_sample = tectonic_strategy.generate_full(wx, wy);
                    let tect = tect_sample.boundary_distance;
                    let pid = tect_sample.plate_id;
                    let raw_peaks = raw_peaks_strategy.ridged_aligned(wx, wy, detail_level, tect_sample.boundary_tangent, tect_sample.stress);
                    let light = light_level_strategy.generate(wx, wy, detail_level);
                    let rock = rock_hardness_strategy.generate(wx, wy, detail_level);
                    let humid = humidity_strategy.generate_terminator_model(wx, wy, detail_level, cont, light);

                    // Phase 2: Derived layers
                    let peaks = derived::derive_peaks_valleys(raw_peaks, tect, rock);
                    let volc = tect_sample.volcanism;
                    let hm = derived::derive_heightmap(cont, tect, peaks);
                    let hm = if detail_level >= 3 {
                        derived::derive_micro_heightmap(hm, wx, wy, &detail_noise)
                    } else {
                        hm
                    };
                    let temp = derived::derive_temperature(light, hm, humid, cont);
                    let eros = derived::derive_erosion(hm, rock, humid);
                    let arid = derived::derive_aridity(temp, humid);
                    let precip = derived::derive_precipitation_type(temp, humid, hm);
                    let res = derived::derive_resource_richness(tect, rock, eros);
                    let snow = derived::derive_snowpack(precip, temp, hm, light);
                    let biome = splines.evaluate_dithered_with_light(cont, temp, tect, eros, peaks, humid, arid, rock, px, py, light);

                    results.push((cont, temp, tect, pid, peaks, volc, hm, eros, humid, light, rock, arid, precip, res, snow, biome));
                }

                // Update progress
                if let Some(p) = progress {
                    let n = chunk.len();
                    p.increment(LayerId::Continentalness, n);
                    p.increment(LayerId::Tectonic, n);
                    p.increment(LayerId::PeaksValleys, n);
                    p.increment(LayerId::Humidity, n);
                    p.increment(LayerId::LightLevel, n);
                    p.increment(LayerId::RockHardness, n);
                    p.increment(LayerId::Derivation, n);
                }

                results
            })
            .collect();

        // Unpack results
        let mut continentalness = Vec::with_capacity(total_pixels);
        let mut temperature = Vec::with_capacity(total_pixels);
        let mut tectonic = Vec::with_capacity(total_pixels);
        let mut tectonic_plate_ids = Vec::with_capacity(total_pixels);
        let mut peaks_valleys = Vec::with_capacity(total_pixels);
        let mut volcanism = Vec::with_capacity(total_pixels);
        let mut heightmap_vec = Vec::with_capacity(total_pixels);
        let mut erosion = Vec::with_capacity(total_pixels);
        let mut humidity = Vec::with_capacity(total_pixels);
        let mut light_level = Vec::with_capacity(total_pixels);
        let mut rock_hardness = Vec::with_capacity(total_pixels);
        let mut aridity = Vec::with_capacity(total_pixels);
        let mut precipitation_type = Vec::with_capacity(total_pixels);
        let mut resource_richness = Vec::with_capacity(total_pixels);
        let mut snowpack = Vec::with_capacity(total_pixels);
        let mut biomes = Vec::with_capacity(total_pixels);

        for (cont, temp, tect, pid, peaks, volc, hm, eros, humid, light, rock, arid, precip, res, snow, biome) in all_data {
            continentalness.push(cont);
            temperature.push(temp);
            tectonic.push(tect);
            tectonic_plate_ids.push(pid);
            peaks_valleys.push(peaks);
            volcanism.push(volc);
            heightmap_vec.push(hm);
            erosion.push(eros);
            humidity.push(humid);
            light_level.push(light);
            rock_hardness.push(rock);
            aridity.push(arid);
            precipitation_type.push(precip);
            resource_richness.push(res);
            snowpack.push(snow);
            biomes.push(biome);
        }

        // Override heightmap with macro eroded values if available
        if let Some(mm) = macro_map {
            if !mm.heightmap.is_empty() {
                for idx in 0..total_pixels {
                    let py = idx / output_size;
                    let px = idx % output_size;
                    let wx = world_x + (px as f64 * scale);
                    let wy = world_y + (py as f64 * scale);
                    let macro_hm = mm.sample_heightmap_at(wx, wy);
                    // In mountainous areas (high stress), add meso-level ridge/valley detail.
                    // The peaks noise at meso resolution creates finer dendritic texture
                    // than the macro 1024x512 erosion grid can provide.
                    let stress = 1.0 - tectonic[idx];
                    // Scale detail by both stress AND macro elevation — only add detail
                    // where the macro says there's mountain, not in valleys
                    let above_sea = (macro_hm - SEA_LEVEL).max(0.0);
                    let mountain_intensity = (stress * above_sea * 3.0).min(1.0);
                    let mountain_detail = peaks_valleys[idx] * mountain_intensity * 0.2;
                    heightmap_vec[idx] = macro_hm + mountain_detail;
                    // Recompute derived layers with eroded heightmap
                    let hm = heightmap_vec[idx];
                    temperature[idx] = derived::derive_temperature(light_level[idx], hm, humidity[idx], continentalness[idx]);
                    erosion[idx] = derived::derive_erosion(hm, rock_hardness[idx], humidity[idx]);
                    let temp = temperature[idx];
                    aridity[idx] = derived::derive_aridity(temp, humidity[idx]);
                    precipitation_type[idx] = derived::derive_precipitation_type(temp, humidity[idx], hm);
                    snowpack[idx] = derived::derive_snowpack(precipitation_type[idx], temp, hm, light_level[idx]);
                    biomes[idx] = splines.evaluate_dithered_with_light(
                        continentalness[idx], temp, tectonic[idx], erosion[idx],
                        peaks_valleys[idx], humidity[idx], aridity[idx], rock_hardness[idx],
                        px, py, light_level[idx],
                    );
                }
            }
        }

        // Rivers: use global network if available, otherwise fall back to per-tile generation
        let rivers = if let Some(net) = river_network {
            let threshold = match detail_level {
                0 | 1 => crate::rivers::LOD_THRESHOLD_MACRO,
                2 => crate::rivers::LOD_THRESHOLD_MESO,
                _ => crate::rivers::LOD_THRESHOLD_MICRO,
            };
            crate::rivers::rasterize_from_network(net, world_x, world_y, world_size, output_size, threshold)
        } else {
            // Legacy fallback: per-tile river generation
            let mut carved_heightmap = heightmap_vec.clone();
            if let Some(macro_map) = macro_map {
                crate::rivers::carve_river_channels(
                    &mut carved_heightmap,
                    output_size, output_size,
                    &macro_map.rivers, macro_map.width, macro_map.height,
                    world_x, world_y, world_size,
                    SEA_LEVEL,
                );
            }
            let river_gen = RiverGenerator::for_map_size_with_detail(SEA_LEVEL, output_size, output_size, detail_level);
            if let Some(macro_map) = macro_map {
                river_gen.generate_with_macro_flow_climate(
                    &carved_heightmap, output_size, output_size,
                    &macro_map.rivers, macro_map.width, macro_map.height,
                    world_x, world_y, world_size,
                    &light_level, &humidity,
                )
            } else {
                river_gen.generate_climate_aware(&heightmap_vec, &light_level, &humidity, output_size, output_size)
            }
        };

        for idx in 0..total_pixels {
            if rivers[idx] > 0.05
                && continentalness[idx] >= SEA_LEVEL
                && temperature[idx] > -10.0
                && temperature[idx] < 45.0
            {
                biomes[idx] = TileType::River;
            }
        }

        let water_table: Vec<f64> = (0..total_pixels).map(|idx| {
            derived::derive_water_table(rivers[idx], humidity[idx], heightmap_vec[idx], precipitation_type[idx], continentalness[idx])
        }).collect();

        // Oasis override: desert biomes near rivers become oases (rare, small)
        for idx in 0..total_pixels {
            if water_table[idx] > 0.45 && continentalness[idx] >= SEA_LEVEL && temperature[idx] < 80.0 {
                match biomes[idx] {
                    TileType::Desert | TileType::Sahara | TileType::Erg | TileType::Hamada => {
                        biomes[idx] = TileType::Oasis;
                    }
                    TileType::Steppe | TileType::Scrubland => {
                        biomes[idx] = TileType::Meadow;
                    }
                    _ => {}
                }
            }
        }

        // Polar ice cap override: jagged ice edge using noise perturbation
        apply_polar_ice_cap(&mut biomes, &light_level, &continentalness, &peaks_valleys, &rock_hardness, &temperature, SEA_LEVEL);

        let vegetation_density: Vec<f64> = biomes.iter().zip(water_table.iter())
            .map(|(&b, &wt)| derived::derive_vegetation_density(b, wt)).collect();
        let soil_type: Vec<f64> = biomes.iter().zip(erosion.iter()).zip(rock_hardness.iter())
            .map(|((&b, &e), &r)| derived::derive_soil_type(b, e, r)).collect();


        // Compute wind field for meso tile
        let sub_x_pixel = (0.5 * world_width - world_x) / scale;
        let sub_y_pixel = (1.0 * world_height - world_y) / scale;
        let sub_stellar_norm = (sub_x_pixel / output_size as f64, sub_y_pixel / output_size as f64);
        let meso_wind = WindField::generate(
            &light_level, &heightmap_vec, output_size, output_size,
            sub_stellar_norm,
        );

        Self {
            width: output_size,
            height: output_size,
            continentalness,
            tectonic,
            tectonic_plate_ids,
            humidity,
            rock_hardness,
            light_level,
            peaks_valleys,
            volcanism,
            heightmap: heightmap_vec,
            temperature,
            erosion,
            rivers,
            aridity,
            precipitation_type,
            water_table,
            wind_speed: meso_wind.speed,
            resource_richness,
            snowpack,
            biomes,
            vegetation_density,
            soil_type,
            resource_map: None,
            river_network: None,
            drainage_area: Vec::new(),
            sediment: Vec::new(),
            world_width: world_size,
            world_height: world_size,
        }
    }

    /// Generate full meso BiomeMap using the specified backend.
    pub fn generate_meso_full_with_backend(
        seed: u32,
        world_x: f64,
        world_y: f64,
        world_size: f64,
        output_size: usize,
        world_height: f64,
        detail_level: u32,
        progress: Option<&Arc<LayerProgress>>,
        _backend: NoiseBackend,
        macro_map: Option<&BiomeMap>,
        river_network: Option<&Arc<RiverNetwork>>,
    ) -> Self {
        // Force CPU backend: GPU shaders use 2D noise which doesn't wrap horizontally.
        // TODO: implement 3D OpenSimplex in WGSL, then restore GPU path.
        match NoiseBackend::Cpu {
            NoiseBackend::Cpu => Self::generate_meso_full(
                seed, world_x, world_y, world_size, output_size, world_height, detail_level, progress, macro_map, river_network,
            ),
            NoiseBackend::Gpu => Self::generate_meso_full_gpu(
                seed, world_x, world_y, world_size, output_size, world_height, detail_level, progress, macro_map, river_network,
            ),
        }
    }

    /// GPU-accelerated meso generation with progress tracking.
    #[cfg(feature = "gpu")]
    fn generate_meso_full_gpu(
        seed: u32,
        world_x: f64,
        world_y: f64,
        world_size: f64,
        output_size: usize,
        world_height: f64,
        detail_level: u32,
        progress: Option<&Arc<LayerProgress>>,
        macro_map: Option<&BiomeMap>,
        river_network: Option<&Arc<RiverNetwork>>,
    ) -> Self {
        use crate::gpu::GpuNoiseContext;

        let Some(gpu) = GpuNoiseContext::global() else {
            return Self::generate_meso_full(
                seed, world_x, world_y, world_size, output_size, world_height, detail_level, progress, macro_map, river_network,
            );
        };

        let total_pixels = output_size * output_size;
        let scale = world_size / output_size as f64;
        let world_width = world_height * 2.0;

        let layers = gpu.generate_layers(
            seed, output_size, output_size, world_x, world_y, scale, world_height, detail_level,
        );

        // Mark base layers as complete
        if let Some(p) = progress {
            p.increment(LayerId::Continentalness, total_pixels);
            p.increment(LayerId::Tectonic, total_pixels);
            p.increment(LayerId::LightLevel, total_pixels);
            p.increment(LayerId::RockHardness, total_pixels);
        }

        let continentalness: Vec<f64> = layers.continentalness.iter().map(|&v| v as f64).collect();
        let gpu_light_level: Vec<f64> = layers.light_level.iter().map(|&v| v as f64).collect();
        let gpu_rock_hardness: Vec<f64> = layers.rock_hardness.iter().map(|&v| v as f64).collect();

        // Tectonic, peaks, and humidity computed on CPU for boundary alignment and terminator model
        let tectonic_strategy = TectonicPlatesStrategy::new_wrapping(seed.wrapping_add(2), world_width);
        let raw_peaks_strategy = PeaksAndValleysStrategy::new_wrapping(seed.wrapping_add(4), world_width);
        let humidity_strategy = HumidityStrategy::new_wrapping(seed.wrapping_add(5), world_width);
        let cpu_data: Vec<_> = (0..total_pixels)
            .into_par_iter()
            .map(|idx| {
                let px = idx % output_size;
                let py = idx / output_size;
                let wx = world_x + (px as f64 * scale);
                let wy = world_y + (py as f64 * scale);
                let tect_sample = tectonic_strategy.generate_full(wx, wy);
                let raw_peaks = raw_peaks_strategy.ridged_aligned(wx, wy, detail_level, tect_sample.boundary_tangent, tect_sample.stress);
                let cont = continentalness[idx];
                let light = gpu_light_level[idx];
                let humid = humidity_strategy.generate_terminator_model(wx, wy, detail_level, cont, light);
                (tect_sample, raw_peaks, humid)
            })
            .collect();

        let gpu_tectonic: Vec<f64> = cpu_data.iter().map(|(s, _, _)| s.boundary_distance).collect();
        let tectonic_plate_ids: Vec<f64> = cpu_data.iter().map(|(s, _, _)| s.plate_id).collect();
        let tectonic_volcanism: Vec<f64> = cpu_data.iter().map(|(s, _, _)| s.volcanism).collect();
        let raw_peaks: Vec<f64> = cpu_data.iter().map(|(_, p, _)| *p).collect();
        let gpu_humidity: Vec<f64> = cpu_data.iter().map(|(_, _, h)| *h).collect();

        if let Some(p) = progress {
            p.increment(LayerId::PeaksValleys, total_pixels);
            p.increment(LayerId::Humidity, total_pixels);
        }

        // Derive all per-pixel layers in parallel
        let splines = BiomeSplines::new(SEA_LEVEL);
        let detail_noise = OpenSimplex::new(seed.wrapping_add(50));
        let derived_data: Vec<_> = (0..total_pixels)
            .into_par_iter()
            .map(|idx| {
                let cont = continentalness[idx];
                let tect = gpu_tectonic[idx];
                let light = gpu_light_level[idx];
                let rock = gpu_rock_hardness[idx];
                let humid = gpu_humidity[idx];

                let px = idx % output_size;
                let py = idx / output_size;
                let wx = world_x + (px as f64 * scale);
                let wy = world_y + (py as f64 * scale);
                let peaks = derived::derive_peaks_valleys(raw_peaks[idx], tect, rock);
                let volc = tectonic_volcanism[idx];
                let hm = derived::derive_heightmap(cont, tect, peaks);
                let hm = if detail_level >= 3 {
                    derived::derive_micro_heightmap(hm, wx, wy, &detail_noise)
                } else {
                    hm
                };
                let temp = derived::derive_temperature(light, hm, humid, cont);
                let eros = derived::derive_erosion(hm, rock, humid);
                let arid = derived::derive_aridity(temp, humid);
                let precip = derived::derive_precipitation_type(temp, humid, hm);
                let res = derived::derive_resource_richness(tect, rock, eros);
                let snow = derived::derive_snowpack(precip, temp, hm, light);
                let biome = splines.evaluate_dithered_with_light(cont, temp, tect, eros, peaks, humid, arid, rock, px, py, light);

                (peaks, volc, hm, temp, eros, arid, precip, res, snow, biome)
            })
            .collect();

        let mut peaks_valleys = Vec::with_capacity(total_pixels);
        let mut volcanism = Vec::with_capacity(total_pixels);
        let mut heightmap_vec = Vec::with_capacity(total_pixels);
        let mut temperature = Vec::with_capacity(total_pixels);
        let mut erosion = Vec::with_capacity(total_pixels);
        let mut aridity = Vec::with_capacity(total_pixels);
        let mut precipitation_type = Vec::with_capacity(total_pixels);
        let mut resource_richness = Vec::with_capacity(total_pixels);
        let mut snowpack = Vec::with_capacity(total_pixels);
        let mut biomes = Vec::with_capacity(total_pixels);

        for (peaks, volc, hm, temp, eros, arid, precip, res, snow, biome) in derived_data {
            peaks_valleys.push(peaks);
            volcanism.push(volc);
            heightmap_vec.push(hm);
            temperature.push(temp);
            erosion.push(eros);
            aridity.push(arid);
            precipitation_type.push(precip);
            resource_richness.push(res);
            snowpack.push(snow);
            biomes.push(biome);
        }

        // Override heightmap with macro eroded values if available
        if let Some(mm) = macro_map {
            if !mm.heightmap.is_empty() {
                let splines_recomp = BiomeSplines::new(SEA_LEVEL);
                for idx in 0..total_pixels {
                    let px = idx % output_size;
                    let py = idx / output_size;
                    let wx = world_x + (px as f64 * scale);
                    let wy = world_y + (py as f64 * scale);
                    let macro_hm = mm.sample_heightmap_at(wx, wy);
                    let stress = 1.0 - gpu_tectonic[idx];
                    let above_sea = (macro_hm - SEA_LEVEL).max(0.0);
                    let mountain_intensity = (stress * above_sea * 3.0).min(1.0);
                    let mountain_detail = peaks_valleys[idx] * mountain_intensity * 0.2;
                    heightmap_vec[idx] = macro_hm + mountain_detail;
                    let hm = heightmap_vec[idx];
                    temperature[idx] = derived::derive_temperature(gpu_light_level[idx], hm, gpu_humidity[idx], continentalness[idx]);
                    erosion[idx] = derived::derive_erosion(hm, gpu_rock_hardness[idx], gpu_humidity[idx]);
                    let temp = temperature[idx];
                    aridity[idx] = derived::derive_aridity(temp, gpu_humidity[idx]);
                    precipitation_type[idx] = derived::derive_precipitation_type(temp, gpu_humidity[idx], hm);
                    snowpack[idx] = derived::derive_snowpack(precipitation_type[idx], temp, hm, gpu_light_level[idx]);
                    biomes[idx] = splines_recomp.evaluate_dithered_with_light(
                        continentalness[idx], temp, gpu_tectonic[idx], erosion[idx],
                        peaks_valleys[idx], gpu_humidity[idx], aridity[idx], gpu_rock_hardness[idx],
                        px, py, gpu_light_level[idx],
                    );
                }
            }
        }

        if let Some(p) = progress {
            p.increment(LayerId::Derivation, total_pixels);
        }

        // Rivers: use global network if available, otherwise fall back to per-tile generation
        let rivers = if let Some(net) = river_network {
            let threshold = match detail_level {
                0 | 1 => crate::rivers::LOD_THRESHOLD_MACRO,
                2 => crate::rivers::LOD_THRESHOLD_MESO,
                _ => crate::rivers::LOD_THRESHOLD_MICRO,
            };
            crate::rivers::rasterize_from_network(net, world_x, world_y, world_size, output_size, threshold)
        } else {
            let mut carved_heightmap = heightmap_vec.clone();
            if let Some(macro_map) = macro_map {
                crate::rivers::carve_river_channels(
                    &mut carved_heightmap,
                    output_size, output_size,
                    &macro_map.rivers, macro_map.width, macro_map.height,
                    world_x, world_y, world_size,
                    SEA_LEVEL,
                );
            }
            let river_gen = RiverGenerator::for_map_size_with_detail(SEA_LEVEL, output_size, output_size, detail_level);
            if let Some(macro_map) = macro_map {
                river_gen.generate_with_macro_flow_climate(
                    &carved_heightmap, output_size, output_size,
                    &macro_map.rivers, macro_map.width, macro_map.height,
                    world_x, world_y, world_size,
                    &gpu_light_level, &gpu_humidity,
                )
            } else {
                river_gen.generate_climate_aware(&heightmap_vec, &gpu_light_level, &gpu_humidity, output_size, output_size)
            }
        };

        for idx in 0..total_pixels {
            if rivers[idx] > 0.05
                && continentalness[idx] >= SEA_LEVEL
                && temperature[idx] > -10.0
                && temperature[idx] < 45.0
            {
                biomes[idx] = TileType::River;
            }
        }

        let water_table: Vec<f64> = (0..total_pixels).map(|idx| {
            derived::derive_water_table(rivers[idx], gpu_humidity[idx], heightmap_vec[idx], precipitation_type[idx], continentalness[idx])
        }).collect();

        // Polar ice cap override: jagged ice edge using noise perturbation
        apply_polar_ice_cap(&mut biomes, &gpu_light_level, &continentalness, &peaks_valleys, &gpu_rock_hardness, &temperature, SEA_LEVEL);

        let vegetation_density: Vec<f64> = biomes.iter().zip(water_table.iter())
            .map(|(&b, &wt)| derived::derive_vegetation_density(b, wt)).collect();
        let soil_type: Vec<f64> = biomes.iter().zip(erosion.iter()).zip(gpu_rock_hardness.iter())
            .map(|((&b, &e), &r)| derived::derive_soil_type(b, e, r)).collect();


        // Compute wind field for GPU meso tile
        let sub_x_pixel = (0.5 * world_width - world_x) / scale;
        let sub_y_pixel = (1.0 * world_height - world_y) / scale;
        let sub_stellar_norm = (sub_x_pixel / output_size as f64, sub_y_pixel / output_size as f64);
        let gpu_meso_wind = WindField::generate(
            &gpu_light_level, &heightmap_vec, output_size, output_size,
            sub_stellar_norm,
        );

        Self {
            width: output_size,
            height: output_size,
            continentalness,
            tectonic: gpu_tectonic,
            tectonic_plate_ids,
            humidity: gpu_humidity,
            rock_hardness: gpu_rock_hardness,
            light_level: gpu_light_level,
            peaks_valleys,
            volcanism,
            heightmap: heightmap_vec,
            temperature,
            erosion,
            rivers,
            aridity,
            precipitation_type,
            water_table,
            wind_speed: gpu_meso_wind.speed,
            resource_richness,
            snowpack,
            biomes,
            vegetation_density,
            soil_type,
            resource_map: None,
            river_network: None,
            drainage_area: Vec::new(),
            sediment: Vec::new(),
            world_width: world_size,
            world_height: world_size,
        }
    }

    /// GPU meso generation stub when gpu feature is disabled.
    #[cfg(not(feature = "gpu"))]
    fn generate_meso_full_gpu(
        seed: u32,
        world_x: f64,
        world_y: f64,
        world_size: f64,
        output_size: usize,
        world_height: f64,
        detail_level: u32,
        progress: Option<&Arc<LayerProgress>>,
        macro_map: Option<&BiomeMap>,
        river_network: Option<&Arc<RiverNetwork>>,
    ) -> Self {
        Self::generate_meso_full(
            seed, world_x, world_y, world_size, output_size, world_height, detail_level, progress, macro_map, river_network,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_correct_size() {
        let map = BiomeMap::generate(42, 64, 32);
        assert_eq!(map.width, 64);
        assert_eq!(map.height, 32);
        assert_eq!(map.biomes.len(), 64 * 32);
        assert_eq!(map.continentalness.len(), 64 * 32);
        assert_eq!(map.temperature.len(), 64 * 32);
        assert_eq!(map.tectonic.len(), 64 * 32);
        assert_eq!(map.erosion.len(), 64 * 32);
        assert_eq!(map.peaks_valleys.len(), 64 * 32);
        assert_eq!(map.humidity.len(), 64 * 32);
        assert_eq!(map.light_level.len(), 64 * 32);
        assert_eq!(map.rock_hardness.len(), 64 * 32);
        assert_eq!(map.rivers.len(), 64 * 32);
        assert_eq!(map.volcanism.len(), 64 * 32);
        assert_eq!(map.heightmap.len(), 64 * 32);
        assert_eq!(map.aridity.len(), 64 * 32);
        assert_eq!(map.precipitation_type.len(), 64 * 32);
        assert_eq!(map.water_table.len(), 64 * 32);
        assert_eq!(map.wind_speed.len(), 64 * 32);
        assert_eq!(map.resource_richness.len(), 64 * 32);
        assert_eq!(map.snowpack.len(), 64 * 32);
        assert_eq!(map.vegetation_density.len(), 64 * 32);
        assert_eq!(map.soil_type.len(), 64 * 32);
    }

    #[test]
    fn biome_image_has_correct_size() {
        let map = BiomeMap::generate(42, 64, 32);
        let image = map.to_biome_image();
        assert_eq!(image.len(), 64 * 32 * 4);
    }

    #[test]
    fn layer_images_all_work() {
        let map = BiomeMap::generate(42, 32, 16);

        for layer in NoiseLayer::all() {
            let image = map.to_layer_image(*layer);
            assert_eq!(
                image.len(),
                32 * 16 * 4,
                "Layer {:?} has wrong image size",
                layer
            );
        }
    }

    #[test]
    fn far_from_sub_stellar_is_cold() {
        let map = BiomeMap::generate(42, 128, 64);
        let temp = map.get_temperature(64, 5).unwrap();
        assert!(temp < 0.0, "Far from sub-stellar temp {} should be cold (< 0)", temp);
    }

    #[test]
    fn near_sub_stellar_is_hot() {
        let map = BiomeMap::generate(42, 128, 64);
        let temp = map.get_temperature(64, 60).unwrap();
        assert!(temp > 30.0, "Near sub-stellar temp {} should be hot (> 30)", temp);
    }

    #[test]
    fn no_vegetation_in_bottom_25_percent() {
        // The bottom 25% of the map is the sun side — constant direct sunlight,
        // temperatures near 100°C, evaporated oceans. The simulation must produce
        // NO green/vegetation biomes there. If this test fails, the temperature
        // model, aridity, or lapse rate is broken.
        let map = BiomeMap::generate(42, 256, 128);
        let green_biomes = [
            TileType::Forest, TileType::Jungle, TileType::Plains, TileType::Meadow,
            TileType::DeciduousForest, TileType::TemperateRainforest, TileType::SubtropicalForest,
            TileType::CloudForest, TileType::Woodland, TileType::DryWoodland, TileType::Taiga,
            TileType::AlpineMeadow, TileType::Marsh, TileType::Mangrove,
            TileType::Savanna, TileType::HighlandSavanna, TileType::Steppe,
        ];
        let y_cutoff = (map.height * 3) / 4; // bottom 25%
        let mut violations = Vec::new();
        for y in y_cutoff..map.height {
            for x in 0..map.width {
                let biome = map.biomes[y * map.width + x];
                if green_biomes.contains(&biome) {
                    let temp = map.temperature[y * map.width + x];
                    violations.push((x, y, biome, temp));
                }
            }
        }
        assert!(violations.is_empty(),
            "Found {} green biomes in bottom 25% (sun side). First 5: {:?}",
            violations.len(), &violations[..violations.len().min(5)]);
    }

    #[test]
    fn no_vegetation_near_sub_stellar() {
        // The sub-stellar point is the hottest spot on the planet (bottom center).
        // Within 10% of the map radius around it, nothing grows — not even oases.
        // This is a spherical check: distance from sub-stellar < 10% of map diagonal.
        let w = 256;
        let h = 128;
        let map = BiomeMap::generate(42, w, h);
        let green_biomes = [
            TileType::Forest, TileType::Jungle, TileType::Plains, TileType::Meadow,
            TileType::DeciduousForest, TileType::TemperateRainforest, TileType::SubtropicalForest,
            TileType::CloudForest, TileType::Woodland, TileType::DryWoodland, TileType::Taiga,
            TileType::AlpineMeadow, TileType::Marsh, TileType::Mangrove, TileType::Oasis,
            TileType::Savanna, TileType::HighlandSavanna, TileType::Steppe,
        ];
        // Sub-stellar at (0.5, 1.0) in normalized coords = (w/2, h) in pixels
        let sub_x = w as f64 * 0.5;
        let sub_y = h as f64; // bottom edge
        let radius = (w as f64).hypot(h as f64) * 0.10; // 10% of diagonal
        let mut violations = Vec::new();
        for y in 0..h {
            for x in 0..w {
                let dx = x as f64 - sub_x;
                let dy = y as f64 - sub_y;
                let dist = (dx * dx + dy * dy).sqrt();
                if dist < radius {
                    let biome = map.biomes[y * w + x];
                    if green_biomes.contains(&biome) {
                        let temp = map.temperature[y * w + x];
                        violations.push((x, y, biome, temp));
                    }
                }
            }
        }
        assert!(violations.is_empty(),
            "Found {} green biomes within 10% radius of sub-stellar. First 5: {:?}",
            violations.len(), &violations[..violations.len().min(5)]);
    }

    #[test]
    fn polar_cap_is_uniform() {
        // At very low light levels (dark pole), ALL biomes should be White.
        // No fragmented IceSheet/Snow/Glacier/FrozenBog mix — solid ice cap.
        let map = BiomeMap::generate(42, 256, 128);
        let mut violations = Vec::new();
        for y in 0..map.height {
            for x in 0..map.width {
                let idx = y * map.width + x;
                let light = map.light_level[idx];
                let biome = map.biomes[idx];
                if light < 0.05 && biome != TileType::White {
                    violations.push((x, y, biome, light));
                }
            }
        }
        assert!(violations.is_empty(),
            "Found {} non-White biomes in polar cap (light < 0.05). First 5: {:?}",
            violations.len(), &violations[..violations.len().min(5)]);
    }

    #[test]
    fn biome_map_bincode_roundtrip() {
        let map = BiomeMap::generate(42, 64, 32);
        let encoded = bincode::serialize(&map).expect("serialize BiomeMap");
        let decoded: BiomeMap = bincode::deserialize(&encoded).expect("deserialize BiomeMap");

        assert_eq!(map.width, decoded.width);
        assert_eq!(map.height, decoded.height);
        assert_eq!(map.continentalness, decoded.continentalness);
        assert_eq!(map.tectonic, decoded.tectonic);
        assert_eq!(map.tectonic_plate_ids, decoded.tectonic_plate_ids);
        assert_eq!(map.humidity, decoded.humidity);
        assert_eq!(map.rock_hardness, decoded.rock_hardness);
        assert_eq!(map.light_level, decoded.light_level);
        assert_eq!(map.peaks_valleys, decoded.peaks_valleys);
        assert_eq!(map.volcanism, decoded.volcanism);
        assert_eq!(map.heightmap, decoded.heightmap);
        assert_eq!(map.temperature, decoded.temperature);
        assert_eq!(map.erosion, decoded.erosion);
        assert_eq!(map.rivers, decoded.rivers);
        assert_eq!(map.aridity, decoded.aridity);
        assert_eq!(map.precipitation_type, decoded.precipitation_type);
        assert_eq!(map.water_table, decoded.water_table);
        assert_eq!(map.wind_speed, decoded.wind_speed);
        assert_eq!(map.resource_richness, decoded.resource_richness);
        assert_eq!(map.snowpack, decoded.snowpack);
        assert_eq!(map.biomes, decoded.biomes);
        assert_eq!(map.vegetation_density, decoded.vegetation_density);
        assert_eq!(map.soil_type, decoded.soil_type);
        assert_eq!(map.drainage_area, decoded.drainage_area);
        assert_eq!(map.sediment, decoded.sediment);
        assert_eq!(map.world_width, decoded.world_width);
        assert_eq!(map.world_height, decoded.world_height);
        // river_network is skipped (Arc<RiverNetwork>), so it deserializes as None
        assert!(decoded.river_network.is_none());
    }

    #[test]
    fn chunk_boundary_heights_deterministic() {
        // Generate the same chunk twice — must produce identical heightmaps.
        // This verifies that derive_micro_heightmap is deterministic and
        // uses absolute world coordinates (not chunk-local ones).
        let bm_a = BiomeMap::generate_meso_full_with_backend(
            42, 200.0, 180.0, 1.0, 512, 512.0, 3, None, NoiseBackend::Cpu, None, None,
        );
        let bm_b = BiomeMap::generate_meso_full_with_backend(
            42, 200.0, 180.0, 1.0, 512, 512.0, 3, None, NoiseBackend::Cpu, None, None,
        );

        // Every pixel must match exactly (same seed, same coordinates).
        for i in 0..bm_a.heightmap.len() {
            assert!(
                (bm_a.heightmap[i] - bm_b.heightmap[i]).abs() < 1e-14,
                "heightmap mismatch at pixel {i}: {} vs {}",
                bm_a.heightmap[i],
                bm_b.heightmap[i],
            );
        }
    }

    #[test]
    fn micro_heightmap_has_variation() {
        // At detail_level=3 (micro), the heightmap should have meaningful
        // block-level variation thanks to derive_micro_heightmap.
        let bm = BiomeMap::generate_meso_full_with_backend(
            42, 200.0, 180.0, 1.0, 512, 512.0, 3, None, NoiseBackend::Cpu, None, None,
        );

        let hmin = bm.heightmap.iter().cloned().fold(f64::INFINITY, f64::min);
        let hmax = bm.heightmap.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        let span = hmax - hmin;

        // Before the fix, span was ~0.004. After, it should be >0.05.
        assert!(
            span > 0.05,
            "micro heightmap span {span:.6} is too small — derive_micro_heightmap may not be working"
        );
    }
}
