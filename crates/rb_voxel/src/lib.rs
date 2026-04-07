//! Comanche-style voxel space terrain raycaster.
//!
//! Column-based heightmap renderer: for each screen column, cast a ray from near
//! to far over a 2D heightmap, draw vertical slices with front-to-back occlusion.
//! Produces the classic terrain-flight perspective (Comanche, Outcast, Delta Force).
//!
//! This is NOT Minecraft-style block voxels -- it is a heightmap column renderer.
//!
//! # Usage
//!
//! ```ignore
//! use rb_voxel::{render_frame, Camera, CameraMode, RenderConfig};
//!
//! let mut output = vec![0u8; 4 * screen_width * screen_height];
//! render_frame(
//!     &heightmap, &colormap, map_width, map_height,
//!     &camera, &config, &mut output, screen_width, screen_height,
//! );
//! ```
//!
//! The output is a flat RGBA buffer. The caller wraps it in whatever rendering
//! framework they use (Bevy `Image`, raw PNG, etc.).

use rayon::prelude::*;

/// Camera mode: first-person or third-person.
#[derive(Debug, Clone, Copy)]
pub enum CameraMode {
    /// Camera at eye level, looking in the yaw direction.
    FirstPerson,
    /// Camera behind and above a target point.
    ThirdPerson {
        /// Distance behind the target along the yaw axis.
        distance: f64,
        /// Downward pitch angle in radians (0 = horizontal, positive = look down).
        pitch: f64,
    },
}

/// Camera state for the raycaster.
#[derive(Debug, Clone)]
pub struct Camera {
    /// X position in map coordinates.
    pub x: f64,
    /// Y position in map coordinates.
    pub y: f64,
    /// Absolute height (terrain height * height_scale + offset).
    pub height: f64,
    /// Yaw angle in radians (0 = east, PI/2 = north).
    pub yaw: f64,
    /// Pitch angle in radians (0 = horizontal, positive = look up).
    pub pitch: f64,
    /// Horizontal field of view in radians (default ~PI/3 = 60 degrees).
    pub fov: f64,
    /// Maximum ray march distance in map units.
    pub draw_distance: f64,
    /// Camera mode.
    pub mode: CameraMode,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            height: 100.0,
            yaw: 0.0,
            pitch: 0.0,
            fov: std::f64::consts::FRAC_PI_3,
            draw_distance: 600.0,
            mode: CameraMode::FirstPerson,
        }
    }
}

/// Rendering configuration.
#[derive(Debug, Clone)]
pub struct RenderConfig {
    /// Multiplier from heightmap values to world height units.
    pub height_scale: f64,
    /// Sky/fog color (RGB).
    pub fog_color: [u8; 3],
    /// Height offset above the terrain surface.
    pub camera_height_offset: f64,
    /// Ray step size (smaller = more detail, slower). Default 1.0.
    pub ray_step: f64,
}

impl Default for RenderConfig {
    fn default() -> Self {
        Self {
            height_scale: 200.0,
            fog_color: [135, 170, 210],
            camera_height_offset: 50.0,
            ray_step: 1.0,
        }
    }
}

/// Sample the heightmap with bilinear interpolation.
///
/// Coordinates are in map space. Out-of-bounds samples clamp to the edge.
#[inline]
fn sample_heightmap(
    heightmap: &[f64],
    map_width: usize,
    map_height: usize,
    x: f64,
    y: f64,
) -> f64 {
    let max_x = (map_width as f64) - 1.0;
    let max_y = (map_height as f64) - 1.0;

    let cx = x.clamp(0.0, max_x);
    let cy = y.clamp(0.0, max_y);

    let x0 = cx.floor() as usize;
    let y0 = cy.floor() as usize;
    let x1 = (x0 + 1).min(map_width - 1);
    let y1 = (y0 + 1).min(map_height - 1);

    let fx = cx - cx.floor();
    let fy = cy - cy.floor();

    let v00 = heightmap[y0 * map_width + x0];
    let v10 = heightmap[y0 * map_width + x1];
    let v01 = heightmap[y1 * map_width + x0];
    let v11 = heightmap[y1 * map_width + x1];

    let top = v00 * (1.0 - fx) + v10 * fx;
    let bot = v01 * (1.0 - fx) + v11 * fx;

    top * (1.0 - fy) + bot * fy
}

