//! Voxel preview: render a sine-wave heightmap to a PNG file.
//!
//! Usage:
//!   cargo run --release -p rb_voxel --example voxel_preview
//!
//! Outputs `voxel_preview.png` in the current directory.

use rb_voxel::{render_frame, terrain_height_at, Camera, CameraMode, RenderConfig};
use std::time::Instant;

const MAP_W: usize = 1024;
const MAP_H: usize = 1024;
const SCREEN_W: usize = 1280;
const SCREEN_H: usize = 720;

fn main() {
    println!("Generating {}x{} sine-wave heightmap...", MAP_W, MAP_H);

    // Generate a sine-wave heightmap with multiple frequencies for visual interest.
    let mut heightmap = vec![0.0f64; MAP_W * MAP_H];
    for y in 0..MAP_H {
        for x in 0..MAP_W {
            let fx = x as f64 / MAP_W as f64;
            let fy = y as f64 / MAP_H as f64;

            // Layered sine waves for rolling hills
            let h = 0.3
                + 0.15 * (fx * 4.0 * std::f64::consts::PI).sin()
                    * (fy * 3.0 * std::f64::consts::PI).sin()
                + 0.08 * (fx * 11.0 + 0.7).sin() * (fy * 9.0 + 1.3).sin()
                + 0.04 * (fx * 23.0 + 2.1).sin() * (fy * 19.0 + 0.5).sin();

            heightmap[y * MAP_W + x] = h.clamp(0.0, 1.0);
        }
    }

    // Generate a colormap: green lowlands fading to brown highlands, blue for low areas.
    let mut colormap = vec![0u8; MAP_W * MAP_H * 4];
    for y in 0..MAP_H {
        for x in 0..MAP_W {
            let h = heightmap[y * MAP_W + x];
            let idx = (y * MAP_W + x) * 4;

            let (r, g, b) = if h < 0.15 {
                // Deep water
                (30, 60, 120)
            } else if h < 0.22 {
                // Shallow water
                (50, 100, 150)
            } else if h < 0.28 {
                // Beach / sand
                (194, 178, 128)
            } else if h < 0.38 {
                // Lowland green
                (80, 140, 60)
            } else if h < 0.48 {
                // Highland green-brown transition
                let t = (h - 0.38) / 0.10;
                (
                    (80.0 + t * 60.0) as u8,
                    (140.0 - t * 50.0) as u8,
                    (60.0 - t * 20.0) as u8,
                )
            } else if h < 0.55 {
                // Mountain brown
                (140, 100, 50)
            } else {
                // Snow caps
                let t = ((h - 0.55) / 0.15).clamp(0.0, 1.0);
                (
                    (140.0 + t * 100.0) as u8,
                    (100.0 + t * 140.0) as u8,
                    (50.0 + t * 190.0) as u8,
                )
            };

            colormap[idx] = r;
            colormap[idx + 1] = g;
            colormap[idx + 2] = b;
            colormap[idx + 3] = 255;
        }
    }

    let config = RenderConfig {
        height_scale: 200.0,
        fog_color: [160, 190, 220],
        camera_height_offset: 60.0,
        ray_step: 0.5,
    };

    // Camera position: center-left of the map, looking east
    let cam_x = MAP_W as f64 * 0.25;
    let cam_y = MAP_H as f64 * 0.5;
    let cam_height = terrain_height_at(
        &heightmap,
        MAP_W,
        MAP_H,
        cam_x,
        cam_y,
        config.height_scale,
        config.camera_height_offset,
    );

    let camera = Camera {
        x: cam_x,
        y: cam_y,
        height: cam_height,
        yaw: 0.3, // slightly northeast
        pitch: 0.05,
        fov: std::f64::consts::FRAC_PI_3,
        draw_distance: 600.0,
        mode: CameraMode::FirstPerson,
    };

    let mut output = vec![0u8; SCREEN_W * SCREEN_H * 4];

    println!(
        "Rendering {}x{} frame (draw_distance={})...",
        SCREEN_W, SCREEN_H, camera.draw_distance
    );

    let start = Instant::now();
    render_frame(
        &heightmap, &colormap, MAP_W, MAP_H, &camera, &config, &mut output, SCREEN_W, SCREEN_H,
    );
    let elapsed = start.elapsed();

    println!("Render time: {:.1}ms", elapsed.as_secs_f64() * 1000.0);
    println!(
        "FPS equivalent: {:.0}",
        1.0 / elapsed.as_secs_f64()
    );

    // Save as PNG
    let img = image::RgbaImage::from_raw(SCREEN_W as u32, SCREEN_H as u32, output)
        .expect("Failed to create image from output buffer");
    img.save("voxel_preview.png")
        .expect("Failed to save voxel_preview.png");
    println!("Saved voxel_preview.png");

    // Also render a third-person view
    let camera_3p = Camera {
        x: cam_x + 40.0,
        y: cam_y,
        height: cam_height + 20.0,
        yaw: 0.3,
        pitch: 0.0,
        fov: std::f64::consts::FRAC_PI_3,
        draw_distance: 600.0,
        mode: CameraMode::ThirdPerson {
            distance: 50.0,
            pitch: 0.4,
        },
    };

    let mut output_3p = vec![0u8; SCREEN_W * SCREEN_H * 4];

    let start = Instant::now();
    render_frame(
        &heightmap,
        &colormap,
        MAP_W,
        MAP_H,
        &camera_3p,
        &config,
        &mut output_3p,
        SCREEN_W,
        SCREEN_H,
    );
    let elapsed = start.elapsed();
    println!(
        "Third-person render time: {:.1}ms ({:.0} FPS)",
        elapsed.as_secs_f64() * 1000.0,
        1.0 / elapsed.as_secs_f64()
    );

    let img_3p = image::RgbaImage::from_raw(SCREEN_W as u32, SCREEN_H as u32, output_3p)
        .expect("Failed to create image from output buffer");
    img_3p
        .save("voxel_preview_3p.png")
        .expect("Failed to save voxel_preview_3p.png");
    println!("Saved voxel_preview_3p.png");
}
