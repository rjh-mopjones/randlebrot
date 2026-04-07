//! Launch a playable level with Minecraft-style 3D block terrain.
//!
//! `randlebrot launch <level-tag>` opens a Bevy 3D window. The heightmap
//! from each chunk is converted into a block mesh (one quad per visible face).
//! Chunks stream in as the player moves.
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

/// Blocks per chunk side. 64x64 = 4096 blocks = manageable mesh.
/// Each block samples the heightmap at its center (512/64 = 8px region).
const BLOCKS_PER_CHUNK: usize = 64;
const BLOCK_WORLD_SIZE: f32 = 1.0;
const CHUNK_BEVY_SIZE: f32 = BLOCKS_PER_CHUNK as f32 * BLOCK_WORLD_SIZE;

/// Max block height range per chunk — local normalization stretches whatever
/// height variation exists to fill this many block levels.
const MAX_BLOCK_HEIGHT: f32 = 64.0;

/// Chunk load radius.
const LOAD_RADIUS: i32 = 8;

/// Chunk unload radius (must be > LOAD_RADIUS to avoid load/unload thrashing).
const UNLOAD_RADIUS: i32 = 10;

/// Max concurrent chunk generation tasks.
const MAX_CONCURRENT: usize = 4;

/// Player movement speed (world units per second).
const MOVE_SPEED: f32 = 2.0;

/// Mouse look sensitivity.
const MOUSE_SENS: f32 = 0.002;

/// Pitch clamp (±60 degrees).
const MAX_PITCH: f32 = std::f32::consts::FRAC_PI_3;

/// Player eye height above terrain (in world units).
const EYE_HEIGHT: f32 = 0.5;

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
    biome_map: BiomeMap,
}

#[derive(Resource, Default)]
struct LoadedChunks {
    entities: HashMap<(i32, i32), Entity>,
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
                // 1. Spawn view — looking forward
                FlyWaypoint { position_offset: Vec3::ZERO, look_dir: Vec3::new(1.0, 0.0, 0.0), duration: 1.0 },
                // 2. Turn left 90
                FlyWaypoint { position_offset: Vec3::ZERO, look_dir: Vec3::new(0.0, 0.0, -1.0), duration: 0.5 },
                // 3. Turn right 180
                FlyWaypoint { position_offset: Vec3::ZERO, look_dir: Vec3::new(0.0, 0.0, 1.0), duration: 0.5 },
                // 4. Look down at feet
                FlyWaypoint { position_offset: Vec3::ZERO, look_dir: Vec3::new(1.0, -1.0, 0.0).normalize(), duration: 0.5 },
                // 5. Look up at sky
                FlyWaypoint { position_offset: Vec3::ZERO, look_dir: Vec3::new(1.0, 1.0, 0.0).normalize(), duration: 0.5 },
                // 6. Move forward 30 blocks
                FlyWaypoint { position_offset: Vec3::new(30.0, 0.0, 0.0), look_dir: Vec3::new(1.0, 0.0, 0.0), duration: 2.0 },
                // 7. Move to high ground (up 20)
                FlyWaypoint { position_offset: Vec3::new(30.0, 20.0, 0.0), look_dir: Vec3::new(1.0, -0.3, 0.0).normalize(), duration: 1.0 },
                // 8. Panoramic spin
                FlyWaypoint { position_offset: Vec3::new(30.0, 20.0, 0.0), look_dir: Vec3::new(-1.0, 0.0, 0.0), duration: 0.5 },
                // 9. Back at ground
                FlyWaypoint { position_offset: Vec3::new(30.0, 0.0, 0.0), look_dir: Vec3::new(1.0, 0.0, 0.0), duration: 0.5 },
                // 10. Final forward view
                FlyWaypoint { position_offset: Vec3::new(50.0, 0.0, 0.0), look_dir: Vec3::new(1.0, 0.0, 0.0), duration: 1.0 },
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

