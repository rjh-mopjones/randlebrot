/// Noise layers that can be visualized in the map view.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum NoiseLayer {
    #[default]
    Biome,

    // Base layers (5 — independent noise)
    Continentalness,
    Tectonic,
    Humidity,
    RockHardness,
    LightLevel,

    // Derived layers (14 — Biome is also derived but listed as default above)
    PeaksValleys,
    Volcanism,
    Heightmap,
    Temperature,
    Erosion,
    RiverFlow,
    Aridity,
    PrecipitationType,
    RiverMoisture,
    Resources,
    Snowpack,
    VegetationDensity,
    SoilType,
}

impl NoiseLayer {
    /// Returns all noise layers.
    pub fn all() -> &'static [NoiseLayer] {
        &[
            Self::Biome,
            Self::Continentalness,
            Self::Tectonic,
            Self::Humidity,
            Self::RockHardness,
            Self::LightLevel,
            Self::PeaksValleys,
            Self::Volcanism,
            Self::Heightmap,
            Self::Temperature,
            Self::Erosion,
            Self::RiverFlow,
            Self::Aridity,
            Self::PrecipitationType,
            Self::RiverMoisture,
            Self::Resources,
            Self::Snowpack,
            Self::VegetationDensity,
            Self::SoilType,
        ]
    }

    /// Returns base layers (independent noise strategies).
    pub fn base_layers() -> &'static [NoiseLayer] {
        &[
            Self::Continentalness,
            Self::Tectonic,
            Self::Humidity,
            Self::RockHardness,
            Self::LightLevel,
        ]
    }

    /// Returns derived layers (computed from base layers).
    pub fn derived_layers() -> &'static [NoiseLayer] {
        &[
            Self::PeaksValleys,
            Self::Volcanism,
            Self::Heightmap,
            Self::Temperature,
            Self::Erosion,
            Self::RiverFlow,
            Self::Aridity,
            Self::PrecipitationType,
            Self::RiverMoisture,
            Self::Resources,
            Self::Snowpack,
            Self::Biome,
            Self::VegetationDensity,
            Self::SoilType,
        ]
    }

    /// Returns the display name for this layer.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Biome => "Biome",
            Self::Continentalness => "Continentalness",
            Self::Tectonic => "Tectonic Stress",
            Self::Humidity => "Humidity",
            Self::RockHardness => "Rock Hardness",
            Self::LightLevel => "Light Level",
            Self::PeaksValleys => "Peaks & Valleys",
            Self::Volcanism => "Volcanism",
            Self::Heightmap => "Heightmap",
            Self::Temperature => "Temperature",
            Self::Erosion => "Erosion",
            Self::RiverFlow => "River Flow",
            Self::Aridity => "Aridity",
            Self::PrecipitationType => "Precipitation Type",
            Self::RiverMoisture => "River Moisture",
            Self::Resources => "Resources",
            Self::Snowpack => "Snowpack",
            Self::VegetationDensity => "Vegetation Density",
            Self::SoilType => "Soil Type",
        }
    }

    /// Returns the category for grouping in the UI.
    pub fn category(&self) -> &'static str {
        match self {
            Self::Biome => "Overview",
            Self::Continentalness | Self::Tectonic | Self::Humidity
            | Self::RockHardness | Self::LightLevel => "Base",
            Self::PeaksValleys | Self::Volcanism | Self::Heightmap | Self::Erosion => "Terrain",
            Self::Temperature | Self::Aridity | Self::PrecipitationType | Self::Snowpack => "Climate",
            Self::RiverFlow | Self::RiverMoisture => "Hydrology",
            Self::VegetationDensity | Self::SoilType | Self::Resources => "Ecology",
        }
    }

    /// Check if this is a base (independent noise) layer.
    pub fn is_base_layer(&self) -> bool {
        matches!(
            self,
            Self::Continentalness
                | Self::Tectonic
                | Self::Humidity
                | Self::RockHardness
                | Self::LightLevel
        )
    }

    /// Check if this is a derived (computed from base) layer.
    pub fn is_derived(&self) -> bool {
        matches!(
            self,
            Self::PeaksValleys
                | Self::Volcanism
                | Self::Heightmap
                | Self::Temperature
                | Self::Erosion
                | Self::RiverFlow
                | Self::Aridity
                | Self::PrecipitationType
                | Self::RiverMoisture
                | Self::Resources
                | Self::Snowpack
                | Self::Biome
                | Self::VegetationDensity
                | Self::SoilType
        )
    }
}

