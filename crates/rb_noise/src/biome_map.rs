use rayon::prelude::*;
use rb_core::{NoiseStrategy, ResourceType, TileType};
use std::sync::Arc;

use crate::biome_splines::BiomeSplines;
use crate::derived::{calculate_political_score_simple, calculate_trade_cost};
use crate::progress::{LayerId, LayerProgress};
use crate::resource_map::ResourceMap;
use crate::strategy::resource::ResourceContext;
use crate::strategy::{
    ContinentalnessStrategy, ErosionStrategy, HumidityStrategy, PeaksAndValleysStrategy,
    ResourceNoiseStrategy, TectonicPlatesStrategy,
};
use crate::tidally_locked::LatitudeTemperatureStrategy;
use crate::visualization::{
    grayscale_to_rgba, humidity_to_rgba, peaks_to_rgba, political_to_rgba, resource_to_rgba,
    tectonic_to_rgba, temperature_to_rgba, trade_to_rgba, NoiseLayer,
};

/// Sea level threshold for continentalness.
/// Values below this are ocean, values above are land.
pub const SEA_LEVEL: f64 = -0.025;

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
    /// Erosion amount (0-1)
    pub erosion: Vec<f64>,
    /// Peaks and valleys ridgeline noise (-1 to 1)
    pub peaks_valleys: Vec<f64>,
    /// Humidity level (0-1)
    pub humidity: Vec<f64>,

    // Derived maps
    /// Settlement suitability score (0-1)
    pub political: Vec<f64>,
    /// Travel/trade cost (1.0 = easy, higher = harder, inf = impassable)
    pub trade_cost: Vec<f64>,

    // Sparse resource map
    pub resources: ResourceMap,
}