    // Camera spawns at center of chunk, on the ground
    let local_hm = build_local_heightmap(&initial_biome);
    // Find local min/max for normalization (same as generate_chunk_mesh)
    let step = initial_biome.width / BLOCKS_PER_CHUNK;
    let step = if step == 0 { 1 } else { step };
    let mut hmin = f32::INFINITY;
    let mut hmax = f32::NEG_INFINITY;
    for bz in 0..BLOCKS_PER_CHUNK {
        for bx in 0..BLOCKS_PER_CHUNK {
            let px = (bx * step + step / 2).min(initial_biome.width - 1);
            let pz = (bz * step + step / 2).min(initial_biome.height - 1);
            let idx = pz * initial_biome.width + px;
            let h = local_hm.get(idx).copied().unwrap_or(0.0);
            if h < hmin { hmin = h; }
            if h > hmax { hmax = h; }
        }
    }
    let h_range = if hmax > hmin { hmax - hmin } else { 1.0 };
    let center_px = (BLOCKS_PER_CHUNK / 2 * step + step / 2).min(initial_biome.width - 1);
    let center_pz = (BLOCKS_PER_CHUNK / 2 * step + step / 2).min(initial_biome.height - 1);
    let center_idx = center_pz * initial_biome.width + center_px;
    let center_h = local_hm.get(center_idx).copied().unwrap_or(0.0);
    let ground_blocks = ((center_h - hmin) / h_range * MAX_BLOCK_HEIGHT).floor();
    let ground_y = ground_blocks * BLOCK_WORLD_SIZE;
    let spawn_y = ground_y + EYE_HEIGHT;
    let spawn_x = chunk_bx + CHUNK_BEVY_SIZE / 2.0;
    let spawn_z = chunk_bz + CHUNK_BEVY_SIZE / 2.0;
    eprintln!("Spawn: ({spawn_x:.1}, {spawn_y:.1}, {spawn_z:.1}), ground blocks: {ground_blocks:.0}");

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
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(spawn_x, spawn_y, spawn_z)
            .looking_to(look_dir, Vec3::Y),
        DistanceFog {
            color: Color::srgb(0.53, 0.71, 0.86),
            falloff: FogFalloff::Linear { start: 2.0, end: 15.0 },
            ..default()
        },
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
    let mesh = generate_chunk_mesh(&initial_biome);
    let mesh_handle = meshes.add(mesh);
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
    loaded.entities.insert(manifest.chunk_coord, entity);
    commands.insert_resource(loaded);
}

fn grab_cursor(mut cursor_q: Query<&mut CursorOptions, With<PrimaryWindow>>) {
    if let Ok(mut cursor) = cursor_q.single_mut() {
        cursor.grab_mode = CursorGrabMode::Locked;
        cursor.visible = false;
    }
}

// ─── Chunk Mesh Generation ─────────────────────────────────────────────────

/// Build a local heightmap from base noise layers (the derived heightmap is
/// flat at micro scale — see debug_level diagnostic).
fn build_local_heightmap(bm: &BiomeMap) -> Vec<f32> {
    let n = bm.width * bm.height;
    let mut hm = vec![0.0f32; n];
    for i in 0..n {
        let cont = *bm.continentalness.get(i).unwrap_or(&0.0) as f32;
        let pv = *bm.peaks_valleys.get(i).unwrap_or(&0.0) as f32;
        let tect = *bm.tectonic.get(i).unwrap_or(&0.5) as f32;
        let erosion = *bm.erosion.get(i).unwrap_or(&0.5) as f32;
        let rock = *bm.rock_hardness.get(i).unwrap_or(&0.5) as f32;
        hm[i] = cont * 3.0 + pv * 2.0 + (1.0 - tect) * 1.5 + erosion * 1.0 + rock * 0.5;
    }
    hm
}