/// Color conversion utilities for visualization.

/// Convert a grayscale value to RGBA.
pub fn grayscale_to_rgba(value: f64, min: f64, max: f64) -> [u8; 4] {
    let normalized = ((value - min) / (max - min)).clamp(0.0, 1.0);
    let gray = (normalized * 255.0) as u8;
    [gray, gray, gray, 255]
}

/// Convert temperature to RGBA (blue = cold, red = hot).
pub fn temperature_to_rgba(temp: f64) -> [u8; 4] {
    // Normalize from [-100, 100] to [0, 1]
    let normalized = ((temp + 100.0) / 200.0).clamp(0.0, 1.0);

    // Blue at cold, red at hot, green in middle
    let r = (normalized * 255.0) as u8;
    let b = ((1.0 - normalized) * 255.0) as u8;
    let g = ((1.0 - (normalized - 0.5).abs() * 2.0).max(0.0) * 180.0) as u8;

    [r, g, b, 255]
}

/// Convert tectonic plate ID + boundary distance to RGBA.
/// Each plate gets a distinct color, boundaries are drawn dark.
pub fn tectonic_to_rgba(plate_id: f64, boundary_distance: f64) -> [u8; 4] {
    // 16 distinct plate colors (hue-spread, saturated)
    let plate_colors: [[u8; 3]; 16] = [
        [34, 180, 100],  // green
        [200, 50, 120],  // magenta
        [140, 130, 140], // gray
        [180, 120, 50],  // brown/orange
        [50, 190, 180],  // cyan
        [200, 50, 60],   // red
        [80, 80, 200],   // blue
        [170, 170, 60],  // olive
        [130, 50, 170],  // purple
        [80, 170, 50],   // lime
        [190, 140, 100], // tan
        [60, 130, 170],  // steel blue
        [190, 90, 170],  // pink
        [100, 160, 130], // teal
        [170, 80, 60],   // rust
        [140, 170, 190], // light blue-gray
    ];

    let idx = ((plate_id * 255.0) as usize) % plate_colors.len();
    let [r, g, b] = plate_colors[idx];

    // Darken near boundaries: boundary_distance 0 = on boundary (black),
    // 1 = plate center (full color). Use sqrt for gentler falloff.
    let brightness = boundary_distance.sqrt().clamp(0.0, 1.0);
    let r = (r as f64 * brightness) as u8;
    let g = (g as f64 * brightness) as u8;
    let b = (b as f64 * brightness) as u8;

    [r, g, b, 255]
}

/// Convert peaks/valleys to RGBA.
/// Blue for valleys (-1), white for ridges (+1).
pub fn peaks_to_rgba(value: f64) -> [u8; 4] {
    // value in [-1, 1]
    if value < 0.0 {
        // Valley - blue tint
        let intensity = (1.0 + value) as f64; // 0 at -1, 1 at 0
        let b = 255;
        let rg = (intensity * 200.0) as u8;
        [rg, rg, b, 255]
    } else {
        // Ridge - white/gray
        let intensity = (128.0 + value * 127.0) as u8;
        [intensity, intensity, intensity, 255]
    }
}

/// Convert humidity to RGBA.
/// Brown (dry) to blue (wet).
pub fn humidity_to_rgba(humidity: f64) -> [u8; 4] {
    // humidity in [0, 1]
    if humidity < 0.5 {
        // Dry - brown to tan
        let t = humidity * 2.0;
        let r = (139.0 + t * 80.0) as u8;
        let g = (69.0 + t * 80.0) as u8;
        let b = (19.0 + t * 80.0) as u8;
        [r, g, b, 255]
    } else {
        // Wet - tan to blue
        let t = (humidity - 0.5) * 2.0;
        let r = (219.0 - t * 150.0) as u8;
        let g = (149.0 - t * 50.0) as u8;
        let b = (99.0 + t * 156.0) as u8;
        [r, g, b, 255]
    }
}

