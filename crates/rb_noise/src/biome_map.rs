use rayon::prelude::*;
use rb_core::{NoiseStrategy, TileType};
use std::sync::Arc;

use crate::biome_splines::BiomeSplines;
use crate::progress::{LayerId, LayerProgress};
use crate::rivers::RiverGenerator;
use crate::derived;
use crate::strategy::{
    ContinentalnessStrategy, HumidityStrategy, LightLevelStrategy,
    PeaksAndValleysStrategy, RockHardnessStrategy,
    TectonicPlatesStrategy,
};
use crate::visualization::{
    aridity_to_rgba, grayscale_to_rgba, heightmap_to_rgba, humidity_to_rgba,
    light_level_to_rgba, peaks_to_rgba, precipitation_type_to_rgba, resources_to_rgba,
    river_moisture_to_rgba, river_to_rgba, rock_hardness_to_rgba, snowpack_to_rgba,
    soil_type_to_rgba, tectonic_to_rgba, temperature_to_rgba, vegetation_to_rgba,
    volcanism_to_rgba, NoiseLayer,
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
    pub river_moisture: Vec<f64>,
    pub resource_richness: Vec<f64>,
    pub snowpack: Vec<f64>,
    pub biomes: Vec<TileType>,
    pub vegetation_density: Vec<f64>,
    pub soil_type: Vec<f64>,
}

impl BiomeMap {
    /// Generate a biome map with all terrain layers using the specified backend.
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
                let tect_sample = tectonic_strategy.generate_full(fx, fy);
                let raw_peaks = raw_peaks_strategy.generate(fx, fy, 0);
                let light = light_level_strategy.generate(fx, fy, 0);
                let rock = rock_hardness_strategy.generate(fx, fy, 0);
                let humid = humidity_strategy.generate_with_continentalness(fx, fy, 0, cont);

