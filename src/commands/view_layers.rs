//! Interactive Bevy viewer for layer artifacts.
//!
//! `randlebrot view layers <tag>` opens a minimal Bevy window that displays
//! any layer from a previously generated layers artifact, with an optional
//! semi-transparent overlay layer composited on top.
//!
//! Unlike the full editor GUI, this viewer does NOT spin up the editor stack
//! (rb_editor, world generation pipelines, mode state machine). It uses
//! `DefaultPlugins` + `EguiPlugin` for the HUD and nothing else — loading
//! is a simple matter of reading PNG files from
//! `~/.randlebrot/layers/<tag>/images/`.
//!
//! Controls:
//!   Left/Right arrows — cycle base layer
//!   Up/Down arrows    — cycle overlay layer (wraps through None)
//!   Scroll wheel      — zoom
//!   Click-drag        — pan
//!   ESC               — exit

use std::path::PathBuf;

use bevy::app::AppExit;
use bevy::input::mouse::{MouseMotion, MouseScrollUnit, MouseWheel};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::window::PrimaryWindow;
use bevy_egui::{egui, EguiContexts, EguiPlugin};

use rb_artifacts::ArtifactStore;

// ─── Entry Point ────────────────────────────────────────────────────────────

/// Launch the interactive layer viewer for the given layers artifact tag.
///
/// Validates the tag (returns an error if it does not exist), loads the
/// manifest to discover available layer images, and then runs a minimal
/// Bevy app. The Bevy `App::run()` call blocks until the window is closed.
pub fn run(tag: String) -> Result<(), String> {
    // ─── Validate tag + load manifest ───────────────────────────────────────
    let store = ArtifactStore::new()
        .map_err(|e| format!("failed to initialise artifact store at ~/.randlebrot: {e}"))?;

    let manifest = store.load_layer_manifest(&tag).map_err(|e| match e {
        rb_artifacts::ArtifactError::NotFound { .. } => {
            // Give a helpful hint that lists known tags.
            match store.list_layers() {
                Ok(entries) if !entries.is_empty() => {
                    let available: Vec<&str> = entries.iter().map(|(t, _)| t.as_str()).collect();
                    format!(
                        "layer artifact '{tag}' not found. Available: {}",
                        available.join(", ")
                    )
                }
                _ => format!(
                    "layer artifact '{tag}' not found. \
                     Run `randlebrot generate layers <seed> <tag>` to create one."
                ),
            }
        }
        other => format!("failed to load layer manifest for '{tag}': {other}"),
    })?;

    if manifest.layer_images.is_empty() {
        return Err(format!(
            "layer artifact '{tag}' has no images in its manifest — cannot display"
        ));
    }

    // Resolve on-disk paths for every layer image in the manifest.
    let layer_files: Vec<String> = manifest.layer_images.clone();
    let layer_paths: Vec<PathBuf> = layer_files
        .iter()
        .map(|name| store.layer_image_path(&tag, name))
        .collect();

    // Verify at least the first image exists (sanity check — a malformed
    // manifest should fail fast rather than panicking inside Bevy startup).
    if !layer_paths[0].exists() {
        return Err(format!(
            "layer image '{}' not found on disk at {}. \
             The artifact may be corrupted — try regenerating it.",
            layer_files[0],
            layer_paths[0].display(),
        ));
    }

    // Pick the initial base layer: prefer "biome.png" if present, otherwise
    // fall back to the first layer in the manifest.
    let base_index = layer_files
        .iter()
        .position(|name| name == "biome.png")
        .unwrap_or(0);

    // ─── Build Bevy app ─────────────────────────────────────────────────────
    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: format!("Randlebrot - Layers: {tag}"),
            resolution: (1280u32, 720u32).into(),
            ..default()
        }),
        ..default()
    }));

    // Egui is only used for the minimal HUD overlay.
    app.add_plugins(EguiPlugin {
        enable_multipass_for_primary_context: false,
        ..Default::default()
    });

    app.insert_resource(ViewerState {
        tag,
        layer_files,
        layer_paths,
        base_index,
        overlay_index: None,
        lru: Vec::with_capacity(LRU_CAPACITY),
        dirty: true,
        needs_fit_to_window: true,
    });

    app.add_systems(Startup, setup);
    app.add_systems(
        Update,
        (
            handle_keyboard_input,
            update_sprite_textures,
            fit_camera_to_first_image.after(update_sprite_textures),
            viewer_camera_zoom,
            viewer_camera_pan,
            hud_system,
            exit_on_window_close,
        ),
    );

    app.run();
    Ok(())
}