/// Convert river flow to RGBA.
/// Higher flow = brighter blue rivers.
pub fn river_to_rgba(flow: f64) -> [u8; 4] {
    if flow < 0.05 {
        [30, 30, 30, 255] // No river - dark background
    } else {
        // Blue intensity scales with flow
        let intensity = (flow.min(1.0) * 200.0) as u8;
        [40, 80 + intensity / 2, 180 + intensity / 3, 255]
    }
}

/// Convert light level to RGBA (black -> yellow gradient).
pub fn light_level_to_rgba(light: f64) -> [u8; 4] {
    let t = light.clamp(0.0, 1.0);
    let r = (t * 255.0) as u8;
    let g = (t * 230.0) as u8;
    let b = (t * 50.0) as u8;
    [r, g, b, 255]
}

/// Convert rock hardness to RGBA (brown -> gray gradient).
pub fn rock_hardness_to_rgba(hardness: f64) -> [u8; 4] {
    let t = hardness.clamp(0.0, 1.0);
    // Soft rock = brown (139, 90, 43), hard rock = gray (180, 180, 180)
    let r = (139.0 + t * 41.0) as u8;
    let g = (90.0 + t * 90.0) as u8;
    let b = (43.0 + t * 137.0) as u8;
    [r, g, b, 255]
}

/// Convert volcanism to RGBA.
/// Black (0) -> Dark red -> Orange -> Bright red (1).
pub fn volcanism_to_rgba(volcanism: f64) -> [u8; 4] {
    let t = volcanism.clamp(0.0, 1.0);
    if t < 0.3 {
        // Black to dark red
        let s = t / 0.3;
        let r = (s * 130.0) as u8;
        [r, 0, 0, 255]
    } else if t < 0.7 {
        // Dark red to orange
        let s = (t - 0.3) / 0.4;
        let r = (130.0 + s * 125.0) as u8;
        let g = (s * 120.0) as u8;
        [r, g, 0, 255]
    } else {
        // Orange to bright red
        let s = (t - 0.7) / 0.3;
        let r = 255;
        let g = (120.0 - s * 80.0) as u8;
        let b = (s * 40.0) as u8;
        [r, g, b, 255]
    }
}

/// Convert heightmap to RGBA.
/// Deep blue (ocean) -> Green -> Brown -> White (alpine).
pub fn heightmap_to_rgba(height: f64) -> [u8; 4] {
    if height < -0.025 {
        // Ocean: deep blue to light blue
        let t = ((height + 0.5) / 0.475).clamp(0.0, 1.0);
        let r = (t * 50.0) as u8;
        let g = (50.0 + t * 100.0) as u8;
        let b = (100.0 + t * 155.0) as u8;
        [r, g, b, 255]
    } else if height < 0.1 {
        // Low land: green
        let t = ((height + 0.025) / 0.125).clamp(0.0, 1.0);
        let r = (50.0 + t * 80.0) as u8;
        let g = (150.0 + t * 50.0) as u8;
        let b = (50.0 + t * 20.0) as u8;
        [r, g, b, 255]
    } else if height < 0.3 {
        // Mid land: green to brown
        let t = ((height - 0.1) / 0.2).clamp(0.0, 1.0);
        let r = (130.0 + t * 60.0) as u8;
        let g = (200.0 - t * 100.0) as u8;
        let b = (70.0 - t * 40.0) as u8;
        [r, g, b, 255]
    } else {
        // High land: brown to white
        let t = ((height - 0.3) / 0.3).clamp(0.0, 1.0);
        let r = (190.0 + t * 65.0) as u8;
        let g = (100.0 + t * 155.0) as u8;
        let b = (30.0 + t * 225.0) as u8;
        [r, g, b, 255]
    }
}