/// Sample the colormap with bilinear interpolation, returning (R, G, B, A).
#[inline]
fn sample_colormap(
    colormap: &[u8],
    map_width: usize,
    map_height: usize,
    x: f64,
    y: f64,
) -> [u8; 4] {
    let max_x = (map_width as f64) - 1.0;
    let max_y = (map_height as f64) - 1.0;

    let cx = x.clamp(0.0, max_x);
    let cy = y.clamp(0.0, max_y);

    let x0 = cx.floor() as usize;
    let y0 = cy.floor() as usize;
    let x1 = (x0 + 1).min(map_width - 1);
    let y1 = (y0 + 1).min(map_height - 1);

    let fx = cx - cx.floor();
    let fy = cy - cy.floor();

    let idx = |px: usize, py: usize| (py * map_width + px) * 4;

    let i00 = idx(x0, y0);
    let i10 = idx(x1, y0);
    let i01 = idx(x0, y1);
    let i11 = idx(x1, y1);

    let mut result = [0u8; 4];
    for c in 0..4 {
        let v00 = colormap[i00 + c] as f64;
        let v10 = colormap[i10 + c] as f64;
        let v01 = colormap[i01 + c] as f64;
        let v11 = colormap[i11 + c] as f64;

        let top = v00 * (1.0 - fx) + v10 * fx;
        let bot = v01 * (1.0 - fx) + v11 * fx;
        let val = top * (1.0 - fy) + bot * fy;

        result[c] = val.round().clamp(0.0, 255.0) as u8;
    }

    result
}

/// Apply distance fog: lerp the color toward fog_color by the fog factor.
#[inline]
fn apply_fog(color: [u8; 4], fog_color: [u8; 3], distance: f64, draw_distance: f64) -> [u8; 4] {
    // Quadratic fog curve for a more natural falloff
    let t = (distance / draw_distance).clamp(0.0, 1.0);
    let fog = t * t;

    [
        lerp_u8(color[0], fog_color[0], fog),
        lerp_u8(color[1], fog_color[1], fog),
        lerp_u8(color[2], fog_color[2], fog),
        color[3],
    ]
}

#[inline]
fn lerp_u8(a: u8, b: u8, t: f64) -> u8 {
    let result = (a as f64) * (1.0 - t) + (b as f64) * t;
    result.round().clamp(0.0, 255.0) as u8
}

/// Compute the effective camera position and look direction, accounting for
/// camera mode (first-person vs third-person).
fn effective_camera(camera: &Camera) -> (f64, f64, f64, f64, f64) {
    match camera.mode {
        CameraMode::FirstPerson => (camera.x, camera.y, camera.height, camera.yaw, camera.pitch),
        CameraMode::ThirdPerson { distance, pitch } => {
            // Camera is behind and above the target point (camera.x, camera.y).
            let cam_x = camera.x - camera.yaw.cos() * distance;
            let cam_y = camera.y - camera.yaw.sin() * distance;
            let cam_height = camera.height + distance * pitch.sin();
            // Look toward the target, so yaw stays the same. Pitch is downward.
            (cam_x, cam_y, cam_height, camera.yaw, -pitch)
        }
    }
}

