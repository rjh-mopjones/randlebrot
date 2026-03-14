//! Horizontal wrapping utilities for cylindrical world projection.
//!
//! The world wraps horizontally (x-axis) but not vertically (y-axis).
//! These utilities compute wrapped coordinates and shortest-path distances.

/// Wrap an x-coordinate to [0, world_width).
#[inline]
pub fn wrap_x(x: f64, world_width: f64) -> f64 {
    let wrapped = x % world_width;
    if wrapped < 0.0 { wrapped + world_width } else { wrapped }
}

/// Compute shortest horizontal distance considering wrapping.
/// Inputs are normalized coordinates in [0, 1].
/// Returns a signed distance in [-0.5, 0.5].
#[inline]
pub fn wrapped_dx_normalized(dx: f64) -> f64 {
    if dx > 0.5 {
        dx - 1.0
    } else if dx < -0.5 {
        dx + 1.0
    } else {
        dx
    }
}

/// Wrap a D8 neighbor x-coordinate for grid algorithms.
/// Returns the wrapped x in [0, width), or the original if already in range.
#[inline]
pub fn wrap_grid_x(nx: i32, width: usize) -> i32 {
    let w = width as i32;
    ((nx % w) + w) % w
}

/// Compute 3D cylindrical noise coordinates for horizontal wrapping.
/// Maps x to a circle (cos/sin) so noise at x=0 equals noise at x=world_width.
/// Returns [cx, cz, cy] for use with 3D noise.
#[inline]
pub fn cylindrical_noise_coords(x: f64, y: f64, freq: f64, scale: f64, world_width: f64) -> [f64; 3] {
    let angle = std::f64::consts::TAU * x / world_width;
    let r = world_width * freq * scale / std::f64::consts::TAU;
    [angle.cos() * r, angle.sin() * r, y * freq * scale]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_x_basic() {
        assert!((wrap_x(0.0, 1024.0) - 0.0).abs() < 1e-10);
        assert!((wrap_x(1024.0, 1024.0) - 0.0).abs() < 1e-10);
        assert!((wrap_x(-1.0, 1024.0) - 1023.0).abs() < 1e-10);
        assert!((wrap_x(1025.0, 1024.0) - 1.0).abs() < 1e-10);
    }

    #[test]
    fn wrapped_dx_shortest_path() {
        assert!((wrapped_dx_normalized(0.1) - 0.1).abs() < 1e-10);
        assert!((wrapped_dx_normalized(-0.1) - -0.1).abs() < 1e-10);
        // 0.8 is farther than going the other way (-0.2)
        assert!((wrapped_dx_normalized(0.8) - -0.2).abs() < 1e-10);
        assert!((wrapped_dx_normalized(-0.8) - 0.2).abs() < 1e-10);
    }

    #[test]
    fn wrap_grid_x_wraps() {
        assert_eq!(wrap_grid_x(-1, 1024), 1023);
        assert_eq!(wrap_grid_x(1024, 1024), 0);
        assert_eq!(wrap_grid_x(0, 1024), 0);
        assert_eq!(wrap_grid_x(500, 1024), 500);
    }
}
