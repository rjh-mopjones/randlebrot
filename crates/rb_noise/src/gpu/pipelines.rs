//! Compute pipeline management for noise shaders.
//!
//! Uses exact OpenSimplex2D algorithm matching the noise crate for CPU/GPU parity.

use wgpu::{BindGroupLayout, ComputePipeline, Device};

/// All noise compute pipelines and their bind group layouts.
pub struct NoisePipelines {
    pub continentalness: ComputePipeline,
    pub continentalness_layout: BindGroupLayout,

    pub temperature: ComputePipeline,
    pub temperature_layout: BindGroupLayout,

    pub tectonic: ComputePipeline,
    pub tectonic_layout: BindGroupLayout,

    pub peaks_valleys: ComputePipeline,
    pub peaks_valleys_layout: BindGroupLayout,

    pub erosion: ComputePipeline,
    pub erosion_layout: BindGroupLayout,

    pub humidity: ComputePipeline,
    pub humidity_layout: BindGroupLayout,
}

impl NoisePipelines {
    /// Create all compute pipelines.
    pub fn new(device: &Device) -> Self {
        // Build full shader strings: bindings + OpenSimplex functions + main
        // Independent shaders (params, perm_table, output)
        let continentalness_shader = format!(
            "{}{}{}",
            INDEPENDENT_BINDINGS, OPEN_SIMPLEX_2D_FUNCS, CONTINENTALNESS_MAIN
        );
        let temperature_shader = format!(
            "{}{}{}",
            INDEPENDENT_BINDINGS, OPEN_SIMPLEX_2D_FUNCS, TEMPERATURE_MAIN
        );
        let tectonic_shader = format!(
            "{}{}{}",
            INDEPENDENT_BINDINGS, OPEN_SIMPLEX_2D_FUNCS, TECTONIC_MAIN
        );
        let peaks_valleys_shader = format!(
            "{}{}{}",
            INDEPENDENT_BINDINGS, OPEN_SIMPLEX_2D_FUNCS, PEAKS_VALLEYS_MAIN
        );
        // Dependent shaders (params, perm_table, continentalness, output)
        let erosion_shader = format!(
            "{}{}{}",
            DEPENDENT_BINDINGS, OPEN_SIMPLEX_2D_FUNCS, EROSION_MAIN
        );
        let humidity_shader = format!(
            "{}{}{}",
            DEPENDENT_BINDINGS, OPEN_SIMPLEX_2D_FUNCS, HUMIDITY_MAIN
        );

        // Create pipelines for each noise type
        let (continentalness, continentalness_layout) =
            Self::create_perm_pipeline(device, "Continentalness", &continentalness_shader);

        let (temperature, temperature_layout) =
            Self::create_perm_pipeline(device, "Temperature", &temperature_shader);

        let (tectonic, tectonic_layout) =
            Self::create_perm_pipeline(device, "Tectonic", &tectonic_shader);

        let (peaks_valleys, peaks_valleys_layout) =
            Self::create_perm_pipeline(device, "PeaksValleys", &peaks_valleys_shader);

        let (erosion, erosion_layout) =
            Self::create_dependent_perm_pipeline(device, "Erosion", &erosion_shader);

        let (humidity, humidity_layout) =
            Self::create_dependent_perm_pipeline(device, "Humidity", &humidity_shader);

        Self {
            continentalness,
            continentalness_layout,
            temperature,
            temperature_layout,
            tectonic,
            tectonic_layout,
            peaks_valleys,
            peaks_valleys_layout,
            erosion,
            erosion_layout,
            humidity,
            humidity_layout,
        }
    }

    /// Create a pipeline with permutation table for OpenSimplex noise.
    /// Bindings: 0=params, 1=perm_table, 2=output
    fn create_perm_pipeline(
        device: &Device,
        name: &str,
        shader_source: &str,
    ) -> (ComputePipeline, BindGroupLayout) {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(&format!("{} shader", name)),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some(&format!("{} bind group layout", name)),
            entries: &[
                // Params uniform buffer
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // Permutation table (256 u32 values)
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // Output storage buffer
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some(&format!("{} pipeline layout", name)),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some(&format!("{} pipeline", name)),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });

        (pipeline, bind_group_layout)
    }

    /// Create a pipeline for noise that depends on continentalness + uses perm table.
    /// Bindings: 0=params, 1=perm_table, 2=continentalness_input, 3=output
    fn create_dependent_perm_pipeline(
        device: &Device,
        name: &str,
        shader_source: &str,
    ) -> (ComputePipeline, BindGroupLayout) {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(&format!("{} shader", name)),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some(&format!("{} bind group layout", name)),
            entries: &[
                // Params uniform buffer
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // Permutation table
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // Continentalness input buffer
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // Output storage buffer
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some(&format!("{} pipeline layout", name)),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some(&format!("{} pipeline", name)),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });

        (pipeline, bind_group_layout)
    }
}