// ─── Resources + Components ─────────────────────────────────────────────────

/// Maximum number of textures to keep in the LRU cache.
///
/// The viewer only ever displays at most 2 layers at once (base + overlay),
/// so a small cache (~4 entries) is enough to keep back-and-forth cycling
/// fast while bounding GPU memory.
const LRU_CAPACITY: usize = 4;

/// Viewer state: active tag, layer list, current base/overlay indices,
/// and a simple Vec-based LRU texture cache keyed by layer index.
#[derive(Resource)]
struct ViewerState {
    tag: String,
    /// Image filenames from the manifest (e.g. "biome.png", "heightmap.png").
    layer_files: Vec<String>,
    /// Absolute paths resolved via `ArtifactStore::layer_image_path`.
    layer_paths: Vec<PathBuf>,
    /// Index into `layer_files` for the currently displayed base layer.
    base_index: usize,
    /// Index into `layer_files` for the overlay layer, or `None` for no overlay.
    overlay_index: Option<usize>,
    /// LRU cache: (layer_index, texture_handle). Most recently used at the end.
    lru: Vec<(usize, Handle<Image>)>,
    /// Flag set when base_index or overlay_index changes, prompting a
    /// sprite texture refresh in `update_sprite_textures`.
    dirty: bool,
    /// On the very first load we don't know the image size until after disk
    /// decode, so we set the camera's ortho scale to fit the image to the
    /// window on the first successful base load and then clear this flag.
    needs_fit_to_window: bool,
}

impl ViewerState {
    /// Fetch a texture handle from the LRU cache, or load it from disk on miss.
    ///
    /// Loading converts the PNG to a Bevy `Image` using nearest-neighbor
    /// sampling so the pixel grid stays crisp when zoomed.
    fn get_or_load(
        &mut self,
        index: usize,
        images: &mut Assets<Image>,
    ) -> Result<Handle<Image>, String> {
        // Cache hit: promote to most-recently-used.
        if let Some(pos) = self.lru.iter().position(|(i, _)| *i == index) {
            let entry = self.lru.remove(pos);
            let handle = entry.1.clone();
            self.lru.push(entry);
            return Ok(handle);
        }

        // Cache miss: load from disk.
        let path = &self.layer_paths[index];
        let img = image::open(path).map_err(|e| {
            format!(
                "failed to load layer image '{}' from {}: {e}",
                self.layer_files[index],
                path.display()
            )
        })?;
        let rgba = img.to_rgba8();
        let (width, height) = rgba.dimensions();
        let raw = rgba.into_raw();

        let mut bevy_image = Image::new(
            Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            raw,
            TextureFormat::Rgba8UnormSrgb,
            default(),
        );
        // Nearest-neighbor so the debug pixels stay crisp at high zoom.
        bevy_image.sampler =
            bevy::image::ImageSampler::Descriptor(bevy::image::ImageSamplerDescriptor {
                mag_filter: bevy::image::ImageFilterMode::Nearest,
                min_filter: bevy::image::ImageFilterMode::Nearest,
                ..default()
            });

        let handle = images.add(bevy_image);

        // Evict the oldest entry if we're at capacity.
        if self.lru.len() >= LRU_CAPACITY {
            self.lru.remove(0);
        }
        self.lru.push((index, handle.clone()));

        Ok(handle)
    }
}

