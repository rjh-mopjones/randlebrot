use rb_core::{NoiseStrategy, TileType};

use crate::strategy::ContinentalnessStrategy;
use crate::tidally_locked::LatitudeTemperatureStrategy;

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
    /// Computed biome for each pixel
    pub biomes: Vec<TileType>,
    /// Raw continentalness values for each pixel
    pub continentalness: Vec<f64>,
    /// Raw temperature values for each pixel
    pub temperature: Vec<f64>,
}

impl BiomeMap {
    /// Generate a biome map with latitude-based temperature.
    /// Cold at top (y=0), hot at bottom (y=height).
    ///
    /// # Arguments
    /// * `seed` - Random seed for noise generation
    /// * `width` - Map width in pixels (e.g., 1024)
    /// * `height` - Map height in pixels (e.g., 512)
    pub fn generate(seed: u32, width: usize, height: usize) -> Self {
        let cont_strategy = ContinentalnessStrategy::new(seed);
        let temp_strategy = LatitudeTemperatureStrategy::new(
            seed.wrapping_add(1),
            height as f64,
        );

        let total_pixels = width * height;
        let mut biomes = Vec::with_capacity(total_pixels);
        let mut continentalness = Vec::with_capacity(total_pixels);
        let mut temperature = Vec::with_capacity(total_pixels);

        for y in 0..height {
            for x in 0..width {
                let fx = x as f64;
                let fy = y as f64;

                // Sample noise layers (detail_level 0 for macro view)
                let cont = cont_strategy.generate(fx, fy, 0);
                let temp = temp_strategy.generate(fx, fy, 0);

                // Determine biome from climate
                let biome = TileType::from_climate(cont, temp, SEA_LEVEL);

                continentalness.push(cont);
                temperature.push(temp);
                biomes.push(biome);
            }
        }

        Self {
            width,
            height,
            biomes,
            continentalness,
            temperature,
        }
    }

    /// Convert biome data to RGBA image bytes.
    pub fn to_biome_image(&self) -> Vec<u8> {
        let mut data = Vec::with_capacity(self.width * self.height * 4);

        for biome in &self.biomes {
            let color = biome.color();
            data.extend_from_slice(&color);
        }

        data
    }

    /// Convert temperature data to RGBA image bytes (blue-to-red gradient).
    pub fn to_temperature_image(&self) -> Vec<u8> {
        let mut data = Vec::with_capacity(self.width * self.height * 4);

        for &temp in &self.temperature {
            // Map temperature range [-100, 120] to [0, 1]
            let normalized = ((temp + 100.0) / 220.0).clamp(0.0, 1.0);

            // Blue (cold) to red (hot) gradient with some green in middle
            let r = (normalized * 255.0) as u8;
            let b = ((1.0 - normalized) * 255.0) as u8;
            let g = ((1.0 - (normalized - 0.5).abs() * 2.0).max(0.0) * 180.0) as u8;

            data.extend_from_slice(&[r, g, b, 255]);
        }

        data
    }

    /// Convert continentalness data to RGBA image bytes (grayscale).
    pub fn to_continentalness_image(&self) -> Vec<u8> {
        let mut data = Vec::with_capacity(self.width * self.height * 4);

        for &cont in &self.continentalness {
            // Map continentalness range [-1, 1] to [0, 255]
            let gray = (((cont + 1.0) / 2.0) * 255.0).clamp(0.0, 255.0) as u8;
            data.extend_from_slice(&[gray, gray, gray, 255]);
        }

        data
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
    }

    #[test]
    fn biome_image_has_correct_size() {
        let map = BiomeMap::generate(42, 64, 32);
        let image = map.to_biome_image();
        assert_eq!(image.len(), 64 * 32 * 4); // RGBA
    }

    #[test]
    fn temperature_image_has_correct_size() {
        let map = BiomeMap::generate(42, 64, 32);
        let image = map.to_temperature_image();
        assert_eq!(image.len(), 64 * 32 * 4); // RGBA
    }

    #[test]
    fn top_is_cold() {
        let map = BiomeMap::generate(42, 1024, 512);
        // Top of map should be cold
        let temp = map.get_temperature(512, 10).unwrap();
        assert!(
            temp < 0.0,
            "Top temperature {} should be cold (< 0)",
            temp
        );
    }

    #[test]
    fn bottom_is_hot() {
        let map = BiomeMap::generate(42, 1024, 512);
        // Bottom of map should be hot
        let temp = map.get_temperature(512, 500).unwrap();
        assert!(
            temp > 50.0,
            "Bottom temperature {} should be hot (> 50)",
            temp
        );
    }
}
