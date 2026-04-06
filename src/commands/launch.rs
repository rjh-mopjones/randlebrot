//! Launch a playable level with Comanche-style 3D terrain rendering.
//!
//! `randlebrot launch <level-tag>` opens a Bevy window using `rb_voxel` to
//! render the heightmap + colormap as a 2.5D terrain flyover. Surrounding
//! chunks stream in as the player moves, stitching into a contiguous terrain
//! buffer consumed by the raycaster each frame.
//!
//! Controls:
//!   WASD            — move (forward/back/strafe relative to camera yaw)
//!   Mouse           — look (yaw + pitch)
//!   V               — toggle first-person / third-person camera
//!   Scroll wheel    — adjust draw distance
//!   M               — toggle world map overlay
//!   ESC             — exit

use std::collections::HashSet;
use std::sync::Arc;

use bevy::app::AppExit;
use bevy::image::{ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::input::mouse::{MouseMotion, MouseScrollUnit, MouseWheel};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::tasks::{AsyncComputeTaskPool, Task, block_on, poll_once};
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};
use bevy_egui::{egui, EguiContexts};

use rb_artifacts::ArtifactStore;
use rb_core::{PlayableLevel, WorldPos};
use rb_noise::{BiomeMap, NoiseBackend, NoiseLayer};
use rb_voxel::{Camera as VoxelCamera, CameraMode as VoxelCameraMode, RenderConfig, render_frame, terrain_height_at};

use crate::cli::coords::{
    chunk_coord_to_world_pos, CHUNK_WORLD_SIZE, WORLD_HEIGHT, WORLD_WIDTH,
};

// ─── Constants ─────────────────────────────────────────────────────────────

/// Output resolution per chunk BiomeMap (512x512 pixels).
const TILE_MAP_SIZE: usize = 512;

/// Screen width for the voxel renderer.
const SCREEN_WIDTH: usize = 1280;

/// Screen height for the voxel renderer.
const SCREEN_HEIGHT: usize = 720;

/// Terrain buffer size (square). Each loaded chunk contributes a 512x512 region.
/// A 4096x4096 buffer holds 8x8 = 64 chunks worth of terrain data.
const TERRAIN_BUF_SIZE: usize = 4096;

/// Level chunk load radius (in chunks around the camera).
const LEVEL_LOAD_RADIUS: i32 = 3;

/// Level chunk unload radius.
const LEVEL_UNLOAD_RADIUS: i32 = 5;

/// Shared render config — single source of truth for height_scale, fog, etc.
const RENDER_CONFIG: RenderConfig = RenderConfig {
    height_scale: 200.0,
    fog_color: [135, 180, 220],
    camera_height_offset: 2.0,
    ray_step: 1.0,
};

/// Max concurrent async tile generation tasks.
const MAX_CONCURRENT_TILES: usize = 8;

/// Max tile completions to process per frame.
const POLL_BUDGET: usize = 8;

/// Player movement speed in world units per second.
const MOVE_SPEED: f64 = 5.0;

/// Mouse look sensitivity.
const MOUSE_SENSITIVITY: f64 = 0.002;

/// Minimum draw distance.
const MIN_DRAW_DISTANCE: f64 = 100.0;

/// Maximum draw distance.
const MAX_DRAW_DISTANCE: f64 = 800.0;

/// Default draw distance.
const DEFAULT_DRAW_DISTANCE: f64 = 400.0;

/// Pitch clamp (±60 degrees in radians).
const MAX_PITCH: f64 = std::f64::consts::FRAC_PI_3;

// ─── Entry Point ───────────────────────────────────────────────────────────