// =============================================================================
// WGSL SHADERS - Using exact OpenSimplex2D algorithm
// =============================================================================

/// OpenSimplex2D implementation - functions access perm_table binding directly.
/// Binding must be declared BEFORE this code is included.
const OPEN_SIMPLEX_2D_FUNCS: &str = r#"
// OpenSimplex2D constants
const STRETCH_2D: f32 = -0.211324865405187; // (1/sqrt(2+1)-1)/2
const SQUISH_2D: f32 = 0.366025403784439;   // (sqrt(2+1)-1)/2
const NORM_2D: f32 = 1.0 / 14.0;
const DIAG: f32 = 0.7071067811865476;       // 1/sqrt(2)

// 8 gradient vectors for 2D (matching noise crate's grad2)
fn grad2(index: u32) -> vec2<f32> {
    switch (index % 8u) {
        case 0u: { return vec2<f32>(1.0, 0.0); }
        case 1u: { return vec2<f32>(-1.0, 0.0); }
        case 2u: { return vec2<f32>(0.0, 1.0); }
        case 3u: { return vec2<f32>(0.0, -1.0); }
        case 4u: { return vec2<f32>(DIAG, DIAG); }
        case 5u: { return vec2<f32>(-DIAG, DIAG); }
        case 6u: { return vec2<f32>(DIAG, -DIAG); }
        case 7u: { return vec2<f32>(-DIAG, -DIAG); }
        default: { return vec2<f32>(0.0, 0.0); }
    }
}

// Hash function matching noise crate's PermutationTable::hash()
// Accesses global perm_table binding directly
fn perm_hash(x: i32, y: i32) -> u32 {
    let a = perm_table[u32(x & 255)];
    let b = a ^ u32(y & 255);
    return perm_table[b & 255u];
}

// Surflet contribution
fn surflet(index: u32, point: vec2<f32>) -> f32 {
    let t = 2.0 - dot(point, point);
    if (t > 0.0) {
        let gradient = grad2(index);
        let t2 = t * t;
        return t2 * t2 * dot(point, gradient);
    }
    return 0.0;
}

// OpenSimplex 2D noise - exact port from noise crate
fn open_simplex_2d(x: f32, y: f32) -> f32 {
    // Place input coordinates onto grid
    let stretch_offset = (x + y) * STRETCH_2D;
    let xs = x + stretch_offset;
    let ys = y + stretch_offset;

    // Floor to get grid coordinates of rhombus cell origin
    let xsb = i32(floor(xs));
    let ysb = i32(floor(ys));

    // Skew out to get actual coordinates of rhombus origin
    let squish_offset = f32(xsb + ysb) * SQUISH_2D;
    let xb = f32(xsb) + squish_offset;
    let yb = f32(ysb) + squish_offset;

    // Compute grid coordinates relative to rhombus origin
    let xins = xs - f32(xsb);
    let yins = ys - f32(ysb);

    // Sum to determine which region we're in
    let in_sum = xins + yins;

    // Positions relative to origin point
    let dx0 = x - xb;
    let dy0 = y - yb;

    var value = 0.0;

    // Contribution (1, 0)
    let dx1 = dx0 - 1.0 - SQUISH_2D;
    let dy1 = dy0 - SQUISH_2D;
    let index1 = perm_hash(xsb + 1, ysb);
    value += surflet(index1, vec2<f32>(dx1, dy1));

    // Contribution (0, 1)
    let dx2 = dx0 - SQUISH_2D;
    let dy2 = dy0 - 1.0 - SQUISH_2D;
    let index2 = perm_hash(xsb, ysb + 1);
    value += surflet(index2, vec2<f32>(dx2, dy2));

    if (in_sum > 1.0) {
        // Contribution (1, 1)
        let dx3 = dx0 - 1.0 - 2.0 * SQUISH_2D;
        let dy3 = dy0 - 1.0 - 2.0 * SQUISH_2D;
        let index3 = perm_hash(xsb + 1, ysb + 1);
        value += surflet(index3, vec2<f32>(dx3, dy3));
    } else {
        // Contribution (0, 0)
        let index0 = perm_hash(xsb, ysb);
        value += surflet(index0, vec2<f32>(dx0, dy0));
    }

    return value * NORM_2D;
}

// fBm using OpenSimplex
fn fbm_open_simplex(x: f32, y: f32, octaves: u32, freq: f32, persistence: f32, lacunarity: f32) -> f32 {
    var value = 0.0;
    var amplitude = 1.0;
    var f = freq;
    var max_amplitude = 0.0;

    for (var i = 0u; i < octaves; i++) {
        value += open_simplex_2d(x * f, y * f) * amplitude;
        max_amplitude += amplitude;
        amplitude *= persistence;
        f *= lacunarity;
    }

    return value / max_amplitude;
}

