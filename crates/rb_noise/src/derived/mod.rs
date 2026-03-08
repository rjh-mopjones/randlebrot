use rb_core::TileType;

/// Temperature derived from light level + elevation + humidity.
///
/// Output range: ~[-80, +150]°C (matches existing BiomeSplines expectations).
/// - light_level: [0, 1] where 1.0 = sub-stellar point
/// - elevation: heightmap value (can be negative for ocean)
/// - humidity: [0, 1] where 1.0 = saturated
pub fn derive_temperature(light_level: f64, elevation: f64, humidity: f64) -> f64 {
    // Map light [0,1] to temp [-80, +150]
    let base_temp = light_level * 230.0 - 80.0;
    // Lapse rate: mountains are colder (only for positive elevation)
    let lapse_rate = elevation.max(0.0) * 60.0;
    // Moisture moderates extremes slightly
    let humidity_buffer = humidity * 5.0;
    base_temp - lapse_rate + humidity_buffer
}

/// Heightmap from geological layers (used as elevation input for temperature).
///
/// Combines continentalness + tectonic boundary effects + peaks into unified elevation.
pub fn derive_heightmap(continentalness: f64, tectonic: f64, peaks_valleys: f64) -> f64 {
    // Tectonic boundaries (low tectonic value) push up elevation slightly
    let tectonic_uplift = (1.0 - tectonic) * 0.05;
    // Peaks add height, valleys subtract
    let peak_contribution = peaks_valleys * 0.15;
    continentalness + tectonic_uplift + peak_contribution
}

/// Erosion derived from heightmap, rock hardness, and humidity.
///
/// Steep terrain erodes more, wet regions erode faster, hard rock resists.
/// - heightmap: unified elevation value
/// - rock_hardness: [0, 1] where 1.0 = very hard
/// - humidity: [0, 1] where 1.0 = saturated
pub fn derive_erosion(heightmap: f64, rock_hardness: f64, humidity: f64) -> f64 {
    ((heightmap.max(0.0) * 0.5 + humidity * 0.5) * (1.0 - rock_hardness * 0.6)).clamp(0.0, 1.0)
}

/// Peaks amplified by tectonic stress, sustained by hard rock.
///
/// - base_pv: raw peaks/valleys noise [-1, 1]
/// - tectonic: boundary distance [0, 1] where 0 = boundary
/// - rock_hardness: [0, 1] where 1.0 = very hard
pub fn derive_peaks_valleys(base_pv: f64, tectonic: f64, rock_hardness: f64) -> f64 {
    // Near tectonic boundaries = taller peaks
    let tectonic_amp = 1.0 + (1.0 - tectonic) * 0.5;
    // Hard rock sustains sharper peaks
    let hardness_sustain = 0.7 + rock_hardness * 0.3;
    (base_pv * tectonic_amp * hardness_sustain).clamp(-1.0, 1.0)
}

/// Volcanism concentrated at plate boundaries.
///
/// On a tidally locked world, tidal flexing intensifies volcanic activity.
/// Only significant very close to plate boundaries (tectonic < ~0.15).
/// - tectonic: boundary distance [0, 1] where 0 = boundary
/// Output: [0, 1] where 1.0 = highly volcanic
pub fn derive_volcanism(tectonic: f64) -> f64 {
    // Sharp falloff: only the closest ~15% to boundaries get significant volcanism
    let proximity = (1.0 - tectonic).max(0.0);
    // Remap [0.85, 1.0] proximity to [0, 1], everything below 0.85 is zero
    let remapped = ((proximity - 0.85) / 0.15).clamp(0.0, 1.0);
    remapped.powf(2.0)
}

/// Aridity from temperature and humidity.
///
/// This is where the tidal-lock drying effect lives. High temperature + low humidity = arid.
/// - temperature: degrees C
/// - humidity: [0, 1]
/// Output: [0, 1] where 1.0 = hyper-arid
pub fn derive_aridity(temperature: f64, humidity: f64) -> f64 {
    let temp_factor = ((temperature - 10.0) / 80.0).clamp(0.0, 1.0);
    (temp_factor * 0.5 + (1.0 - humidity) * 0.5).clamp(0.0, 1.0)
}

/// Precipitation type from temperature, humidity, and elevation.
///
/// Output: [-1, 1] where -1 = snow, 0 = rain, +1 = no precipitation.
pub fn derive_precipitation_type(temperature: f64, humidity: f64, heightmap: f64) -> f64 {
    if humidity < 0.15 {
        return 1.0; // Too dry for any precip
    }
    let snow_factor = ((-temperature + 5.0) / 25.0).clamp(0.0, 1.0);
    let altitude_bonus = (heightmap - 0.1).max(0.0) * 2.0; // Higher = more snow
    let snow = (snow_factor + altitude_bonus).min(1.0);
    let humid_capped = humidity.min(0.8);
    -snow * humid_capped + (1.0 - humid_capped) * (1.0 - snow)
}