/// On macOS Sequoia, terminal-launched binaries cannot gain keyboard focus
/// regardless of NSApplication activation calls. The only reliable fix is to
/// run inside a .app bundle launched via `open`. This function creates a
/// minimal bundle in /tmp, copies the current binary into it, and re-execs
/// via `open --args`. The re-launched process detects the bundle env var and
/// skips the trampoline.
#[cfg(target_os = "macos")]
pub(crate) fn macos_ensure_app_bundle(args: &[String]) {
    let exe = std::env::current_exe().expect("current_exe");

    // If we're already inside a .app bundle, do nothing
    if exe.to_string_lossy().contains(".app/Contents/MacOS/") {
        return;
    }
    let app_dir = std::path::PathBuf::from("/tmp/Randlebrot.app/Contents/MacOS");
    let plist_path = std::path::PathBuf::from("/tmp/Randlebrot.app/Contents/Info.plist");

    // Create bundle structure
    std::fs::create_dir_all(&app_dir).expect("create .app dirs");

    // Copy binary (only if changed)
    let dest = app_dir.join("randlebrot");
    let needs_copy = if dest.exists() {
        let src_meta = std::fs::metadata(&exe).ok();
        let dst_meta = std::fs::metadata(&dest).ok();
        match (src_meta, dst_meta) {
            (Some(s), Some(d)) => s.len() != d.len(),
            _ => true,
        }
    } else {
        true
    };
    if needs_copy {
        std::fs::copy(&exe, &dest).expect("copy binary to .app bundle");
    }

    // Write Info.plist
    if !plist_path.exists() {
        std::fs::write(&plist_path, r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key><string>randlebrot</string>
    <key>CFBundleIdentifier</key><string>com.randlebrot.engine</string>
    <key>CFBundleName</key><string>Randlebrot</string>
    <key>CFBundleVersion</key><string>0.1.0</string>
    <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>"#).expect("write Info.plist");
    }

    // Re-launch via `open` — skip argv[0] since the bundle provides its own binary
    eprintln!("[macOS] Trampolining through .app bundle for keyboard focus...");
    let mut cmd = std::process::Command::new("open");
    cmd.arg("/tmp/Randlebrot.app")
        .arg("--args");
    for arg in args.iter().skip(1) {
        cmd.arg(arg);
    }
    let status = cmd.status().expect("open .app bundle");
    std::process::exit(status.code().unwrap_or(0));
}

/// Params passed into the Bevy app for deferred loading in the startup system.
#[derive(Resource)]
struct LaunchParams {
    level_tag: String,
}

/// Launch a playable level using the Comanche-style 3D terrain renderer.
pub fn run(level_tag: String) -> Result<(), String> {
    // On macOS: re-launch inside a .app bundle so we get proper GUI app status.
    // Without this, macOS Sequoia refuses to give keyboard focus to terminal-launched binaries.
    #[cfg(target_os = "macos")]
    macos_ensure_app_bundle(&std::env::args().collect::<Vec<_>>());

    // ─── Only validate the tag exists — keep this fast so app.run() starts immediately ───
    let store = ArtifactStore::new()
        .map_err(|e| format!("failed to initialise artifact store at ~/.randlebrot: {e}"))?;

    if !store.exists(rb_artifacts::ArtifactKind::Levels, &level_tag) {
        match store.list_levels() {
            Ok(entries) if !entries.is_empty() => {
                let available: Vec<&str> = entries.iter().map(|(t, _)| t.as_str()).collect();
                return Err(format!(
                    "level artifact '{level_tag}' not found. Available: {}",
                    available.join(", ")
                ));
            }
            _ => {
                return Err(format!(
                    "level artifact '{level_tag}' not found. \
                     Run `randlebrot generate level <layers-tag|--seed N> <x,y> <tag>` to create one."
                ));
            }
        }
    }

    // ─── Build Bevy app IMMEDIATELY — window must appear before heavy loading ───
    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: format!("Randlebrot - Playing: {level_tag}"),
            resolution: (SCREEN_WIDTH as u32, SCREEN_HEIGHT as u32).into(),
            focused: true,
            ..default()
        }),
        ..default()
    }));

    app.add_plugins(bevy_egui::EguiPlugin {
        enable_multipass_for_primary_context: false,
        ..Default::default()
    });

    // Only insert the tag — everything else loads in the startup system
    app.insert_resource(LaunchParams { level_tag });

    app.add_systems(Startup, (deferred_load_and_setup, grab_cursor));
    // Gameplay systems only run after deferred_load_and_setup has inserted resources
    let loaded = resource_exists::<PlayableLevel>;
    app.add_systems(
        Update,
        (
            camera_input_system.run_if(loaded),
            chunk_streaming_system.run_if(loaded),
            chunk_poll_system.run_if(loaded),
            chunk_unload_system.run_if(loaded),
            voxel_render_system.run_if(loaded),
            toggle_map_overlay.run_if(loaded),
            update_map_player_marker.run_if(loaded),
            map_overlay_zoom.run_if(loaded),
            launch_hud_system.run_if(loaded),
            fps_update_system.run_if(loaded),
            exit_on_esc,
            exit_on_window_close,
        ),
    );

    app.run();
    Ok(())
}

// ─── Parent Layers Loading ─────────────────────────────────────────────────

/// Load the parent layers artifact (macro BiomeMap + RiverNetwork) for context.
fn load_parent_layers(
    store: &ArtifactStore,
    manifest: &rb_artifacts::LevelManifest,
    seed: u32,
) -> Result<(BiomeMap, Option<Arc<rb_noise::RiverNetwork>>), String> {
    if let Some(ref parent_tag) = manifest.parent_layers_tag {
        match store.load_layers_data(parent_tag) {
            Ok((mut biome_map, river_network, _lifegen)) => {
                let river_arc = Arc::new(river_network);
                biome_map.river_network = Some(river_arc.clone());
                println!("Loaded parent layers artifact '{parent_tag}'");
                return Ok((biome_map, Some(river_arc)));
            }
            Err(e) => {
                println!(
                    "Warning: could not load parent layers '{parent_tag}': {e}\n\
                     Regenerating macro data from seed {seed}..."
                );
            }
        }
    }

    println!("Generating macro BiomeMap from seed {seed} (this may take a moment)...");
    let biome_map = BiomeMap::generate_with_backend(
        seed,
        WORLD_WIDTH,
        WORLD_HEIGHT,
        NoiseBackend::Cpu,
    );
    let river_network = biome_map.river_network.clone();
    println!("Macro BiomeMap generated");
    Ok((biome_map, river_network))
}

/// Load the biome.png from the parent layers, or render one from the macro BiomeMap.
fn load_or_render_map_image(
    store: &ArtifactStore,
    manifest: &rb_artifacts::LevelManifest,
    macro_biome: &Arc<BiomeMap>,
) -> Option<(u32, u32, Vec<u8>)> {
    if let Some(ref parent_tag) = manifest.parent_layers_tag {
        let biome_path = store.layer_image_path(parent_tag, "biome.png");
        if biome_path.exists() {
            match image::open(&biome_path) {
                Ok(img) => {
                    let rgba = img.to_rgba8();
                    let (w, h) = rgba.dimensions();
                    println!("Loaded map image from '{parent_tag}' ({w}x{h})");
                    return Some((w, h, rgba.into_raw()));
                }
                Err(e) => {
                    println!("Warning: failed to load biome.png from '{parent_tag}': {e}");
                }
            }
        }
    }

    let image_data = macro_biome.to_layer_image(NoiseLayer::Biome);
    let w = macro_biome.width as u32;
    let h = macro_biome.height as u32;
    if image_data.len() == (w * h * 4) as usize {
        println!("Rendered map image from macro BiomeMap ({w}x{h})");
        Some((w, h, image_data))
    } else {
        None
    }
}