/// Marker for the base-layer sprite (z=0, full opacity).
#[derive(Component)]
struct BaseLayerSprite;

/// Marker for the overlay-layer sprite (z=1, alpha-blended, hidden when no overlay).
#[derive(Component)]
struct OverlayLayerSprite;

// ─── Startup ────────────────────────────────────────────────────────────────

/// Spawn the camera and two empty sprites (base + overlay).
///
/// Textures are filled in by `update_sprite_textures` on the first frame,
/// which reads the initial `base_index` from `ViewerState` and the
/// `dirty = true` flag set during `run()`.
fn setup(mut commands: Commands) {
    commands.spawn(Camera2d);

    // Base sprite — filled in on first frame by update_sprite_textures.
    commands.spawn((
        Sprite {
            color: Color::WHITE,
            ..default()
        },
        Transform::from_xyz(0.0, 0.0, 0.0),
        BaseLayerSprite,
    ));

    // Overlay sprite — hidden until an overlay is selected.
    commands.spawn((
        Sprite {
            // Semi-transparent white tint multiplied with the texture.
            color: Color::srgba(1.0, 1.0, 1.0, 0.5),
            ..default()
        },
        Transform::from_xyz(0.0, 0.0, 1.0),
        Visibility::Hidden,
        OverlayLayerSprite,
    ));
}

// ─── Input Handling ─────────────────────────────────────────────────────────

/// Keyboard cycling + ESC exit.
///
/// Left/Right cycle the base layer through `layer_files` (wrapping).
/// Up/Down cycle the overlay layer through `None → 0 → ... → last → None`.
/// Same-frame multiple keypresses are handled via `just_pressed`, so holding
/// a key does not cycle rapidly.
fn handle_keyboard_input(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut state: ResMut<ViewerState>,
    mut exit_events: MessageWriter<AppExit>,
) {
    if keyboard.just_pressed(KeyCode::Escape) {
        exit_events.write(AppExit::Success);
        return;
    }

    let count = state.layer_files.len();
    if count == 0 {
        return;
    }

    // Base layer cycling (Left/Right).
    if keyboard.just_pressed(KeyCode::ArrowRight) {
        state.base_index = (state.base_index + 1) % count;
        state.dirty = true;
    }
    if keyboard.just_pressed(KeyCode::ArrowLeft) {
        state.base_index = (state.base_index + count - 1) % count;
        state.dirty = true;
    }

    // Overlay cycling (Up/Down) — steps through None → 0 → ... → last → None.
    // Using `count + 1` slots lets us encode the None state as index `count`.
    if keyboard.just_pressed(KeyCode::ArrowDown) {
        let next = match state.overlay_index {
            None => Some(0),
            Some(i) if i + 1 >= count => None,
            Some(i) => Some(i + 1),
        };
        state.overlay_index = next;
        state.dirty = true;
    }
    if keyboard.just_pressed(KeyCode::ArrowUp) {
        let prev = match state.overlay_index {
            None => Some(count - 1),
            Some(0) => None,
            Some(i) => Some(i - 1),
        };
        state.overlay_index = prev;
        state.dirty = true;
    }
}

/// Exit the Bevy app cleanly when the user closes the window with the OS
/// close button. `AppExit` is what `DefaultPlugins` listens for to shut down
/// the runner.
fn exit_on_window_close(
    windows: Query<(), With<PrimaryWindow>>,
    mut exit_events: MessageWriter<AppExit>,
) {
    if windows.is_empty() {
        exit_events.write(AppExit::Success);
    }
}

// ─── Sprite Updates ─────────────────────────────────────────────────────────