/// Snowpack — persistent snow accumulation.
///
/// Needs cold + snow-type precipitation.
/// - precipitation_type: [-1, 1] where -1 = snow
/// - temperature: degrees C
/// Output: [0, 1]
pub fn derive_snowpack(precipitation_type: f64, temperature: f64) -> f64 {
    if temperature > 3.0 {
        return 0.0;
    }
    let cold_factor = ((3.0 - temperature) / 40.0).clamp(0.0, 1.0);
    let snow_precip = (-precipitation_type).max(0.0);
    (cold_factor * snow_precip).clamp(0.0, 1.0)
}

/// River moisture — how much moisture rivers contribute to surrounding area.
///
/// Simple amplification of flow.
/// - river_flow: [0, 1]
/// Output: [0, 1]
pub fn derive_river_moisture(river_flow: f64) -> f64 {
    (river_flow * 3.0).min(1.0)
}

/// Resource richness from geological factors.
///
/// Near plate boundaries = more minerals. Hard rock = ore. Erosion exposes veins.
/// - tectonic: boundary distance [0, 1] where 0 = boundary
/// - rock_hardness: [0, 1]
/// - erosion: [0, 1]
/// Output: [0, 1]
pub fn derive_resource_richness(tectonic: f64, rock_hardness: f64, erosion: f64) -> f64 {
    let boundary = (1.0 - tectonic).powf(1.5);
    (boundary * 0.5 + rock_hardness * 0.3 + erosion * 0.2).clamp(0.0, 1.0)
}

/// Vegetation density from biome type and river moisture.
///
/// Each biome has base vegetation. River moisture boosts it.
/// Output: [0, 1]
pub fn derive_vegetation_density(biome: TileType, river_moisture: f64) -> f64 {
    let base = match biome {
        TileType::Jungle => 0.95,
        TileType::Forest => 0.8,
        TileType::Marsh => 0.7,
        TileType::Taiga => 0.6,
        TileType::Plains => 0.5,
        TileType::Savanna => 0.4,
        TileType::Steppe => 0.25,
        TileType::Plateau => 0.2,
        TileType::Mountain => 0.15,
        TileType::Tundra => 0.1,
        TileType::Beach => 0.1,
        TileType::Badlands => 0.08,
        TileType::Desert | TileType::Sahara => 0.05,
        TileType::Volcanic => 0.02,
        TileType::Snow | TileType::Glacier | TileType::White => 0.0,
        TileType::Sea | TileType::River | TileType::OceanTrench => 0.0,
    };
    (base + river_moisture * 0.3).clamp(0.0, 1.0)
}

