//! GPU context and noise generation dispatch.

use super::{generate_permutation_table, GpuNoiseResult, NoisePipelines};
use super::perm_table::permutation_table_to_u32;
use bytemuck::{Pod, Zeroable};
use std::sync::OnceLock;
use wgpu::util::DeviceExt;

/// Global GPU context, lazily initialized on first use.
static GPU_CONTEXT: OnceLock<Option<GpuNoiseContext>> = OnceLock::new();

/// Parameters passed to noise compute shaders.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct NoiseParams {
    pub seed: u32,
    pub width: u32,
    pub height: u32,
    pub octaves: u32,
    pub frequency: f32,
    pub persistence: f32,
    pub lacunarity: f32,
    pub scale: f32,
    pub world_x: f32,
    pub world_y: f32,
    pub world_height: f32,
    pub _padding: f32,
}

/// GPU context for noise generation.
pub struct GpuNoiseContext {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipelines: NoisePipelines,
}

impl GpuNoiseContext {
    /// Get the global GPU context, initializing it if necessary.
    /// Returns None if no GPU is available.
    pub fn global() -> Option<&'static GpuNoiseContext> {
        GPU_CONTEXT
            .get_or_init(|| {
                pollster::block_on(Self::new()).ok()
            })
            .as_ref()
    }

    /// Create a new GPU context.
    pub async fn new() -> Result<Self, GpuInitError> {
        // Request adapter
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
            })
            .await
            .ok_or(GpuInitError::NoAdapter)?;

        // Request device
        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("rb_noise GPU device"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::default(),
                    memory_hints: wgpu::MemoryHints::Performance,
                },
                None,
            )
            .await
            .map_err(GpuInitError::DeviceRequest)?;

        // Create compute pipelines
        let pipelines = NoisePipelines::new(&device);

        Ok(Self {
            device,
            queue,
            pipelines,
        })
    }

    /// Check if a GPU is available for noise generation.
    pub fn is_available() -> bool {
        Self::global().is_some()
    }

    /// Generate all noise layers on the GPU.
    ///
    /// # Arguments
    /// * `seed` - Random seed for noise generation
    /// * `width` - Output width in pixels
    /// * `height` - Output height in pixels
    /// * `world_x` - World X offset (for region generation)
    /// * `world_y` - World Y offset (for region generation)
    /// * `scale` - Pixel-to-world scale (1.0 for macro, smaller for meso)
    /// * `world_height` - Total world height (for latitude calculations)
    /// * `detail_level` - Extra octaves to add (0 for macro, 1+ for meso)
    pub fn generate_layers(
        &self,
        seed: u32,
        width: usize,
        height: usize,
        world_x: f64,
        world_y: f64,
        scale: f64,
        world_height: f64,
        detail_level: u32,
    ) -> GpuNoiseResult {
        // Helper to create permutation buffer for a specific seed
        let make_perm_buffer = |layer_seed: u32, label: &str| {
            let perm_table = generate_permutation_table(layer_seed);
            let perm_table_u32 = permutation_table_to_u32(&perm_table);
            self.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(label),
                    contents: bytemuck::cast_slice(&perm_table_u32),
                    usage: wgpu::BufferUsages::STORAGE,
                })
        };

        // Each layer needs its own permutation table (matching CPU behavior)
        let cont_seed = seed;
        let temp_seed = seed.wrapping_add(1);
        let tect_seed = seed.wrapping_add(2);
        let eros_seed = seed.wrapping_add(3);
        let peak_seed = seed.wrapping_add(4);
        let humi_seed = seed.wrapping_add(5);

        let cont_perm = make_perm_buffer(cont_seed, "Continentalness perm");
        let temp_perm = make_perm_buffer(temp_seed, "Temperature perm");
        let tect_perm = make_perm_buffer(tect_seed, "Tectonic perm");
        let peak_perm = make_perm_buffer(peak_seed, "PeaksValleys perm");
        let eros_perm = make_perm_buffer(eros_seed, "Erosion perm");
        let humi_perm = make_perm_buffer(humi_seed, "Humidity perm");

        // Generate each layer with its own permutation table
        let continentalness = self.dispatch_continentalness(
            cont_seed,
            width,
            height,
            world_x,
            world_y,
            scale,
            detail_level,
            &cont_perm,
        );

        let temperature = self.dispatch_temperature(
            temp_seed,
            width,
            height,
            world_x,
            world_y,
            scale,
            world_height,
            detail_level,
            &temp_perm,
        );

        let tectonic = self.dispatch_tectonic(
            tect_seed,
            width,
            height,
            world_x,
            world_y,
            scale,
            detail_level,
            &tect_perm,
        );

        let peaks_valleys = self.dispatch_peaks_valleys(
            peak_seed,
            width,
            height,
            world_x,
            world_y,
            scale,
            detail_level,
            &peak_perm,
        );

        // Dependent layers - need continentalness first
        let erosion = self.dispatch_erosion(
            eros_seed,
            width,
            height,
            world_x,
            world_y,
            scale,
            detail_level,
            &continentalness,
            &eros_perm,
        );

        let humidity = self.dispatch_humidity(
            humi_seed,
            width,
            height,
            world_x,
            world_y,
            scale,
            world_height,
            detail_level,
            &continentalness,
            &humi_perm,
        );

        GpuNoiseResult {
            continentalness,
            temperature,
            tectonic,
            erosion,
            peaks_valleys,
            humidity,
        }
    }

    /// Dispatch a compute shader and read back results.
    fn dispatch_compute(
        &self,
        pipeline: &wgpu::ComputePipeline,
        bind_group: &wgpu::BindGroup,
        output_buffer: &wgpu::Buffer,
        staging_buffer: &wgpu::Buffer,
        width: usize,
        height: usize,
        buffer_size: u64,
    ) -> Vec<f32> {
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Noise compute encoder"),
            });

        {
            let mut compute_pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("Noise compute pass"),
                timestamp_writes: None,
            });
            compute_pass.set_pipeline(pipeline);
            compute_pass.set_bind_group(0, bind_group, &[]);

            // Dispatch with 16x16 workgroups
            let workgroups_x = (width as u32 + 15) / 16;
            let workgroups_y = (height as u32 + 15) / 16;
            compute_pass.dispatch_workgroups(workgroups_x, workgroups_y, 1);
        }

        // Copy output to staging buffer
        encoder.copy_buffer_to_buffer(output_buffer, 0, staging_buffer, 0, buffer_size);

        self.queue.submit(std::iter::once(encoder.finish()));

        // Read back results
        let buffer_slice = staging_buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        buffer_slice.map_async(wgpu::MapMode::Read, move |result| {
            tx.send(result).unwrap();
        });
        self.device.poll(wgpu::Maintain::Wait);
        rx.recv().unwrap().unwrap();

        let data = buffer_slice.get_mapped_range();
        let result: Vec<f32> = bytemuck::cast_slice(&data).to_vec();
        drop(data);
        staging_buffer.unmap();

        result
    }

    /// Dispatch continentalness noise generation.
    fn dispatch_continentalness(
        &self,
        seed: u32,
        width: usize,
        height: usize,
        world_x: f64,
        world_y: f64,
        scale: f64,
        detail_level: u32,
        perm_buffer: &wgpu::Buffer,
    ) -> Vec<f32> {
        let params = NoiseParams {
            seed,
            width: width as u32,
            height: height as u32,
            octaves: 16 + detail_level,
            frequency: 1.0,
            persistence: 0.59,
            lacunarity: 2.0,
            scale: scale as f32,
            world_x: world_x as f32,
            world_y: world_y as f32,
            world_height: 0.0,
            _padding: 0.0,
        };

        let total_pixels = width * height;
        let buffer_size = (total_pixels * std::mem::size_of::<f32>()) as u64;

        let params_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Continentalness params"),
                contents: bytemuck::cast_slice(&[params]),
                usage: wgpu::BufferUsages::UNIFORM,
            });

        let output_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Continentalness output"),
            size: buffer_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });

        let staging_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Continentalness staging"),
            size: buffer_size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Continentalness bind group"),
            layout: &self.pipelines.continentalness_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: params_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: perm_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: output_buffer.as_entire_binding(),
                },
            ],
        });

        self.dispatch_compute(
            &self.pipelines.continentalness,
            &bind_group,
            &output_buffer,
            &staging_buffer,
            width,
            height,
            buffer_size,
        )
    }

    /// Dispatch temperature noise generation.
    fn dispatch_temperature(
        &self,
        seed: u32,
        width: usize,
        height: usize,
        world_x: f64,
        world_y: f64,
        scale: f64,
        world_height: f64,
        detail_level: u32,
        perm_buffer: &wgpu::Buffer,
    ) -> Vec<f32> {
        let params = NoiseParams {
            seed,
            width: width as u32,
            height: height as u32,
            octaves: 8 + detail_level,
            frequency: 1.0,
            persistence: 0.59,
            lacunarity: 2.0,
            scale: scale as f32,
            world_x: world_x as f32,
            world_y: world_y as f32,
            world_height: world_height as f32,
            _padding: 0.0,
        };

        let total_pixels = width * height;
        let buffer_size = (total_pixels * std::mem::size_of::<f32>()) as u64;

        let params_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Temperature params"),
                contents: bytemuck::cast_slice(&[params]),
                usage: wgpu::BufferUsages::UNIFORM,
            });

        let output_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Temperature output"),
            size: buffer_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });

        let staging_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Temperature staging"),
            size: buffer_size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Temperature bind group"),
            layout: &self.pipelines.temperature_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: params_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: perm_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: output_buffer.as_entire_binding(),
                },
            ],
        });

        self.dispatch_compute(
            &self.pipelines.temperature,
            &bind_group,
            &output_buffer,
            &staging_buffer,
            width,
            height,
            buffer_size,
        )
    }

    /// Dispatch tectonic plates noise generation.
    fn dispatch_tectonic(
        &self,
        seed: u32,
        width: usize,
        height: usize,
        world_x: f64,
        world_y: f64,
        scale: f64,
        _detail_level: u32,
        perm_buffer: &wgpu::Buffer,
    ) -> Vec<f32> {
        let params = NoiseParams {
            seed,
            width: width as u32,
            height: height as u32,
            octaves: 0, // Voronoi doesn't use octaves
            frequency: 1.0,
            persistence: 0.0,
            lacunarity: 0.0,
            scale: scale as f32,
            world_x: world_x as f32,
            world_y: world_y as f32,
            world_height: 0.0,
            _padding: 0.0,
        };

        let total_pixels = width * height;
        let buffer_size = (total_pixels * std::mem::size_of::<f32>()) as u64;

        let params_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Tectonic params"),
                contents: bytemuck::cast_slice(&[params]),
                usage: wgpu::BufferUsages::UNIFORM,
            });

        let output_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Tectonic output"),
            size: buffer_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });

        let staging_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Tectonic staging"),
            size: buffer_size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Tectonic bind group"),
            layout: &self.pipelines.tectonic_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: params_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: perm_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: output_buffer.as_entire_binding(),
                },
            ],
        });

        self.dispatch_compute(
            &self.pipelines.tectonic,
            &bind_group,
            &output_buffer,
            &staging_buffer,
            width,
            height,
            buffer_size,
        )
    }

    /// Dispatch peaks and valleys noise generation.
    fn dispatch_peaks_valleys(
        &self,
        seed: u32,
        width: usize,
        height: usize,
        world_x: f64,
        world_y: f64,
        scale: f64,
        detail_level: u32,
        perm_buffer: &wgpu::Buffer,
    ) -> Vec<f32> {
        let params = NoiseParams {
            seed,
            width: width as u32,
            height: height as u32,
            octaves: 8 + detail_level,
            frequency: 1.0,
            persistence: 0.5,
            lacunarity: 2.0,
            scale: scale as f32,
            world_x: world_x as f32,
            world_y: world_y as f32,
            world_height: 0.0,
            _padding: 0.0,
        };

        let total_pixels = width * height;
        let buffer_size = (total_pixels * std::mem::size_of::<f32>()) as u64;

        let params_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("PeaksValleys params"),
                contents: bytemuck::cast_slice(&[params]),
                usage: wgpu::BufferUsages::UNIFORM,
            });

        let output_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("PeaksValleys output"),
            size: buffer_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });

        let staging_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("PeaksValleys staging"),
            size: buffer_size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("PeaksValleys bind group"),
            layout: &self.pipelines.peaks_valleys_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: params_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: perm_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: output_buffer.as_entire_binding(),
                },
            ],
        });

        self.dispatch_compute(
            &self.pipelines.peaks_valleys,
            &bind_group,
            &output_buffer,
            &staging_buffer,
            width,
            height,
            buffer_size,
        )
    }

    /// Dispatch erosion noise generation (depends on continentalness).
    fn dispatch_erosion(
        &self,
        seed: u32,
        width: usize,
        height: usize,
        world_x: f64,
        world_y: f64,
        scale: f64,
        detail_level: u32,
        continentalness: &[f32],
        perm_buffer: &wgpu::Buffer,
    ) -> Vec<f32> {
        let params = NoiseParams {
            seed,
            width: width as u32,
            height: height as u32,
            octaves: 6 + detail_level,
            frequency: 1.0,
            persistence: 0.5,
            lacunarity: 2.0,
            scale: scale as f32,
            world_x: world_x as f32,
            world_y: world_y as f32,
            world_height: 0.0,
            _padding: 0.0,
        };

        let total_pixels = width * height;
        let buffer_size = (total_pixels * std::mem::size_of::<f32>()) as u64;

        let params_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Erosion params"),
                contents: bytemuck::cast_slice(&[params]),
                usage: wgpu::BufferUsages::UNIFORM,
            });

        let cont_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Erosion continentalness input"),
                contents: bytemuck::cast_slice(continentalness),
                usage: wgpu::BufferUsages::STORAGE,
            });

        let output_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Erosion output"),
            size: buffer_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });

        let staging_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Erosion staging"),
            size: buffer_size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Erosion bind group"),
            layout: &self.pipelines.erosion_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: params_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: perm_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: cont_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: output_buffer.as_entire_binding(),
                },
            ],
        });

        self.dispatch_compute(
            &self.pipelines.erosion,
            &bind_group,
            &output_buffer,
            &staging_buffer,
            width,
            height,
            buffer_size,
        )
    }

    /// Dispatch humidity noise generation (depends on continentalness).
    fn dispatch_humidity(
        &self,
        seed: u32,
        width: usize,
        height: usize,
        world_x: f64,
        world_y: f64,
        scale: f64,
        world_height: f64,
        detail_level: u32,
        continentalness: &[f32],
        perm_buffer: &wgpu::Buffer,
    ) -> Vec<f32> {
        let params = NoiseParams {
            seed,
            width: width as u32,
            height: height as u32,
            octaves: 5 + detail_level,
            frequency: 1.0,
            persistence: 0.5,
            lacunarity: 2.0,
            scale: scale as f32,
            world_x: world_x as f32,
            world_y: world_y as f32,
            world_height: world_height as f32,
            _padding: 0.0,
        };

        let total_pixels = width * height;
        let buffer_size = (total_pixels * std::mem::size_of::<f32>()) as u64;

        let params_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Humidity params"),
                contents: bytemuck::cast_slice(&[params]),
                usage: wgpu::BufferUsages::UNIFORM,
            });

        let cont_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Humidity continentalness input"),
                contents: bytemuck::cast_slice(continentalness),
                usage: wgpu::BufferUsages::STORAGE,
            });

        let output_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Humidity output"),
            size: buffer_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });

        let staging_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Humidity staging"),
            size: buffer_size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Humidity bind group"),
            layout: &self.pipelines.humidity_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: params_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: perm_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: cont_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: output_buffer.as_entire_binding(),
                },
            ],
        });

        self.dispatch_compute(
            &self.pipelines.humidity,
            &bind_group,
            &output_buffer,
            &staging_buffer,
            width,
            height,
            buffer_size,
        )
    }
}

/// Errors that can occur during GPU initialization.
#[derive(Debug)]
pub enum GpuInitError {
    NoAdapter,
    DeviceRequest(wgpu::RequestDeviceError),
}

impl std::fmt::Display for GpuInitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GpuInitError::NoAdapter => write!(f, "No GPU adapter found"),
            GpuInitError::DeviceRequest(e) => write!(f, "Failed to request GPU device: {}", e),
        }
    }
}

impl std::error::Error for GpuInitError {}
