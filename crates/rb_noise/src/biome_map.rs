use rayon::prelude::*;
use rb_core::{NoiseStrategy, ResourceType, TileType};
use std::sync::Arc;

use crate::biome_splines::BiomeSplines;
use crate::progress::{LayerId, LayerProgress};
use crate::resource_map::ResourceMap;
use crate::rivers::RiverGenerator;
use crate::strategy::resource::ResourceContext;
use crate::derived;
use crate::strategy::{
    ContinentalnessStrategy, ErosionStrategy, HumidityStrategy, LightLevelStrategy,
    PeaksAndValleysStrategy, ResourceNoiseStrategy, RockHardnessStrategy,
    TectonicPlatesStrategy,
};
use crate::visualization::{
    grayscale_to_rgba, humidity_to_rgba, light_level_to_rgba, peaks_to_rgba,
    river_to_rgba, rock_hardness_to_rgba, tectonic_to_rgba, temperature_to_rgba, NoiseLayer,
};

/// Sea level threshold for continentalness.
/// Values below this are ocean, values above are land.
pub const SEA_LEVEL: f64 = -0.025;

/// Backend selection for noise generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
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
pub struct BiomeMap {
    pub width: usize,
    pub height: usize,

    // Existing layers
    /// Computed biome for each pixel
    pub biomes: Vec<TileType>,
    /// Raw continentalness values for each pixel
    pub continentalness: Vec<f64>,
    /// Raw temperature values for each pixel
    pub temperature: Vec<f64>,

    // New terrain layers
    /// Tectonic plate boundary distance (0 = boundary, 1 = center)
    pub tectonic: Vec<f64>,
    /// Tectonic plate ID for each pixel (0.0-1.0, same ID = same plate)
    pub tectonic_plate_ids: Vec<f64>,
    /// Erosion amount (0-1)
    pub erosion: Vec<f64>,
    /// Peaks and valleys ridgeline noise (-1 to 1)
    pub peaks_valleys: Vec<f64>,
    /// Humidity level (0-1)
    pub humidity: Vec<f64>,
    /// Light level from sub-stellar point (0-1)
    pub light_level: Vec<f64>,
    /// Rock hardness (0-1)
    pub rock_hardness: Vec<f64>,

    // Derived maps
    /// River flow accumulation (0-1, higher = larger river)
    pub rivers: Vec<f64>,

    // Sparse resource map
    pub resources: ResourceMap,
}

impl BiomeMap {
    /// Generate a biome map with all terrain layers using the specified backend.
    ///
    /// # Arguments
    /// * `seed` - Random seed for noise generation
    /// * `width` - Map width in pixels (e.g., 1024)
    /// * `height` - Map height in pixels (e.g., 512)
    /// * `backend` - CPU or GPU backend selection
    pub fn generate_with_backend(
        seed: u32,
        width: usize,
        height: usize,
        backend: NoiseBackend,
    ) -> Self {
        match backend {
            NoiseBackend::Cpu => Self::generate(seed, width, height),
            NoiseBackend::Gpu => Self::generate_gpu(seed, width, height),
        }
    }

    /// Generate a biome map with all terrain layers using parallel processing.
    ///
    /// # Arguments
    /// * `seed` - Random seed for noise generation
    /// * `width` - Map width in pixels (e.g., 1024)
    /// * `height` - Map height in pixels (e.g., 512)
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
        let cont_strategy = ContinentalnessStrategy::new(seed);
        let tectonic_strategy = TectonicPlatesStrategy::new(seed.wrapping_add(2));
        let raw_erosion_strategy = ErosionStrategy::new(seed.wrapping_add(3));
        let raw_peaks_strategy = PeaksAndValleysStrategy::new(seed.wrapping_add(4));
        let humidity_strategy = HumidityStrategy::new(seed.wrapping_add(5));
        let light_level_strategy = LightLevelStrategy::new(
            seed.wrapping_add(6),
            sub_stellar_x,
            sub_stellar_y,
            width as f64,
            height as f64,
        );
        let rock_hardness_strategy = RockHardnessStrategy::new(seed.wrapping_add(7));

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
                let (plate_id, tectonic) = tectonic_strategy.generate_voronoi(fx, fy);
                let raw_peaks = raw_peaks_strategy.generate(fx, fy, 0);
                let light = light_level_strategy.generate(fx, fy, 0);
                let rock = rock_hardness_strategy.generate(fx, fy, 0);

