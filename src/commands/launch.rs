//! Launch a playable level with Minecraft-style 3D block terrain.
//!
//! `randlebrot launch <level-tag>` opens a Bevy 3D window. The derived
//! heightmap from each chunk's BiomeMap is converted into a block mesh
//! (one quad per visible face). At micro scale (detail_level=3), the
//! heightmap includes independently-normalized high-frequency detail via
//! `derive_micro_heightmap` in `rb_noise`. Chunks stream in as the player
//! moves.
//!
//! Controls:
//!   WASD            — move (forward/back/strafe relative to camera yaw)
//!   Mouse           — look (yaw + pitch)
//!   V               — toggle first-person / third-person camera
//!   Scroll wheel    — adjust render distance
//!   M               — toggle world map overlay
//!   Tab             — release/grab cursor
//!   ESC             — exit

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use bevy::app::AppExit;
use bevy::input::mouse::{MouseMotion, MouseScrollUnit, MouseWheel};
use bevy::prelude::*;
use bevy::mesh::Indices;
use bevy::render::view::screenshot::{save_to_disk, Screenshot};
use bevy::tasks::{AsyncComputeTaskPool, Task, block_on, poll_once};
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};
use bevy_egui::{egui, EguiContexts};

use rb_artifacts::ArtifactStore;
use rb_core::{PlayableLevel, WorldPos};
use rb_noise::{BiomeMap, NoiseBackend, NoiseLayer};

use crate::cli::coords::{
    chunk_coord_to_world_pos, CHUNK_WORLD_SIZE, WORLD_HEIGHT, WORLD_WIDTH,
};

// ─── Constants ─────────────────────────────────────────────────────────────

/// Output resolution per chunk BiomeMap.
const TILE_MAP_SIZE: usize = 512;

/// One block per BiomeMap pixel — full 512×512 resolution, no downsampling.
const BLOCKS_PER_CHUNK: usize = 512;
const BLOCK_WORLD_SIZE: f32 = 1.0;
const CHUNK_BEVY_SIZE: f32 = BLOCKS_PER_CHUNK as f32 * BLOCK_WORLD_SIZE;

/// Height scale: raw heightmap → block Y.
/// 128 gives ~10-60 blocks of relief depending on terrain type.
/// Mountain areas can span 0.5+ heightmap range → 64+ blocks of cliffs.
const HEIGHT_SCALE: f32 = 128.0;

/// Dirt layer colors (RGB, 0-255).
const DIRT_COLOR: [u8; 3] = [101, 67, 33];
/// Stone layer color (RGB, 0-255).
const STONE_COLOR: [u8; 3] = [120, 120, 120];

/// Keep load radius tiny — a single 512×512 chunk is already 262K blocks.
const LOAD_RADIUS: i32 = 2;

/// Chunk unload radius (must be > LOAD_RADIUS to avoid load/unload thrashing).
const UNLOAD_RADIUS: i32 = 3;

/// Max concurrent chunk generation tasks.
const MAX_CONCURRENT: usize = 4;

/// Player movement speed in Bevy units/sec. 1 Bevy unit = 1 block.
/// Minecraft walk speed is ~4.3 blocks/sec; 10.0 gives snappy exploration.
const MOVE_SPEED: f32 = 10.0;

/// Mouse look sensitivity (tuned for natural feel at ~1280x720).
const MOUSE_SENS: f32 = 0.003;

/// Pitch clamp (±60 degrees).
const MAX_PITCH: f32 = std::f32::consts::FRAC_PI_3;

/// Player eye height above terrain (in world units).
/// 1.7 blocks gives Minecraft-like eye position, and ensures we clear
/// blocks that are 1 block taller than the sampled ground center.
const EYE_HEIGHT: f32 = 1.7;

// ─── macOS .app bundle trampoline ──────────────────────────────────────────

#[cfg(target_os = "macos")]
pub(crate) fn macos_ensure_app_bundle(args: &[String]) {
    let exe = std::env::current_exe().expect("current_exe");
    if exe.to_string_lossy().contains(".app/Contents/MacOS/") {
        return;
    }
    let app_dir = std::path::PathBuf::from("/tmp/Randlebrot.app/Contents/MacOS");
    let plist_path = std::path::PathBuf::from("/tmp/Randlebrot.app/Contents/Info.plist");

    std::fs::create_dir_all(&app_dir).expect("create .app dirs");
    let dest = app_dir.join("randlebrot");
    std::fs::copy(&exe, &dest).expect("copy binary to .app bundle");

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

    eprintln!("[macOS] Trampolining through .app bundle for keyboard focus...");
    let mut cmd = std::process::Command::new("open");
    cmd.arg("--wait-apps")
        .arg("--stdout").arg("/tmp/randlebrot_launch.log")
        .arg("--stderr").arg("/tmp/randlebrot_launch.log")
        .arg("/tmp/Randlebrot.app")
        .arg("--args");
    for arg in args.iter().skip(1) {
        cmd.arg(arg);
    }
    let status = cmd.status().expect("open .app bundle");
    if !status.success() {
        if let Ok(log) = std::fs::read_to_string("/tmp/randlebrot_launch.log") {
            if !log.is_empty() { eprintln!("{}", log); }
        }
        eprintln!("Launch failed (exit code {})", status.code().unwrap_or(-1));
    }
    std::process::exit(status.code().unwrap_or(1));
}