/// Render a single frame of the Comanche-style voxel terrain.
///
/// # Arguments
///
/// * `heightmap` - Flat f64 array of height values, row-major, dimensions `map_width * map_height`.
/// * `colormap` - Flat RGBA u8 array, same dimensions as heightmap (`map_width * map_height * 4`).
/// * `map_width` - Width of the heightmap/colormap in pixels.
/// * `map_height` - Height of the heightmap/colormap in pixels.
/// * `camera` - Camera position, orientation, and settings.
/// * `config` - Render configuration (height scale, fog, etc.).
/// * `output` - Mutable RGBA output buffer, `screen_width * screen_height * 4` bytes.
/// * `screen_width` - Output image width in pixels.
/// * `screen_height` - Output image height in pixels.
///
/// # Panics
///
/// Panics if buffer sizes do not match the stated dimensions.
pub fn render_frame(
    heightmap: &[f64],
    colormap: &[u8],
    map_width: usize,
    map_height: usize,
    camera: &Camera,
    config: &RenderConfig,
    output: &mut [u8],
    screen_width: usize,
    screen_height: usize,
) {
    assert_eq!(
        heightmap.len(),
        map_width * map_height,
        "heightmap length must be map_width * map_height"
    );
    assert_eq!(
        colormap.len(),
        map_width * map_height * 4,
        "colormap length must be map_width * map_height * 4 (RGBA)"
    );
    assert_eq!(
        output.len(),
        screen_width * screen_height * 4,
        "output length must be screen_width * screen_height * 4 (RGBA)"
    );

    let (cam_x, cam_y, cam_height, cam_yaw, cam_pitch) = effective_camera(camera);
    let half_fov = camera.fov / 2.0;
    let half_screen_h = screen_height as f64 / 2.0;
    let draw_distance = camera.draw_distance;
    let step = config.ray_step;
    let height_scale = config.height_scale;
    let fog_color = config.fog_color;

    // Pitch offset in screen pixels: positive pitch shifts the horizon down
    // (looking up reveals more sky).
    let pitch_offset = cam_pitch.tan() * half_screen_h;

    // We will collect column slices and render them in parallel.
    // Each column is independent -- embarrassingly parallel.

    // Split the output buffer into per-column slices.
    // Since the output is row-major RGBA, we cannot get contiguous per-column slices.
    // Instead, we collect per-column draw commands, then write them.

    // Strategy: compute draw commands per column in parallel, then write.
    // A draw command is (screen_y_start, screen_y_end, color) per segment.
    // But for simplicity and performance, we write directly into the output buffer
    // using atomic-free column-independent indexing.

    // Since output is row-major, pixel (x, y) is at offset (y * screen_width + x) * 4.
    // Each column only writes to its own x coordinate across all rows, so no data races.

    // First, fill entire output with fog/sky color.
    for pixel in output.chunks_exact_mut(4) {
        pixel[0] = fog_color[0];
        pixel[1] = fog_color[1];
        pixel[2] = fog_color[2];
        pixel[3] = 255;
    }

    // We use a raw pointer (stored as usize) to allow parallel mutable access
    // to non-overlapping columns. Each column writes only to pixels at its own
    // x coordinate across all rows, so no two parallel iterations share bytes.
    let output_ptr = output.as_mut_ptr() as usize;
    let output_len = output.len();

    // Block size = 1.0 — each heightmap coordinate is one block
    let block_size = 1.0_f64;

    (0..screen_width).into_par_iter().for_each(|col| {
        let ptr = output_ptr as *mut u8;

        // Ray angle for this column
        let col_fraction = col as f64 / screen_width as f64;
        let ray_angle = cam_yaw - half_fov + col_fraction * camera.fov;
        let ray_dx = ray_angle.cos();
        let ray_dy = ray_angle.sin();

        // Correct for fisheye: the perpendicular distance for height projection
        let cos_correction = (ray_angle - cam_yaw).cos();

        let mut max_drawn_y = screen_height; // bottom of the screen (we draw upward)
        let mut prev_block_height = f64::MIN;

        let mut distance = 1.0_f64;
        while distance < draw_distance {
            let sample_x = cam_x + ray_dx * distance;
            let sample_y = cam_y + ray_dy * distance;

            // Sample terrain height and quantize to blocks
            let raw_height = sample_heightmap(heightmap, map_width, map_height, sample_x, sample_y);
            let block_height = (raw_height * height_scale / block_size).floor() * block_size;

            // Perpendicular distance to avoid fisheye distortion
            let perp_dist = distance * cos_correction;
            if perp_dist <= 0.0 {
                distance += step;
                continue;
            }

            let height_diff = block_height - cam_height;
            let screen_y_f =
                half_screen_h - (height_diff * half_screen_h) / perp_dist - pitch_offset;
            let screen_y = screen_y_f as isize;
            let draw_top = screen_y.max(0) as usize;

            if draw_top < max_drawn_y {
                // Sample base color from colormap
                let base_color =
                    sample_colormap(colormap, map_width, map_height, sample_x, sample_y);

                // Minecraft-style face shading with top/side color distinction
                let is_side_face = block_height > prev_block_height + 0.5;

                // Fog factor for this distance (applied to all pixels in the strip)
                let fog_t = (distance / draw_distance).clamp(0.0, 1.0);
                let fog_t2 = fog_t * fog_t; // quadratic falloff

                if is_side_face {
                    // SIDE FACE: draw individual block layers within the cliff.
                    // Each block_size unit of height gets its own row with a visible edge.
                    // Walk from draw_top to max_drawn_y, computing which block layer
                    // each screen row belongs to, and shade accordingly.
                    for row in draw_top..max_drawn_y {
                        // Reverse-project screen row to world height at this distance
                        let row_height = cam_height + (half_screen_h - row as f64 - pitch_offset) * perp_dist / half_screen_h;
                        let block_layer = (row_height / block_size).floor() as i64;

                        // Alternate block layers between two shades for visibility
                        let layer_bright = if block_layer % 2 == 0 { 0.6 } else { 0.7 };

                        // Edge between block layers: dark line at the boundary
                        let frac_in_block = ((row_height / block_size) - (row_height / block_size).floor()).abs();
                        let on_edge = frac_in_block < 0.08 || frac_in_block > 0.92;
                        let edge_mult = if on_edge { 0.4 } else { 1.0 };

                        // Side color: darken the biome color
                        let shade = layer_bright * edge_mult;
                        let r = lerp_u8(base_color[0], fog_color[0], fog_t2);
                        let g = lerp_u8(base_color[1], fog_color[1], fog_t2);
                        let b = lerp_u8(base_color[2], fog_color[2], fog_t2);

                        let offset = (row * screen_width + col) * 4;
                        debug_assert!(offset + 3 < output_len);
                        unsafe {
                            *ptr.add(offset) = (r as f64 * shade) as u8;
                            *ptr.add(offset + 1) = (g as f64 * shade) as u8;
                            *ptr.add(offset + 2) = (b as f64 * shade) as u8;
                            *ptr.add(offset + 3) = 255;
                        }
                    }
                } else {
                    // TOP FACE: biome color with fog
                    let r = lerp_u8(base_color[0], fog_color[0], fog_t2);
                    let g = lerp_u8(base_color[1], fog_color[1], fog_t2);
                    let b = lerp_u8(base_color[2], fog_color[2], fog_t2);

                    for row in draw_top..max_drawn_y {
                        let offset = (row * screen_width + col) * 4;
                        debug_assert!(offset + 3 < output_len);
                        unsafe {
                            *ptr.add(offset) = r;
                            *ptr.add(offset + 1) = g;
                            *ptr.add(offset + 2) = b;
                            *ptr.add(offset + 3) = 255;
                        }
                    }
                };
                max_drawn_y = draw_top;
            }

            prev_block_height = block_height;

            // If the entire column is filled, stop marching
            if max_drawn_y == 0 {
                break;
            }

            // Adaptive step: coarser at distance for performance
            distance += step + distance * 0.005;
        }
    });
}

