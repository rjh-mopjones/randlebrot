/// Noise layers that can be visualized in the map view.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum NoiseLayer {
    #[default]
    Aggregate,
    // Base layers (independent noise)
    Continentalness,
    Tectonic,
    LightLevel,
    RockHardness,
    // Derived layers (computed from base layers)
    Temperature,
    Erosion,
    PeaksValleys,
    Humidity,
    Rivers,
}

impl NoiseLayer {
    /// Returns all noise layers.
    pub fn all() -> &'static [NoiseLayer] {
        &[
            Self::Aggregate,
            Self::Continentalness,
            Self::Tectonic,
            Self::LightLevel,
            Self::RockHardness,
            Self::Temperature,
            Self::Erosion,
            Self::PeaksValleys,
            Self::Humidity,
            Self::Rivers,
        ]
    }

    /// Returns base layers (independent noise strategies).
    pub fn base_layers() -> &'static [NoiseLayer] {
        &[
            Self::Continentalness,
            Self::Tectonic,
            Self::LightLevel,
            Self::RockHardness,
        ]
    }

    /// Returns derived layers (computed from base layers).
    pub fn derived_layers() -> &'static [NoiseLayer] {
        &[
            Self::Temperature,
            Self::Erosion,
            Self::PeaksValleys,
            Self::Humidity,
            Self::Rivers,
        ]
    }

    /// Returns the display name for this layer.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Aggregate => "Aggregate",
            Self::Continentalness => "Continentalness",
            Self::Temperature => "Temperature",
            Self::Tectonic => "Tectonic Plates",
            Self::Erosion => "Erosion",
            Self::PeaksValleys => "Peaks & Valleys",
            Self::Humidity => "Humidity",
            Self::LightLevel => "Light Level",
            Self::RockHardness => "Rock Hardness",
            Self::Rivers => "Rivers",
        }
    }

    /// Check if this is a base (independent noise) layer.
    pub fn is_base_layer(&self) -> bool {
        matches!(
            self,
            Self::Continentalness
                | Self::Tectonic
                | Self::LightLevel
                | Self::RockHardness
        )
    }

    /// Check if this is a derived (computed from base) layer.
    pub fn is_derived(&self) -> bool {
        matches!(
            self,
            Self::Temperature
                | Self::Erosion
                | Self::PeaksValleys
                | Self::Humidity
                | Self::Rivers
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
pub fn tectonic_to_rgba(plate_id: f64, _boundary_distance: f64) -> [u8; 4] {
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
    fn base_and_derived_cover_all_non_aggregate() {
        let base = NoiseLayer::base_layers();
        let derived = NoiseLayer::derived_layers();
        let all_non_agg: Vec<_> = NoiseLayer::all().iter().filter(|l| **l != NoiseLayer::Aggregate).collect();
        assert_eq!(base.len() + derived.len(), all_non_agg.len());
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