// ─── Entry Point ───────────────────────────────────────────────────────────

pub fn run(level_tag: String, flythrough: bool) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    if !flythrough {
        macos_ensure_app_bundle(&std::env::args().collect::<Vec<_>>());
    }

    // Validate tag exists
    let store = ArtifactStore::new()
        .map_err(|e| format!("artifact store: {e}"))?;
    if !store.exists(rb_artifacts::ArtifactKind::Levels, &level_tag) {
        match store.list_levels() {
            Ok(entries) if !entries.is_empty() => {
                let available: Vec<&str> = entries.iter().map(|(t, _)| t.as_str()).collect();
                return Err(format!("level '{}' not found. Available: {}", level_tag, available.join(", ")));
            }
            _ => return Err(format!("level '{}' not found.", level_tag)),
        }
    }

    // Build Bevy app immediately — window appears before loading
    let mut app = App::new();
    app.insert_resource(ClearColor(Color::srgb(0.53, 0.71, 0.86))); // sky blue

    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: format!("Randlebrot - Playing: {level_tag}"),
            resolution: bevy::window::WindowResolution::new(1280, 720),
            ..default()
        }),
        ..default()
    }));
    app.add_plugins(bevy_egui::EguiPlugin {
        enable_multipass_for_primary_context: false,
        ..Default::default()
    });

    app.insert_resource(LaunchParams { level_tag });

    let loaded = resource_exists::<PlayerState>;

    if flythrough {
        // Flythrough mode: automated camera path, no cursor grab, no HUD, no manual input
        app.add_systems(Startup, setup_scene);
        app.insert_resource(FlyThroughState::new());
        app.add_systems(Update, (
            flythrough_system.run_if(loaded),
            chunk_stream.run_if(loaded),
            chunk_poll.run_if(loaded),
            chunk_unload.run_if(loaded),
        ));
    } else {
        // Normal interactive mode
        app.add_systems(Startup, (setup_scene, grab_cursor));
        app.add_systems(Update, (
            camera_input.run_if(loaded),
            chunk_stream.run_if(loaded),
            chunk_poll.run_if(loaded),
            chunk_unload.run_if(loaded),
            hud_system.run_if(loaded),
            exit_on_esc,
        ));
    }

    app.run();
    Ok(())
}

// ─── Resources ─────────────────────────────────────────────────────────────

#[derive(Resource)]
struct LaunchParams { level_tag: String }

#[derive(Resource)]
struct PlayerState {
    yaw: f32,
    pitch: f32,
    world_x: f32,
    world_z: f32,
    seed: u32,
    level_tag: String,
    chunk_coord: (i32, i32),
}

#[derive(Resource)]
struct MacroBiome { biome_map: Arc<BiomeMap> }

#[derive(Resource)]
struct RiverNet { network: Arc<rb_noise::RiverNetwork> }

#[derive(Resource, Default)]
struct ChunkQueue { in_flight: Vec<ChunkTask> }

struct ChunkTask {
    coord: (i32, i32),
    task: Task<((i32, i32), ChunkMeshData)>,
}

struct ChunkMeshData {
    mesh: Mesh,
    /// Block heights grid (BLOCKS_PER_CHUNK x BLOCKS_PER_CHUNK) for terrain following.
    block_heights: Vec<f32>,
}

/// Stores per-chunk height data for terrain following.
struct ChunkHeightInfo {
    entity: Entity,
    /// Block heights (BLOCKS_PER_CHUNK x BLOCKS_PER_CHUNK). Each entry is the
    /// Y position (in Bevy world units) of the top of the block at that grid cell.
    block_heights: Vec<f32>,
}

#[derive(Resource, Default)]
struct LoadedChunks {
    chunks: HashMap<(i32, i32), ChunkHeightInfo>,
}

#[derive(Resource)]
struct FpsCounter { frames: u32, elapsed: f32, fps: f32 }
impl Default for FpsCounter {
    fn default() -> Self { Self { frames: 0, elapsed: 0.0, fps: 0.0 } }
}

#[derive(Component)]
struct ChunkEntity;

// ─── Flythrough ───────────────────────────────────────────────────────────

struct FlyWaypoint {
    position_offset: Vec3,
    look_dir: Vec3,
    duration: f32,
}

#[derive(Resource)]
struct FlyThroughState {
    waypoints: Vec<FlyWaypoint>,
    current: usize,
    elapsed: f32,
    frame_count: u32,
    output_dir: PathBuf,
    spawn_pos: Option<Vec3>,
    screenshot_pending: bool,
}

