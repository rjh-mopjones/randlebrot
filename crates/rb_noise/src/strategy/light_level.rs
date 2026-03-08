use noise::{NoiseFn, OpenSimplex};
use rb_core::NoiseStrategy;

/// Generates light level values based on angular distance from the sub-stellar point.
///
/// For a tidally locked planet, one hemisphere permanently faces the star.
/// Light level is highest at the sub-stellar point and falls off with angular distance.
///
/// Output range: [0.0, 1.0] where 1.0 = sub-stellar point, 0.0 = antipodal dark side
pub struct LightLevelStrategy {
    noise: OpenSimplex,
    sub_stellar_x: f64,
    sub_stellar_y: f64,
    map_width: f64,
    map_height: f64,
}

impl LightLevelStrategy {
    pub fn new(seed: u32, sub_stellar_x: f64, sub_stellar_y: f64, map_width: f64, map_height: f64) -> Self {
        Self {
            noise: OpenSimplex::new(seed),
            sub_stellar_x,
            sub_stellar_y,
            map_width,
            map_height,
        }
    }

    /// Default for a standard 1024x512 map with sub-stellar at bottom center.
    pub fn default_for_map(seed: u32) -> Self {
        Self::new(seed, 0.5, 1.0, 1024.0, 512.0)
    }

    /// Low-amplitude atmospheric scatter noise.
    fn scatter_noise(&self, x: f64, y: f64) -> f64 {
        let nx = x * 0.005;
        let ny = y * 0.005;
        // 3 octaves for gentle atmospheric variation
        let mut value = 0.0;
        let mut amplitude = 1.0;
        let mut freq = 1.0;
        let mut max_amp = 0.0;
        for _ in 0..3 {
            value += self.noise.get([nx * freq, ny * freq]) * amplitude;
            max_amp += amplitude;
            amplitude *= 0.5;
            freq *= 2.0;
        }
        (value / max_amp) * 0.05 // ~5% variation
    }
}

impl NoiseStrategy for LightLevelStrategy {
    fn generate(&self, x: f64, y: f64, _detail_level: u32) -> f64 {
        // Normalize coordinates to [0, 1]
        let nx = x / self.map_width;
        let ny = y / self.map_height;

        // Angular distance from sub-stellar point (normalized)
        let dx = nx - self.sub_stellar_x;
        let dy = ny - self.sub_stellar_y;
        let dist = (dx * dx + dy * dy).sqrt().min(1.0);

        // Cosine falloff: dist=0 => cos(0)=1 (full light), dist=1 => cos(PI/2)=0 (dark)
        let base_light = (dist * std::f64::consts::FRAC_PI_2).cos();

        // Add atmospheric scatter noise
        let scatter = self.scatter_noise(x, y);

        (base_light + scatter).clamp(0.0, 1.0)
    }

    fn name(&self) -> &'static str {
        "LightLevel"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sub_stellar_is_brightest() {
        // Sub-stellar at (0.5, 1.0) in normalized coords = (512, 512) in world coords
        let strategy = LightLevelStrategy::new(42, 0.5, 1.0, 1024.0, 512.0);
        let light = strategy.generate(512.0, 512.0, 0);
        assert!(light > 0.8, "Sub-stellar light {} should be bright", light);
    }

    #[test]
    fn antipodal_is_dark() {
        // Opposite side of the sub-stellar point
        let strategy = LightLevelStrategy::new(42, 0.5, 1.0, 1024.0, 512.0);
        let light = strategy.generate(512.0, 0.0, 0);
        assert!(light < 0.3, "Antipodal light {} should be dark", light);
    }

    #[test]
    fn output_in_range() {
        let strategy = LightLevelStrategy::default_for_map(42);
        for i in 0..100 {
            let x = i as f64 * 10.0;
            let y = i as f64 * 5.0;
            let val = strategy.generate(x, y, 0);
            assert!(val >= 0.0 && val <= 1.0, "Value {} out of range", val);
        }
    }
}