// ─── Resources ─────────────────────────────────────────────────────────────

/// Macro BiomeMap for generating chunks on the fly.
#[derive(Resource)]
struct LaunchMacroBiomeData {
    biome_map: Arc<BiomeMap>,
}

/// Global river network for consistent rivers across zoom levels.
#[derive(Resource)]
struct LaunchRiverNetwork {
    network: Arc<rb_noise::RiverNetwork>,
}

/// Queue of in-flight level chunk generation tasks.
#[derive(Resource, Default)]
struct LaunchLevelChunkQueue {
    in_flight: Vec<LaunchLevelChunkTask>,
}

/// An in-flight level chunk generation task.
struct LaunchLevelChunkTask {
    coord: (i32, i32),
    task: Task<((i32, i32), ChunkTerrainData)>,
}

/// Extracted terrain data from a generated chunk: heightmap + colormap.
struct ChunkTerrainData {
    heightmap: Vec<f64>,
    colormap: Vec<u8>,
    width: usize,
    height: usize,
}

/// The contiguous terrain buffer fed to rb_voxel each frame.
#[derive(Resource)]
struct TerrainBuffer {
    /// Heightmap values (f64), size x size.
    heightmap: Vec<f64>,
    /// RGBA colormap, size x size x 4.
    colormap: Vec<u8>,
    /// Side length of the square buffer.
    size: usize,
}

impl TerrainBuffer {
    fn new(size: usize) -> Self {
        Self {
            heightmap: vec![0.0; size * size],
            colormap: vec![0u8; size * size * 4],
            size,
        }
    }

    /// Copy a chunk's terrain data into the buffer at the given pixel offset.
    fn blit_chunk(
        &mut self,
        chunk_data: &ChunkTerrainData,
        buf_x: usize,
        buf_y: usize,
    ) {
        let cw = chunk_data.width;
        let ch = chunk_data.height;
        let bs = self.size;

        for row in 0..ch {
            let dst_y = buf_y + row;
            if dst_y >= bs {
                break;
            }
            for col in 0..cw {
                let dst_x = buf_x + col;
                if dst_x >= bs {
                    break;
                }

                let src_idx = row * cw + col;
                let dst_idx = dst_y * bs + dst_x;

                self.heightmap[dst_idx] = chunk_data.heightmap[src_idx];

                let src_rgba = src_idx * 4;
                let dst_rgba = dst_idx * 4;
                self.colormap[dst_rgba..dst_rgba + 4]
                    .copy_from_slice(&chunk_data.colormap[src_rgba..src_rgba + 4]);
            }
        }
    }
}

/// Voxel camera state (wraps rb_voxel::Camera).
#[derive(Resource)]
struct VoxelCameraState {
    camera: VoxelCamera,
}

/// Player's position in world coordinates.
#[derive(Resource)]
struct PlayerWorldPos {
    x: f64,
    y: f64,
}

/// World coordinate that maps to buffer pixel (0,0).
#[derive(Resource)]
struct BufferOrigin {
    world_x: f64,
    world_y: f64,
}

/// Tracks which chunks are loaded (by chunk coord) and their buffer-pixel offsets.
#[derive(Resource, Default)]
struct LoadedTerrainChunks {
    /// Maps chunk coord -> buffer pixel offset (bx, by) where the chunk was blitted.
    chunks: std::collections::HashMap<(i32, i32), (usize, usize)>,
}

/// Map overlay state (toggled with M key).
#[derive(Resource)]
struct MapOverlayState {
    visible: bool,
}

impl Default for MapOverlayState {
    fn default() -> Self {
        Self { visible: false }
    }
}

/// Pre-loaded map image data for the overlay.
#[derive(Resource)]
struct MapImageData {
    width: u32,
    height: u32,
    rgba_data: Vec<u8>,
    world_width: f32,
    world_height: f32,
}

/// General launch state info for the HUD.
#[derive(Resource)]
struct LaunchState {
    level_tag: String,
    chunk_coord: (i32, i32),
}

/// FPS counter.
#[derive(Resource)]
struct FpsCounter {
    frame_count: u32,
    elapsed: f64,
    fps: f64,
}

impl Default for FpsCounter {
    fn default() -> Self {
        Self {
            frame_count: 0,
            elapsed: 0.0,
            fps: 0.0,
        }
    }
}

/// Pre-allocated RGBA output buffer for the voxel renderer (reused each frame).
#[derive(Resource)]
struct RenderOutputBuffer(Vec<u8>);

/// Marker for the fullscreen sprite that displays the voxel-rendered frame.
#[derive(Component)]
struct VoxelDisplaySprite;

/// Marker for the map overlay sprite.
#[derive(Component)]
struct MapOverlaySprite;

/// Marker for the player position dot on the map overlay.
#[derive(Component)]
struct MapPlayerMarker;

/// Marker for map overlay entities.
#[derive(Component)]
struct MapOverlayEntity;

// ─── Startup ───────────────────────────────────────────────────────────────