impl FlyThroughState {
    fn new() -> Self {
        let output_dir = PathBuf::from("/tmp/randlebrot_flythrough");
        // Clean old frames from previous runs
        if output_dir.exists() {
            let _ = std::fs::remove_dir_all(&output_dir);
        }
        std::fs::create_dir_all(&output_dir).expect("create flythrough output dir");
        Self {
            waypoints: vec![
                // 1. Ground level looking forward along terrain
                FlyWaypoint { position_offset: Vec3::new(0.0, 3.0, 0.0), look_dir: Vec3::new(1.0, -0.2, 0.0).normalize(), duration: 1.0 },
                // 2. Step back, look at the terrain in front
                FlyWaypoint { position_offset: Vec3::new(-20.0, 5.0, 0.0), look_dir: Vec3::new(1.0, -0.15, 0.0).normalize(), duration: 1.0 },
                // 3. Side angle — look diagonally across terrain
                FlyWaypoint { position_offset: Vec3::new(0.0, 4.0, -30.0), look_dir: Vec3::new(0.5, -0.15, 1.0).normalize(), duration: 1.0 },
                // 4. Other direction strafe
                FlyWaypoint { position_offset: Vec3::new(0.0, 4.0, 30.0), look_dir: Vec3::new(0.5, -0.15, -1.0).normalize(), duration: 1.0 },
                // 5. Low aerial — 15 blocks up, looking down-forward
                FlyWaypoint { position_offset: Vec3::new(30.0, 15.0, 0.0), look_dir: Vec3::new(0.6, -0.6, 0.0).normalize(), duration: 1.0 },
                // 6. Aerial pan — 30 blocks up, wide horizon view
                FlyWaypoint { position_offset: Vec3::new(50.0, 30.0, 0.0), look_dir: Vec3::new(1.0, -0.35, 0.0).normalize(), duration: 1.0 },
                // 7. Aerial looking back at landscape
                FlyWaypoint { position_offset: Vec3::new(50.0, 30.0, 0.0), look_dir: Vec3::new(-1.0, -0.25, 0.4).normalize(), duration: 1.0 },
                // 8. Swoop toward ground
                FlyWaypoint { position_offset: Vec3::new(80.0, 4.0, 0.0), look_dir: Vec3::new(1.0, -0.15, 0.0).normalize(), duration: 1.5 },
                // 9. Final ground run
                FlyWaypoint { position_offset: Vec3::new(120.0, 3.0, -10.0), look_dir: Vec3::new(1.0, -0.1, 0.1).normalize(), duration: 1.0 },
            ],
            current: 0,
            elapsed: 0.0,
            frame_count: 0,
            output_dir,
            spawn_pos: None,
            screenshot_pending: false,
        }
    }
}

fn flythrough_system(
    mut commands: Commands,
    mut state: ResMut<FlyThroughState>,
    time: Res<Time>,
    mut camera_q: Query<&mut Transform, With<Camera3d>>,
    mut exit: MessageWriter<AppExit>,
) {
    let Ok(mut cam_transform) = camera_q.single_mut() else { return };

    // Record spawn position on first frame
    if state.spawn_pos.is_none() {
        state.spawn_pos = Some(cam_transform.translation);
    }
    let spawn_pos = state.spawn_pos.unwrap();

    // If a screenshot was just taken, advance to next waypoint
    if state.screenshot_pending {
        state.screenshot_pending = false;
        state.frame_count += 1;
        state.current += 1;
        state.elapsed = 0.0;

        if state.current >= state.waypoints.len() {
            eprintln!(
                "Flythrough complete: {} frames saved to {:?}",
                state.frame_count, state.output_dir
            );
            exit.write(AppExit::Success);
            return;
        }
    }

    if state.current >= state.waypoints.len() {
        return;
    }

    state.elapsed += time.delta_secs();
    let wp = &state.waypoints[state.current];

    // Interpolate camera position toward current waypoint target
    let target_pos = spawn_pos + wp.position_offset;
    let t = (state.elapsed / wp.duration).min(1.0);
    let smoothed = t * t * (3.0 - 2.0 * t); // smoothstep
    cam_transform.translation = cam_transform.translation.lerp(target_pos, smoothed.min(1.0));
    cam_transform.look_to(wp.look_dir, Vec3::Y);

    // When duration is reached, take a screenshot
    if state.elapsed >= wp.duration && !state.screenshot_pending {
        state.screenshot_pending = true;
        let path = state.output_dir.join(format!("frame_{:03}.png", state.frame_count + 1));
        eprintln!("Capturing frame {} -> {:?}", state.frame_count + 1, path);
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(path));
    }
}

// ─── Startup ───────────────────────────────────────────────────────────────