// Ridged multifractal using OpenSimplex
fn ridged_open_simplex(x: f32, y: f32, octaves: u32, freq: f32, persistence: f32, lacunarity: f32) -> f32 {
    var value = 0.0;
    var amplitude = 1.0;
    var f = freq;
    var max_amplitude = 0.0;

    for (var i = 0u; i < octaves; i++) {
        let n = open_simplex_2d(x * f, y * f);
        value += (1.0 - abs(n)) * amplitude;
        max_amplitude += amplitude;
        amplitude *= persistence;
        f *= lacunarity;
    }

    return (value / max_amplitude) * 2.0 - 1.0;
}
"#;

/// Common bindings header for independent shaders (params, perm_table, output)
const INDEPENDENT_BINDINGS: &str = r#"
struct Params {
    seed: u32,
    width: u32,
    height: u32,
    octaves: u32,
    frequency: f32,
    persistence: f32,
    lacunarity: f32,
    scale: f32,
    world_x: f32,
    world_y: f32,
    world_height: f32,
    _padding: f32,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> perm_table: array<u32>;
@group(0) @binding(2) var<storage, read_write> output: array<f32>;
"#;

/// Continentalness shader main - 16-octave fBm with exact OpenSimplex.
const CONTINENTALNESS_MAIN: &str = r#"
@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= params.width || gid.y >= params.height) { return; }

    let idx = gid.y * params.width + gid.x;
    let wx = params.world_x + f32(gid.x) * params.scale;
    let wy = params.world_y + f32(gid.y) * params.scale;

    // fBm with 0.01 scale factor (matching CPU)
    let nx = wx * 0.01;
    let ny = wy * 0.01;

    output[idx] = fbm_open_simplex(nx, ny, params.octaves, params.frequency, params.persistence, params.lacunarity);
}
"#;

/// Temperature shader main - latitude-based with OpenSimplex noise.
const TEMPERATURE_MAIN: &str = r#"
@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= params.width || gid.y >= params.height) { return; }

    let idx = gid.y * params.width + gid.x;
    let wx = params.world_x + f32(gid.x) * params.scale;
    let wy = params.world_y + f32(gid.y) * params.scale;

    // Latitude-based temperature (matching CPU LatitudeTemperatureStrategy)
    let latitude_factor = wy / params.world_height;

    // Noise for variation (scale 0.02)
    let nx = wx * 0.02;
    let ny = wy * 0.02;
    let noise = fbm_open_simplex(nx, ny, params.octaves, params.frequency, params.persistence, params.lacunarity);

    // Temperature formula: (latitude * 150) - 50 + (noise * 100)
    let temperature = latitude_factor * 150.0 - 50.0 + noise * 100.0;

    output[idx] = temperature;
}
"#;

/// Tectonic plates shader - Voronoi cells with OpenSimplex roughness.
/// Contains the cell_hash function and main entry point.
const TECTONIC_MAIN: &str = r#"
// Hash function matching CPU TectonicPlatesStrategy::hash()
// IMPORTANT: Must do signed multiplication first, then convert to unsigned,
// to match Rust's (ix.wrapping_mul(374761393) as u32) behavior
fn cell_hash(ix: i32, iy: i32, seed: u32) -> vec2<f32> {
    // Signed multiplication (wrapping) then convert to unsigned
    let term1 = ix * 374761393;      // i32 wrapping multiplication
    let term2 = iy * 668265263;      // i32 wrapping multiplication
    var n = u32(term1) + u32(term2) + seed;  // Now convert to u32 and add
    let n1 = n * 1103515245u + 12345u;
    let n2 = n1 * 1103515245u + 12345u;
    let x = f32(n1 & 0x7FFFFFFFu) / f32(0x7FFFFFFFu);
    let y = f32(n2 & 0x7FFFFFFFu) / f32(0x7FFFFFFFu);
    return vec2<f32>(x, y);
}

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= params.width || gid.y >= params.height) { return; }

    let idx = gid.y * params.width + gid.x;
    let wx = params.world_x + f32(gid.x) * params.scale;
    let wy = params.world_y + f32(gid.y) * params.scale;

    // Scale for tectonic plates - matches CPU plate_scale = 0.004
    let plate_scale = 0.004;
    let sx = wx * plate_scale;
    let sy = wy * plate_scale;

    let ix = i32(floor(sx));
    let iy = i32(floor(sy));

    var min_dist = 1e10f;
    var second_dist = 1e10f;

    // Check 3x3 grid of cells
    for (var dy = -1; dy <= 1; dy++) {
        for (var dx = -1; dx <= 1; dx++) {
            let cell_x = ix + dx;
            let cell_y = iy + dy;
            let offset = cell_hash(cell_x, cell_y, params.seed);
            let cx = f32(cell_x) + offset.x;
            let cy = f32(cell_y) + offset.y;
            let dist = sqrt((sx - cx) * (sx - cx) + (sy - cy) * (sy - cy));

            if (dist < min_dist) {
                second_dist = min_dist;
                min_dist = dist;
            } else if (dist < second_dist) {
                second_dist = dist;
            }
        }
    }

    // Boundary distance calculation - matches CPU
    var ratio = 0.0f;
    if (second_dist > 0.001) {
        ratio = min_dist / second_dist;
    }
    var boundary_dist = clamp(1.0 - ratio, 0.0, 1.0);

    // Add roughness using OpenSimplex (matching CPU's noise.get([x * 0.02, y * 0.02]) * 0.1)
    let roughness = open_simplex_2d(wx * 0.02, wy * 0.02) * 0.1;
    let adjusted_boundary = clamp(boundary_dist + roughness, 0.0, 1.0);

    output[idx] = adjusted_boundary;
}
"#;