/// Refresh the base and overlay sprite textures whenever `ViewerState.dirty`
/// is set. Sizes the sprites to match the loaded PNG dimensions so the
/// full image is visible after the first load.
fn update_sprite_textures(
    mut state: ResMut<ViewerState>,
    mut images: ResMut<Assets<Image>>,
    mut base_query: Query<
        &mut Sprite,
        (With<BaseLayerSprite>, Without<OverlayLayerSprite>),
    >,
    mut overlay_query: Query<
        (&mut Sprite, &mut Visibility),
        (With<OverlayLayerSprite>, Without<BaseLayerSprite>),
    >,
) {
    if !state.dirty {
        return;
    }
    state.dirty = false;

    // ─── Base layer ────────────────────────────────────────────────────────
    let base_idx = state.base_index;
    let base_handle = match state.get_or_load(base_idx, &mut images) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("viewer: {e}");
            return;
        }
    };

    // Determine natural pixel size from the loaded image — the sprite's
    // `custom_size` defaults to `None`, which makes Bevy render the texture
    // at 1 pixel per texel. We explicitly set `custom_size` to the image
    // dimensions so that both base and overlay sprites are always the same
    // world size (matters when cycling between layers with different PNG
    // sizes, though in practice they should all be 4096x2048).
    let natural_size = images
        .get(&base_handle)
        .map(|img| {
            let d = img.texture_descriptor.size;
            Vec2::new(d.width as f32, d.height as f32)
        })
        .unwrap_or(Vec2::new(4096.0, 2048.0));

    if let Ok(mut base_sprite) = base_query.single_mut() {
        base_sprite.image = base_handle.clone();
        base_sprite.custom_size = Some(natural_size);
    }

    // ─── Overlay layer ─────────────────────────────────────────────────────
    if let Some(overlay_idx) = state.overlay_index {
        let overlay_handle = match state.get_or_load(overlay_idx, &mut images) {
            Ok(h) => h,
            Err(e) => {
                eprintln!("viewer: {e}");
                return;
            }
        };
        if let Ok((mut overlay_sprite, mut visibility)) = overlay_query.single_mut() {
            overlay_sprite.image = overlay_handle;
            overlay_sprite.custom_size = Some(natural_size);
            *visibility = Visibility::Visible;
        }
    } else if let Ok((_, mut visibility)) = overlay_query.single_mut() {
        *visibility = Visibility::Hidden;
    }
}

/// On the very first frame where a base texture is loaded, set the camera's
/// orthographic scale so the full image fits inside the window with a small
/// margin. Runs after `update_sprite_textures` so the base sprite has its
/// `custom_size` populated with the natural PNG dimensions.
///
/// This is a one-shot system — `needs_fit_to_window` is cleared after the
/// first successful fit so the user's subsequent zoom inputs are respected.
fn fit_camera_to_first_image(
    mut state: ResMut<ViewerState>,
    mut projection_query: Query<&mut Projection, With<Camera2d>>,
    window_query: Query<&Window, With<PrimaryWindow>>,
    base_query: Query<&Sprite, With<BaseLayerSprite>>,
) {
    if !state.needs_fit_to_window {
        return;
    }

    let Ok(base_sprite) = base_query.single() else { return };
    let Some(natural_size) = base_sprite.custom_size else { return };
    if natural_size.x <= 0.0 || natural_size.y <= 0.0 {
        return;
    }
    let Ok(window) = window_query.single() else { return };
    let (window_w, window_h) = (window.width().max(1.0), window.height().max(1.0));

    // Pick the axis that needs the larger ortho scale so the whole image fits,
    // then add 5% padding so the edges aren't flush against the window.
    let fit_scale = (natural_size.x / window_w).max(natural_size.y / window_h) * 1.05;
    let clamped = fit_scale.clamp(0.05, 20.0);

    for mut projection in &mut projection_query {
        if let Projection::Orthographic(ref mut ortho) = *projection {
            ortho.scale = clamped;
        }
    }

    state.needs_fit_to_window = false;
}

// ─── Camera ─────────────────────────────────────────────────────────────────