/// Deferred loading — runs as a startup system AFTER the window is created and focused.
/// This is where all the heavy I/O (level artifact, parent layers, map image) happens.
fn deferred_load_and_setup(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    params: Res<LaunchParams>,
) {
    let store = ArtifactStore::new().expect("artifact store should be available");
    let (initial_biome, level_manifest) = store
        .load_level(&params.level_tag)
        .expect("level artifact should exist (validated before app.run)");

    let (world_x, world_y) = chunk_coord_to_world_pos(level_manifest.chunk_coord);
    let seed = level_manifest.seed;

    eprintln!(
        "Loading level '{}': seed={seed}, coord=({},{})",
        params.level_tag, level_manifest.chunk_coord.0, level_manifest.chunk_coord.1,
    );

    // Load parent layers
    let (macro_biome, river_network) =
        load_parent_layers(&store, &level_manifest, seed)
            .expect("parent layers should load");

    let macro_biome_arc = Arc::new(macro_biome);
    let river_network_arc = river_network;

    // Load map overlay
    let map_image_data = load_or_render_map_image(&store, &level_manifest, &macro_biome_arc);

    let origin = WorldPos::new(world_x, world_y);
    let chunk_x = (world_x / 64.0).floor() as i32;
    let chunk_y = (world_y / 64.0).floor() as i32;

    // Insert all resources
    commands.insert_resource(PlayableLevel {
        origin,
        chunk_coord: (chunk_x, chunk_y),
        seed,
        world_height: WORLD_HEIGHT as f64,
    });
    commands.insert_resource(LaunchMacroBiomeData {
        biome_map: macro_biome_arc,
    });
    if let Some(net) = river_network_arc {
        commands.insert_resource(LaunchRiverNetwork { network: net });
    }
    commands.insert_resource(LaunchLevelChunkQueue::default());
    commands.insert_resource(MapOverlayState::default());
    commands.insert_resource(LaunchState {
        level_tag: params.level_tag.clone(),
        chunk_coord: level_manifest.chunk_coord,
    });
    commands.insert_resource(FpsCounter::default());

    // Terrain buffer — prime with initial chunk
    let mut terrain_buffer = TerrainBuffer::new(TERRAIN_BUF_SIZE);
    {
        let initial_data = ChunkTerrainData {
            heightmap: initial_biome.heightmap.clone(),
            colormap: initial_biome.to_layer_image(NoiseLayer::Biome),
            width: initial_biome.width,
            height: initial_biome.height,
        };
        let center = TERRAIN_BUF_SIZE / 2 - TILE_MAP_SIZE / 2;
        terrain_buffer.blit_chunk(&initial_data, center, center);
    }
    commands.insert_resource(terrain_buffer);

    commands.insert_resource(RenderOutputBuffer(vec![0u8; SCREEN_WIDTH * SCREEN_HEIGHT * 4]));

    let voxel_camera = VoxelCamera {
        x: (TERRAIN_BUF_SIZE / 2) as f64,
        y: (TERRAIN_BUF_SIZE / 2) as f64,
        height: 100.0,
        yaw: 0.0,
        pitch: 0.0,
        fov: std::f64::consts::FRAC_PI_3,
        draw_distance: DEFAULT_DRAW_DISTANCE,
        mode: VoxelCameraMode::ThirdPerson { distance: 30.0, pitch: 0.5 },
    };
    commands.insert_resource(VoxelCameraState { camera: voxel_camera });

    commands.insert_resource(PlayerWorldPos { x: world_x, y: world_y });

    commands.insert_resource(BufferOrigin {
        world_x: world_x - (TERRAIN_BUF_SIZE as f64 / (2.0 * TILE_MAP_SIZE as f64)) * CHUNK_WORLD_SIZE,
        world_y: world_y - (TERRAIN_BUF_SIZE as f64 / (2.0 * TILE_MAP_SIZE as f64)) * CHUNK_WORLD_SIZE,
    });

    commands.insert_resource(LoadedTerrainChunks::default());

    // ─── Spawn visual entities ─────────────────────────────────────────
    // Spawn a 2D camera for the fullscreen sprite
    commands.spawn(Camera2d);

    // Create the fullscreen sprite for voxel rendering output.
    // Initial image is filled with fog color.
    let mut initial_data = vec![0u8; SCREEN_WIDTH * SCREEN_HEIGHT * 4];
    for pixel in initial_data.chunks_exact_mut(4) {
        pixel[0] = RENDER_CONFIG.fog_color[0];
        pixel[1] = RENDER_CONFIG.fog_color[1];
        pixel[2] = RENDER_CONFIG.fog_color[2];
        pixel[3] = 255;
    }

    let image = create_image(SCREEN_WIDTH, SCREEN_HEIGHT, initial_data);
    let texture = images.add(image);

    commands.spawn((
        Sprite {
            image: texture,
            custom_size: Some(Vec2::new(SCREEN_WIDTH as f32, SCREEN_HEIGHT as f32)),
            ..default()
        },
        Transform::from_xyz(0.0, 0.0, 0.0),
        VoxelDisplaySprite,
    ));

    // Create the map overlay sprite (initially hidden)
    if let Some((map_w, map_h, map_rgba)) = map_image_data {
        // Insert MapImageData resource for other systems (overlay toggle, marker)
        commands.insert_resource(MapImageData {
            width: map_w,
            height: map_h,
            rgba_data: map_rgba.clone(),
            world_width: WORLD_WIDTH as f32,
            world_height: WORLD_HEIGHT as f32,
        });

        let map_image = create_image(map_w as usize, map_h as usize, map_rgba);
        let map_texture = images.add(map_image);

        commands.spawn((
            Sprite {
                image: map_texture,
                custom_size: Some(Vec2::new(map_w as f32, map_h as f32)),
                color: Color::srgba(1.0, 1.0, 1.0, 0.85),
                ..default()
            },
            Transform::from_xyz(0.0, 0.0, 50.0),
            Visibility::Hidden,
            MapOverlaySprite,
            MapOverlayEntity,
        ));

        // Player position marker (red dot on the map)
        commands.spawn((
            Sprite {
                color: Color::srgb(1.0, 0.1, 0.1),
                custom_size: Some(Vec2::new(12.0, 12.0)),
                ..default()
            },
            Transform::from_xyz(0.0, 0.0, 51.0),
            Visibility::Hidden,
            MapPlayerMarker,
            MapOverlayEntity,
        ));
    }
}