fn setup_scene(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    params: Res<LaunchParams>,
) {
    let store = ArtifactStore::new().expect("artifact store");
    let (initial_biome, manifest) = store.load_level(&params.level_tag).expect("load level");

    let (world_x, world_y) = chunk_coord_to_world_pos(manifest.chunk_coord);
    let seed = manifest.seed;

    eprintln!("Loading level '{}': seed={}, coord=({},{})",
        params.level_tag, seed, manifest.chunk_coord.0, manifest.chunk_coord.1);

    // Load parent layers
    let (macro_biome, river_net) = load_parent_layers(&store, &manifest, seed).expect("parent layers");
    let macro_arc = Arc::new(macro_biome);
    if let Some(net) = river_net {
        commands.insert_resource(RiverNet { network: net });
    }
    commands.insert_resource(MacroBiome { biome_map: macro_arc });

    let initial_yaw: f32 = 0.0;
    let initial_pitch: f32 = -0.2;
    commands.insert_resource(PlayableLevel {
        origin: WorldPos::new(world_x, world_y),
        chunk_coord: ((world_x / 64.0).floor() as i32, (world_y / 64.0).floor() as i32),
        seed,
        world_height: WORLD_HEIGHT as f64,
    });
    commands.insert_resource(ChunkQueue::default());
    commands.insert_resource(LoadedChunks::default());
    commands.insert_resource(FpsCounter::default());

    // Chunk position in Bevy units
    let chunk_bx = manifest.chunk_coord.0 as f32 * CHUNK_BEVY_SIZE;
    let chunk_bz = manifest.chunk_coord.1 as f32 * CHUNK_BEVY_SIZE;

    // Camera spawns at center of chunk, on the ground (absolute height)
    let step = initial_biome.width / BLOCKS_PER_CHUNK;
    let step = if step == 0 { 1 } else { step };
    let center_px = (BLOCKS_PER_CHUNK / 2 * step + step / 2).min(initial_biome.width - 1);
    let center_pz = (BLOCKS_PER_CHUNK / 2 * step + step / 2).min(initial_biome.height - 1);
    let center_idx = center_pz * initial_biome.width + center_px;
    let center_h = *initial_biome.heightmap.get(center_idx).unwrap_or(&0.0) as f32;
    let ground_y = (center_h * HEIGHT_SCALE).floor() * BLOCK_WORLD_SIZE;
    let spawn_y = ground_y + EYE_HEIGHT;
    let spawn_x = chunk_bx + CHUNK_BEVY_SIZE / 2.0;
    let spawn_z = chunk_bz + CHUNK_BEVY_SIZE / 2.0;
    eprintln!("Spawn: ({spawn_x:.1}, {spawn_y:.1}, {spawn_z:.1}), ground_y: {ground_y:.0}");

    commands.insert_resource(PlayerState {
        yaw: initial_yaw,
        pitch: initial_pitch,
        world_x: spawn_x,
        world_z: spawn_z,
        seed,
        level_tag: params.level_tag.clone(),
        chunk_coord: manifest.chunk_coord,
    });

    // Initial look: forward along +X, slightly down
    let look_dir = Vec3::new(
        initial_yaw.cos() * initial_pitch.cos(),
        initial_pitch.sin(),
        initial_yaw.sin() * initial_pitch.cos(),
    );
    // 3D camera for terrain rendering
    commands.spawn((
        Camera3d::default(),
        Camera { order: 0, ..default() },
        Transform::from_xyz(spawn_x, spawn_y, spawn_z)
            .looking_to(look_dir, Vec3::Y),
        DistanceFog {
            color: Color::srgb(0.53, 0.71, 0.86),
            falloff: FogFalloff::Linear { start: 200.0, end: 800.0 },
            ..default()
        },
    ));

    // 2D camera for egui HUD overlay (bevy_egui requires a Camera2d to render)
    commands.spawn((
        Camera2d,
        Camera { order: 1, clear_color: ClearColorConfig::None, ..default() },
    ));

    // Directional light (sun)
    commands.spawn((
        DirectionalLight {
            illuminance: 10000.0,
            shadows_enabled: false,
            ..default()
        },
        Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, -0.8, 0.5, 0.0)),
    ));

    // Ambient light — spawn as entity in Bevy 0.18
    commands.spawn(AmbientLight {
        color: Color::srgb(0.6, 0.7, 0.9),
        brightness: 500.0,
        affects_lightmapped_meshes: false,
    });

    // Generate initial chunk mesh
    let chunk_data = generate_chunk_mesh(&initial_biome);
    let mesh_handle = meshes.add(chunk_data.mesh);
    let material = materials.add(StandardMaterial {
        base_color: Color::WHITE,
        perceptual_roughness: 0.9,
        ..default()
    });

    let entity = commands.spawn((
        Mesh3d(mesh_handle),
        MeshMaterial3d(material),
        Transform::from_xyz(chunk_bx, 0.0, chunk_bz),
        ChunkEntity,
    )).id();

    let mut loaded = LoadedChunks::default();
    loaded.chunks.insert(manifest.chunk_coord, ChunkHeightInfo {
        entity,
        block_heights: chunk_data.block_heights,
    });
    commands.insert_resource(loaded);
}

fn grab_cursor(mut cursor_q: Query<&mut CursorOptions, With<PrimaryWindow>>) {
    if let Ok(mut cursor) = cursor_q.single_mut() {
        cursor.grab_mode = CursorGrabMode::Locked;
        cursor.visible = false;
    }
}

// ─── Chunk Mesh Generation ─────────────────────────────────────────────────

/// Interpolate between two RGB colors by factor t (0..1).
fn lerp_color(a: [u8; 3], b: [u8; 3], t: f32) -> [f32; 4] {
    let t = t.clamp(0.0, 1.0);
    [
        (a[0] as f32 * (1.0 - t) + b[0] as f32 * t) / 255.0,
        (a[1] as f32 * (1.0 - t) + b[1] as f32 * t) / 255.0,
        (a[2] as f32 * (1.0 - t) + b[2] as f32 * t) / 255.0,
        1.0,
    ]
}

/// Minecraft-style directional brightness multiplier for a face normal.
/// Top=1.0, North/South=0.8, East=0.75, West=0.6 — baked into vertex colours
/// since we use unlit materials. This is the core of the Minecraft "3D feel".
fn face_brightness(normal: [f32; 3]) -> f32 {
    if normal[1] > 0.5 { return 1.0; }   // top
    if normal[1] < -0.5 { return 0.5; }  // bottom
    if normal[0].abs() > 0.5 {
        if normal[0] > 0.0 { 0.75 } else { 0.6 }  // east / west
    } else {
        0.8  // north / south
    }
}