/// Scroll-wheel zoom with the same feel as the main-crate world map camera.
/// Cursor-over-egui-UI suppresses scroll events so the HUD panel doesn't
/// eat zoom input.
fn viewer_camera_zoom(
    mut scroll_events: MessageReader<MouseWheel>,
    mut query: Query<&mut Projection, With<Camera2d>>,
    mut contexts: EguiContexts,
) {
    if contexts
        .ctx_mut()
        .map(|ctx| ctx.is_pointer_over_area())
        .unwrap_or(false)
    {
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

    for mut projection in &mut query {
        if let Projection::Orthographic(ref mut ortho) = *projection {
            // Wider upper clamp than the main world map camera (which caps at 10.0)
            // so a 4096x2048 layer image can be fully zoomed out in the default
            // 1280x720 window (~3.2x scale needed to fit) with headroom to spare.
            let zoom_factor = 1.0 - scroll_delta;
            ortho.scale = (ortho.scale * zoom_factor).clamp(0.05, 20.0);
        }
    }
}

/// Click-drag panning. Copies the pattern used by `camera_pan` in main.rs:
/// accumulate motion deltas while left mouse is held (and not over egui UI),
/// then apply them in screen-space (scaled by the ortho scale).
fn viewer_camera_pan(
    mouse: Res<ButtonInput<MouseButton>>,
    mut motion_events: MessageReader<MouseMotion>,
    mut query: Query<(&mut Transform, &Projection), With<Camera2d>>,
    mut contexts: EguiContexts,
) {
    let over_ui = contexts
        .ctx_mut()
        .map(|ctx| ctx.is_pointer_over_area())
        .unwrap_or(false);

    let mut pan_delta = Vec2::ZERO;
    if mouse.pressed(MouseButton::Left) && !over_ui {
        for event in motion_events.read() {
            pan_delta.x -= event.delta.x;
            pan_delta.y += event.delta.y;
        }
    } else {
        motion_events.clear();
    }

    if pan_delta == Vec2::ZERO {
        return;
    }

    for (mut transform, projection) in &mut query {
        let scale = match projection {
            Projection::Orthographic(o) => o.scale,
            _ => 1.0,
        };
        transform.translation.x += pan_delta.x * scale;
        transform.translation.y += pan_delta.y * scale;
    }
}

// ─── HUD ────────────────────────────────────────────────────────────────────

/// Minimal egui HUD showing the active tag, base/overlay layer names, and
/// the current zoom as a percentage (100% = default scale).
fn hud_system(
    mut contexts: EguiContexts,
    state: Res<ViewerState>,
    camera_query: Query<&Projection, With<Camera2d>>,
) {
    let Ok(ctx) = contexts.ctx_mut() else { return };

    let base_name = state
        .layer_files
        .get(state.base_index)
        .map(|f| display_name_from_filename(f))
        .unwrap_or_else(|| "—".to_string());

    let overlay_name = state
        .overlay_index
        .and_then(|i| state.layer_files.get(i))
        .map(|f| display_name_from_filename(f))
        .unwrap_or_else(|| "None".to_string());

    // Zoom as a percentage (100% = default scale of 1.0).
    let zoom_percent = camera_query
        .single()
        .ok()
        .and_then(|p| match p {
            Projection::Orthographic(o) => Some(100.0 / o.scale),
            _ => None,
        })
        .unwrap_or(100.0);

    egui::Window::new("Layer Viewer")
        .anchor(egui::Align2::LEFT_TOP, [8.0, 8.0])
        .resizable(false)
        .collapsible(false)
        .show(ctx, |ui| {
            ui.label(format!("Tag:     {}", state.tag));
            ui.label(format!("Base:    {base_name}"));
            ui.label(format!("Overlay: {overlay_name}"));
            ui.label(format!("Zoom:    {zoom_percent:.0}%"));
            ui.separator();
            ui.small("Left/Right: base · Up/Down: overlay");
            ui.small("Scroll: zoom · Drag: pan · ESC: exit");
        });
}

/// Convert a layer image filename (e.g. "rock_hardness.png") to a
/// human-readable display name (e.g. "Rock Hardness").
///
/// Mirrors the inverse of `layer_file_name()` in `generate_layers.rs`,
/// but we operate on strings here so we don't need to import
/// `rb_noise::NoiseLayer` just to look up a label. Filenames not in the
/// known set fall back to a best-effort snake_case → Title Case conversion.
fn display_name_from_filename(filename: &str) -> String {
    // Strip .png (or any extension) for the lookup + fallback formatting.
    let stem = filename
        .rsplit_once('.')
        .map(|(stem, _)| stem)
        .unwrap_or(filename);

    match stem {
        "biome" => "Biome".to_string(),
        "continentalness" => "Continentalness".to_string(),
        "tectonic" => "Tectonic Stress".to_string(),
        "humidity" => "Humidity".to_string(),
        "rock_hardness" => "Rock Hardness".to_string(),
        "light_level" => "Light Level".to_string(),
        "peaks_valleys" => "Peaks & Valleys".to_string(),
        "volcanism" => "Volcanism".to_string(),
        "heightmap" => "Heightmap".to_string(),
        "temperature" => "Temperature".to_string(),
        "erosion" => "Erosion".to_string(),
        "river_flow" => "River Flow".to_string(),
        "aridity" => "Aridity".to_string(),
        "precipitation_type" => "Precipitation Type".to_string(),
        "water_table" => "Water Table".to_string(),
        "wind" => "Wind Speed".to_string(),
        "resources" => "Resources".to_string(),
        "snowpack" => "Snowpack".to_string(),
        "vegetation_density" => "Vegetation Density".to_string(),
        "soil_type" => "Soil Type".to_string(),
        // Unknown filename — fall back to snake_case → Title Case.
        other => snake_case_to_title_case(other),
    }
}

/// Fallback filename formatter for layer names not in the known set.
/// Converts `rock_hardness` → `Rock Hardness`.
fn snake_case_to_title_case(s: &str) -> String {
    s.split('_')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_name_maps_known_layers() {
        assert_eq!(display_name_from_filename("biome.png"), "Biome");
        assert_eq!(display_name_from_filename("rock_hardness.png"), "Rock Hardness");
        assert_eq!(display_name_from_filename("peaks_valleys.png"), "Peaks & Valleys");
        assert_eq!(display_name_from_filename("precipitation_type.png"), "Precipitation Type");
        assert_eq!(display_name_from_filename("wind.png"), "Wind Speed");
    }

    #[test]
    fn display_name_fallback_on_unknown() {
        // Unknown filenames fall back to snake_case → Title Case.
        assert_eq!(display_name_from_filename("some_new_layer.png"), "Some New Layer");
        assert_eq!(display_name_from_filename("foo.png"), "Foo");
    }

    #[test]
    fn display_name_handles_missing_extension() {
        assert_eq!(display_name_from_filename("biome"), "Biome");
    }

    #[test]
    fn snake_case_title_case() {
        assert_eq!(snake_case_to_title_case("rock_hardness"), "Rock Hardness");
        assert_eq!(snake_case_to_title_case("single"), "Single");
        assert_eq!(snake_case_to_title_case(""), "");
    }

    /// Drift check: for every `NoiseLayer`, the filename emitted by
    /// `generate_layers::layer_file_name` (what ends up in the manifest)
    /// must round-trip through `display_name_from_filename` back to the same
    /// string as `NoiseLayer::name()`. If someone renames a layer in
    /// `rb_noise` and forgets to update either the file name table in
    /// `generate_layers.rs` or the display-name table here, this test fails.
    #[test]
    fn display_name_matches_noise_layer_name_for_all_layers() {
        use crate::commands::generate_layers::layer_file_name;
        use rb_noise::NoiseLayer;

        for &layer in NoiseLayer::all() {
            let filename = layer_file_name(layer);
            let display = display_name_from_filename(&filename);
            assert_eq!(
                display,
                layer.name(),
                "display_name_from_filename({filename:?}) = {display:?}, \
                 but NoiseLayer::{layer:?}::name() = {:?}",
                layer.name(),
            );
        }
    }
}