// ─── Camera Input ──────────────────────────────────────────────────────────

/// Process WASD + mouse look + V toggle + scroll wheel.
fn camera_input_system(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut motion_events: MessageReader<MouseMotion>,
    mut scroll_events: MessageReader<MouseWheel>,
    time: Res<Time>,
    mut camera_state: ResMut<VoxelCameraState>,
    mut player_pos: ResMut<PlayerWorldPos>,
    buf_origin: Res<BufferOrigin>,
    terrain_buf: Res<TerrainBuffer>,
    map_state: Res<MapOverlayState>,
    mut contexts: EguiContexts,
) {
    let dt = time.delta_secs() as f64;
    let cam = &mut camera_state.camera;

    // Force egui to release keyboard/pointer focus so WASD and mouse always work.
    // The HUD is non-interactive (.interactable(false)) but egui can still claim
    // focus in some configurations. This is the nuclear option.
    if let Ok(ctx) = contexts.ctx_mut() {
        ctx.memory_mut(|mem| mem.surrender_focus(egui::Id::NULL));
    }

    // ─── Mouse look ────────────────────────────────────────────────
    if !map_state.visible {
        let mut dx = 0.0_f64;
        let mut dy = 0.0_f64;
        for event in motion_events.read() {
            dx += event.delta.x as f64;
            dy += event.delta.y as f64;
        }

        cam.yaw -= dx * MOUSE_SENSITIVITY;
        cam.pitch = (cam.pitch - dy * MOUSE_SENSITIVITY).clamp(-MAX_PITCH, MAX_PITCH);
    } else {
        // Drain motion events so they don't accumulate
        motion_events.clear();
    }

    // ─── WASD movement ─────────────────────────────────────────────
    {
        // DEBUG: log any key presses to diagnose input issues
        let any_pressed = keyboard.pressed(KeyCode::KeyW)
            || keyboard.pressed(KeyCode::KeyS)
            || keyboard.pressed(KeyCode::KeyA)
            || keyboard.pressed(KeyCode::KeyD);
        if any_pressed {
            eprintln!("[INPUT] WASD detected: W={} S={} A={} D={}",
                keyboard.pressed(KeyCode::KeyW),
                keyboard.pressed(KeyCode::KeyS),
                keyboard.pressed(KeyCode::KeyA),
                keyboard.pressed(KeyCode::KeyD),
            );
        }
        if keyboard.just_pressed(KeyCode::KeyV) {
            eprintln!("[INPUT] V pressed — toggling camera mode");
        }
        if keyboard.just_pressed(KeyCode::Escape) {
            eprintln!("[INPUT] ESC pressed");
        }

        let mut forward = 0.0_f64;
        let mut strafe = 0.0_f64;

        if keyboard.pressed(KeyCode::KeyW) {
            forward += 1.0;
        }
        if keyboard.pressed(KeyCode::KeyS) {
            forward -= 1.0;
        }
        if keyboard.pressed(KeyCode::KeyA) {
            strafe -= 1.0;
        }
        if keyboard.pressed(KeyCode::KeyD) {
            strafe += 1.0;
        }

        if forward != 0.0 || strafe != 0.0 {
            // Normalize diagonal movement
            let len = (forward * forward + strafe * strafe).sqrt();
            forward /= len;
            strafe /= len;

            let speed = MOVE_SPEED * dt;

            // Move in world space relative to yaw
            let dx = cam.yaw.cos() * forward - cam.yaw.sin() * strafe;
            let dy = cam.yaw.sin() * forward + cam.yaw.cos() * strafe;

            player_pos.x += dx * speed;
            player_pos.y += dy * speed;
        }
    }

    // ─── V key: toggle camera mode ─────────────────────────────────
    if keyboard.just_pressed(KeyCode::KeyV) {
        cam.mode = match cam.mode {
            VoxelCameraMode::FirstPerson => VoxelCameraMode::ThirdPerson {
                distance: 30.0,
                pitch: 0.5,
            },
            VoxelCameraMode::ThirdPerson { .. } => VoxelCameraMode::FirstPerson,
        };
    }

    // ─── Scroll wheel: adjust draw distance ────────────────────────
    if !map_state.visible {
        for event in scroll_events.read() {
            let delta = match event.unit {
                MouseScrollUnit::Line => event.y as f64 * 20.0,
                MouseScrollUnit::Pixel => event.y as f64 * 2.0,
            };
            cam.draw_distance = (cam.draw_distance + delta).clamp(MIN_DRAW_DISTANCE, MAX_DRAW_DISTANCE);
        }
    } else {
        // Don't consume scroll events here when map is visible --
        // they are handled by map_overlay_zoom
    }

    // ─── Update camera buffer position from world position ─────────
    let pixels_per_world = TILE_MAP_SIZE as f64 / CHUNK_WORLD_SIZE;
    cam.x = (player_pos.x - buf_origin.world_x) * pixels_per_world;
    cam.y = (player_pos.y - buf_origin.world_y) * pixels_per_world;

    // ─── Auto-follow terrain height ────────────────────────────────
    cam.height = terrain_height_at(
        &terrain_buf.heightmap,
        terrain_buf.size,
        terrain_buf.size,
        cam.x,
        cam.y,
        RENDER_CONFIG.height_scale,
        RENDER_CONFIG.camera_height_offset,
    );
}