/// Soil type from biome, erosion, and rock hardness.
///
/// 0 = bare rock, 0.5 = loam, 1.0 = rich organic.
pub fn derive_soil_type(biome: TileType, erosion: f64, rock_hardness: f64) -> f64 {
    let base = match biome {
        TileType::Forest | TileType::Jungle => 0.8,
        TileType::Plains | TileType::Marsh => 0.7,
        TileType::Savanna => 0.5,
        TileType::Taiga => 0.4,
        TileType::Steppe => 0.3,
        TileType::Tundra => 0.2,
        TileType::Beach => 0.15,
        TileType::Desert | TileType::Sahara | TileType::Badlands => 0.1,
        TileType::Mountain | TileType::Glacier | TileType::Plateau => 0.05,
        TileType::Volcanic => 0.08,
        TileType::Snow | TileType::White => 0.03,
        TileType::Sea | TileType::River | TileType::OceanTrench => 0.0,
    };
    (base + erosion * 0.3 - rock_hardness * 0.2).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temperature_at_sub_stellar() {
        // Full light, flat terrain, moderate humidity
        let temp = derive_temperature(1.0, 0.0, 0.5);
        assert!(temp > 100.0, "Sub-stellar temp {} should be very hot", temp);
    }

    #[test]
    fn temperature_at_dark_side() {
        // No light
        let temp = derive_temperature(0.0, 0.0, 0.0);
        assert!(temp < -70.0, "Dark side temp {} should be very cold", temp);
    }

    #[test]
    fn temperature_lapse_rate() {
        // Same light but higher elevation = colder
        let low = derive_temperature(0.5, 0.0, 0.0);
        let high = derive_temperature(0.5, 0.5, 0.0);
        assert!(low > high, "Lower elevation ({}) should be warmer than higher ({})", low, high);
    }

    #[test]
    fn erosion_varies_with_inputs() {
        // High elevation + wet + soft rock = high erosion
        let high_eros = derive_erosion(0.5, 0.0, 0.8);
        // Low elevation + dry + hard rock = low erosion
        let low_eros = derive_erosion(0.0, 1.0, 0.1);
        assert!(high_eros > low_eros, "Steep wet soft ({}) should erode more than flat dry hard ({})", high_eros, low_eros);
    }

    #[test]
    fn erosion_hard_rock_resists() {
        let soft = derive_erosion(0.3, 0.0, 0.5);
        let hard = derive_erosion(0.3, 1.0, 0.5);
        assert!(soft > hard, "Soft rock ({}) should erode more than hard rock ({})", soft, hard);
    }

    #[test]
    fn peaks_amplified_at_boundaries() {
        // At boundary (tectonic=0) vs plate center (tectonic=1)
        let at_boundary = derive_peaks_valleys(0.5, 0.0, 0.5);
        let at_center = derive_peaks_valleys(0.5, 1.0, 0.5);
        assert!(at_boundary > at_center, "Boundary peaks ({}) should be taller than center ({})", at_boundary, at_center);
    }

    #[test]
    fn heightmap_includes_tectonic_uplift() {
        // At tectonic boundary (tectonic=0) vs center (tectonic=1)
        let at_boundary = derive_heightmap(0.1, 0.0, 0.0);
        let at_center = derive_heightmap(0.1, 1.0, 0.0);
        assert!(at_boundary > at_center, "Boundary ({}) should have more uplift than center ({})", at_boundary, at_center);
    }

    #[test]
    fn volcanism_at_boundaries() {
        let at_boundary = derive_volcanism(0.0); // tectonic = 0 means at boundary
        let at_center = derive_volcanism(1.0);   // tectonic = 1 means plate center
        let mid = derive_volcanism(0.5);         // mid-plate
        assert!(at_boundary > at_center, "Boundary volcanism ({}) should exceed center ({})", at_boundary, at_center);
        assert!(at_boundary > 0.5, "Boundary volcanism ({}) should be significant", at_boundary);
        assert!(mid < 0.01, "Mid-plate volcanism ({}) should be near zero", mid);
    }

    #[test]
    fn aridity_hot_dry() {
        let arid = derive_aridity(80.0, 0.1); // hot + dry
        let lush = derive_aridity(15.0, 0.8); // cool + humid
        assert!(arid > lush, "Hot dry ({}) should be more arid than cool humid ({})", arid, lush);
    }

    #[test]
    fn precipitation_type_cold_is_snow() {
        let precip = derive_precipitation_type(-20.0, 0.5, 0.0);
        assert!(precip < 0.0, "Cold precip ({}) should be snow (negative)", precip);
    }

    #[test]
    fn precipitation_type_dry_is_none() {
        let precip = derive_precipitation_type(20.0, 0.05, 0.0);
        assert!((precip - 1.0).abs() < 0.01, "Very dry precip ({}) should be +1 (no precip)", precip);
    }

    #[test]
    fn snowpack_cold_snowy() {
        let snow = derive_snowpack(-0.5, -20.0); // snow precip, very cold
        assert!(snow > 0.0, "Cold snowy conditions should produce snowpack ({})", snow);
    }

    #[test]
    fn snowpack_warm_is_zero() {
        let snow = derive_snowpack(-0.5, 10.0); // snow precip type but warm
        assert!((snow - 0.0).abs() < 0.01, "Warm conditions should have no snowpack ({})", snow);
    }

    #[test]
    fn river_moisture_amplifies() {
        let low = derive_river_moisture(0.1);
        let high = derive_river_moisture(0.5);
        assert!(high > low, "More flow ({}) should give more moisture than less ({})", high, low);
    }

    #[test]
    fn resource_richness_at_boundaries() {
        let boundary = derive_resource_richness(0.0, 0.5, 0.5); // at plate boundary
        let center = derive_resource_richness(1.0, 0.5, 0.5);   // plate center
        assert!(boundary > center, "Boundary resources ({}) should exceed center ({})", boundary, center);
    }

    #[test]
    fn vegetation_forest_high() {
        let forest = derive_vegetation_density(TileType::Forest, 0.0);
        let desert = derive_vegetation_density(TileType::Desert, 0.0);
        assert!(forest > desert, "Forest veg ({}) should exceed desert ({})", forest, desert);
    }

    #[test]
    fn vegetation_river_boost() {
        let dry = derive_vegetation_density(TileType::Plains, 0.0);
        let wet = derive_vegetation_density(TileType::Plains, 1.0);
        assert!(wet > dry, "River moisture should boost vegetation ({} > {})", wet, dry);
    }

    #[test]
    fn soil_type_range() {
        let forest = derive_soil_type(TileType::Forest, 0.5, 0.3);
        let mountain = derive_soil_type(TileType::Mountain, 0.1, 0.9);
        assert!(forest > mountain, "Forest soil ({}) should be richer than mountain ({})", forest, mountain);
    }
}
