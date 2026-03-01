use rb_core::ResourceType;

/// Noise layers that can be visualized in the map view.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum NoiseLayer {
    #[default]
    Biome,
    Continentalness,
    Temperature,
    Tectonic,
    Erosion,
    PeaksValleys,
    Humidity,
    Political,
    TradeCost,
    // Resource layers
    ResourceIron,
    ResourceGold,
    ResourceCopper,
    ResourceSilver,
    ResourceGems,
    ResourceCoal,
    ResourceStone,
    ResourceSalt,
    ResourceTimber,
    ResourceFish,
    ResourceFertileSoil,
    ResourceWildGame,
}

impl NoiseLayer {
    /// Returns all noise layers.
    pub fn all() -> &'static [NoiseLayer] {
        &[
            Self::Biome,
            Self::Continentalness,
            Self::Temperature,
            Self::Tectonic,
            Self::Erosion,
            Self::PeaksValleys,
            Self::Humidity,
            Self::Political,
            Self::TradeCost,
            Self::ResourceIron,
            Self::ResourceGold,
            Self::ResourceCopper,
            Self::ResourceSilver,
            Self::ResourceGems,
            Self::ResourceCoal,
            Self::ResourceStone,
            Self::ResourceSalt,
            Self::ResourceTimber,
            Self::ResourceFish,
            Self::ResourceFertileSoil,
            Self::ResourceWildGame,
        ]
    }

    /// Returns the display name for this layer.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Biome => "Biome",
            Self::Continentalness => "Continentalness",
            Self::Temperature => "Temperature",
            Self::Tectonic => "Tectonic Plates",
            Self::Erosion => "Erosion",
            Self::PeaksValleys => "Peaks & Valleys",
            Self::Humidity => "Humidity",
            Self::Political => "Settlement Suitability",
            Self::TradeCost => "Travel Cost",
            Self::ResourceIron => "Iron Deposits",
            Self::ResourceGold => "Gold Deposits",
            Self::ResourceCopper => "Copper Deposits",
            Self::ResourceSilver => "Silver Deposits",
            Self::ResourceGems => "Gem Deposits",
            Self::ResourceCoal => "Coal Deposits",
            Self::ResourceStone => "Stone Deposits",
            Self::ResourceSalt => "Salt Deposits",
            Self::ResourceTimber => "Timber",
            Self::ResourceFish => "Fishing Grounds",
            Self::ResourceFertileSoil => "Fertile Soil",
            Self::ResourceWildGame => "Wild Game",
        }
    }

    /// Check if this is a resource layer.
    pub fn is_resource(&self) -> bool {
        matches!(
            self,
            Self::ResourceIron
                | Self::ResourceGold
                | Self::ResourceCopper
                | Self::ResourceSilver
                | Self::ResourceGems
                | Self::ResourceCoal
                | Self::ResourceStone
                | Self::ResourceSalt
                | Self::ResourceTimber
                | Self::ResourceFish
                | Self::ResourceFertileSoil
                | Self::ResourceWildGame
        )
    }

    /// Convert to ResourceType if this is a resource layer.
    pub fn to_resource_type(&self) -> Option<ResourceType> {
        match self {
            Self::ResourceIron => Some(ResourceType::Iron),
            Self::ResourceGold => Some(ResourceType::Gold),
            Self::ResourceCopper => Some(ResourceType::Copper),
            Self::ResourceSilver => Some(ResourceType::Silver),
            Self::ResourceGems => Some(ResourceType::Gems),
            Self::ResourceCoal => Some(ResourceType::Coal),
            Self::ResourceStone => Some(ResourceType::Stone),
            Self::ResourceSalt => Some(ResourceType::Salt),
            Self::ResourceTimber => Some(ResourceType::Timber),
            Self::ResourceFish => Some(ResourceType::Fish),
            Self::ResourceFertileSoil => Some(ResourceType::FertileSoil),
            Self::ResourceWildGame => Some(ResourceType::WildGame),
            _ => None,
        }
    }

    /// Create from ResourceType.
    pub fn from_resource_type(resource: ResourceType) -> Self {
        match resource {
            ResourceType::Iron => Self::ResourceIron,
            ResourceType::Gold => Self::ResourceGold,
            ResourceType::Copper => Self::ResourceCopper,
            ResourceType::Silver => Self::ResourceSilver,
            ResourceType::Gems => Self::ResourceGems,
            ResourceType::Coal => Self::ResourceCoal,
            ResourceType::Stone => Self::ResourceStone,
            ResourceType::Salt => Self::ResourceSalt,
            ResourceType::Timber => Self::ResourceTimber,
            ResourceType::Fish => Self::ResourceFish,
            ResourceType::FertileSoil => Self::ResourceFertileSoil,
            ResourceType::WildGame => Self::ResourceWildGame,
        }
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

/// Convert tectonic boundary distance to RGBA.
/// Red at boundaries (0), gray at plate centers (1).
pub fn tectonic_to_rgba(boundary_distance: f64) -> [u8; 4] {
    let r = ((1.0 - boundary_distance) * 255.0) as u8;
    let g = (boundary_distance * 128.0) as u8;
    let b = (boundary_distance * 128.0) as u8;
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

/// Convert political/settlement score to RGBA.
/// Dark (unsuitable) to green (ideal).
pub fn political_to_rgba(score: f64) -> [u8; 4] {
    // score in [0, 1]
    if score < 0.01 {
        return [20, 20, 30, 255]; // Water/impassable
    }

    let r = (50.0 - score * 30.0) as u8;
    let g = (50.0 + score * 200.0) as u8;
    let b = (50.0 - score * 30.0) as u8;
    [r, g, b, 255]
}

/// Convert trade cost to RGBA.
/// Light (cheap) to dark (expensive).
pub fn trade_to_rgba(cost: f64) -> [u8; 4] {
    if cost.is_infinite() {
        return [10, 10, 30, 255]; // Impassable - dark blue
    }

    // Normalize cost (1.0 = cheap, 10.0 = expensive)
    let normalized = ((cost - 1.0) / 9.0).clamp(0.0, 1.0);

    // Light to dark gradient
    let intensity = ((1.0 - normalized) * 200.0 + 30.0) as u8;
    [intensity, intensity, (intensity as f64 * 0.9) as u8, 255]
}

/// Convert resource abundance to RGBA.
pub fn resource_to_rgba(abundance: f64, resource: ResourceType) -> [u8; 4] {
    if abundance < 0.01 {
        return [30, 30, 30, 255]; // No resources - dark background
    }

    let base_color = resource.color();
    let intensity = (0.5 + abundance * 0.5).min(1.0);

    [
        (base_color[0] as f64 * intensity) as u8,
        (base_color[1] as f64 * intensity) as u8,
        (base_color[2] as f64 * intensity) as u8,
        255,
    ]
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
    fn resource_layer_conversion() {
        for layer in NoiseLayer::all() {
            if layer.is_resource() {
                let resource = layer.to_resource_type().expect("Should have resource type");
                let back = NoiseLayer::from_resource_type(resource);
                assert_eq!(*layer, back, "Round-trip conversion failed for {:?}", layer);
            }
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