// ─── Chunk Streaming ───────────────────────────────────────────────────────

/// Queue generation for chunks around the camera.
fn chunk_streaming_system(
    level: Res<PlayableLevel>,
    player_pos: Res<PlayerWorldPos>,
    loaded_chunks: Res<LoadedTerrainChunks>,
    mut queue: ResMut<LaunchLevelChunkQueue>,
    world_textures: Res<LaunchMacroBiomeData>,
    global_rivers: Option<Res<LaunchRiverNetwork>>,
) {
    let seed = level.seed;
    let height = level.world_height;
    let river_net = global_rivers.map(|r| r.network.clone());

    // Determine which chunk the camera is in
    let cam_chunk_x = (player_pos.x / CHUNK_WORLD_SIZE).floor() as i32;
    let cam_chunk_y = (player_pos.y / CHUNK_WORLD_SIZE).floor() as i32;

    let in_flight_coords: HashSet<(i32, i32)> = queue
        .in_flight
        .iter()
        .map(|t| t.coord)
        .collect();

    for dy in -LEVEL_LOAD_RADIUS..=LEVEL_LOAD_RADIUS {
        for dx in -LEVEL_LOAD_RADIUS..=LEVEL_LOAD_RADIUS {
            let cx = cam_chunk_x + dx;
            let cy = cam_chunk_y + dy;
            let coord = (cx, cy);

            // Skip out-of-bounds chunks
            if cx < 0 || cy < 0 || cx >= (WORLD_WIDTH as i32) || cy >= (WORLD_HEIGHT as i32) {
                continue;
            }

            if loaded_chunks.chunks.contains_key(&coord) || in_flight_coords.contains(&coord) {
                continue;
            }

            if queue.in_flight.len() >= MAX_CONCURRENT_TILES {
                return;
            }

            let world_x = cx as f64 * CHUNK_WORLD_SIZE;
            let world_y = cy as f64 * CHUNK_WORLD_SIZE;

            let macro_map = world_textures.biome_map.clone();
            let river_net_clone = river_net.clone();
            let task = AsyncComputeTaskPool::get().spawn(async move {
                let river_ref = river_net_clone.as_ref();
                let biome_map = BiomeMap::generate_meso_full_with_backend(
                    seed,
                    world_x,
                    world_y,
                    CHUNK_WORLD_SIZE,
                    TILE_MAP_SIZE,
                    height,
                    3, // micro detail level
                    None,
                    NoiseBackend::Cpu,
                    Some(&macro_map),
                    river_ref,
                );

                // Extract heightmap and colormap from the generated BiomeMap
                let heightmap = biome_map.heightmap.clone();
                let colormap = biome_map.to_layer_image(NoiseLayer::Biome);

                let data = ChunkTerrainData {
                    heightmap,
                    colormap,
                    width: biome_map.width,
                    height: biome_map.height,
                };
                (coord, data)
            });

            queue
                .in_flight
                .push(LaunchLevelChunkTask { coord, task });
        }
    }
}

/// Poll completed chunk tasks and blit terrain data into the buffer.
fn chunk_poll_system(
    mut queue: ResMut<LaunchLevelChunkQueue>,
    mut loaded_chunks: ResMut<LoadedTerrainChunks>,
    mut terrain_buf: ResMut<TerrainBuffer>,
    buf_origin: Res<BufferOrigin>,
) {
    let mut completed = 0;
    let mut i = 0;
    while i < queue.in_flight.len() && completed < POLL_BUDGET {
        if let Some(result) = block_on(poll_once(&mut queue.in_flight[i].task)) {
            queue.in_flight.swap_remove(i);
            let (coord, chunk_data) = result;

            // Convert chunk world position to buffer pixel position
            let world_x = coord.0 as f64 * CHUNK_WORLD_SIZE;
            let world_y = coord.1 as f64 * CHUNK_WORLD_SIZE;
            let pixels_per_world = TILE_MAP_SIZE as f64 / CHUNK_WORLD_SIZE;
            let buf_x = ((world_x - buf_origin.world_x) * pixels_per_world) as isize;
            let buf_y = ((world_y - buf_origin.world_y) * pixels_per_world) as isize;

            // Only blit if within buffer bounds
            if buf_x >= 0
                && buf_y >= 0
                && (buf_x as usize) < terrain_buf.size
                && (buf_y as usize) < terrain_buf.size
            {
                terrain_buf.blit_chunk(&chunk_data, buf_x as usize, buf_y as usize);
                loaded_chunks
                    .chunks
                    .insert(coord, (buf_x as usize, buf_y as usize));
            }

            completed += 1;
        } else {
            i += 1;
        }
    }
}