/// Compute side face colour for a given depth and face direction.
/// Geological layers: rich dirt for top 2 blocks, stone below.
/// Brightness is baked per face direction (Minecraft-style directional shading).
fn side_color_at_depth(depth: f32, normal: [f32; 3]) -> [f32; 4] {
    let brightness = face_brightness(normal);
    let base = if depth < 2.0 {
        lerp_color(DIRT_COLOR, STONE_COLOR, depth / 2.0 * 0.6)
    } else {
        let deep = ((depth - 2.0) / 15.0).min(0.35);
        [
            STONE_COLOR[0] as f32 / 255.0 * (1.0 - deep),
            STONE_COLOR[1] as f32 / 255.0 * (1.0 - deep),
            STONE_COLOR[2] as f32 / 255.0 * (1.0 - deep),
            1.0,
        ]
    };
    [base[0] * brightness, base[1] * brightness, base[2] * brightness, 1.0]
}

/// Convert a chunk's BiomeMap into a Bevy Mesh of block faces.
/// Uses absolute height scaling so all chunks share a consistent vertical
/// reference, eliminating seams between adjacent chunks.
/// Applies greedy meshing to merge adjacent same-height top face blocks.
fn generate_chunk_mesh(bm: &BiomeMap) -> ChunkMeshData {
    let step = bm.width / BLOCKS_PER_CHUNK;
    let step = if step == 0 { 1 } else { step };

    // Sample heightmap at block resolution (absolute heights, no local normalization)
    let mut heights = vec![0i32; BLOCKS_PER_CHUNK * BLOCKS_PER_CHUNK];
    let mut colors = vec![[0u8; 4]; BLOCKS_PER_CHUNK * BLOCKS_PER_CHUNK];
    let color_data = bm.to_layer_image(NoiseLayer::Biome);

    for bz in 0..BLOCKS_PER_CHUNK {
        for bx in 0..BLOCKS_PER_CHUNK {
            let px = (bx * step + step / 2).min(bm.width - 1);
            let pz = (bz * step + step / 2).min(bm.height - 1);
            let idx = pz * bm.width + px;
            let h = *bm.heightmap.get(idx).unwrap_or(&0.0) as f32;
            heights[bz * BLOCKS_PER_CHUNK + bx] = (h * HEIGHT_SCALE).floor() as i32;

            // Sample biome color
            let ci = idx * 4;
            if ci + 3 < color_data.len() {
                colors[bz * BLOCKS_PER_CHUNK + bx] = [
                    color_data[ci], color_data[ci + 1], color_data[ci + 2], 255
                ];
            } else {
                colors[bz * BLOCKS_PER_CHUNK + bx] = [100, 100, 80, 255]; // fallback
            }
        }
    }

    // 3×3 box-blur on BOTH heights and colours — 2 passes to smooth pixel-scale
    // fBm spikes without destroying terrain shape.
    let n = BLOCKS_PER_CHUNK;
    for _ in 0..2 {
        let orig_h = heights.clone();
        let orig_c = colors.clone();
        for bz in 0..n {
            for bx in 0..n {
                let mut sum_h = 0i64;
                let mut sum_r = 0i64;
                let mut sum_g = 0i64;
                let mut sum_b = 0i64;
                let mut count = 0i64;
                for dz in -1i32..=1 {
                    for dx in -1i32..=1 {
                        let nx = bx as i32 + dx;
                        let nz = bz as i32 + dz;
                        if nx >= 0 && nz >= 0 && nx < n as i32 && nz < n as i32 {
                            let ni = nz as usize * n + nx as usize;
                            sum_h += orig_h[ni] as i64;
                            sum_r += orig_c[ni][0] as i64;
                            sum_g += orig_c[ni][1] as i64;
                            sum_b += orig_c[ni][2] as i64;
                            count += 1;
                        }
                    }
                }
                let idx = bz * n + bx;
                heights[idx] = (sum_h / count) as i32;
                colors[idx] = [(sum_r / count) as u8, (sum_g / count) as u8, (sum_b / count) as u8, 255];
            }
        }
    }

    // Build block_heights for terrain following (in Bevy world units)
    let block_heights: Vec<f32> = heights.iter()
        .map(|&h| h as f32 * BLOCK_WORLD_SIZE)
        .collect();

    let bs = BLOCK_WORLD_SIZE;
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut normals: Vec<[f32; 3]> = Vec::new();
    let mut vertex_colors: Vec<[f32; 4]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    let get_h = |bx: i32, bz: i32| -> i32 {
        if bx < 0 || bz < 0 || bx >= BLOCKS_PER_CHUNK as i32 || bz >= BLOCKS_PER_CHUNK as i32 {
            return i32::MIN; // out of bounds = empty
        }
        heights[bz as usize * BLOCKS_PER_CHUNK + bx as usize]
    };

    // --- Greedy meshing for TOP faces ---
    // visited[z][x] tracks whether a block's top face has been merged
    let n = BLOCKS_PER_CHUNK;
    let mut visited = vec![false; n * n];

    for bz in 0..n {
        for bx in 0..n {
            if visited[bz * n + bx] { continue; }

            let h = heights[bz * n + bx];
            let ci = bz * n + bx;
            let [r, g, b, _] = colors[ci];

            // Extend the run along X (same height and same color)
            let mut run_x = 1;
            while bx + run_x < n {
                let ni = bz * n + bx + run_x;
                if visited[ni] { break; }
                if heights[ni] != h { break; }
                let [nr, ng, nb, _] = colors[ni];
                if nr != r || ng != g || nb != b { break; }
                run_x += 1;
            }

            // Extend the run along Z (all blocks in the wider row must match)
            let mut run_z = 1;
            'outer: while bz + run_z < n {
                for dx in 0..run_x {
                    let ni = (bz + run_z) * n + bx + dx;
                    if visited[ni] { break 'outer; }
                    if heights[ni] != h { break 'outer; }
                    let [nr, ng, nb, _] = colors[ni];
                    if nr != r || ng != g || nb != b { break 'outer; }
                }
                run_z += 1;
            }

            // Mark all blocks in the merged quad as visited
            for dz in 0..run_z {
                for dx in 0..run_x {
                    visited[(bz + dz) * n + bx + dx] = true;
                }
            }

            // Emit a single merged top face quad
            let x0 = bx as f32 * bs;
            let z0 = bz as f32 * bs;
            let y = h as f32 * bs;
            let w = run_x as f32 * bs;
            let d = run_z as f32 * bs;
            let top_color = [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0];

            let vi = positions.len() as u32;
            positions.extend_from_slice(&[
                [x0, y, z0],
                [x0 + w, y, z0],
                [x0 + w, y, z0 + d],
                [x0, y, z0 + d],
            ]);
            normals.extend_from_slice(&[[0.0, 1.0, 0.0]; 4]);
            vertex_colors.extend_from_slice(&[top_color; 4]);
            indices.extend_from_slice(&[vi, vi + 2, vi + 1, vi, vi + 3, vi + 2]);
        }
    }

    // --- Side faces (per-block, with depth-based color variation) ---
    for bz in 0..n as i32 {
        for bx in 0..n as i32 {
            let h = get_h(bx, bz);
            let x0 = bx as f32 * bs;
            let z0 = bz as f32 * bs;
            let y = h as f32 * bs;
            // Helper: emit a side face with directional + depth-based coloring
            let mut emit_side = |normal: [f32; 3], corners: [[f32; 3]; 4], y_top: f32, y_bottom: f32| {
                let depth = (y_top - y_bottom).abs();
                if depth < 0.001 { return; }
                let mid_depth = depth / 2.0;
                let color = side_color_at_depth(mid_depth, normal);

                let vi = positions.len() as u32;
                positions.extend_from_slice(&corners);
                normals.extend_from_slice(&[normal; 4]);
                vertex_colors.extend_from_slice(&[color; 4]);
                indices.extend_from_slice(&[vi, vi + 1, vi + 2, vi, vi + 2, vi + 3]);
            };

            // North face (-Z)
            if get_h(bx, bz - 1) < h {
                let nb_h = get_h(bx, bz - 1).max(0);
                let y_bottom = nb_h as f32 * bs;
                emit_side([0.0, 0.0, -1.0], [
                    [x0, y, z0], [x0 + bs, y, z0],
                    [x0 + bs, y_bottom, z0], [x0, y_bottom, z0],
                ], y, y_bottom);
            }
            // South face (+Z)
            if get_h(bx, bz + 1) < h {
                let nb_h = get_h(bx, bz + 1).max(0);
                let y_bottom = nb_h as f32 * bs;
                emit_side([0.0, 0.0, 1.0], [
                    [x0 + bs, y, z0 + bs], [x0, y, z0 + bs],
                    [x0, y_bottom, z0 + bs], [x0 + bs, y_bottom, z0 + bs],
                ], y, y_bottom);
            }
            // West face (-X)
            if get_h(bx - 1, bz) < h {
                let nb_h = get_h(bx - 1, bz).max(0);
                let y_bottom = nb_h as f32 * bs;
                emit_side([-1.0, 0.0, 0.0], [
                    [x0, y, z0 + bs], [x0, y, z0],
                    [x0, y_bottom, z0], [x0, y_bottom, z0 + bs],
                ], y, y_bottom);
            }
            // East face (+X)
            if get_h(bx + 1, bz) < h {
                let nb_h = get_h(bx + 1, bz).max(0);
                let y_bottom = nb_h as f32 * bs;
                emit_side([1.0, 0.0, 0.0], [
                    [x0 + bs, y, z0], [x0 + bs, y, z0 + bs],
                    [x0 + bs, y_bottom, z0 + bs], [x0 + bs, y_bottom, z0],
                ], y, y_bottom);
            }
        }
    }

    let mut mesh = Mesh::new(
        bevy::mesh::PrimitiveTopology::TriangleList,
        bevy::asset::RenderAssetUsages::RENDER_WORLD | bevy::asset::RenderAssetUsages::MAIN_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, vertex_colors);
    mesh.insert_indices(Indices::U32(indices));
    ChunkMeshData { mesh, block_heights }
}

// ─── Camera Input ──────────────────────────────────────────────────────────

/// Sample terrain height at a world XZ position from the loaded chunk data.
/// Returns the block top Y in Bevy world units, or None if no chunk is loaded there.
fn sample_terrain_height(loaded: &LoadedChunks, world_x: f32, world_z: f32) -> Option<f32> {
    let cx = (world_x / CHUNK_BEVY_SIZE).floor() as i32;
    let cz = (world_z / CHUNK_BEVY_SIZE).floor() as i32;

    let info = loaded.chunks.get(&(cx, cz))?;

    // Local position within chunk, in block coordinates
    let local_x = world_x - cx as f32 * CHUNK_BEVY_SIZE;
    let local_z = world_z - cz as f32 * CHUNK_BEVY_SIZE;
    let bx = (local_x / BLOCK_WORLD_SIZE).floor() as usize;
    let bz = (local_z / BLOCK_WORLD_SIZE).floor() as usize;

    let bx = bx.min(BLOCKS_PER_CHUNK - 1);
    let bz = bz.min(BLOCKS_PER_CHUNK - 1);

    Some(info.block_heights[bz * BLOCKS_PER_CHUNK + bx])
}

fn camera_input(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut motion: MessageReader<MouseMotion>,
    time: Res<Time>,
    mut player: ResMut<PlayerState>,
    mut camera_q: Query<&mut Transform, With<Camera3d>>,
    mut cursor_q: Query<&mut CursorOptions, With<PrimaryWindow>>,
    loaded: Res<LoadedChunks>,
) {
    let dt = time.delta_secs();
    let Ok(mut cam_transform) = camera_q.single_mut() else { return };

    // Tab: toggle cursor grab
    if keyboard.just_pressed(KeyCode::Tab) {
        if let Ok(mut cursor) = cursor_q.single_mut() {
            if matches!(cursor.grab_mode, CursorGrabMode::Locked) {
                cursor.grab_mode = CursorGrabMode::None;
                cursor.visible = true;
            } else {
                cursor.grab_mode = CursorGrabMode::Locked;
                cursor.visible = false;
            }
        }
    }

    // Mouse look
    for ev in motion.read() {
        player.yaw += ev.delta.x * MOUSE_SENS;
        player.pitch = (player.pitch - ev.delta.y * MOUSE_SENS).clamp(-MAX_PITCH, MAX_PITCH);
    }

    // WASD
    let mut forward = 0.0f32;
    let mut strafe = 0.0f32;
    if keyboard.pressed(KeyCode::KeyW) { forward += 1.0; }
    if keyboard.pressed(KeyCode::KeyS) { forward -= 1.0; }
    if keyboard.pressed(KeyCode::KeyA) { strafe -= 1.0; }
    if keyboard.pressed(KeyCode::KeyD) { strafe += 1.0; }

    if forward != 0.0 || strafe != 0.0 {
        let len = (forward * forward + strafe * strafe).sqrt();
        forward /= len;
        strafe /= len;

        let dx = player.yaw.cos() * forward - player.yaw.sin() * strafe;
        let dz = player.yaw.sin() * forward + player.yaw.cos() * strafe;

        player.world_x += dx * MOVE_SPEED * dt;
        player.world_z += dz * MOVE_SPEED * dt;
    }

    // Update camera transform
    let look_x = player.yaw.cos() * player.pitch.cos();
    let look_y = player.pitch.sin();
    let look_z = player.yaw.sin() * player.pitch.cos();

    // Terrain height following: sample the chunk heightmap at player position
    let target_y = sample_terrain_height(&loaded, player.world_x, player.world_z)
        .unwrap_or(cam_transform.translation.y - EYE_HEIGHT)
        + EYE_HEIGHT;
    // Smooth interpolation to avoid jarring jumps
    let lerp_speed = 10.0 * dt;
    cam_transform.translation.y += (target_y - cam_transform.translation.y) * lerp_speed.min(1.0);
    cam_transform.translation.x = player.world_x;
    cam_transform.translation.z = player.world_z;
    cam_transform.look_to(Vec3::new(look_x, look_y, look_z), Vec3::Y);
}

// ─── Chunk Streaming ───────────────────────────────────────────────────────

fn chunk_stream(
    player: Res<PlayerState>,
    loaded: Res<LoadedChunks>,
    mut queue: ResMut<ChunkQueue>,
    macro_data: Res<MacroBiome>,
    river_data: Option<Res<RiverNet>>,
    level: Res<PlayableLevel>,
) {
    let cam_cx = (player.world_x / CHUNK_BEVY_SIZE).floor() as i32;
    let cam_cz = (player.world_z / CHUNK_BEVY_SIZE).floor() as i32;
    let seed = player.seed;
    let height = level.world_height;
    let river_net = river_data.map(|r| r.network.clone());

    let in_flight: HashSet<(i32, i32)> = queue.in_flight.iter().map(|t| t.coord).collect();

    for dz in -LOAD_RADIUS..=LOAD_RADIUS {
        for dx in -LOAD_RADIUS..=LOAD_RADIUS {
            let cx = cam_cx + dx;
            let cz = cam_cz + dz;
            let coord = (cx, cz);

            if cx < 0 || cz < 0 || cx >= WORLD_WIDTH as i32 || cz >= WORLD_HEIGHT as i32 {
                continue;
            }
            if loaded.chunks.contains_key(&coord) || in_flight.contains(&coord) {
                continue;
            }
            if queue.in_flight.len() >= MAX_CONCURRENT {
                return;
            }

            let wx = cx as f64 * CHUNK_WORLD_SIZE;
            let wz = cz as f64 * CHUNK_WORLD_SIZE;
            let macro_map = macro_data.biome_map.clone();
            let rn = river_net.clone();

            let task = AsyncComputeTaskPool::get().spawn(async move {
                let rn_ref = rn.as_ref();
                let biome_map = BiomeMap::generate_meso_full_with_backend(
                    seed, wx, wz, CHUNK_WORLD_SIZE as f64, TILE_MAP_SIZE,
                    height, 3, None, NoiseBackend::Cpu,
                    Some(&macro_map), rn_ref,
                );
                let chunk_data = generate_chunk_mesh(&biome_map);
                (coord, chunk_data)
            });

            queue.in_flight.push(ChunkTask { coord, task });
        }
    }
}

fn chunk_poll(
    mut commands: Commands,
    mut queue: ResMut<ChunkQueue>,
    mut loaded: ResMut<LoadedChunks>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let mut i = 0;
    let mut completed = 0;
    while i < queue.in_flight.len() && completed < 4 {
        if let Some(result) = block_on(poll_once(&mut queue.in_flight[i].task)) {
            queue.in_flight.swap_remove(i);
            let (coord, data) = result;

            let mesh_handle = meshes.add(data.mesh);
            let material = materials.add(StandardMaterial {
                base_color: Color::WHITE,
                unlit: true,  // vertex colours carry all shading; PBR shadows cause black faces
                perceptual_roughness: 0.9,
                ..default()
            });

            let wx = coord.0 as f32 * CHUNK_BEVY_SIZE;
            let wz = coord.1 as f32 * CHUNK_BEVY_SIZE;

            let entity = commands.spawn((
                Mesh3d(mesh_handle),
                MeshMaterial3d(material),
                Transform::from_xyz(wx, 0.0, wz),
                ChunkEntity,
            )).id();

            loaded.chunks.insert(coord, ChunkHeightInfo {
                entity,
                block_heights: data.block_heights,
            });
            completed += 1;
        } else {
            i += 1;
        }
    }
}

fn chunk_unload(
    mut commands: Commands,
    player: Res<PlayerState>,
    mut loaded: ResMut<LoadedChunks>,
) {
    let cam_cx = (player.world_x / CHUNK_BEVY_SIZE).floor() as i32;
    let cam_cz = (player.world_z / CHUNK_BEVY_SIZE).floor() as i32;

    let to_remove: Vec<(i32, i32)> = loaded.chunks.keys()
        .filter(|(cx, cz)| {
            (cx - cam_cx).abs() > UNLOAD_RADIUS || (cz - cam_cz).abs() > UNLOAD_RADIUS
        })
        .copied()
        .collect();

    for coord in to_remove {
        if let Some(info) = loaded.chunks.remove(&coord) {
            commands.entity(info.entity).despawn();
        }
    }
}

// ─── HUD ───────────────────────────────────────────────────────────────────

fn hud_system(
    mut contexts: EguiContexts,
    player: Res<PlayerState>,
    time: Res<Time>,
    mut fps: ResMut<FpsCounter>,
) {
    fps.frames += 1;
    fps.elapsed += time.delta_secs();
    if fps.elapsed >= 1.0 {
        fps.fps = fps.frames as f32 / fps.elapsed;
        fps.frames = 0;
        fps.elapsed = 0.0;
    }

    if let Ok(ctx) = contexts.ctx_mut() {
        ctx.memory_mut(|mem| mem.surrender_focus(egui::Id::NULL));
        egui::Area::new(egui::Id::new("hud"))
            .fixed_pos(egui::pos2(10.0, 10.0))
            .interactable(false)
            .show(ctx, |ui| {
                egui::Frame::dark_canvas(ui.style()).show(ui, |ui| {
                    ui.label(format!("Level: {}", player.level_tag));
                    ui.label(format!("Coord: ({}, {})", player.chunk_coord.0, player.chunk_coord.1));
                    ui.label(format!("Seed: {}", player.seed));
                    ui.label(format!("Pos: ({:.1}, {:.1})", player.world_x, player.world_z));
                    ui.label(format!("FPS: {:.0}", fps.fps));
                    ui.separator();
                    ui.label("WASD: move  Mouse: look  Tab: cursor  ESC: exit");
                });
            });
    }
}

// ─── Exit ──────────────────────────────────────────────────────────────────

fn exit_on_esc(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut exit: MessageWriter<AppExit>,
    mut cursor_q: Query<&mut CursorOptions, With<PrimaryWindow>>,
) {
    if keyboard.just_pressed(KeyCode::Escape) {
        if let Ok(mut cursor) = cursor_q.single_mut() {
            cursor.grab_mode = CursorGrabMode::None;
            cursor.visible = true;
        }
        exit.write(AppExit::Success);
    }
}

// ─── Parent Layers Loading ─────────────────────────────────────────────────

fn load_parent_layers(
    store: &ArtifactStore,
    manifest: &rb_artifacts::LevelManifest,
    seed: u32,
) -> Result<(BiomeMap, Option<Arc<rb_noise::RiverNetwork>>), String> {
    if let Some(ref parent_tag) = manifest.parent_layers_tag {
        match store.load_layers_data(parent_tag) {
            Ok((mut biome_map, river_network, _)) => {
                let arc = Arc::new(river_network);
                biome_map.river_network = Some(arc.clone());
                eprintln!("Loaded parent layers '{parent_tag}'");
                return Ok((biome_map, Some(arc)));
            }
            Err(e) => {
                eprintln!("Warning: parent layers '{parent_tag}': {e}");
            }
        }
    }
    eprintln!("Generating macro BiomeMap from seed {seed}...");
    let bm = BiomeMap::generate_with_backend(seed, WORLD_WIDTH, WORLD_HEIGHT, NoiseBackend::Cpu);
    let rn = bm.river_network.clone();
    Ok((bm, rn))
}
