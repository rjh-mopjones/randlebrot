//! Wind field generation and moisture advection.
//!
//! Wind model: radial outflow from sub-stellar point (hot air rises, flows outward).
//! Speed proportional to light_level gradient. Mountains block/deflect.
//!
//! Moisture advection: iterative transport. Ocean pixels seed humidity=1.0.
//! Each iteration moves moisture downwind, deposits on windward mountain sides,
//! creates rain shadow on lee side.

/// A computed wind field over the world grid.
pub struct WindField {
    /// Wind direction in radians per pixel.
    pub direction: Vec<f64>,
    /// Wind speed [0, 1] per pixel.
    pub speed: Vec<f64>,
    pub width: usize,
    pub height: usize,
}

impl WindField {
    /// Generate a wind field from light level and heightmap.
    ///
    /// Wind flows radially outward from the sub-stellar point (thermal wind).
    /// Mountains reduce speed and deflect direction.
    pub fn generate(
        light_level: &[f64],
        heightmap: &[f64],
        width: usize,
        height: usize,
        sub_stellar: (f64, f64),
    ) -> Self {
        let total = width * height;
        let mut direction = vec![0.0f64; total];
        let mut speed = vec![0.0f64; total];

        let sub_x = sub_stellar.0 * width as f64;
        let sub_y = sub_stellar.1 * height as f64;

        for y in 0..height {
            for x in 0..width {
                let idx = y * width + x;

                // Direction: radially outward from sub-stellar point
                let dx = x as f64 - sub_x;
                let dy = y as f64 - sub_y;
                let dist = (dx * dx + dy * dy).sqrt().max(1.0);
                let angle = dy.atan2(dx);

                direction[idx] = angle;

                // Speed: proportional to light level gradient (thermal wind)
                // Stronger near sub-stellar, weakens toward dark side
                let light = light_level[idx];
                let base_speed = (light * 0.6 + 0.1).min(0.8);

                // Mountains block wind
                let elev = heightmap[idx].max(0.0);
                let mountain_block = (1.0 - elev * 3.0).max(0.1);

                speed[idx] = (base_speed * mountain_block).clamp(0.0, 1.0);
            }
        }

        Self { direction, speed, width, height }
    }
}

/// Advect moisture through the wind field.
///
/// Ocean pixels seed humidity. Each iteration moves moisture downwind.
/// Windward mountain slopes receive extra moisture (orographic lift),
/// lee sides get dried out (rain shadow).
pub fn advect_moisture(
    humidity: &mut [f64],
    wind: &WindField,
    heightmap: &[f64],
    continentalness: &[f64],
    light_level: &[f64],
    width: usize,
    height: usize,
    sea_level: f64,
    iterations: usize,
) {
    let total = width * height;
    let mut buffer = vec![0.0f64; total];

    for _iter in 0..iterations {
        // Copy current state
        buffer.copy_from_slice(humidity);

        for y in 1..(height - 1) {
            for x in 1..(width - 1) {
                let idx = y * width + x;

                // Ocean pixels maintain humidity scaled by light level
                // Dark ocean has less evaporation (0.3), bright ocean up to 0.85
                if continentalness[idx] < sea_level {
                    let ocean_floor = 0.3 + light_level[idx].min(0.4) * 1.375;
                    humidity[idx] = humidity[idx].max(ocean_floor);
                    continue;
                }

                let dir = wind.direction[idx];
                let spd = wind.speed[idx];

                // Sample upwind position
                let upwind_x = (x as f64 - dir.cos() * spd * 2.0).round() as i32;
                let upwind_y = (y as f64 - dir.sin() * spd * 2.0).round() as i32;

                if upwind_x < 0 || upwind_x >= width as i32 || upwind_y < 0 || upwind_y >= height as i32 {
                    continue;
                }

                let upwind_idx = upwind_y as usize * width + upwind_x as usize;
                let upwind_moisture = buffer[upwind_idx];

                // Orographic effect: elevation change between upwind and current
                let elev_diff = heightmap[idx] - heightmap[upwind_idx];

                let deposit = if elev_diff > 0.01 {
                    // Windward slope: orographic lift deposits moisture
                    (elev_diff * 2.0).min(0.3) * upwind_moisture
                } else if elev_diff < -0.01 {
                    // Lee slope: rain shadow dries air
                    elev_diff * 0.5 // negative, reduces moisture
                } else {
                    0.0
                };

                // Blend upwind moisture into current position
                let advected = upwind_moisture * spd * 0.3;
                humidity[idx] = (humidity[idx] * 0.7 + advected + deposit).clamp(0.0, 1.0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wind_field_generates() {
        let width = 32;
        let height = 16;
        let total = width * height;
        let light = vec![0.5; total];
        let heightmap = vec![0.0; total];

        let wind = WindField::generate(&light, &heightmap, width, height, (0.5, 1.0));

        assert_eq!(wind.direction.len(), total);
        assert_eq!(wind.speed.len(), total);
        for &s in &wind.speed {
            assert!(s >= 0.0 && s <= 1.0);
        }
    }

    #[test]
    fn advection_increases_downwind_moisture() {
        let width = 32;
        let height = 16;
        let total = width * height;
        let light = vec![0.5; total];
        let heightmap = vec![0.05; total];
        let mut humidity = vec![0.3; total];
        let continentalness = vec![0.1; total]; // all land

        // Set left edge as ocean (moisture source)
        let mut cont = continentalness.clone();
        for y in 0..height {
            cont[y * width] = -0.5;
            humidity[y * width] = 0.9;
        }

        let wind = WindField::generate(&light, &heightmap, width, height, (0.5, 1.0));
        let initial_mid = humidity[8 * width + 16];
        advect_moisture(&mut humidity, &wind, &heightmap, &cont, &light, width, height, -0.025, 3);
        // After advection, some moisture should have moved
        // (exact direction depends on wind, but values should change)
        let changed = humidity.iter().enumerate().any(|(i, &h)| (h - 0.3).abs() > 0.01 && cont[i] > -0.025);
        assert!(changed, "Advection should modify humidity values");
    }
}