/// Unload chunks beyond the unload radius.
fn chunk_unload_system(
    player_pos: Res<PlayerWorldPos>,
    mut loaded_chunks: ResMut<LoadedTerrainChunks>,
) {
    let cam_chunk_x = (player_pos.x / CHUNK_WORLD_SIZE).floor() as i32;
    let cam_chunk_y = (player_pos.y / CHUNK_WORLD_SIZE).floor() as i32;

    let to_remove: Vec<(i32, i32)> = loaded_chunks
        .chunks
        .keys()
        .filter(|(cx, cy)| {
            let dx = (cx - cam_chunk_x).abs();
            let dy = (cy - cam_chunk_y).abs();
            dx > LEVEL_UNLOAD_RADIUS || dy > LEVEL_UNLOAD_RADIUS
        })
        .copied()
        .collect();

    for coord in to_remove {
        loaded_chunks.chunks.remove(&coord);
    }
}

// ─── Voxel Rendering ──────────────────────────────────────────────────────

/// Render the voxel frame and update the fullscreen sprite texture.
fn voxel_render_system(
    camera_state: Res<VoxelCameraState>,
    terrain_buf: Res<TerrainBuffer>,
    mut render_buf: ResMut<RenderOutputBuffer>,
    mut images: ResMut<Assets<Image>>,
    sprite_query: Query<&Sprite, With<VoxelDisplaySprite>>,
) {
    let Ok(sprite) = sprite_query.single() else {
        return;
    };

    // Render into the pre-allocated buffer (no per-frame allocation)
    render_frame(
        &terrain_buf.heightmap,
        &terrain_buf.colormap,
        terrain_buf.size,
        terrain_buf.size,
        &camera_state.camera,
        &RENDER_CONFIG,
        &mut render_buf.0,
        SCREEN_WIDTH,
        SCREEN_HEIGHT,
    );

    // Copy into the existing image data buffer (preserves Bevy's allocation)
    let handle = &sprite.image;
    if let Some(image) = images.get_mut(handle) {
        if let Some(ref mut data) = image.data {
            data.copy_from_slice(&render_buf.0);
        }
    }
}

// ─── Map Overlay ───────────────────────────────────────────────────────────

/// Toggle the map overlay on/off with the M key.
fn toggle_map_overlay(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut state: ResMut<MapOverlayState>,
    mut overlay_query: Query<&mut Visibility, With<MapOverlaySprite>>,
    mut marker_query: Query<
        &mut Visibility,
        (With<MapPlayerMarker>, Without<MapOverlaySprite>),
    >,
    mut map_sprite_query: Query<
        &mut Transform,
        (With<MapOverlaySprite>, Without<MapPlayerMarker>),
    >,
    map_data: Option<Res<MapImageData>>,
) {
    if !keyboard.just_pressed(KeyCode::KeyM) {
        return;
    }

    state.visible = !state.visible;

    let new_vis = if state.visible {
        Visibility::Visible
    } else {
        Visibility::Hidden
    };

    for mut vis in &mut overlay_query {
        *vis = new_vis;
    }
    for mut vis in &mut marker_query {
        *vis = new_vis;
    }

    // Center the map overlay and scale it
    if state.visible {
        if let Some(map_data) = map_data {
            for mut transform in &mut map_sprite_query {
                transform.translation.x = 0.0;
                transform.translation.y = 0.0;
                let display_width = 800.0;
                let scale = display_width / map_data.width as f32;
                transform.scale = Vec3::splat(scale);
            }
        }
    }
}

/// Update the player position marker on the map overlay.
fn update_map_player_marker(
    state: Res<MapOverlayState>,
    player_pos: Res<PlayerWorldPos>,
    map_data: Option<Res<MapImageData>>,
    mut marker_query: Query<
        &mut Transform,
        (With<MapPlayerMarker>, Without<MapOverlaySprite>),
    >,
    overlay_query: Query<
        &Transform,
        (With<MapOverlaySprite>, Without<MapPlayerMarker>),
    >,
) {
    if !state.visible {
        return;
    }
    let Some(map_data) = map_data else { return };
    let Ok(overlay_transform) = overlay_query.single() else { return };
    let Ok(mut marker_transform) = marker_query.single_mut() else { return };

    // Normalize player world position to [0,1]
    let norm_x = (player_pos.x / map_data.world_width as f64) as f32;
    let norm_y = (player_pos.y / map_data.world_height as f64) as f32;

    let overlay_scale = overlay_transform.scale.x;
    let map_pixel_x = (norm_x - 0.5) * map_data.width as f32 * overlay_scale;
    let map_pixel_y = (0.5 - norm_y) * map_data.height as f32 * overlay_scale;

    marker_transform.translation.x = overlay_transform.translation.x + map_pixel_x;
    marker_transform.translation.y = overlay_transform.translation.y + map_pixel_y;
    marker_transform.translation.z = 51.0;

    let marker_scale = overlay_scale * 1.5;
    marker_transform.scale = Vec3::splat(marker_scale);
}

/// Scroll-wheel zoom on the map overlay (when visible).
fn map_overlay_zoom(
    state: Res<MapOverlayState>,
    mut scroll_events: MessageReader<MouseWheel>,
    mut overlay_query: Query<
        &mut Transform,
        (With<MapOverlaySprite>, Without<MapPlayerMarker>),
    >,
) {
    if !state.visible {
        scroll_events.clear();
        return;
    }

    let mut scroll_delta = 0.0;
    for event in scroll_events.read() {
        scroll_delta += match event.unit {
            MouseScrollUnit::Line => event.y * 0.1,
            MouseScrollUnit::Pixel => event.y * 0.001,
        };
    }

    if scroll_delta == 0.0 {
        return;
    }

    for mut transform in &mut overlay_query {
        let zoom_factor = 1.0 - scroll_delta;
        let current_scale = transform.scale.x;
        let new_scale = (current_scale * zoom_factor).clamp(0.01, 5.0);
        transform.scale = Vec3::splat(new_scale);
    }
}