impl BiomeMap {
    /// Generate a biome map with all terrain layers using parallel processing.
    ///
    /// # Arguments
    /// * `seed` - Random seed for noise generation
    /// * `width` - Map width in pixels (e.g., 1024)
    /// * `height` - Map height in pixels (e.g., 512)
    pub fn generate(seed: u32, width: usize, height: usize) -> Self {
        // Create all strategies
        let cont_strategy = ContinentalnessStrategy::new(seed);
        let temp_strategy = LatitudeTemperatureStrategy::new(seed.wrapping_add(1), height as f64);
        let tectonic_strategy = TectonicPlatesStrategy::new(seed.wrapping_add(2));
        let erosion_strategy = ErosionStrategy::new(seed.wrapping_add(3));
        let peaks_strategy = PeaksAndValleysStrategy::new(seed.wrapping_add(4));
        let humidity_strategy = HumidityStrategy::new(seed.wrapping_add(5));

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
                let temp = temp_strategy.generate(fx, fy, 0);
                let tectonic = tectonic_strategy.generate(fx, fy, 0);
                let peaks = peaks_strategy.generate(fx, fy, 0);

                (cont, temp, tectonic, peaks)
            })
            .collect();

        // Phase 2: Generate dependent layers (need continentalness)
        let dependent_data: Vec<_> = indices
            .par_iter()
            .enumerate()
            .map(|(idx, &(x, y))| {
                let (cont, _, _, _) = base_data[idx];
                let fx = x as f64;
                let fy = y as f64;

                let erosion = erosion_strategy.generate_with_continentalness(fx, fy, 0, cont);
                let humidity = humidity_strategy.generate_with_continentalness(fx, fy, 0, cont);

                (erosion, humidity)
            })
            .collect();

        // Unpack into separate vectors and compute biomes
        let mut biomes = Vec::with_capacity(total_pixels);
        let mut continentalness = Vec::with_capacity(total_pixels);
        let mut temperature = Vec::with_capacity(total_pixels);
        let mut tectonic = Vec::with_capacity(total_pixels);
        let mut peaks_valleys = Vec::with_capacity(total_pixels);
        let mut erosion = Vec::with_capacity(total_pixels);
        let mut humidity = Vec::with_capacity(total_pixels);
        let mut political = Vec::with_capacity(total_pixels);
        let mut trade_cost = Vec::with_capacity(total_pixels);

        // Use spline-based biome evaluation for consistency with meso tiles
        let splines = BiomeSplines::new(SEA_LEVEL);

        for (_idx, ((cont, temp, tect, peaks), (eros, humid))) in
            base_data.iter().zip(dependent_data.iter()).enumerate()
        {
            continentalness.push(*cont);
            temperature.push(*temp);
            tectonic.push(*tect);
            peaks_valleys.push(*peaks);
            erosion.push(*eros);
            humidity.push(*humid);

            // Determine biome using splines (same as meso tiles)
            let biome = splines.evaluate(*cont, *temp, *tect, *eros, *peaks, *humid);
            biomes.push(biome);

            // Calculate derived values
            let pol = calculate_political_score_simple(biome, *temp, *humid);
            political.push(pol);

            let trade = calculate_trade_cost(biome, *eros);
            trade_cost.push(trade);
        }

        // Phase 3: Generate resources
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
            erosion,
            peaks_valleys,
            humidity,
            political,
            trade_cost,
            resources,
        }
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
                    NoiseLayer::Biome => self.biomes[idx].color(),
                    NoiseLayer::Continentalness => {
                        grayscale_to_rgba(self.continentalness[idx], -1.0, 1.0)
                    }
                    NoiseLayer::Temperature => temperature_to_rgba(self.temperature[idx]),
                    NoiseLayer::Tectonic => tectonic_to_rgba(self.tectonic[idx]),
                    NoiseLayer::Erosion => grayscale_to_rgba(self.erosion[idx], 0.0, 1.0),
                    NoiseLayer::PeaksValleys => peaks_to_rgba(self.peaks_valleys[idx]),
                    NoiseLayer::Humidity => humidity_to_rgba(self.humidity[idx]),
                    NoiseLayer::Political => political_to_rgba(self.political[idx]),
                    NoiseLayer::TradeCost => trade_to_rgba(self.trade_cost[idx]),
                    _ if layer.is_resource() => {
                        let resource = layer.to_resource_type().unwrap();
                        let abundance = self.resources.get(x, y, resource) as f64;
                        resource_to_rgba(abundance, resource)
                    }
                    _ => [128, 128, 128, 255],
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

    /// Get political/settlement score at specific coordinates.
    pub fn get_political(&self, x: usize, y: usize) -> Option<f64> {
        if x < self.width && y < self.height {
            Some(self.political[y * self.width + x])
        } else {
            None
        }
    }

    /// Get trade cost at specific coordinates.
    pub fn get_trade_cost(&self, x: usize, y: usize) -> Option<f64> {
        if x < self.width && y < self.height {
            Some(self.trade_cost[y * self.width + x])
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
        let cont_strategy = ContinentalnessStrategy::new(seed);
        let temp_strategy =
            LatitudeTemperatureStrategy::new(seed.wrapping_add(1), world_height);
        let tectonic_strategy = TectonicPlatesStrategy::new(seed.wrapping_add(2));
        let erosion_strategy = ErosionStrategy::new(seed.wrapping_add(3));
        let peaks_strategy = PeaksAndValleysStrategy::new(seed.wrapping_add(4));
        let humidity_strategy = HumidityStrategy::new(seed.wrapping_add(5));
        let splines = BiomeSplines::new(SEA_LEVEL);

        let total_pixels = output_size * output_size;
        let scale = world_size / output_size as f64;

        let mut biomes = Vec::with_capacity(total_pixels);
        let mut continentalness = Vec::with_capacity(total_pixels);
        let mut temperature = Vec::with_capacity(total_pixels);
        let mut tectonic = Vec::with_capacity(total_pixels);
        let mut erosion = Vec::with_capacity(total_pixels);
        let mut peaks_valleys = Vec::with_capacity(total_pixels);
        let mut humidity = Vec::with_capacity(total_pixels);
        let mut political = Vec::with_capacity(total_pixels);
        let mut trade_cost_vec = Vec::with_capacity(total_pixels);

        for py in 0..output_size {
            for px in 0..output_size {
                let wx = world_x + (px as f64 * scale);
                let wy = world_y + (py as f64 * scale);

                let cont = cont_strategy.generate(wx, wy, detail_level);
                let temp = temp_strategy.generate(wx, wy, detail_level);
                let tect = tectonic_strategy.generate(wx, wy, detail_level);
                let peaks = peaks_strategy.generate(wx, wy, detail_level);
                let eros = erosion_strategy.generate_with_continentalness(wx, wy, detail_level, cont);
                let humid = humidity_strategy.generate_with_continentalness(wx, wy, detail_level, cont);

                // Use splines for consistency with macro map and generate_meso_full
                let biome = splines.evaluate(cont, temp, tect, eros, peaks, humid);

                continentalness.push(cont);
                temperature.push(temp);
                tectonic.push(tect);
                peaks_valleys.push(peaks);
                erosion.push(eros);
                humidity.push(humid);
                biomes.push(biome);

                political.push(calculate_political_score_simple(biome, temp, humid));
                trade_cost_vec.push(calculate_trade_cost(biome, eros));
            }
        }

        Self {
            width: output_size,
            height: output_size,
            biomes,
            continentalness,
            temperature,
            tectonic,
            erosion,
            peaks_valleys,
            humidity,
            political,
            trade_cost: trade_cost_vec,
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
        let cont_strategy = ContinentalnessStrategy::new(seed);
        let temp_strategy =
            LatitudeTemperatureStrategy::new(seed.wrapping_add(1), world_height);

        let total_pixels = output_size * output_size;
        let scale = world_size / output_size as f64;
        let mut image_data = Vec::with_capacity(total_pixels * 4);

        for py in 0..output_size {
            for px in 0..output_size {
                let wx = world_x + (px as f64 * scale);
                let wy = world_y + (py as f64 * scale);

                let cont = cont_strategy.generate(wx, wy, detail_level);
                let temp = temp_strategy.generate(wx, wy, detail_level);
                let biome = TileType::from_climate(cont, temp, SEA_LEVEL);

                image_data.extend_from_slice(&biome.color());
            }
        }

        image_data
    }

    /// Generate full meso BiomeMap with all layers in parallel + progress tracking.
    ///
    /// Unlike `generate_biome_only` which outputs RGBA only, this returns a complete
    /// BiomeMap with all 7 terrain layers + derived layers for instant layer switching.
    ///
    /// # Arguments
    /// * `seed` - Random seed for noise generation
    /// * `world_x`, `world_y` - Top-left corner in world coordinates
    /// * `world_size` - Size of the region in world units
    /// * `output_size` - Output resolution (e.g., 512 for 512x512)
    /// * `world_height` - Total world height (for latitude-based temperature)
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
    ) -> Self {
        // Create all strategies
        let cont_strategy = ContinentalnessStrategy::new(seed);
        let temp_strategy = LatitudeTemperatureStrategy::new(seed.wrapping_add(1), world_height);
        let tectonic_strategy = TectonicPlatesStrategy::new(seed.wrapping_add(2));
        let erosion_strategy = ErosionStrategy::new(seed.wrapping_add(3));
        let peaks_strategy = PeaksAndValleysStrategy::new(seed.wrapping_add(4));
        let humidity_strategy = HumidityStrategy::new(seed.wrapping_add(5));
        let splines = BiomeSplines::new(SEA_LEVEL);

        let total_pixels = output_size * output_size;
        let scale = world_size / output_size as f64;

        // Progress chunk size - update every ~1% or 256 pixels minimum
        let progress_chunk = (total_pixels / 100).max(256);

        // Generate all pixel indices
        let indices: Vec<usize> = (0..total_pixels).collect();

        // Phase 1: Generate all layers in parallel with progress tracking
        // Each chunk of pixels updates progress atomically
        let all_data: Vec<_> = indices
            .par_chunks(progress_chunk)
            .flat_map(|chunk| {
                let mut results = Vec::with_capacity(chunk.len());

                for &idx in chunk {
                    let py = idx / output_size;
                    let px = idx % output_size;
                    let wx = world_x + (px as f64 * scale);
                    let wy = world_y + (py as f64 * scale);

                    // Generate base layers
                    let cont = cont_strategy.generate(wx, wy, detail_level);
                    let temp = temp_strategy.generate(wx, wy, detail_level);
                    let tect = tectonic_strategy.generate(wx, wy, detail_level);
                    let peaks = peaks_strategy.generate(wx, wy, detail_level);

                    // Generate dependent layers
                    let eros = erosion_strategy.generate_with_continentalness(wx, wy, detail_level, cont);
                    let humid = humidity_strategy.generate_with_continentalness(wx, wy, detail_level, cont);

                    // Compute biome using splines
                    let biome = splines.evaluate(cont, temp, tect, eros, peaks, humid);

                    // Compute derived values
                    let political = calculate_political_score_simple(biome, temp, humid);
                    let trade = calculate_trade_cost(biome, eros);

                    results.push((cont, temp, tect, peaks, eros, humid, biome, political, trade));
                }

                // Update progress for all layers
                let n = chunk.len();
                progress.increment(LayerId::Continentalness, n);
                progress.increment(LayerId::Temperature, n);
                progress.increment(LayerId::Tectonic, n);
                progress.increment(LayerId::PeaksValleys, n);
                progress.increment(LayerId::Erosion, n);
                progress.increment(LayerId::Humidity, n);
                progress.increment(LayerId::Resources, n);

                results
            })
            .collect();

        // Unpack results into separate vectors
        let mut biomes = Vec::with_capacity(total_pixels);
        let mut continentalness = Vec::with_capacity(total_pixels);
        let mut temperature = Vec::with_capacity(total_pixels);
        let mut tectonic = Vec::with_capacity(total_pixels);
        let mut peaks_valleys = Vec::with_capacity(total_pixels);
        let mut erosion = Vec::with_capacity(total_pixels);
        let mut humidity = Vec::with_capacity(total_pixels);
        let mut political = Vec::with_capacity(total_pixels);
        let mut trade_cost = Vec::with_capacity(total_pixels);

        for (cont, temp, tect, peaks, eros, humid, biome, pol, trade) in all_data {
            continentalness.push(cont);
            temperature.push(temp);
            tectonic.push(tect);
            peaks_valleys.push(peaks);
            erosion.push(eros);
            humidity.push(humid);
            biomes.push(biome);
            political.push(pol);
            trade_cost.push(trade);
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
            erosion,
            peaks_valleys,
            humidity,
            political,
            trade_cost,
            resources,
        }
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
        assert_eq!(map.political.len(), 64 * 32);
        assert_eq!(map.trade_cost.len(), 64 * 32);
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
    fn top_is_cold() {
        let map = BiomeMap::generate(42, 128, 64);
        let temp = map.get_temperature(64, 5).unwrap();
        assert!(temp < 0.0, "Top temperature {} should be cold (< 0)", temp);
    }

    #[test]
    fn bottom_is_hot() {
        let map = BiomeMap::generate(42, 128, 64);
        let temp = map.get_temperature(64, 60).unwrap();
        assert!(temp > 30.0, "Bottom temperature {} should be hot (> 30)", temp);
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

    #[test]
    fn trade_cost_is_valid() {
        let map = BiomeMap::generate(42, 64, 32);
        for y in 0..32 {
            for x in 0..64 {
                let cost = map.get_trade_cost(x, y).unwrap();
                let biome = map.get_biome(x, y).unwrap();

                if matches!(biome, TileType::Sea | TileType::White) {
                    assert!(cost.is_infinite(), "Water should be impassable");
                } else {
                    assert!(cost >= 0.8 && cost <= 10.0, "Land cost {} out of range", cost);
                }
            }
        }
    }
}