                (cont, tect_sample.boundary_distance, tect_sample.plate_id, raw_peaks, light, rock, humid, tect_sample.volcanism)
            })
            .collect();

        // Phase 2: Derive all per-pixel layers
        let splines = BiomeSplines::new(SEA_LEVEL);

        let phase2_data: Vec<_> = base_data
            .par_iter()
            .map(|&(cont, tect, _, raw_peaks, light, rock, humid, volc)| {
                let peaks = derived::derive_peaks_valleys(raw_peaks, tect, rock);
                let hm = derived::derive_heightmap(cont, tect, peaks);
                let temp = derived::derive_temperature(light, hm, humid);
                let eros = derived::derive_erosion(hm, rock, humid);
                let arid = derived::derive_aridity(temp, humid);
                let precip = derived::derive_precipitation_type(temp, humid, hm);
                let res = derived::derive_resource_richness(tect, rock, eros);
                let snow = derived::derive_snowpack(precip, temp);
                let biome = splines.evaluate(cont, temp, tect, eros, peaks, humid, arid);

                (peaks, volc, hm, temp, eros, arid, precip, res, snow, biome)
            })
            .collect();

        // Unpack into separate vectors
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

        for (base, p2) in base_data.iter().zip(phase2_data.iter()) {
            let (cont, tect, pid, _raw_peaks, light, rock, humid, _volc_from_base) = *base;
            let (peaks, volc, hm, temp, eros, arid, precip, res, snow, biome) = *p2;

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

        // Phase 3: Generate rivers using D8 flow accumulation with climate awareness
        let river_gen = RiverGenerator::for_map_size(SEA_LEVEL, width, height);
        let rivers = river_gen.generate_climate_aware(&heightmap_vec, &light_level, &humidity, width, height);

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

        // Post-river derivation: river_moisture, vegetation, soil
        let river_moisture: Vec<f64> = rivers.iter().map(|&r| derived::derive_river_moisture(r)).collect();
        let vegetation_density: Vec<f64> = biomes.iter().zip(river_moisture.iter())
            .map(|(&b, &rm)| derived::derive_vegetation_density(b, rm)).collect();
        let soil_type: Vec<f64> = biomes.iter().zip(erosion.iter()).zip(rock_hardness.iter())
            .map(|((&b, &e), &r)| derived::derive_soil_type(b, e, r)).collect();

        // Volcanism post-process: override biome for high volcanism on land
        for idx in 0..total_pixels {
            if volcanism[idx] > 0.8 && continentalness[idx] >= SEA_LEVEL {
                biomes[idx] = TileType::Volcanic;
            }
        }

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
            river_moisture,
            resource_richness,
            snowpack,
            biomes,
            vegetation_density,
            soil_type,
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
        let raw_peaks: Vec<f64> = layers.peaks_valleys.iter().map(|&v| v as f64).collect();
        let gpu_light_level: Vec<f64> = layers.light_level.iter().map(|&v| v as f64).collect();
        let gpu_rock_hardness: Vec<f64> = layers.rock_hardness.iter().map(|&v| v as f64).collect();

        // Tectonic computed on CPU (too complex for GPU shader)
        let tectonic_strategy = TectonicPlatesStrategy::new(seed.wrapping_add(2));
        let tectonic_data: Vec<_> = (0..total_pixels)
            .into_par_iter()
            .map(|idx| {
                let x = (idx % width) as f64;
                let y = (idx / width) as f64;
                tectonic_strategy.generate_full(x, y)
            })
            .collect();

        let gpu_tectonic: Vec<f64> = tectonic_data.iter().map(|s| s.boundary_distance).collect();
        let tectonic_plate_ids: Vec<f64> = tectonic_data.iter().map(|s| s.plate_id).collect();
        let tectonic_volcanism: Vec<f64> = tectonic_data.iter().map(|s| s.volcanism).collect();

        // Humidity from GPU is pure base (fBm + water distance, no light drying)
        let gpu_humidity: Vec<f64> = layers.humidity.iter().map(|&v| v as f64).collect();

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

                let peaks = derived::derive_peaks_valleys(raw_peaks[idx], tect, rock);
                let volc = tectonic_volcanism[idx];
                let hm = derived::derive_heightmap(cont, tect, peaks);
                let temp = derived::derive_temperature(light, hm, humid);
                let eros = derived::derive_erosion(hm, rock, humid);
                let arid = derived::derive_aridity(temp, humid);
                let precip = derived::derive_precipitation_type(temp, humid, hm);
                let res = derived::derive_resource_richness(tect, rock, eros);
                let snow = derived::derive_snowpack(precip, temp);
                let biome = splines.evaluate(cont, temp, tect, eros, peaks, humid, arid);

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

        // Rivers with climate awareness
        let river_gen = RiverGenerator::for_map_size(SEA_LEVEL, width, height);
        let rivers = river_gen.generate_climate_aware(&heightmap_vec, &gpu_light_level, &gpu_humidity, width, height);

        for idx in 0..total_pixels {
            if rivers[idx] > 0.0
                && continentalness[idx] >= SEA_LEVEL
                && temperature[idx] > -10.0
                && temperature[idx] < 70.0
            {
                biomes[idx] = TileType::River;
            }
        }

        let river_moisture: Vec<f64> = rivers.iter().map(|&r| derived::derive_river_moisture(r)).collect();
        let vegetation_density: Vec<f64> = biomes.iter().zip(river_moisture.iter())
            .map(|(&b, &rm)| derived::derive_vegetation_density(b, rm)).collect();
        let soil_type: Vec<f64> = biomes.iter().zip(erosion.iter()).zip(gpu_rock_hardness.iter())
            .map(|((&b, &e), &r)| derived::derive_soil_type(b, e, r)).collect();

        // Volcanism post-process
        for idx in 0..total_pixels {
            if volcanism[idx] > 0.8 && continentalness[idx] >= SEA_LEVEL {
                biomes[idx] = TileType::Volcanic;
            }
        }

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
            river_moisture,
            resource_richness,
            snowpack,
            biomes,
            vegetation_density,
            soil_type,
        }
    }

    /// GPU generation stub when gpu feature is disabled.
    #[cfg(not(feature = "gpu"))]
    fn generate_gpu(seed: u32, width: usize, height: usize) -> Self {
        Self::generate(seed, width, height)
    }

    /// Convert any layer to RGBA image bytes.
    pub fn to_layer_image(&self, layer: NoiseLayer) -> Vec<u8> {
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
                    NoiseLayer::RiverMoisture => river_moisture_to_rgba(self.river_moisture[idx]),
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
            (NoiseLayer::RiverMoisture, "river_moisture"),
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
        let cont_strategy = ContinentalnessStrategy::new(seed);
        let tectonic_strategy = TectonicPlatesStrategy::new(seed.wrapping_add(2));
        let raw_peaks_strategy = PeaksAndValleysStrategy::new(seed.wrapping_add(4));
        let humidity_strategy = HumidityStrategy::new(seed.wrapping_add(5));
        let light_level_strategy = LightLevelStrategy::new(
            seed.wrapping_add(6), sub_stellar_x, sub_stellar_y, world_width, world_height,
        );
        let rock_hardness_strategy = RockHardnessStrategy::new(seed.wrapping_add(7));
        let splines = BiomeSplines::new(SEA_LEVEL);

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
                let raw_peaks = raw_peaks_strategy.generate(wx, wy, detail_level);
                let light = light_level_strategy.generate(wx, wy, detail_level);
                let rock = rock_hardness_strategy.generate(wx, wy, detail_level);
                let humid = humidity_strategy.generate_with_continentalness(wx, wy, detail_level, cont);

                let peaks = derived::derive_peaks_valleys(raw_peaks, tect, rock);
                let volc = tect_sample.volcanism;
                let hm = derived::derive_heightmap(cont, tect, peaks);
                let temp = derived::derive_temperature(light, hm, humid);
                let eros = derived::derive_erosion(hm, rock, humid);
                let arid = derived::derive_aridity(temp, humid);
                let precip = derived::derive_precipitation_type(temp, humid, hm);
                let res = derived::derive_resource_richness(tect, rock, eros);
                let snow = derived::derive_snowpack(precip, temp);
                let biome = splines.evaluate(cont, temp, tect, eros, peaks, humid, arid);

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
        let river_gen = RiverGenerator::for_map_size(SEA_LEVEL, output_size, output_size);
        let rivers = river_gen.generate(&heightmap_vec, output_size, output_size);

        for idx in 0..total_pixels {
            if rivers[idx] > 0.0
                && continentalness[idx] >= SEA_LEVEL
                && temperature[idx] > -10.0
                && temperature[idx] < 70.0
            {
                biomes[idx] = TileType::River;
            }
        }

        let river_moisture: Vec<f64> = rivers.iter().map(|&r| derived::derive_river_moisture(r)).collect();
        let vegetation_density: Vec<f64> = biomes.iter().zip(river_moisture.iter())
            .map(|(&b, &rm)| derived::derive_vegetation_density(b, rm)).collect();
        let soil_type: Vec<f64> = biomes.iter().zip(erosion.iter()).zip(rock_hardness.iter())
            .map(|((&b, &e), &r)| derived::derive_soil_type(b, e, r)).collect();

        for idx in 0..total_pixels {
            if volcanism[idx] > 0.8 && continentalness[idx] >= SEA_LEVEL {
                biomes[idx] = TileType::Volcanic;
            }
        }

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
            river_moisture,
            resource_richness,
            snowpack,
            biomes,
            vegetation_density,
            soil_type,
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
                let temp = derived::derive_temperature(light, cont.max(0.0), 0.3);
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
        progress: &Arc<LayerProgress>,
        macro_map: Option<&BiomeMap>,
    ) -> Self {
        let world_width = world_height * 2.0;
        let cont_strategy = ContinentalnessStrategy::new(seed);
        let tectonic_strategy = TectonicPlatesStrategy::new(seed.wrapping_add(2));
        let raw_peaks_strategy = PeaksAndValleysStrategy::new(seed.wrapping_add(4));
        let humidity_strategy = HumidityStrategy::new(seed.wrapping_add(5));
        let light_level_strategy = LightLevelStrategy::new(
            seed.wrapping_add(6), 0.5, 1.0, world_width, world_height,
        );
        let rock_hardness_strategy = RockHardnessStrategy::new(seed.wrapping_add(7));
        let splines = BiomeSplines::new(SEA_LEVEL);

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
                    let raw_peaks = raw_peaks_strategy.generate(wx, wy, detail_level);
                    let light = light_level_strategy.generate(wx, wy, detail_level);
                    let rock = rock_hardness_strategy.generate(wx, wy, detail_level);
                    let humid = humidity_strategy.generate_with_continentalness(wx, wy, detail_level, cont);

                    // Phase 2: Derived layers
                    let peaks = derived::derive_peaks_valleys(raw_peaks, tect, rock);
                    let volc = tect_sample.volcanism;
                    let hm = derived::derive_heightmap(cont, tect, peaks);
                    let temp = derived::derive_temperature(light, hm, humid);
                    let eros = derived::derive_erosion(hm, rock, humid);
                    let arid = derived::derive_aridity(temp, humid);
                    let precip = derived::derive_precipitation_type(temp, humid, hm);
                    let res = derived::derive_resource_richness(tect, rock, eros);
                    let snow = derived::derive_snowpack(precip, temp);
                    let biome = splines.evaluate(cont, temp, tect, eros, peaks, humid, arid);

                    results.push((cont, temp, tect, pid, peaks, volc, hm, eros, humid, light, rock, arid, precip, res, snow, biome));
                }

                // Update progress
                let n = chunk.len();
                progress.increment(LayerId::Continentalness, n);
                progress.increment(LayerId::Tectonic, n);
                progress.increment(LayerId::PeaksValleys, n);
                progress.increment(LayerId::Humidity, n);
                progress.increment(LayerId::LightLevel, n);
                progress.increment(LayerId::RockHardness, n);
                progress.increment(LayerId::Derivation, n);

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

        // Carve macro river channels into meso heightmap before D8 flow
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

        // Rivers (use carved heightmap for D8 routing) with climate awareness
        let river_gen = RiverGenerator::for_map_size(SEA_LEVEL, output_size, output_size);
        let rivers = if let Some(macro_map) = macro_map {
            river_gen.generate_with_macro_flow_climate(
                &carved_heightmap,
                output_size,
                output_size,
                &macro_map.rivers,
                macro_map.width,
                macro_map.height,
                world_x,
                world_y,
                world_size,
                &light_level,
                &humidity,
            )
        } else {
            river_gen.generate_climate_aware(&heightmap_vec, &light_level, &humidity, output_size, output_size)
        };

        for idx in 0..total_pixels {
            if rivers[idx] > 0.0
                && continentalness[idx] >= SEA_LEVEL
                && temperature[idx] > -10.0
                && temperature[idx] < 70.0
            {
                biomes[idx] = TileType::River;
            }
        }

        let river_moisture: Vec<f64> = rivers.iter().map(|&r| derived::derive_river_moisture(r)).collect();
        let vegetation_density: Vec<f64> = biomes.iter().zip(river_moisture.iter())
            .map(|(&b, &rm)| derived::derive_vegetation_density(b, rm)).collect();
        let soil_type: Vec<f64> = biomes.iter().zip(erosion.iter()).zip(rock_hardness.iter())
            .map(|((&b, &e), &r)| derived::derive_soil_type(b, e, r)).collect();

        for idx in 0..total_pixels {
            if volcanism[idx] > 0.8 && continentalness[idx] >= SEA_LEVEL {
                biomes[idx] = TileType::Volcanic;
            }
        }

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
            river_moisture,
            resource_richness,
            snowpack,
            biomes,
            vegetation_density,
            soil_type,
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
        progress: &Arc<LayerProgress>,
        backend: NoiseBackend,
        macro_map: Option<&BiomeMap>,
    ) -> Self {
        match backend {
            NoiseBackend::Cpu => Self::generate_meso_full(
                seed, world_x, world_y, world_size, output_size, world_height, detail_level, progress, macro_map,
            ),
            NoiseBackend::Gpu => Self::generate_meso_full_gpu(
                seed, world_x, world_y, world_size, output_size, world_height, detail_level, progress, macro_map,
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

        let Some(gpu) = GpuNoiseContext::global() else {
            return Self::generate_meso_full(
                seed, world_x, world_y, world_size, output_size, world_height, detail_level, progress, macro_map,
            );
        };

        let total_pixels = output_size * output_size;
        let scale = world_size / output_size as f64;

        let layers = gpu.generate_layers(
            seed, output_size, output_size, world_x, world_y, scale, world_height, detail_level,
        );

        // Mark base layers as complete
        progress.increment(LayerId::Continentalness, total_pixels);
        progress.increment(LayerId::Tectonic, total_pixels);
        progress.increment(LayerId::LightLevel, total_pixels);
        progress.increment(LayerId::RockHardness, total_pixels);

        let continentalness: Vec<f64> = layers.continentalness.iter().map(|&v| v as f64).collect();
        let raw_peaks: Vec<f64> = layers.peaks_valleys.iter().map(|&v| v as f64).collect();
        let gpu_light_level: Vec<f64> = layers.light_level.iter().map(|&v| v as f64).collect();
        let gpu_rock_hardness: Vec<f64> = layers.rock_hardness.iter().map(|&v| v as f64).collect();
        let gpu_humidity: Vec<f64> = layers.humidity.iter().map(|&v| v as f64).collect();

        // Tectonic computed on CPU (too complex for GPU shader)
        let tectonic_strategy = TectonicPlatesStrategy::new(seed.wrapping_add(2));
        let tectonic_data: Vec<_> = (0..total_pixels)
            .into_par_iter()
            .map(|idx| {
                let px = idx % output_size;
                let py = idx / output_size;
                let wx = world_x + (px as f64 * scale);
                let wy = world_y + (py as f64 * scale);
                tectonic_strategy.generate_full(wx, wy)
            })
            .collect();

        let gpu_tectonic: Vec<f64> = tectonic_data.iter().map(|s| s.boundary_distance).collect();
        let tectonic_plate_ids: Vec<f64> = tectonic_data.iter().map(|s| s.plate_id).collect();
        let tectonic_volcanism: Vec<f64> = tectonic_data.iter().map(|s| s.volcanism).collect();

        progress.increment(LayerId::PeaksValleys, total_pixels);
        progress.increment(LayerId::Humidity, total_pixels);

        // Derive all per-pixel layers in parallel
        let splines = BiomeSplines::new(SEA_LEVEL);
        let derived_data: Vec<_> = (0..total_pixels)
            .into_par_iter()
            .map(|idx| {
                let cont = continentalness[idx];
                let tect = gpu_tectonic[idx];
                let light = gpu_light_level[idx];
                let rock = gpu_rock_hardness[idx];
                let humid = gpu_humidity[idx];

                let peaks = derived::derive_peaks_valleys(raw_peaks[idx], tect, rock);
                let volc = tectonic_volcanism[idx];
                let hm = derived::derive_heightmap(cont, tect, peaks);
                let temp = derived::derive_temperature(light, hm, humid);
                let eros = derived::derive_erosion(hm, rock, humid);
                let arid = derived::derive_aridity(temp, humid);
                let precip = derived::derive_precipitation_type(temp, humid, hm);
                let res = derived::derive_resource_richness(tect, rock, eros);
                let snow = derived::derive_snowpack(precip, temp);
                let biome = splines.evaluate(cont, temp, tect, eros, peaks, humid, arid);

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

        progress.increment(LayerId::Derivation, total_pixels);

        // Carve macro river channels into meso heightmap before D8 flow
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

        // Rivers (use carved heightmap for D8 routing) with climate awareness
        let river_gen = RiverGenerator::for_map_size(SEA_LEVEL, output_size, output_size);
        let rivers = if let Some(macro_map) = macro_map {
            river_gen.generate_with_macro_flow_climate(
                &carved_heightmap, output_size, output_size,
                &macro_map.rivers, macro_map.width, macro_map.height,
                world_x, world_y, world_size,
                &gpu_light_level, &gpu_humidity,
            )
        } else {
            river_gen.generate_climate_aware(&heightmap_vec, &gpu_light_level, &gpu_humidity, output_size, output_size)
        };

        for idx in 0..total_pixels {
            if rivers[idx] > 0.0
                && continentalness[idx] >= SEA_LEVEL
                && temperature[idx] > -10.0
                && temperature[idx] < 70.0
            {
                biomes[idx] = TileType::River;
            }
        }

        let river_moisture: Vec<f64> = rivers.iter().map(|&r| derived::derive_river_moisture(r)).collect();
        let vegetation_density: Vec<f64> = biomes.iter().zip(river_moisture.iter())
            .map(|(&b, &rm)| derived::derive_vegetation_density(b, rm)).collect();
        let soil_type: Vec<f64> = biomes.iter().zip(erosion.iter()).zip(gpu_rock_hardness.iter())
            .map(|((&b, &e), &r)| derived::derive_soil_type(b, e, r)).collect();

        for idx in 0..total_pixels {
            if volcanism[idx] > 0.8 && continentalness[idx] >= SEA_LEVEL {
                biomes[idx] = TileType::Volcanic;
            }
        }

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
            river_moisture,
            resource_richness,
            snowpack,
            biomes,
            vegetation_density,
            soil_type,
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
        Self::generate_meso_full(
            seed, world_x, world_y, world_size, output_size, world_height, detail_level, progress, macro_map,
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
        assert_eq!(map.river_moisture.len(), 64 * 32);
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
}