/// Convert aridity to RGBA.
/// Deep green (0, lush) -> Yellow -> Orange -> Red-brown (1, hyper-arid).
pub fn aridity_to_rgba(aridity: f64) -> [u8; 4] {
    let t = aridity.clamp(0.0, 1.0);
    if t < 0.33 {
        // Lush green to yellow-green
        let s = t / 0.33;
        let r = (30.0 + s * 180.0) as u8;
        let g = (150.0 + s * 50.0) as u8;
        let b = (30.0 - s * 20.0) as u8;
        [r, g, b, 255]
    } else if t < 0.66 {
        // Yellow-green to orange
        let s = (t - 0.33) / 0.33;
        let r = (210.0 + s * 45.0) as u8;
        let g = (200.0 - s * 80.0) as u8;
        let b = (10.0 + s * 10.0) as u8;
        [r, g, b, 255]
    } else {
        // Orange to red-brown
        let s = (t - 0.66) / 0.34;
        let r = (255.0 - s * 80.0) as u8;
        let g = (120.0 - s * 80.0) as u8;
        let b = (20.0 + s * 10.0) as u8;
        [r, g, b, 255]
    }
}

/// Convert precipitation type to RGBA.
/// White-blue (-1, snow) -> Cyan (0, rain) -> Brown (+1, dry).
pub fn precipitation_type_to_rgba(precip: f64) -> [u8; 4] {
    let t = ((precip + 1.0) / 2.0).clamp(0.0, 1.0); // map [-1,1] to [0,1]
    if t < 0.5 {
        // Snow (white-blue) to rain (cyan)
        let s = t / 0.5;
        let r = (220.0 - s * 180.0) as u8;
        let g = (230.0 - s * 30.0) as u8;
        let b = 255;
        [r, g, b, 255]
    } else {
        // Rain (cyan) to dry (brown)
        let s = (t - 0.5) / 0.5;
        let r = (40.0 + s * 140.0) as u8;
        let g = (200.0 - s * 90.0) as u8;
        let b = (255.0 - s * 200.0) as u8;
        [r, g, b, 255]
    }
}

/// Convert snowpack to RGBA.
/// Brown ground (0) -> Light blue (0.3) -> Bright white (1).
pub fn snowpack_to_rgba(snowpack: f64) -> [u8; 4] {
    let t = snowpack.clamp(0.0, 1.0);
    if t < 0.1 {
        // Brown ground
        let s = t / 0.1;
        let r = (120.0 + s * 40.0) as u8;
        let g = (80.0 + s * 40.0) as u8;
        let b = (40.0 + s * 80.0) as u8;
        [r, g, b, 255]
    } else if t < 0.5 {
        // Light blue
        let s = (t - 0.1) / 0.4;
        let r = (160.0 + s * 60.0) as u8;
        let g = (120.0 + s * 100.0) as u8;
        let b = (120.0 + s * 110.0) as u8;
        [r, g, b, 255]
    } else {
        // Bright white
        let s = (t - 0.5) / 0.5;
        let r = (220.0 + s * 35.0) as u8;
        let g = (220.0 + s * 35.0) as u8;
        let b = (230.0 + s * 25.0) as u8;
        [r, g, b, 255]
    }
}

/// Convert river moisture to RGBA.
/// Tan (0) -> Blue-green (1).
pub fn river_moisture_to_rgba(moisture: f64) -> [u8; 4] {
    let t = moisture.clamp(0.0, 1.0);
    let r = (180.0 - t * 140.0) as u8;
    let g = (150.0 - t * 30.0) as u8;
    let b = (100.0 + t * 120.0) as u8;
    [r, g, b, 255]
}

/// Convert resource richness to RGBA.
/// Dark (0) -> Gold/amber (1).
pub fn resources_to_rgba(richness: f64) -> [u8; 4] {
    let t = richness.clamp(0.0, 1.0);
    let r = (30.0 + t * 225.0) as u8;
    let g = (20.0 + t * 170.0) as u8;
    let b = (10.0 + t * 20.0) as u8;
    [r, g, b, 255]
}