/// Peaks and valleys shader main - ridged multifractal with OpenSimplex.
const PEAKS_VALLEYS_MAIN: &str = r#"
@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= params.width || gid.y >= params.height) { return; }

    let idx = gid.y * params.width + gid.x;
    let wx = params.world_x + f32(gid.x) * params.scale;
    let wy = params.world_y + f32(gid.y) * params.scale;

    // Ridged multifractal for peaks/valleys (scale 0.015 matching CPU)
    let nx = wx * 0.015;
    let ny = wy * 0.015;

    output[idx] = ridged_open_simplex(nx, ny, params.octaves, params.frequency, params.persistence, params.lacunarity);
}
"#;

/// Bindings for dependent shaders (params, perm_table, continentalness, output)
const DEPENDENT_BINDINGS: &str = r#"
struct Params {
    seed: u32,
    width: u32,
    height: u32,
    octaves: u32,
    frequency: f32,
    persistence: f32,
    lacunarity: f32,
    scale: f32,
    world_x: f32,
    world_y: f32,
    world_height: f32,
    _padding: f32,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> perm_table: array<u32>;
@group(0) @binding(2) var<storage, read> continentalness: array<f32>;
@group(0) @binding(3) var<storage, read_write> output: array<f32>;
"#;

/// Erosion shader main - depends on continentalness.
const EROSION_MAIN: &str = r#"
@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= params.width || gid.y >= params.height) { return; }

    let idx = gid.y * params.width + gid.x;
    let wx = params.world_x + f32(gid.x) * params.scale;
    let wy = params.world_y + f32(gid.y) * params.scale;
    let cont = continentalness[idx];

    // Erosion with continentalness modulation (scale 0.02 matching CPU)
    let nx = wx * 0.02;
    let ny = wy * 0.02;
    let base_erosion = fbm_open_simplex(nx, ny, params.octaves, params.frequency, params.persistence, params.lacunarity);

    // Normalize to [0, 1] and modulate by elevation
    let normalized = (base_erosion + 1.0) * 0.5;
    let elevation_factor = clamp((cont + 0.025) * 2.0, 0.0, 1.0);
    let erosion = normalized * elevation_factor;

    output[idx] = clamp(erosion, 0.0, 1.0);
}
"#;

/// Humidity shader main - depends on continentalness, tidally locked model.
const HUMIDITY_MAIN: &str = r#"
@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= params.width || gid.y >= params.height) { return; }

    let idx = gid.y * params.width + gid.x;
    let wx = params.world_x + f32(gid.x) * params.scale;
    let wy = params.world_y + f32(gid.y) * params.scale;
    let cont = continentalness[idx];

    // Tidally locked humidity model (matching CPU HumidityStrategy)
    let latitude_factor = abs(wy / params.world_height - 0.5) * 2.0;

    // Ocean proximity
    let sea_level = -0.025;
    let ocean_proximity = select(0.0, clamp(1.0 - (cont - sea_level) * 5.0, 0.0, 1.0), cont >= sea_level);

    // Noise component (scale 0.015 matching CPU)
    let nx = wx * 0.015;
    let ny = wy * 0.015;
    let noise = fbm_open_simplex(nx, ny, params.octaves, params.frequency, params.persistence, params.lacunarity);

    // Combine factors
    let base_humidity = ocean_proximity * 0.5 + (1.0 - latitude_factor) * 0.3;
    let humidity = clamp(base_humidity + noise * 0.3, 0.0, 1.0);

    output[idx] = humidity;
}
"#;