/// Convert a chunk's BiomeMap into a Bevy Mesh of block faces.
/// Uses local normalization: the height variation within the chunk is stretched
/// to fill [0, MAX_BLOCK_HEIGHT] block levels. This preserves fractal shape
/// continuity while making variation visible at block scale.
fn generate_chunk_mesh(bm: &BiomeMap) -> Mesh {
    let full_hm = build_local_heightmap(bm);
    let step = bm.width / BLOCKS_PER_CHUNK;
    let step = if step == 0 { 1 } else { step };

    // Sample heightmap at block resolution and find local min/max
    let mut raw_heights = vec![0.0f32; BLOCKS_PER_CHUNK * BLOCKS_PER_CHUNK];
    let mut colors = vec![[0u8; 4]; BLOCKS_PER_CHUNK * BLOCKS_PER_CHUNK];
    let color_data = bm.to_layer_image(NoiseLayer::Biome);

    let mut hmin = f32::INFINITY;
    let mut hmax = f32::NEG_INFINITY;

    for bz in 0..BLOCKS_PER_CHUNK {
        for bx in 0..BLOCKS_PER_CHUNK {
            let px = (bx * step + step / 2).min(bm.width - 1);
            let pz = (bz * step + step / 2).min(bm.height - 1);
            let idx = pz * bm.width + px;
            let h = full_hm.get(idx).copied().unwrap_or(0.0);
            raw_heights[bz * BLOCKS_PER_CHUNK + bx] = h;
            if h < hmin { hmin = h; }
            if h > hmax { hmax = h; }

            // Sample biome color
            let ci = idx * 4;
            if ci + 3 < color_data.len() {
                colors[bz * BLOCKS_PER_CHUNK + bx] = [
                    color_data[ci], color_data[ci + 1], color_data[ci + 2], 255
                ];
            } else {
                colors[bz * BLOCKS_PER_CHUNK + bx] = [100, 150, 80, 255]; // fallback green
            }
        }
    }

    // Local normalization: stretch height range to [0, MAX_BLOCK_HEIGHT]
    let h_range = if hmax > hmin { hmax - hmin } else { 1.0 };
    let mut heights = vec![0i32; BLOCKS_PER_CHUNK * BLOCKS_PER_CHUNK];
    for i in 0..heights.len() {
        let normalized = (raw_heights[i] - hmin) / h_range;
        heights[i] = (normalized * MAX_BLOCK_HEIGHT).floor() as i32;
    }

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

    for bz in 0..BLOCKS_PER_CHUNK as i32 {
        for bx in 0..BLOCKS_PER_CHUNK as i32 {
            let h = get_h(bx, bz);
            let x0 = bx as f32 * bs;
            let z0 = bz as f32 * bs;
            let y = h as f32 * bs; // block top Y in world units

            let ci = bz as usize * BLOCKS_PER_CHUNK + bx as usize;
            let [r, g, b, _] = colors[ci];
            let top_color = [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0];
            let side_color = [
                r as f32 / 255.0 * 0.6,
                g as f32 / 255.0 * 0.6,
                b as f32 / 255.0 * 0.6,
                1.0
            ];

            // TOP face (always visible)
            let vi = positions.len() as u32;
            positions.extend_from_slice(&[
                [x0, y, z0],
                [x0 + bs, y, z0],
                [x0 + bs, y, z0 + bs],
                [x0, y, z0 + bs],
            ]);
            normals.extend_from_slice(&[[0.0, 1.0, 0.0]; 4]);
            vertex_colors.extend_from_slice(&[top_color; 4]);
            indices.extend_from_slice(&[vi, vi + 2, vi + 1, vi, vi + 3, vi + 2]);

            // SIDE faces: only where neighbor is shorter
            // North face (-Z)
            if get_h(bx, bz - 1) < h {
                let neighbor_h = get_h(bx, bz - 1).max(0);
                let y_bottom = neighbor_h as f32 * bs;
                let vi = positions.len() as u32;
                positions.extend_from_slice(&[
                    [x0, y, z0], [x0 + bs, y, z0],
                    [x0 + bs, y_bottom, z0], [x0, y_bottom, z0],
                ]);
                normals.extend_from_slice(&[[0.0, 0.0, -1.0]; 4]);
                vertex_colors.extend_from_slice(&[side_color; 4]);
                indices.extend_from_slice(&[vi, vi + 1, vi + 2, vi, vi + 2, vi + 3]);
            }
            // South face (+Z)
            if get_h(bx, bz + 1) < h {
                let neighbor_h = get_h(bx, bz + 1).max(0);
                let y_bottom = neighbor_h as f32 * bs;
                let vi = positions.len() as u32;
                positions.extend_from_slice(&[
                    [x0 + bs, y, z0 + bs], [x0, y, z0 + bs],
                    [x0, y_bottom, z0 + bs], [x0 + bs, y_bottom, z0 + bs],
                ]);
                normals.extend_from_slice(&[[0.0, 0.0, 1.0]; 4]);
                vertex_colors.extend_from_slice(&[side_color; 4]);
                indices.extend_from_slice(&[vi, vi + 1, vi + 2, vi, vi + 2, vi + 3]);
            }
            // West face (-X)
            if get_h(bx - 1, bz) < h {
                let neighbor_h = get_h(bx - 1, bz).max(0);
                let y_bottom = neighbor_h as f32 * bs;
                let vi = positions.len() as u32;
                positions.extend_from_slice(&[
                    [x0, y, z0 + bs], [x0, y, z0],
                    [x0, y_bottom, z0], [x0, y_bottom, z0 + bs],
                ]);
                normals.extend_from_slice(&[[-1.0, 0.0, 0.0]; 4]);
                vertex_colors.extend_from_slice(&[side_color; 4]);
                indices.extend_from_slice(&[vi, vi + 1, vi + 2, vi, vi + 2, vi + 3]);
            }
            // East face (+X)
            if get_h(bx + 1, bz) < h {
                let neighbor_h = get_h(bx + 1, bz).max(0);
                let y_bottom = neighbor_h as f32 * bs;
                let vi = positions.len() as u32;
                positions.extend_from_slice(&[
                    [x0 + bs, y, z0], [x0 + bs, y, z0 + bs],
                    [x0 + bs, y_bottom, z0 + bs], [x0 + bs, y_bottom, z0],
                ]);
                normals.extend_from_slice(&[[1.0, 0.0, 0.0]; 4]);
                vertex_colors.extend_from_slice(&[side_color; 4]);
                indices.extend_from_slice(&[vi, vi + 1, vi + 2, vi, vi + 2, vi + 3]);
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
    mesh
}

// ─── Camera Input ──────────────────────────────────────────────────────────

fn camera_input(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut motion: MessageReader<MouseMotion>,
    time: Res<Time>,
    mut player: ResMut<PlayerState>,
    mut camera_q: Query<&mut Transform, With<Camera3d>>,
    mut cursor_q: Query<&mut CursorOptions, With<PrimaryWindow>>,
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
        player.pitch = (player.pitch + ev.delta.y * MOUSE_SENS).clamp(-MAX_PITCH, MAX_PITCH);
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

    // Camera follows terrain — smoothly interpolate Y toward terrain height
    // For now, just maintain current Y and let user adjust with mouse pitch
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
            if loaded.entities.contains_key(&coord) || in_flight.contains(&coord) {
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
                let mesh = generate_chunk_mesh(&biome_map);
                (coord, ChunkMeshData { mesh, biome_map })
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

            loaded.entities.insert(coord, entity);
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

    let to_remove: Vec<(i32, i32)> = loaded.entities.keys()
        .filter(|(cx, cz)| {
            (cx - cam_cx).abs() > UNLOAD_RADIUS || (cz - cam_cz).abs() > UNLOAD_RADIUS
        })
        .copied()
        .collect();

    for coord in to_remove {
        if let Some(entity) = loaded.entities.remove(&coord) {
            commands.entity(entity).despawn();
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