/// Convert vegetation density to RGBA.
/// Sandy brown (0) -> Olive (0.3) -> Emerald green (1).
pub fn vegetation_to_rgba(density: f64) -> [u8; 4] {
    let t = density.clamp(0.0, 1.0);
    if t < 0.3 {
        // Sandy brown to olive
        let s = t / 0.3;
        let r = (180.0 - s * 80.0) as u8;
        let g = (150.0 - s * 30.0) as u8;
        let b = (80.0 - s * 50.0) as u8;
        [r, g, b, 255]
    } else {
        // Olive to emerald green
        let s = (t - 0.3) / 0.7;
        let r = (100.0 - s * 80.0) as u8;
        let g = (120.0 + s * 80.0) as u8;
        let b = (30.0 + s * 20.0) as u8;
        [r, g, b, 255]
    }
}

/// Convert soil type to RGBA.
/// Gray rock (0) -> Sandy loam (0.5) -> Rich dark brown (1).
pub fn soil_type_to_rgba(soil: f64) -> [u8; 4] {
    let t = soil.clamp(0.0, 1.0);
    if t < 0.5 {
        // Gray rock to sandy loam
        let s = t / 0.5;
        let r = (140.0 + s * 50.0) as u8;
        let g = (140.0 - s * 10.0) as u8;
        let b = (140.0 - s * 80.0) as u8;
        [r, g, b, 255]
    } else {
        // Sandy loam to rich dark brown
        let s = (t - 0.5) / 0.5;
        let r = (190.0 - s * 100.0) as u8;
        let g = (130.0 - s * 70.0) as u8;
        let b = (60.0 - s * 30.0) as u8;
        [r, g, b, 255]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_layers_have_unique_names() {
        let layers = NoiseLayer::all();
        let mut names: Vec<_> = layers.iter().map(|l| l.name()).collect();
        let original_len = names.len();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), original_len, "Duplicate layer names found");
    }

    #[test]
    fn base_and_derived_cover_all_non_biome() {
        let base = NoiseLayer::base_layers();
        let derived = NoiseLayer::derived_layers();
        let all_non_biome: Vec<_> = NoiseLayer::all().iter().filter(|l| **l != NoiseLayer::Biome).collect();
        // Biome appears in both all() and derived_layers(), so derived includes Biome
        // all_non_biome has 19 entries, base has 5, derived has 14 (including Biome)
        // base(5) + derived_without_biome(13) = 18 = all_non_biome(18) -- wait
        // Actually: all() has 19 entries. Biome is in all() and derived_layers().
        // all_non_biome = 18. base=5, derived=14 (includes Biome).
        // So base + derived - 1(Biome counted in derived) = 5 + 14 - 1 = 18. Check.
        assert_eq!(base.len() + derived.len() - 1, all_non_biome.len(),
            "base({}) + derived({}) - 1 should equal all_non_biome({})",
            base.len(), derived.len(), all_non_biome.len());
    }

    #[test]
    fn total_layer_count() {
        // 5 base + 15 derived (including Biome) = 20 total
        assert_eq!(NoiseLayer::all().len(), 19, "Should have 19 entries in all() (Biome + 5 base + 13 derived-without-biome)");
        assert_eq!(NoiseLayer::base_layers().len(), 5);
        assert_eq!(NoiseLayer::derived_layers().len(), 14);
    }

    #[test]
    fn all_layers_have_categories() {
        for layer in NoiseLayer::all() {
            let cat = layer.category();
            assert!(!cat.is_empty(), "Layer {:?} should have a category", layer);
        }
    }

    #[test]
    fn temperature_color_range() {
        let cold = temperature_to_rgba(-100.0);
        let hot = temperature_to_rgba(100.0);

        // Cold should be mostly blue
        assert!(cold[2] > cold[0]);
        // Hot should be mostly red
        assert!(hot[0] > hot[2]);
    }
}