// ─── HUD ───────────────────────────────────────────────────────────────────

/// Minimal HUD showing level info and controls.
fn launch_hud_system(
    mut contexts: EguiContexts,
    launch_state: Res<LaunchState>,
    map_state: Res<MapOverlayState>,
    camera_state: Res<VoxelCameraState>,
    player_pos: Res<PlayerWorldPos>,
    level: Res<PlayableLevel>,
    fps: Res<FpsCounter>,
) {
    let Ok(ctx) = contexts.ctx_mut() else { return };

    let cam = &camera_state.camera;
    let mode_str = match cam.mode {
        VoxelCameraMode::FirstPerson => "FP",
        VoxelCameraMode::ThirdPerson { .. } => "TP",
    };

    egui::Area::new(egui::Id::new("launch_hud"))
        .anchor(egui::Align2::LEFT_TOP, [8.0, 8.0])
        .interactable(false)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.label(format!("Level: {}", launch_state.level_tag));
                ui.label(format!(
                    "Coord: ({}, {})",
                    launch_state.chunk_coord.0, launch_state.chunk_coord.1
                ));
                ui.label(format!("Seed: {}", level.seed));
                ui.label(format!(
                    "Pos: ({:.1}, {:.1})",
                    player_pos.x, player_pos.y
                ));
                ui.label(format!("Camera: {}  Draw: {:.0}", mode_str, cam.draw_distance));
                ui.label(format!("FPS: {:.0}", fps.fps));
                if map_state.visible {
                    ui.label("Map: ON");
                }
                ui.separator();
                ui.small("WASD: move  Mouse: look  V: camera  M: map  Scroll: distance  ESC: exit");
            });
        });
}

/// Update FPS counter.
fn fps_update_system(
    time: Res<Time>,
    mut fps: ResMut<FpsCounter>,
) {
    fps.frame_count += 1;
    fps.elapsed += time.delta_secs() as f64;

    if fps.elapsed >= 1.0 {
        fps.fps = fps.frame_count as f64 / fps.elapsed;
        fps.frame_count = 0;
        fps.elapsed = 0.0;
    }
}

// ─── Exit ──────────────────────────────────────────────────────────────────

/// Exit on ESC key.
/// Grab the cursor AND force macOS to activate the app as a foreground GUI process.
/// Called as a Startup system AFTER winit has created the NSApplication and window.
fn grab_cursor(mut cursor_query: Query<&mut CursorOptions, With<PrimaryWindow>>) {
    // On macOS: force the process to become a foreground app with keyboard focus.
    // This MUST happen after winit creates the NSApplication (inside app.run()).
    #[cfg(target_os = "macos")]
    {
        unsafe {
            use std::ffi::CStr;
            let cls = objc2::runtime::AnyClass::get(
                CStr::from_bytes_with_nul(b"NSApplication\0").unwrap()
            );
            if let Some(cls) = cls {
                let shared_app: *mut objc2::runtime::AnyObject =
                    objc2::msg_send![cls, sharedApplication];
                if !shared_app.is_null() {
                    let _: () = objc2::msg_send![shared_app, setActivationPolicy: 0i64];
                    let _: () = objc2::msg_send![shared_app, activateIgnoringOtherApps: true];
                    eprintln!("[macOS] Activated as foreground app");
                } else {
                    eprintln!("[macOS] sharedApplication returned null!");
                }
            } else {
                eprintln!("[macOS] NSApplication class not found!");
            }
        }
    }

    if let Ok(mut cursor) = cursor_query.single_mut() {
        cursor.grab_mode = CursorGrabMode::Locked;
        cursor.visible = false;
        eprintln!("[cursor] Locked cursor grab mode");
    } else {
        eprintln!("[cursor] No PrimaryWindow found for cursor grab!");
    }
}

fn exit_on_esc(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut exit_events: MessageWriter<AppExit>,
    mut cursor_query: Query<&mut CursorOptions, With<PrimaryWindow>>,
) {
    if keyboard.just_pressed(KeyCode::Escape) {
        // Release cursor before exiting so the OS doesn't get confused
        if let Ok(mut cursor) = cursor_query.single_mut() {
            cursor.grab_mode = CursorGrabMode::None;
            cursor.visible = true;
        }
        exit_events.write(AppExit::Success);
    }
}

/// Exit when the window is closed.
fn exit_on_window_close(
    windows: Query<(), With<PrimaryWindow>>,
    mut exit_events: MessageWriter<AppExit>,
) {
    if windows.is_empty() {
        exit_events.write(AppExit::Success);
    }
}

// ─── Image Helper ──────────────────────────────────────────────────────────

/// Create a Bevy Image from RGBA data with nearest-neighbor sampling.
fn create_image(width: usize, height: usize, data: Vec<u8>) -> Image {
    let mut image = Image::new(
        Extent3d {
            width: width as u32,
            height: height as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        default(),
    );

    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        mag_filter: ImageFilterMode::Nearest,
        min_filter: ImageFilterMode::Nearest,
        ..default()
    });

    image
}