/// Convenience: sample the terrain height at a camera position and return
/// the appropriate camera height (terrain * scale + offset).
pub fn terrain_height_at(
    heightmap: &[f64],
    map_width: usize,
    map_height: usize,
    x: f64,
    y: f64,
    height_scale: f64,
    offset: f64,
) -> f64 {
    sample_heightmap(heightmap, map_width, map_height, x, y) * height_scale + offset
}

/// Draw a simple 3D player placeholder (vertical capsule/pillar) at a world
/// position onto the already-rendered output buffer. Call this AFTER `render_frame`.
///
/// The player is rendered as a colored vertical bar projected into the scene
/// using the same perspective math as the terrain. In third-person mode, the
/// player appears at `(player_x, player_y)` — the camera's logical position.
pub fn draw_player_marker(
    heightmap: &[f64],
    map_width: usize,
    map_height: usize,
    player_x: f64,
    player_y: f64,
    camera: &Camera,
    config: &RenderConfig,
    output: &mut [u8],
    screen_width: usize,
    screen_height: usize,
    player_color: [u8; 3],
    player_height: f64, // world units tall (e.g., 3.0)
) {
    // Only draw in third-person (in first-person you ARE the player)
    if matches!(camera.mode, CameraMode::FirstPerson) {
        return;
    }

    let (cam_x, cam_y, cam_height, cam_yaw, cam_pitch) = effective_camera(camera);
    let half_fov = camera.fov / 2.0;
    let half_screen_h = screen_height as f64 / 2.0;
    let pitch_offset = cam_pitch.tan() * half_screen_h;

    // Vector from camera to player
    let dx = player_x - cam_x;
    let dy = player_y - cam_y;
    let distance = (dx * dx + dy * dy).sqrt();
    if distance < 1.0 {
        return; // Too close, skip
    }

    // Angle from camera to player
    let angle_to_player = dy.atan2(dx);

    // Relative angle within the FOV
    let mut rel_angle = angle_to_player - cam_yaw;
    // Normalize to [-PI, PI]
    while rel_angle > std::f64::consts::PI {
        rel_angle -= 2.0 * std::f64::consts::PI;
    }
    while rel_angle < -std::f64::consts::PI {
        rel_angle += 2.0 * std::f64::consts::PI;
    }

    // Check if player is within FOV
    if rel_angle.abs() > half_fov {
        return;
    }

    // Screen X column
    let screen_x = ((rel_angle + half_fov) / camera.fov * screen_width as f64) as isize;
    if screen_x < 0 || screen_x >= screen_width as isize {
        return;
    }
    let col = screen_x as usize;

    // Perpendicular distance (for correct projection, avoiding fisheye)
    let cos_correction = rel_angle.cos();
    let perp_dist = distance * cos_correction;
    if perp_dist <= 0.0 {
        return;
    }

    // Terrain height at player position
    let terrain_h = sample_heightmap(heightmap, map_width, map_height, player_x, player_y)
        * config.height_scale;

    // Project feet (terrain surface) and head (terrain + player_height)
    let feet_diff = terrain_h - cam_height;
    let head_diff = (terrain_h + player_height) - cam_height;

    let feet_screen_y =
        (half_screen_h - (feet_diff * half_screen_h) / perp_dist - pitch_offset) as isize;
    let head_screen_y =
        (half_screen_h - (head_diff * half_screen_h) / perp_dist - pitch_offset) as isize;

    let top = head_screen_y.max(0) as usize;
    let bottom = feet_screen_y.min(screen_height as isize).max(0) as usize;
    if top >= bottom {
        return;
    }

    // Apply distance fog to player color
    let fogged = apply_fog(
        [player_color[0], player_color[1], player_color[2], 255],
        config.fog_color,
        distance,
        camera.draw_distance,
    );

    // Draw a 3-pixel-wide vertical bar (the "player") with a brighter center column
    let col_start = if col >= 1 { col - 1 } else { 0 };
    let col_end = (col + 2).min(screen_width);

    for row in top..bottom {
        for c in col_start..col_end {
            let offset = (row * screen_width + c) * 4;
            if offset + 3 < output.len() {
                // Center column is full brightness, sides are slightly darker
                let brightness = if c == col { 1.0 } else { 0.7 };
                output[offset] = (fogged[0] as f64 * brightness) as u8;
                output[offset + 1] = (fogged[1] as f64 * brightness) as u8;
                output[offset + 2] = (fogged[2] as f64 * brightness) as u8;
                output[offset + 3] = 255;
            }
        }
    }

    // Draw a small "head" circle (5x5 pixels) at the top
    let head_center_y = top;
    let head_radius: isize = 2;
    for dy_px in -head_radius..=head_radius {
        for dx_px in -head_radius..=head_radius {
            if dx_px * dx_px + dy_px * dy_px <= head_radius * head_radius {
                let px = col as isize + dx_px;
                let py = head_center_y as isize + dy_px;
                if px >= 0 && px < screen_width as isize && py >= 0 && py < screen_height as isize {
                    let offset = (py as usize * screen_width + px as usize) * 4;
                    if offset + 3 < output.len() {
                        output[offset] = fogged[0];
                        output[offset + 1] = fogged[1];
                        output[offset + 2] = fogged[2];
                        output[offset + 3] = 255;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: usize = 64;
    const H: usize = 64;
    const SW: usize = 320;
    const SH: usize = 240;

    fn flat_heightmap(val: f64) -> Vec<f64> {
        vec![val; W * H]
    }

    fn solid_colormap(r: u8, g: u8, b: u8) -> Vec<u8> {
        let mut cm = vec![0u8; W * H * 4];
        for i in 0..W * H {
            cm[i * 4] = r;
            cm[i * 4 + 1] = g;
            cm[i * 4 + 2] = b;
            cm[i * 4 + 3] = 255;
        }
        cm
    }

    fn default_camera_at(x: f64, y: f64, height: f64) -> Camera {
        Camera {
            x,
            y,
            height,
            yaw: 0.0,
            pitch: 0.0,
            fov: std::f64::consts::FRAC_PI_3,
            draw_distance: 200.0,
            mode: CameraMode::FirstPerson,
        }
    }

    #[test]
    fn flat_terrain_uniform_row() {
        // A flat heightmap at value 0.2 with height_scale 200 = world height 40.
        // Camera at height 100, looking east. All terrain columns should project
        // to the same screen Y band.
        let hm = flat_heightmap(0.2);
        let cm = solid_colormap(100, 150, 50);
        let config = RenderConfig {
            height_scale: 200.0,
            fog_color: [135, 170, 210],
            camera_height_offset: 50.0,
            ray_step: 1.0,
        };
        let camera = default_camera_at(32.0, 32.0, 100.0);
        let mut output = vec![0u8; SW * SH * 4];

        render_frame(&hm, &cm, W, H, &camera, &config, &mut output, SW, SH);

        // Find the topmost drawn (non-fog) row for several columns.
        // They should all be the same because the terrain is flat.
        let fog = config.fog_color;
        let mut top_rows = Vec::new();
        for col in [SW / 4, SW / 2, 3 * SW / 4] {
            let mut top = SH;
            for row in 0..SH {
                let off = (row * SW + col) * 4;
                if output[off] != fog[0] || output[off + 1] != fog[1] || output[off + 2] != fog[2]
                {
                    top = row;
                    break;
                }
            }
            top_rows.push(top);
        }

        // Allow a tolerance of 2 pixels for edge-column ray angle differences
        let min_top = *top_rows.iter().min().unwrap();
        let max_top = *top_rows.iter().max().unwrap();
        assert!(
            max_top - min_top <= 2,
            "Flat terrain should produce a uniform horizon line, but got top_rows: {:?}",
            top_rows
        );
        // Terrain should be drawn somewhere (not all fog)
        assert!(
            min_top < SH,
            "Terrain was never drawn -- entire screen is fog"
        );
    }

    #[test]
    fn peak_rises_above_flat_baseline() {
        // Flat terrain at 0.0 with a single peak at (32, 32).
        let mut hm = flat_heightmap(0.0);
        // Place a 3x3 peak centered at (32, 32)
        for dy in -1i32..=1 {
            for dx in -1i32..=1 {
                let px = (32 + dx) as usize;
                let py = (32 + dy) as usize;
                hm[py * W + px] = 0.5;
            }
        }
        let cm = solid_colormap(100, 50, 50);
        let config = RenderConfig {
            height_scale: 200.0,
            fog_color: [135, 170, 210],
            camera_height_offset: 30.0,
            ray_step: 0.5,
        };
        // Camera west of the peak looking east, at a height above the flat but below the peak
        let flat_height = 0.0 * config.height_scale + config.camera_height_offset;
        let camera = default_camera_at(10.0, 32.0, flat_height);
        let mut output = vec![0u8; SW * SH * 4];

        render_frame(&hm, &cm, W, H, &camera, &config, &mut output, SW, SH);

        // The center column (SW/2) should show the peak: its topmost drawn row
        // should be HIGHER (smaller Y) than a column off to the side.
        let fog = config.fog_color;

        let find_top = |col: usize| -> usize {
            for row in 0..SH {
                let off = (row * SW + col) * 4;
                if output[off] != fog[0] || output[off + 1] != fog[1] || output[off + 2] != fog[2]
                {
                    return row;
                }
            }
            SH
        };

        let center_top = find_top(SW / 2);
        // Check an off-center column that should only see flat terrain
        let side_top = find_top(SW / 8);

        assert!(
            center_top < side_top || (center_top < SH && side_top == SH),
            "Peak column should be drawn higher than flat column. center_top={}, side_top={}",
            center_top,
            side_top
        );
    }

    #[test]
    fn fog_at_max_distance() {
        // Flat terrain, long draw distance. The farthest-drawn pixels should be
        // very close to the fog color.
        let hm = flat_heightmap(0.3);
        let cm = solid_colormap(255, 0, 0); // bright red terrain
        let fog = [135, 170, 210];
        let config = RenderConfig {
            height_scale: 200.0,
            fog_color: fog,
            camera_height_offset: 30.0,
            ray_step: 1.0,
        };
        let camera = default_camera_at(32.0, 32.0, 90.0);
        let mut output = vec![0u8; SW * SH * 4];

        render_frame(&hm, &cm, W, H, &camera, &config, &mut output, SW, SH);

        // Find the topmost drawn row in the center column -- that represents
        // the farthest distance (just at the horizon). Its color should be
        // close to the fog color.
        let col = SW / 2;
        let mut top_row = SH;
        for row in 0..SH {
            let off = (row * SW + col) * 4;
            if output[off] != fog[0] || output[off + 1] != fog[1] || output[off + 2] != fog[2] {
                top_row = row;
                break;
            }
        }

        if top_row < SH {
            let off = (top_row * SW + col) * 4;
            let r = output[off];
            let g = output[off + 1];
            let b = output[off + 2];

            // The farthest visible terrain should be heavily fogged --
            // close to the fog color (within 30 per channel).
            let dr = (r as i32 - fog[0] as i32).unsigned_abs();
            let dg = (g as i32 - fog[1] as i32).unsigned_abs();
            let db = (b as i32 - fog[2] as i32).unsigned_abs();

            assert!(
                dr <= 30 && dg <= 30 && db <= 30,
                "Farthest terrain pixel should be close to fog color. \
                 Got ({},{},{}) vs fog ({},{},{}), deltas ({},{},{})",
                r,
                g,
                b,
                fog[0],
                fog[1],
                fog[2],
                dr,
                dg,
                db
            );
        }
    }

    #[test]
    fn third_person_renders_without_panic() {
        let hm = flat_heightmap(0.1);
        let cm = solid_colormap(80, 120, 80);
        let config = RenderConfig::default();
        let camera = Camera {
            x: 32.0,
            y: 32.0,
            height: 100.0,
            yaw: 0.0,
            pitch: 0.0,
            fov: std::f64::consts::FRAC_PI_3,
            draw_distance: 200.0,
            mode: CameraMode::ThirdPerson {
                distance: 20.0,
                pitch: 0.5,
            },
        };
        let mut output = vec![0u8; SW * SH * 4];

        // Should not panic
        render_frame(&hm, &cm, W, H, &camera, &config, &mut output, SW, SH);

        // Verify some terrain was drawn (not all fog)
        let fog = config.fog_color;
        let has_terrain = output.chunks(4).any(|px| {
            px[0] != fog[0] || px[1] != fog[1] || px[2] != fog[2]
        });
        assert!(has_terrain, "Third-person mode should render some terrain");
    }

    #[test]
    fn bilinear_interpolation_midpoint() {
        // 2x2 heightmap: corners at 0, 1, 0, 1
        let hm = vec![0.0, 1.0, 0.0, 1.0];
        let val = sample_heightmap(&hm, 2, 2, 0.5, 0.5);
        // Bilinear at center of a 2x2 with values 0,1,0,1 = 0.5
        assert!(
            (val - 0.5).abs() < 1e-10,
            "Bilinear midpoint should be 0.5, got {}",
            val
        );
    }

    #[test]
    fn terrain_height_at_helper() {
        let hm = flat_heightmap(0.5);
        let h = terrain_height_at(&hm, W, H, 32.0, 32.0, 200.0, 10.0);
        assert!(
            (h - 110.0).abs() < 1e-10,
            "terrain_height_at should return 0.5*200+10=110, got {}",
            h
        );
    }
}
