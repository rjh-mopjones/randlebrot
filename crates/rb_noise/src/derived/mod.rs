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

/// Erosion modified by rock hardness (soft rock erodes faster).
///
/// - base_erosion: raw erosion noise [0, 1]
/// - rock_hardness: [0, 1] where 1.0 = very hard
pub fn derive_erosion(base_erosion: f64, rock_hardness: f64) -> f64 {
    // Hard rock reduces erosion by up to 60%
    (base_erosion * (1.0 - rock_hardness * 0.6)).clamp(0.0, 1.0)
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
    fn erosion_reduced_by_hard_rock() {
        let soft = derive_erosion(0.8, 0.0);
        let hard = derive_erosion(0.8, 1.0);
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
}