                (cont, tectonic, plate_id, raw_peaks, light, rock)
            })
            .collect();

        // Phase 2: Generate dependent layers + derive temperature
        let dependent_data: Vec<_> = indices
            .par_iter()
            .enumerate()
            .map(|(idx, &(x, y))| {
                let (cont, tect, _, raw_peaks, light, rock) = base_data[idx];
                let fx = x as f64;
                let fy = y as f64;

                let raw_erosion = raw_erosion_strategy.generate_with_continentalness(fx, fy, 0, cont);

                // Derive peaks/valleys and erosion from rock hardness
                let peaks = derived::derive_peaks_valleys(raw_peaks, tect, rock);
                let erosion = derived::derive_erosion(raw_erosion, rock);

                // Humidity using light level for tidal lock drying
                let humidity = humidity_strategy.generate_with_light_level(fx, fy, 0, cont, light, height as f64);

                // Derive temperature from light level + elevation
                let heightmap = derived::derive_heightmap(cont, tect, peaks);
                let temp = derived::derive_temperature(light, heightmap, humidity);

                (peaks, erosion, humidity, temp)
            })
            .collect();

        // Unpack into separate vectors and compute biomes
        let mut biomes = Vec::with_capacity(total_pixels);
        let mut continentalness = Vec::with_capacity(total_pixels);
        let mut temperature = Vec::with_capacity(total_pixels);
        let mut tectonic = Vec::with_capacity(total_pixels);
        let mut tectonic_plate_ids = Vec::with_capacity(total_pixels);
        let mut peaks_valleys = Vec::with_capacity(total_pixels);
        let mut erosion = Vec::with_capacity(total_pixels);
        let mut humidity = Vec::with_capacity(total_pixels);
        let mut light_level = Vec::with_capacity(total_pixels);
        let mut rock_hardness = Vec::with_capacity(total_pixels);

        // Use spline-based biome evaluation for consistency with meso tiles
        let splines = BiomeSplines::new(SEA_LEVEL);

        for (base, dep) in base_data.iter().zip(dependent_data.iter()) {
            let (cont, tect, pid, _raw_peaks, light, rock) = *base;
            let (peaks, eros, humid, temp) = *dep;

            continentalness.push(cont);
            temperature.push(temp);
            tectonic.push(tect);
            tectonic_plate_ids.push(pid);
            peaks_valleys.push(peaks);
            erosion.push(eros);
            humidity.push(humid);
            light_level.push(light);
            rock_hardness.push(rock);

            // Determine biome using splines (same as meso tiles)
            let biome = splines.evaluate(cont, temp, tect, eros, peaks, humid);
            biomes.push(biome);
        }

        // Phase 3: Generate rivers using D8 flow accumulation
        // Compute elevation with tectonic amplification for mountain chains
        let elevation: Vec<f64> = continentalness
            .iter()
            .zip(peaks_valleys.iter())
            .zip(erosion.iter())
            .map(|((&cont, &peaks), &eros)| {
                let is_land = cont >= SEA_LEVEL;
                let erosion_damp = 1.0 - eros * 0.7;

                let peak_height = if is_land {
                    peaks.max(0.0) * 0.15 * erosion_damp
                } else {
                    0.0
                };
                let valley_depth = if is_land { peaks.min(0.0).abs() * 0.08 } else { 0.0 };

                cont + peak_height - valley_depth
            })
            .collect();

        let river_gen = RiverGenerator::for_map_size(SEA_LEVEL, width, height);
        let rivers = river_gen.generate(&elevation, width, height);

        // Override biomes where rivers flow - only in habitable climate zones
        // No rivers in: ocean, frozen regions (< -10°C), or scorched regions (> 70°C)
        for idx in 0..total_pixels {
            if rivers[idx] > 0.0
                && continentalness[idx] >= SEA_LEVEL
                && temperature[idx] > -10.0
                && temperature[idx] < 70.0
            {
                biomes[idx] = TileType::River;
            }
        }

        // Phase 4: Generate resources
        let resources = Self::generate_resources(
            seed,
            width,
            height,
            &continentalness,
            &tectonic,
            &biomes,
        );

        Self {
            width,
            height,
            biomes,
            continentalness,
            temperature,
            tectonic,
            tectonic_plate_ids,
            erosion,
            peaks_valleys,
            humidity,
            light_level,
            rock_hardness,
            rivers,
            resources,
        }
    }

    /// Generate a biome map using GPU-accelerated noise generation.
    /// Falls back to CPU if GPU is unavailable.
    ///
    /// GPU generates base layers, then CPU derives temperature, erosion, peaks from them.
    #[cfg(feature = "gpu")]
    fn generate_gpu(seed: u32, width: usize, height: usize) -> Self {
        use crate::gpu::GpuNoiseContext;

        // Try to get GPU context, fallback to CPU if unavailable
        let Some(gpu) = GpuNoiseContext::global() else {
            return Self::generate(seed, width, height);
        };

        let total_pixels = width * height;

        // Generate base noise layers on GPU
        let layers = gpu.generate_layers(
            seed,
            width,
            height,
            0.0, // world_x
            0.0, // world_y
            1.0, // scale (1:1 for macro)
            height as f64,
            0, // detail_level (macro)
        );

        // Convert f32 GPU results to f64 (base layers from GPU)
        let continentalness: Vec<f64> = layers.continentalness.iter().map(|&v| v as f64).collect();
        let gpu_tectonic: Vec<f64> = layers.tectonic.iter().map(|&v| v as f64).collect();
        let raw_peaks: Vec<f64> = layers.peaks_valleys.iter().map(|&v| v as f64).collect();
        let raw_erosion: Vec<f64> = layers.erosion.iter().map(|&v| v as f64).collect();
        let gpu_light_level: Vec<f64> = layers.light_level.iter().map(|&v| v as f64).collect();
        let gpu_rock_hardness: Vec<f64> = layers.rock_hardness.iter().map(|&v| v as f64).collect();

        // GPU doesn't compute plate IDs — generate them on CPU
        let tectonic_strategy = TectonicPlatesStrategy::new(seed.wrapping_add(2));
        let tectonic_plate_ids: Vec<f64> = (0..total_pixels)
            .map(|idx| {
                let x = (idx % width) as f64;
                let y = (idx / width) as f64;
                tectonic_strategy.plate_id(x, y)
            })
            .collect();

        // Derive peaks, erosion, humidity, temperature on CPU from GPU base layers
        let humidity_strategy = HumidityStrategy::new(seed.wrapping_add(5));
        let splines = BiomeSplines::new(SEA_LEVEL);
        let mut biomes = Vec::with_capacity(total_pixels);
        let mut temperature = Vec::with_capacity(total_pixels);
        let mut peaks_valleys = Vec::with_capacity(total_pixels);
        let mut erosion = Vec::with_capacity(total_pixels);
        let mut humidity = Vec::with_capacity(total_pixels);

        for idx in 0..total_pixels {
            let cont = continentalness[idx];
            let tect = gpu_tectonic[idx];
            let light = gpu_light_level[idx];
            let rock = gpu_rock_hardness[idx];

            let peaks = derived::derive_peaks_valleys(raw_peaks[idx], tect, rock);
            let eros = derived::derive_erosion(raw_erosion[idx], rock);

            // Derive humidity with light level
            let x = (idx % width) as f64;
            let y = (idx / width) as f64;
            let humid = humidity_strategy.generate_with_light_level(x, y, 0, cont, light, height as f64);

            let heightmap = derived::derive_heightmap(cont, tect, peaks);
            let temp = derived::derive_temperature(light, heightmap, humid);

            peaks_valleys.push(peaks);
            erosion.push(eros);
            humidity.push(humid);
            temperature.push(temp);

            let biome = splines.evaluate(cont, temp, tect, eros, peaks, humid);
            biomes.push(biome);
        }

        // Generate rivers on CPU (D8 flow requires sequential processing)
        let elevation: Vec<f64> = continentalness
            .iter()
            .zip(peaks_valleys.iter())
            .zip(erosion.iter())
            .map(|((&cont, &peaks), &eros)| {
                let is_land = cont >= SEA_LEVEL;
                let erosion_damp = 1.0 - eros * 0.7;

                let peak_height = if is_land {
                    peaks.max(0.0) * 0.15 * erosion_damp
                } else {
                    0.0
                };
                let valley_depth = if is_land { peaks.min(0.0).abs() * 0.08 } else { 0.0 };

                cont + peak_height - valley_depth
            })
            .collect();

        let river_gen = RiverGenerator::for_map_size(SEA_LEVEL, width, height);
        let rivers = river_gen.generate(&elevation, width, height);

        // Override biomes where rivers flow
        for idx in 0..total_pixels {
            if rivers[idx] > 0.0
                && continentalness[idx] >= SEA_LEVEL
                && temperature[idx] > -10.0
                && temperature[idx] < 70.0
            {
                biomes[idx] = TileType::River;
            }
        }

        // Generate resources on CPU
        let resources = Self::generate_resources(
            seed,
            width,
            height,
            &continentalness,
            &gpu_tectonic,
            &biomes,
        );

        Self {
            width,
            height,
            biomes,
            continentalness,
            temperature,
            tectonic: gpu_tectonic,
            tectonic_plate_ids,
            erosion,
            peaks_valleys,
            humidity,
            light_level: gpu_light_level,
            rock_hardness: gpu_rock_hardness,
            rivers,
            resources,
        }
    }

    /// GPU generation stub when gpu feature is disabled.
    #[cfg(not(feature = "gpu"))]
    fn generate_gpu(seed: u32, width: usize, height: usize) -> Self {
        // GPU feature not enabled, fallback to CPU
        Self::generate(seed, width, height)
    }

    /// Generate resources for all resource types.
    fn generate_resources(
        seed: u32,
        width: usize,
        height: usize,
        continentalness: &[f64],
        tectonic: &[f64],
        biomes: &[TileType],
    ) -> ResourceMap {
        let mut resources = ResourceMap::new(width, height);

        // Generate each resource type
        for resource_type in ResourceType::all() {
            let strategy = ResourceNoiseStrategy::new(seed, *resource_type);

            for y in 0..height {
                for x in 0..width {
                    let idx = y * width + x;
                    let context = ResourceContext {
                        continentalness: continentalness[idx],
                        tectonic_boundary_distance: tectonic[idx],
                        water_distance: if continentalness[idx] < SEA_LEVEL {
                            0.0
                        } else {
                            ((continentalness[idx] + 0.025) * 5.0).min(1.0)
                        },
                        biome: biomes[idx],
                    };

                    let abundance =
                        strategy.generate_with_context(x as f64, y as f64, 0, &context);
                    if abundance > 0.01 {
                        resources.set(x, y, *resource_type, abundance as f32);
                    }
                }
            }
        }

        resources
    }

    /// Convert any layer to RGBA image bytes.
    pub fn to_layer_image(&self, layer: NoiseLayer) -> Vec<u8> {
        let mut data = Vec::with_capacity(self.width * self.height * 4);

        for y in 0..self.height {
            for x in 0..self.width {
                let idx = y * self.width + x;
                let color = match layer {
                    NoiseLayer::Aggregate => self.biomes[idx].color(),
                    NoiseLayer::Continentalness => {
                        grayscale_to_rgba(self.continentalness[idx], -1.0, 1.0)
                    }
                    NoiseLayer::Temperature => temperature_to_rgba(self.temperature[idx]),
                    NoiseLayer::Tectonic => tectonic_to_rgba(self.tectonic_plate_ids[idx], self.tectonic[idx]),
                    NoiseLayer::Erosion => grayscale_to_rgba(self.erosion[idx], 0.0, 1.0),
                    NoiseLayer::PeaksValleys => peaks_to_rgba(self.peaks_valleys[idx]),
                    NoiseLayer::Humidity => humidity_to_rgba(self.humidity[idx]),
                    NoiseLayer::LightLevel => light_level_to_rgba(self.light_level[idx]),
                    NoiseLayer::RockHardness => rock_hardness_to_rgba(self.rock_hardness[idx]),
                    NoiseLayer::Rivers => river_to_rgba(self.rivers[idx]),
                };
                data.extend_from_slice(&color);
            }
        }

        data
    }

    /// Convert biome data to RGBA image bytes.
    pub fn to_biome_image(&self) -> Vec<u8> {
        self.to_layer_image(NoiseLayer::Aggregate)
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
    ///
    /// Layout:
    /// - `debug_layers/aggregate.png` (root)
    /// - `debug_layers/base/continentalness.png`, `tectonic.png`, `light_level.png`, `rock_hardness.png`
    /// - `debug_layers/derived/temperature.png`, `erosion.png`, `peaks_valleys.png`, `humidity.png`, `rivers.png`
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

        // Aggregate at root
        save(NoiseLayer::Aggregate, &base_path.join("aggregate.png"));

        // Base layers
        let base_layers = [
            (NoiseLayer::Continentalness, "continentalness"),
            (NoiseLayer::Tectonic, "tectonic"),
            (NoiseLayer::LightLevel, "light_level"),
            (NoiseLayer::RockHardness, "rock_hardness"),
        ];
        for (layer, name) in &base_layers {
            save(*layer, &base_dir.join(format!("{name}.png")));
        }

        // Derived layers
        let derived_layers = [
            (NoiseLayer::Temperature, "temperature"),
            (NoiseLayer::Erosion, "erosion"),
            (NoiseLayer::PeaksValleys, "peaks_valleys"),
            (NoiseLayer::Humidity, "humidity"),
            (NoiseLayer::Rivers, "rivers"),
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

    /// Generate a meso-level (zoomed in) biome map for a specific world region.
    /// Note: This is a simplified version that only generates basic layers.
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
        // Use world-scale width for light level (assume 2:1 aspect ratio)
        let world_width = world_height * 2.0;
        let cont_strategy = ContinentalnessStrategy::new(seed);
        let tectonic_strategy = TectonicPlatesStrategy::new(seed.wrapping_add(2));
        let raw_erosion_strategy = ErosionStrategy::new(seed.wrapping_add(3));
        let raw_peaks_strategy = PeaksAndValleysStrategy::new(seed.wrapping_add(4));
        let humidity_strategy = HumidityStrategy::new(seed.wrapping_add(5));
        let light_level_strategy = LightLevelStrategy::new(
            seed.wrapping_add(6), sub_stellar_x, sub_stellar_y, world_width, world_height,
        );
        let rock_hardness_strategy = RockHardnessStrategy::new(seed.wrapping_add(7));
        let splines = BiomeSplines::new(SEA_LEVEL);

        let total_pixels = output_size * output_size;
        let scale = world_size / output_size as f64;

        let mut biomes = Vec::with_capacity(total_pixels);
        let mut continentalness = Vec::with_capacity(total_pixels);
        let mut temperature = Vec::with_capacity(total_pixels);
        let mut tectonic = Vec::with_capacity(total_pixels);
        let mut tectonic_plate_ids = Vec::with_capacity(total_pixels);
        let mut erosion = Vec::with_capacity(total_pixels);
        let mut peaks_valleys = Vec::with_capacity(total_pixels);
        let mut humidity = Vec::with_capacity(total_pixels);
        let mut light_level = Vec::with_capacity(total_pixels);
        let mut rock_hardness = Vec::with_capacity(total_pixels);

        for py in 0..output_size {
            for px in 0..output_size {
                let wx = world_x + (px as f64 * scale);
                let wy = world_y + (py as f64 * scale);

                let cont = cont_strategy.generate(wx, wy, detail_level);
                let (pid, tect) = tectonic_strategy.generate_voronoi(wx, wy);
                let raw_peaks = raw_peaks_strategy.generate(wx, wy, detail_level);
                let light = light_level_strategy.generate(wx, wy, detail_level);
                let rock = rock_hardness_strategy.generate(wx, wy, detail_level);
                let raw_eros = raw_erosion_strategy.generate_with_continentalness(wx, wy, detail_level, cont);

                let peaks = derived::derive_peaks_valleys(raw_peaks, tect, rock);
                let eros = derived::derive_erosion(raw_eros, rock);
                let humid = humidity_strategy.generate_with_light_level(wx, wy, detail_level, cont, light, world_height);
                let heightmap = derived::derive_heightmap(cont, tect, peaks);
                let temp = derived::derive_temperature(light, heightmap, humid);

                let biome = splines.evaluate(cont, temp, tect, eros, peaks, humid);

                continentalness.push(cont);
                temperature.push(temp);
                tectonic.push(tect);
                tectonic_plate_ids.push(pid);
                peaks_valleys.push(peaks);
                erosion.push(eros);
                humidity.push(humid);
                light_level.push(light);
                rock_hardness.push(rock);
                biomes.push(biome);
            }
        }

        // Generate rivers using D8 flow accumulation with tectonic elevation
        let elevation: Vec<f64> = continentalness
            .iter()
            .zip(peaks_valleys.iter())
            .zip(erosion.iter())
            .map(|((&cont, &peaks), &eros)| {
                let is_land = cont >= SEA_LEVEL;
                let erosion_damp = 1.0 - eros * 0.7;

                let peak_height = if is_land {
                    peaks.max(0.0) * 0.15 * erosion_damp
                } else {
                    0.0
                };
                let valley_depth = if is_land { peaks.min(0.0).abs() * 0.08 } else { 0.0 };

                cont + peak_height - valley_depth
            })
            .collect();

        let river_gen = RiverGenerator::for_map_size(SEA_LEVEL, output_size, output_size);
        let rivers = river_gen.generate(&elevation, output_size, output_size);

        // Override biomes where rivers flow - only in habitable climate zones
        for idx in 0..total_pixels {
            if rivers[idx] > 0.0
                && continentalness[idx] >= SEA_LEVEL
                && temperature[idx] > -10.0
                && temperature[idx] < 70.0
            {
                biomes[idx] = TileType::River;
            }
        }

        Self {
            width: output_size,
            height: output_size,
            biomes,
            continentalness,
            temperature,
            tectonic,
            tectonic_plate_ids,
            erosion,
            peaks_valleys,
            humidity,
            light_level,
            rock_hardness,
            rivers,
            resources: ResourceMap::new(output_size, output_size),
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
        let cont_strategy = ContinentalnessStrategy::new(seed);
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
                // Quick temperature derivation (no humidity/elevation detail)
                let temp = derived::derive_temperature(light, cont.max(0.0), 0.3);
                let biome = TileType::from_climate(cont, temp, SEA_LEVEL);

                image_data.extend_from_slice(&biome.color());
            }
        }

        image_data
    }

    /// Generate full meso BiomeMap with all layers in parallel + progress tracking.
    ///
    /// Unlike `generate_biome_only` which outputs RGBA only, this returns a complete
    /// BiomeMap with all terrain layers + derived layers for instant layer switching.
    ///
    /// # Arguments
    /// * `seed` - Random seed for noise generation
    /// * `world_x`, `world_y` - Top-left corner in world coordinates
    /// * `world_size` - Size of the region in world units
    /// * `output_size` - Output resolution (e.g., 512 for 512x512)
    /// * `world_height` - Total world height
    /// * `detail_level` - Noise detail level (0=macro, 1=meso, 2=micro)
    /// * `progress` - Shared progress tracker for UI updates
    pub fn generate_meso_full(
        seed: u32,
        world_x: f64,
        world_y: f64,
        world_size: f64,
        output_size: usize,
        world_height: f64,
        detail_level: u32,
        progress: &Arc<LayerProgress>,
        macro_map: Option<&BiomeMap>,
    ) -> Self {
        let world_width = world_height * 2.0;
        // Create all strategies
        let cont_strategy = ContinentalnessStrategy::new(seed);
        let tectonic_strategy = TectonicPlatesStrategy::new(seed.wrapping_add(2));
        let raw_erosion_strategy = ErosionStrategy::new(seed.wrapping_add(3));
        let raw_peaks_strategy = PeaksAndValleysStrategy::new(seed.wrapping_add(4));
        let humidity_strategy = HumidityStrategy::new(seed.wrapping_add(5));
        let light_level_strategy = LightLevelStrategy::new(
            seed.wrapping_add(6), 0.5, 1.0, world_width, world_height,
        );
        let rock_hardness_strategy = RockHardnessStrategy::new(seed.wrapping_add(7));
        let splines = BiomeSplines::new(SEA_LEVEL);

        let total_pixels = output_size * output_size;
        let scale = world_size / output_size as f64;

        // Progress chunk size - update every ~1% or 256 pixels minimum
        let progress_chunk = (total_pixels / 100).max(256);

        // Generate all pixel indices
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
                    let (pid, tect) = tectonic_strategy.generate_voronoi(wx, wy);
                    let raw_peaks = raw_peaks_strategy.generate(wx, wy, detail_level);
                    let light = light_level_strategy.generate(wx, wy, detail_level);
                    let rock = rock_hardness_strategy.generate(wx, wy, detail_level);
                    let raw_eros = raw_erosion_strategy.generate_with_continentalness(wx, wy, detail_level, cont);

                    // Phase 2: Derived layers
                    let peaks = derived::derive_peaks_valleys(raw_peaks, tect, rock);
                    let eros = derived::derive_erosion(raw_eros, rock);
                    let humid = humidity_strategy.generate_with_light_level(wx, wy, detail_level, cont, light, world_height);
                    let heightmap = derived::derive_heightmap(cont, tect, peaks);
                    let temp = derived::derive_temperature(light, heightmap, humid);

                    let biome = splines.evaluate(cont, temp, tect, eros, peaks, humid);

                    results.push((cont, temp, tect, pid, peaks, eros, humid, light, rock, biome));
                }

                // Update progress for all layers
                let n = chunk.len();
                progress.increment(LayerId::Continentalness, n);
                progress.increment(LayerId::Temperature, n);
                progress.increment(LayerId::Tectonic, n);
                progress.increment(LayerId::PeaksValleys, n);
                progress.increment(LayerId::Erosion, n);
                progress.increment(LayerId::Humidity, n);
                progress.increment(LayerId::LightLevel, n);
                progress.increment(LayerId::RockHardness, n);
                progress.increment(LayerId::Resources, n);

                results
            })
            .collect();

        // Unpack results into separate vectors
        let mut biomes = Vec::with_capacity(total_pixels);
        let mut continentalness = Vec::with_capacity(total_pixels);
        let mut temperature = Vec::with_capacity(total_pixels);
        let mut tectonic = Vec::with_capacity(total_pixels);
        let mut tectonic_plate_ids = Vec::with_capacity(total_pixels);
        let mut peaks_valleys = Vec::with_capacity(total_pixels);
        let mut erosion = Vec::with_capacity(total_pixels);
        let mut humidity = Vec::with_capacity(total_pixels);
        let mut light_level = Vec::with_capacity(total_pixels);
        let mut rock_hardness = Vec::with_capacity(total_pixels);

        for (cont, temp, tect, pid, peaks, eros, humid, light, rock, biome) in all_data {
            continentalness.push(cont);
            temperature.push(temp);
            tectonic.push(tect);
            tectonic_plate_ids.push(pid);
            peaks_valleys.push(peaks);
            erosion.push(eros);
            humidity.push(humid);
            light_level.push(light);
            rock_hardness.push(rock);
            biomes.push(biome);
        }

        // Compute meso elevation for D8 river generation
        let elevation: Vec<f64> = continentalness
            .iter()
            .zip(peaks_valleys.iter())
            .zip(erosion.iter())
            .map(|((&cont, &peaks), &eros)| {
                let is_land = cont >= SEA_LEVEL;
                let erosion_damp = 1.0 - eros * 0.7;

                let peak_height = if is_land {
                    peaks.max(0.0) * 0.15 * erosion_damp
                } else {
                    0.0
                };
                let valley_depth = if is_land { peaks.min(0.0).abs() * 0.08 } else { 0.0 };

                cont + peak_height - valley_depth
            })
            .collect();

        let river_gen = RiverGenerator::for_map_size(SEA_LEVEL, output_size, output_size);

        // If macro map available, seed edges with macro flow for cross-tile connectivity
        let rivers = if let Some(macro_map) = macro_map {
            river_gen.generate_with_macro_flow(
                &elevation,
                output_size,
                output_size,
                &macro_map.rivers,
                macro_map.width,
                macro_map.height,
                world_x,
                world_y,
                world_size,
            )
        } else {
            river_gen.generate(&elevation, output_size, output_size)
        };

        // Override biomes where rivers flow - only in habitable climate zones
        for idx in 0..total_pixels {
            if rivers[idx] > 0.0
                && continentalness[idx] >= SEA_LEVEL
                && temperature[idx] > -10.0
                && temperature[idx] < 70.0
            {
                biomes[idx] = TileType::River;
            }
        }

        // Skip resource generation for meso tiles (too expensive, sparse anyway)
        let resources = ResourceMap::new(output_size, output_size);

        Self {
            width: output_size,
            height: output_size,
            biomes,
            continentalness,
            temperature,
            tectonic,
            tectonic_plate_ids,
            erosion,
            peaks_valleys,
            humidity,
            light_level,
            rock_hardness,
            rivers,
            resources,
        }
    }

    /// Generate full meso BiomeMap using the specified backend.
    ///
    /// # Arguments
    /// * `seed` - Random seed for noise generation
    /// * `world_x`, `world_y` - Top-left corner in world coordinates
    /// * `world_size` - Size of the region in world units
    /// * `output_size` - Output resolution (e.g., 512 for 512x512)
    /// * `world_height` - Total world height (for latitude-based temperature)
    /// * `detail_level` - Noise detail level (0=macro, 1=meso, 2=micro)
    /// * `progress` - Shared progress tracker for UI updates
    /// * `backend` - CPU or GPU backend selection
    pub fn generate_meso_full_with_backend(
        seed: u32,
        world_x: f64,
        world_y: f64,
        world_size: f64,
        output_size: usize,
        world_height: f64,
        detail_level: u32,
        progress: &Arc<LayerProgress>,
        backend: NoiseBackend,
        macro_map: Option<&BiomeMap>,
    ) -> Self {
        match backend {
            NoiseBackend::Cpu => Self::generate_meso_full(
                seed,
                world_x,
                world_y,
                world_size,
                output_size,
                world_height,
                detail_level,
                progress,
                macro_map,
            ),
            NoiseBackend::Gpu => Self::generate_meso_full_gpu(
                seed,
                world_x,
                world_y,
                world_size,
                output_size,
                world_height,
                detail_level,
                progress,
                macro_map,
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
        progress: &Arc<LayerProgress>,
        macro_map: Option<&BiomeMap>,
    ) -> Self {
        use crate::gpu::GpuNoiseContext;

        // Try to get GPU context, fallback to CPU if unavailable
        let Some(gpu) = GpuNoiseContext::global() else {
            return Self::generate_meso_full(
                seed,
                world_x,
                world_y,
                world_size,
                output_size,
                world_height,
                detail_level,
                progress,
                macro_map,
            );
        };

        let total_pixels = output_size * output_size;
        let scale = world_size / output_size as f64;

        // Generate base noise layers on GPU
        let layers = gpu.generate_layers(
            seed,
            output_size,
            output_size,
            world_x,
            world_y,
            scale,
            world_height,
            detail_level,
        );

        // Mark all noise layers as complete (GPU does them all at once)
        progress.increment(LayerId::Continentalness, total_pixels);
        progress.increment(LayerId::Tectonic, total_pixels);
        progress.increment(LayerId::LightLevel, total_pixels);
        progress.increment(LayerId::RockHardness, total_pixels);

        // Convert f32 GPU results to f64
        let continentalness: Vec<f64> = layers.continentalness.iter().map(|&v| v as f64).collect();
        let gpu_tectonic: Vec<f64> = layers.tectonic.iter().map(|&v| v as f64).collect();
        let raw_peaks: Vec<f64> = layers.peaks_valleys.iter().map(|&v| v as f64).collect();
        let raw_erosion: Vec<f64> = layers.erosion.iter().map(|&v| v as f64).collect();
        let gpu_light_level: Vec<f64> = layers.light_level.iter().map(|&v| v as f64).collect();
        let gpu_rock_hardness: Vec<f64> = layers.rock_hardness.iter().map(|&v| v as f64).collect();

        // GPU doesn't compute plate IDs — generate them on CPU
        let tectonic_strategy = TectonicPlatesStrategy::new(seed.wrapping_add(2));
        let tectonic_plate_ids: Vec<f64> = (0..total_pixels)
            .map(|idx| {
                let px = idx % output_size;
                let py = idx / output_size;
                let wx = world_x + (px as f64 * scale);
                let wy = world_y + (py as f64 * scale);
                tectonic_strategy.plate_id(wx, wy)
            })
            .collect();

        // Derive peaks, erosion, humidity, temperature on CPU from GPU base layers
        let humidity_strategy = HumidityStrategy::new(seed.wrapping_add(5));
        let splines = BiomeSplines::new(SEA_LEVEL);
        let mut biomes = Vec::with_capacity(total_pixels);
        let mut temperature = Vec::with_capacity(total_pixels);
        let mut peaks_valleys = Vec::with_capacity(total_pixels);
        let mut erosion = Vec::with_capacity(total_pixels);
        let mut humidity = Vec::with_capacity(total_pixels);

        for idx in 0..total_pixels {
            let cont = continentalness[idx];
            let tect = gpu_tectonic[idx];
            let light = gpu_light_level[idx];
            let rock = gpu_rock_hardness[idx];

            let peaks = derived::derive_peaks_valleys(raw_peaks[idx], tect, rock);
            let eros = derived::derive_erosion(raw_erosion[idx], rock);

            let px = idx % output_size;
            let py = idx / output_size;
            let wx = world_x + (px as f64 * scale);
            let wy = world_y + (py as f64 * scale);
            let humid = humidity_strategy.generate_with_light_level(wx, wy, detail_level, cont, light, world_height);

            let heightmap = derived::derive_heightmap(cont, tect, peaks);
            let temp = derived::derive_temperature(light, heightmap, humid);

            peaks_valleys.push(peaks);
            erosion.push(eros);
            humidity.push(humid);
            temperature.push(temp);

            let biome = splines.evaluate(cont, temp, tect, eros, peaks, humid);
            biomes.push(biome);
        }

        progress.increment(LayerId::Temperature, total_pixels);
        progress.increment(LayerId::PeaksValleys, total_pixels);
        progress.increment(LayerId::Erosion, total_pixels);
        progress.increment(LayerId::Humidity, total_pixels);

        // Compute meso elevation for D8 river generation
        let elevation: Vec<f64> = continentalness
            .iter()
            .zip(peaks_valleys.iter())
            .zip(erosion.iter())
            .map(|((&cont, &peaks), &eros)| {
                let is_land = cont >= SEA_LEVEL;
                let erosion_damp = 1.0 - eros * 0.7;

                let peak_height = if is_land {
                    peaks.max(0.0) * 0.15 * erosion_damp
                } else {
                    0.0
                };
                let valley_depth = if is_land { peaks.min(0.0).abs() * 0.08 } else { 0.0 };

                cont + peak_height - valley_depth
            })
            .collect();

        let river_gen = RiverGenerator::for_map_size(SEA_LEVEL, output_size, output_size);

        // If macro map available, seed edges with macro flow for cross-tile connectivity
        let rivers = if let Some(macro_map) = macro_map {
            river_gen.generate_with_macro_flow(
                &elevation,
                output_size,
                output_size,
                &macro_map.rivers,
                macro_map.width,
                macro_map.height,
                world_x,
                world_y,
                world_size,
            )
        } else {
            river_gen.generate(&elevation, output_size, output_size)
        };

        // Override biomes where rivers flow
        for idx in 0..total_pixels {
            if rivers[idx] > 0.0
                && continentalness[idx] >= SEA_LEVEL
                && temperature[idx] > -10.0
                && temperature[idx] < 70.0
            {
                biomes[idx] = TileType::River;
            }
        }

        progress.increment(LayerId::Resources, total_pixels);

        Self {
            width: output_size,
            height: output_size,
            biomes,
            continentalness,
            temperature,
            tectonic: gpu_tectonic,
            tectonic_plate_ids,
            erosion,
            peaks_valleys,
            humidity,
            light_level: gpu_light_level,
            rock_hardness: gpu_rock_hardness,
            rivers,
            resources: ResourceMap::new(output_size, output_size),
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
        progress: &Arc<LayerProgress>,
        macro_map: Option<&BiomeMap>,
    ) -> Self {
        // GPU feature not enabled, fallback to CPU
        Self::generate_meso_full(
            seed,
            world_x,
            world_y,
            world_size,
            output_size,
            world_height,
            detail_level,
            progress,
            macro_map,
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
        // Sub-stellar is at (0.5, 1.0) normalized = (64, 64) in a 128x64 map
        // Top of map (y=5) is far from sub-stellar (bottom center)
        let map = BiomeMap::generate(42, 128, 64);
        let temp = map.get_temperature(64, 5).unwrap();
        assert!(temp < 0.0, "Far from sub-stellar temp {} should be cold (< 0)", temp);
    }

    #[test]
    fn near_sub_stellar_is_hot() {
        // Bottom center is near sub-stellar point
        let map = BiomeMap::generate(42, 128, 64);
        let temp = map.get_temperature(64, 60).unwrap();
        assert!(temp > 30.0, "Near sub-stellar temp {} should be hot (> 30)", temp);
    }

    #[test]
    fn resources_are_generated() {
        let map = BiomeMap::generate(42, 128, 64);
        // Should have at least some resources
        assert!(
            map.resources.cells_with_resources() > 0,
            "Should have some resource deposits"
        );
    }

}
