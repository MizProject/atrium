//! Modular `wgpu` 3D renderer and model registry for Aula keyboards.
//!
//! Renders an offscreen `Bgra8Unorm` frame matching `gpui::RenderImage`'s Blade
//! atlas format so `reaula-gui` can display real-time 3D RGB effects, per-key
//! Custom paints, selection highlights, and live analog key depression.
//!
//! Models are grouped under `kb_3d/<model>.rs` and indexed in [`MODELS`].

pub mod mascots;
pub mod switch;
pub mod win60he;

pub use switch::{
    SWITCH_3D_HEIGHT, SWITCH_3D_WIDTH, SwitchCamera, SwitchCameraPreset, SwitchRenderer3D,
    SwitchSceneInput,
};

use std::collections::BTreeSet;
use std::f32::consts::TAU;
use std::mem::size_of;
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use gpui_kit::RenderImage;
use image::{Frame, RgbaImage};
use smallvec::SmallVec;

use reaula_core::ipc::{DAEMON_INDICATOR_AQUA_RGB, DAEMON_INDICATOR_PULSE_RGB};
use reaula_core::keyboards::{KeyDef, KeyboardSpec};
use reaula_core::keymap::WIN60HE_KEYS;
use reaula_core::protocol::{LightState, MATRIX_SLOTS};

/// Output width in pixels (`832 * 4 = 3328` bytes per row, a multiple of 256).
pub const KB3D_WIDTH: u32 = 832;
pub const KB3D_HEIGHT: u32 = 468;

pub(crate) const BOARD_TILT_RAD: f32 = 0.095; // ~5.4° ergonomic typing tilt

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CameraPreset {
    Perspective,
    Top,
    Side,
    Front,
}

#[derive(Debug, Clone, Copy)]
pub struct Kb3dCamera {
    pub yaw: f32,
    pub pitch: f32,
    pub zoom: f32,
}

impl Default for Kb3dCamera {
    fn default() -> Self {
        Self::from_preset(CameraPreset::Perspective)
    }
}

impl Kb3dCamera {
    pub fn from_preset(preset: CameraPreset) -> Self {
        win60he::preset_camera(preset)
    }

    pub fn from_preset_for(model: &KeyboardModel3d, preset: CameraPreset) -> Self {
        if model.id == win60he::MODEL.id {
            win60he::preset_camera(preset)
        } else {
            match preset {
                CameraPreset::Perspective => model.default_camera,
                CameraPreset::Top => Self {
                    yaw: 0.0,
                    pitch: 1.46,
                    zoom: model.default_camera.zoom,
                },
                CameraPreset::Side => Self {
                    yaw: -1.18,
                    pitch: 0.44,
                    zoom: model.default_camera.zoom,
                },
                CameraPreset::Front => Self {
                    yaw: 0.0,
                    pitch: 0.30,
                    zoom: model.default_camera.zoom,
                },
            }
        }
    }

    pub fn orbit(&mut self, dx: f32, dy: f32) {
        self.yaw = (self.yaw + dx * 0.0085).clamp(-std::f32::consts::PI, std::f32::consts::PI);
        self.pitch = (self.pitch + dy * 0.0085).clamp(0.18, 1.50);
    }

    pub fn adjust_zoom(&mut self, delta: f32) {
        self.zoom = (self.zoom + delta).clamp(4.1, 10.5);
    }
}

#[derive(Clone, Copy, Debug)]
pub struct KeyImpulse {
    pub index: u8,
    pub nx: f32,
    pub ny: f32,
    pub age_s: f32,
    pub strength: f32,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct MascotPose {
    pub scale: f32,
    pub age_s: f32,
}

pub struct Kb3dSceneInput<'a> {
    pub light: &'a LightState,
    pub custom_colors: Option<&'a [[u8; 3]; MATRIX_SLOTS]>,
    pub use_custom_colors: bool,
    pub typing_only: bool,
    pub impulses: &'a [KeyImpulse],
    pub selected_keys: &'a BTreeSet<u8>,
    pub focused_key: Option<u8>,
    pub live_depths_mm: &'a [f32; MATRIX_SLOTS],
    pub time_s: f32,
    pub miku: MascotPose,
    pub teto: MascotPose,
    pub daemon_aqua_keys: &'a BTreeSet<u8>,
    pub daemon_aqua_pulsing: &'a BTreeSet<u8>,
}

/// Static 3D scene registration entry for one Aula keyboard model.
#[derive(Clone, Copy)]
pub struct KeyboardModel3d {
    pub id: &'static str,
    pub spec: &'static KeyboardSpec,
    pub keys: &'static [KeyDef],
    pub default_camera: Kb3dCamera,
    pub build_mesh: fn(&mut Vec<Vertex>, &mut Vec<u32>),
}

/// Indexed 3D keyboard models compiled into `reaula-gui`.
pub const MODELS: &[KeyboardModel3d] = &[win60he::MODEL];

/// Look up a registered 3D keyboard model by ID or alias, defaulting to the WIN 60 HE.
pub fn find_model(id: Option<&str>) -> &'static KeyboardModel3d {
    id.and_then(|tok| MODELS.iter().find(|m| m.spec.matches_id(tok)))
        .unwrap_or(&MODELS[0])
}

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
pub struct Vertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    pub color: [f32; 3],
    /// x = part kind (`0`=chassis/boot, `1`=plate/accent/plug, `2`=keycap, `3`=led halo, `4`=legend, `5`=desk, `6`=braided cable, `6.5`=cable cuff, `7`=miku, `8`=teto),
    /// y = key slot or tube arc / mascot sway flag, z = emission or ring/offset, w = bevel or glow.
    pub meta: [f32; 4],
}

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct SceneUniforms {
    view_proj: [[f32; 4]; 4],
    camera_pos: [f32; 4],
    board_params: [f32; 4],
    mascot_params: [f32; 4],
    key_colors: [[f32; 4]; MATRIX_SLOTS],
    key_motion: [[f32; 4]; MATRIX_SLOTS],
}

pub(crate) struct OffscreenTarget {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) color_tex: wgpu::Texture,
    pub(crate) color_view: wgpu::TextureView,
    pub(crate) depth_view: wgpu::TextureView,
    pub(crate) readback_buf: wgpu::Buffer,
}

impl OffscreenTarget {
    pub(crate) fn new(device: &wgpu::Device, label: &str, width: u32, height: u32) -> Self {
        assert!((width * 4).is_multiple_of(256));
        let color_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(&format!("{label}-color")),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Bgra8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let color_view = color_tex.create_view(&wgpu::TextureViewDescriptor::default());

        let depth_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(&format!("{label}-depth")),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let depth_view = depth_tex.create_view(&wgpu::TextureViewDescriptor::default());

        let readback_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(&format!("{label}-readback")),
            size: u64::from(width * height * 4),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        Self {
            width,
            height,
            color_tex,
            color_view,
            depth_view,
            readback_buf,
        }
    }

    pub(crate) fn readback_render_image(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        mut encoder: wgpu::CommandEncoder,
    ) -> Result<Arc<RenderImage>, String> {
        encoder.copy_texture_to_buffer(
            self.color_tex.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &self.readback_buf,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.width * 4),
                    rows_per_image: Some(self.height),
                },
            },
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );

        queue.submit(Some(encoder.finish()));

        let slice = self.readback_buf.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |res| {
            let _ = tx.send(res);
        });
        let _ = device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        });
        rx.recv()
            .map_err(|_| "GPU readback channel dropped".to_string())?
            .map_err(|e| format!("GPU buffer map failed: {e}"))?;

        let raw = slice
            .get_mapped_range()
            .map_err(|e| format!("GPU mapped readback failed: {e}"))?
            .to_vec();
        self.readback_buf.unmap();

        let rgba = RgbaImage::from_raw(self.width, self.height, raw)
            .ok_or_else(|| "failed to pack 3D frame buffer".to_string())?;
        let mut frames: SmallVec<[Frame; 1]> = SmallVec::new();
        frames.push(Frame::new(rgba));
        Ok(Arc::new(RenderImage::new(frames)))
    }
}

pub(crate) fn create_wgpu_device(
    label: &str,
) -> Result<(Arc<wgpu::Device>, Arc<wgpu::Queue>), String> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN | wgpu::Backends::GL,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
        apply_limit_buckets: false,
    }))
    .map_err(|e| format!("no GPU adapter for 3D preview: {e}"))?;

    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some(label),
        ..Default::default()
    }))
    .map_err(|e| format!("failed to open wgpu device: {e}"))?;

    Ok((Arc::new(device), Arc::new(queue)))
}

pub(crate) fn create_vertex_pipeline(
    device: &wgpu::Device,
    label: &str,
    wgsl: &str,
    bind_layout: &wgpu::BindGroupLayout,
) -> Result<wgpu::RenderPipeline, String> {
    let error_scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(&format!("{label}-shader")),
        source: wgpu::ShaderSource::Wgsl(wgsl.into()),
    });

    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(&format!("{label}-pipeline-layout")),
        bind_group_layouts: &[Some(bind_layout)],
        immediate_size: 0,
    });

    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(&format!("{label}-pipeline")),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: size_of::<Vertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &wgpu::vertex_attr_array![
                    0 => Float32x3,
                    1 => Float32x3,
                    2 => Float32x3,
                    3 => Float32x4
                ],
            })],
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            cull_mode: Some(wgpu::Face::Back),
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: wgpu::TextureFormat::Bgra8Unorm,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    });
    if let Some(err) = pollster::block_on(error_scope.pop()) {
        return Err(format!("wgpu shader/pipeline validation error: {err}"));
    }
    Ok(pipeline)
}

pub struct KeyboardRenderer3D {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    pipeline: wgpu::RenderPipeline,
    uniform_buf: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    vertex_buf: wgpu::Buffer,
    index_buf: wgpu::Buffer,
    index_count: u32,
    model: &'static KeyboardModel3d,
    target: OffscreenTarget,
}

impl KeyboardRenderer3D {
    pub fn new_for_keyboard(keyboard_id: Option<&str>) -> Result<Self, String> {
        let model = find_model(keyboard_id);
        let (device, queue) = create_wgpu_device("reaula-kb3d-device")?;

        let (vertices, indices) = build_scene_mesh(model);
        let index_count = u32::try_from(indices.len()).map_err(|_| "mesh too large".to_string())?;

        let vertex_buf = create_init_buffer(
            &device,
            "kb3d-vb",
            bytemuck::cast_slice(&vertices),
            wgpu::BufferUsages::VERTEX,
        );
        let index_buf = create_init_buffer(
            &device,
            "kb3d-ib",
            bytemuck::cast_slice(&indices),
            wgpu::BufferUsages::INDEX,
        );

        let uniform_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("kb3d-uniforms"),
            size: size_of::<SceneUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("kb3d-bind-layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("kb3d-bind-group"),
            layout: &bind_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buf.as_entire_binding(),
            }],
        });

        let pipeline = create_vertex_pipeline(&device, "kb3d", SHADER_WGSL, &bind_layout)?;
        let target = OffscreenTarget::new(&device, "kb3d", KB3D_WIDTH, KB3D_HEIGHT);

        Ok(Self {
            device,
            queue,
            pipeline,
            uniform_buf,
            bind_group,
            vertex_buf,
            index_buf,
            index_count,
            model,
            target,
        })
    }

    pub fn shared_gpu(&self) -> (Arc<wgpu::Device>, Arc<wgpu::Queue>) {
        (Arc::clone(&self.device), Arc::clone(&self.queue))
    }

    pub fn model(&self) -> &'static KeyboardModel3d {
        self.model
    }

    /// Swap the GPU mesh if the active keyboard model ID differs from the loaded mesh.
    pub fn set_model(&mut self, keyboard_id: Option<&str>) -> bool {
        let next = find_model(keyboard_id);
        if next.id == self.model.id {
            return false;
        }
        let (vertices, indices) = build_scene_mesh(next);
        let Ok(index_count) = u32::try_from(indices.len()) else {
            return false;
        };
        self.vertex_buf = create_init_buffer(
            &self.device,
            "kb3d-vb",
            bytemuck::cast_slice(&vertices),
            wgpu::BufferUsages::VERTEX,
        );
        self.index_buf = create_init_buffer(
            &self.device,
            "kb3d-ib",
            bytemuck::cast_slice(&indices),
            wgpu::BufferUsages::INDEX,
        );
        self.index_count = index_count;
        self.model = next;
        true
    }

    pub fn render_frame(
        &self,
        cam: &Kb3dCamera,
        scene: &Kb3dSceneInput<'_>,
    ) -> Result<Arc<RenderImage>, String> {
        let uniforms = build_uniforms(self.model, cam, scene);
        self.queue
            .write_buffer(&self.uniform_buf, 0, bytemuck::bytes_of(&uniforms));

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("kb3d-encoder"),
            });

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("kb3d-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.target.color_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: crate::theme::STUDIO_CLEAR_LINEAR[0],
                            g: crate::theme::STUDIO_CLEAR_LINEAR[1],
                            b: crate::theme::STUDIO_CLEAR_LINEAR[2],
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.target.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_vertex_buffer(0, self.vertex_buf.slice(..));
            pass.set_index_buffer(self.index_buf.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..self.index_count, 0, 0..1);
        }

        self.target
            .readback_render_image(&self.device, &self.queue, encoder)
    }
}

pub fn build_scene_mesh(model: &KeyboardModel3d) -> (Vec<Vertex>, Vec<u32>) {
    let mut verts = Vec::with_capacity(11000);
    let mut indices = Vec::with_capacity(19000);

    // Regal Navy paper-matte desk pad shared across all 3D models.
    push_box(
        &mut verts,
        &mut indices,
        [-6.5, -0.34, -4.2],
        [6.5, -0.30, 4.2],
        crate::theme::STUDIO_DESK_LINEAR,
        [5.0, -1.0, 0.0, 0.0],
    );

    (model.build_mesh)(&mut verts, &mut indices);
    mascots::push_easter_egg_mascots(&mut verts, &mut indices);

    (verts, indices)
}

pub(crate) fn create_init_buffer(
    device: &wgpu::Device,
    label: &str,
    data: &[u8],
    usage: wgpu::BufferUsages,
) -> wgpu::Buffer {
    let size = (data.len() as u64).max(16);
    let buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size,
        usage: usage | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: true,
    });
    buf.slice(0..data.len() as u64)
        .get_mapped_range_mut()
        .expect("newly created mapped buffer range should be valid")
        .copy_from_slice(data);
    buf.unmap();
    buf
}

fn is_typing_reactive_mode(mode: u8) -> bool {
    matches!(mode, 6 | 8 | 11 | 12 | 16)
}

fn compute_typing_response(
    mode: u8,
    key_index: u8,
    nx: f32,
    ny: f32,
    depth_mm: f32,
    speed: f32,
    impulses: &[KeyImpulse],
) -> f32 {
    let mut env = (depth_mm / 1.8).clamp(0.0, 1.0);
    for imp in impulses {
        let dist = ((nx - imp.nx).powi(2) + (ny - imp.ny).powi(2)).sqrt();
        if mode == 16 {
            // Fireworks: glows the pressed key and surrounding cluster for ~2.85s
            // with varying per-key ember brightness modulation.
            if imp.age_s > 2.85 || dist > 0.36 {
                continue;
            }
            let fade_fw =
                (1.0 - imp.age_s / 2.85).clamp(0.0, 1.0).powf(1.35) * imp.strength.clamp(0.35, 1.0);
            let locality = (1.0 - dist / 0.36).clamp(0.0, 1.0).powf(1.35);
            let phase_a = (nx * 31.0 + ny * 19.0) * 1.37 + imp.age_s * (5.2 + speed * 2.4);
            let phase_b = (nx * 17.0 - ny * 29.0) + imp.age_s * (8.6 + speed * 3.1) + imp.nx * 11.0;
            let vary = 0.44 + 0.56 * (0.5 + 0.5 * (phase_a.sin() * phase_b.cos()));
            let bloom = (imp.age_s / 0.08).clamp(0.35, 1.0);
            let core = if imp.index == key_index {
                fade_fw * (0.72 + 0.28 * (imp.age_s * 9.0).cos().abs())
            } else {
                0.0
            };
            let hit = (locality * fade_fw * vary * bloom).max(core);
            if hit > env {
                env = hit;
            }
            continue;
        }

        if imp.age_s > 1.25 {
            continue;
        }
        let fade = (1.0 - imp.age_s / 1.25).clamp(0.0, 1.0).powi(2) * imp.strength.clamp(0.25, 1.0);
        let hit = match mode {
            8 => {
                // Ripple: expanding wave ring propagating outward across the board.
                let radius = imp.age_s * (1.15 * speed);
                let ring = (1.0 - (dist - radius).abs() / 0.16).clamp(0.0, 1.0) * fade;
                if imp.index == key_index {
                    ring.max(fade)
                } else {
                    ring
                }
            }
            11 => {
                let cross_h = (1.0 - (ny - imp.ny).abs() / 0.16).clamp(0.0, 1.0);
                let cross_v = (1.0 - (nx - imp.nx).abs() / 0.08).clamp(0.0, 1.0);
                cross_h.max(cross_v) * fade
            }
            12 => {
                let splash = fade * 0.45;
                if imp.index == key_index { fade } else { splash }
            }
            _ => {
                let near = (1.0 - dist / 0.18).clamp(0.0, 1.0) * fade * 0.35;
                if imp.index == key_index { fade } else { near }
            }
        };
        if hit > env {
            env = hit;
        }
    }
    env.clamp(0.0, 1.0)
}

/// Evaluate animated linear RGB `[0.0..1.0]` for all 61 WIN 60 HE keys.
pub fn evaluate_key_colors(scene: &Kb3dSceneInput<'_>) -> [[f32; 3]; 61] {
    let vec_colors = evaluate_key_colors_for(&WIN60HE_KEYS, scene);
    let mut out = [[0.0f32; 3]; 61];
    for (slot, c) in vec_colors.into_iter().take(61).enumerate() {
        out[slot] = c;
    }
    out
}

/// Evaluate animated linear RGB `[0.0..1.0]` across an arbitrary key layout.
pub fn evaluate_key_colors_for(keys: &[KeyDef], scene: &Kb3dSceneInput<'_>) -> Vec<[f32; 3]> {
    let mut out = vec![[0.0f32; 3]; keys.len()];
    if !scene.light.power_on && !scene.use_custom_colors {
        return out;
    }
    let bri = if scene.use_custom_colors {
        (f32::from(scene.light.brightness.max(1)) / 4.0).clamp(0.25, 1.0)
    } else {
        (f32::from(scene.light.brightness) / 4.0).clamp(0.0, 1.0)
    };
    let spd = 0.6 + f32::from(scene.light.speed) * 0.48;
    let t = scene.time_s * spd;
    let fg = rgb_to_linear(scene.light.fg);
    let bg = rgb_to_linear(scene.light.bg);

    for (slot, k) in keys.iter().enumerate() {
        let depth = scene
            .live_depths_mm
            .get(k.index as usize)
            .copied()
            .unwrap_or(0.0);
        let nx = ((k.x as f32 + k.width as f32 * 0.5) - 10.0) / 577.0;
        let ny = ((k.y as f32 + k.height as f32 * 0.5) - 10.0) / 207.0;
        let typing_env = compute_typing_response(
            scene.light.mode,
            k.index,
            nx,
            ny,
            depth,
            spd,
            scene.impulses,
        );

        if scene.use_custom_colors
            && let Some(table) = scene.custom_colors
            && let Some(&rgb) = table.get(k.index as usize)
        {
            let c = rgb_to_linear(rgb);
            let gate = if scene.typing_only { typing_env } else { 1.0 };
            out[slot] = [c[0] * bri * gate, c[1] * bri * gate, c[2] * bri * gate];
            continue;
        }

        let axis = match scene.light.direction {
            1 => 1.0 - nx,
            2 => ny,
            3 => 1.0 - ny,
            _ => nx,
        };

        let raw_c = if is_typing_reactive_mode(scene.light.mode) {
            let hit_color = if scene.light.full_color {
                let spark_hue = if scene.light.mode == 16 {
                    (nx * 0.65 + ny * 0.45 + (slot as f32) * 0.07 + t * 0.22).rem_euclid(1.0)
                } else {
                    (nx * 0.75 + ny * 0.35 + t * 0.15).rem_euclid(1.0)
                };
                hsv_to_linear(spark_hue, 0.94, 1.0)
            } else {
                fg
            };
            let rest = scale3(bg, 0.08);
            lerp3(rest, hit_color, typing_env)
        } else {
            let ambient = match scene.light.mode {
                0 => {
                    if scene.light.full_color {
                        hsv_to_linear((nx * 0.85) % 1.0, 0.92, 1.0)
                    } else {
                        fg
                    }
                }
                1 => {
                    let pulse = 0.5 - 0.5 * (t * 2.1).cos();
                    let base = if scene.light.full_color {
                        hsv_to_linear((scene.time_s * 0.14) % 1.0, 0.95, 1.0)
                    } else {
                        fg
                    };
                    lerp3(bg, base, pulse)
                }
                2 => hsv_to_linear((axis * 0.85 - t * 0.45).rem_euclid(1.0), 0.94, 1.0),
                3 => hsv_to_linear((t * 0.22).rem_euclid(1.0), 0.94, 1.0),
                4 => {
                    let ang = (ny - 0.5).atan2(nx - 0.5) / TAU + 0.5;
                    let sweep = (ang - t * 0.35).rem_euclid(1.0);
                    if scene.light.full_color {
                        hsv_to_linear(sweep, 0.95, 1.0)
                    } else {
                        let tail = (1.0 - sweep * 1.6).clamp(0.0, 1.0);
                        lerp3(bg, fg, tail * tail)
                    }
                }
                7 | 15 => {
                    let wave = 0.5 + 0.5 * ((nx * 4.5 + ny * 2.2 - t * 2.2).sin());
                    let c =
                        hsv_to_linear((0.46 + 0.28 * wave + t * 0.06).rem_euclid(1.0), 0.88, 1.0);
                    scale3(c, 0.55 + 0.45 * wave)
                }
                9 => {
                    let phase = ((slot as f32 * 1.732_05 + t * 2.6).sin() * 0.5 + 0.5).powf(4.0);
                    if scene.light.full_color {
                        hsv_to_linear(
                            ((slot as f32 * 0.13) + t * 0.15).rem_euclid(1.0),
                            0.9,
                            phase,
                        )
                    } else {
                        lerp3(bg, fg, phase)
                    }
                }
                14 => {
                    let dist = ((nx - 0.5).powi(2) + (ny - 0.5).powi(2)).sqrt();
                    let ring = 0.5 + 0.5 * ((dist * 14.0 - t * 4.8).cos());
                    if scene.light.full_color {
                        hsv_to_linear((dist * 0.8 - t * 0.3).rem_euclid(1.0), 0.92, ring)
                    } else {
                        lerp3(bg, fg, ring.powf(1.5))
                    }
                }
                _ => {
                    if let Some(table) = scene.custom_colors
                        && let Some(&rgb) = table.get(k.index as usize)
                    {
                        rgb_to_linear(rgb)
                    } else {
                        fg
                    }
                }
            };
            if scene.typing_only {
                scale3(ambient, typing_env)
            } else {
                ambient
            }
        };

        let press_frac = (depth / 3.4).clamp(0.0, 1.0);
        let dark_fallback = press_frac * 0.12;
        if scene.daemon_aqua_pulsing.contains(&k.index) {
            let pulse = rgb_to_linear(DAEMON_INDICATOR_PULSE_RGB);
            out[slot] = [pulse[0] * 1.05, pulse[1] * 1.05, pulse[2] * 1.05];
        } else if scene.daemon_aqua_keys.contains(&k.index) {
            out[slot] = rgb_to_linear(DAEMON_INDICATOR_AQUA_RGB);
        } else {
            out[slot] = [
                (raw_c[0] * bri * (1.0 + 0.25 * press_frac))
                    .max(dark_fallback * 0.6)
                    .min(1.2),
                (raw_c[1] * bri * (1.0 + 0.25 * press_frac))
                    .max(dark_fallback * 0.85)
                    .min(1.2),
                (raw_c[2] * bri * (1.0 + 0.25 * press_frac))
                    .max(dark_fallback)
                    .min(1.2),
            ];
        }
    }

    out
}

fn build_uniforms(
    model: &KeyboardModel3d,
    cam: &Kb3dCamera,
    scene: &Kb3dSceneInput<'_>,
) -> SceneUniforms {
    let eye = [
        cam.zoom * cam.pitch.cos() * cam.yaw.sin(),
        cam.zoom * cam.pitch.sin(),
        cam.zoom * cam.pitch.cos() * cam.yaw.cos(),
    ];
    let target = [0.0, 0.08, 0.0];
    let view = look_at(eye, target, [0.0, 1.0, 0.0]);
    let proj = perspective(
        40.0_f32.to_radians(),
        KB3D_WIDTH as f32 / KB3D_HEIGHT as f32,
        0.1,
        40.0,
    );
    let view_proj = mul_mat4(proj, view);

    let colors = evaluate_key_colors_for(model.keys, scene);
    let mut key_colors = [[0.0f32; 4]; MATRIX_SLOTS];
    let mut key_motion = [[0.0f32; 4]; MATRIX_SLOTS];

    for (slot, k) in model.keys.iter().enumerate().take(MATRIX_SLOTS) {
        let c = colors[slot];
        let depth = scene
            .live_depths_mm
            .get(k.index as usize)
            .copied()
            .unwrap_or(0.0)
            .clamp(0.0, 4.0);
        key_colors[slot] = [c[0], c[1], c[2], (depth / 3.4).clamp(0.0, 1.0)];
        key_motion[slot] = [
            depth * 0.044,
            if scene.selected_keys.contains(&k.index) {
                1.0
            } else {
                0.0
            },
            if scene.focused_key == Some(k.index) {
                1.0
            } else {
                0.0
            },
            0.0,
        ];
    }

    SceneUniforms {
        view_proj,
        camera_pos: [eye[0], eye[1], eye[2], scene.time_s],
        board_params: [
            BOARD_TILT_RAD.sin(),
            BOARD_TILT_RAD.cos(),
            if scene.light.power_on || scene.use_custom_colors {
                1.0
            } else {
                0.0
            },
            f32::from(scene.light.brightness) / 4.0,
        ],
        mascot_params: [
            scene.miku.scale,
            scene.teto.scale,
            scene.miku.age_s,
            scene.teto.age_s,
        ],
        key_colors,
        key_motion,
    }
}

pub(crate) fn apply_board_tilt(p: [f32; 3]) -> [f32; 3] {
    let s = BOARD_TILT_RAD.sin();
    let c = BOARD_TILT_RAD.cos();
    [p[0], p[1] * c - p[2] * s + 0.04, p[1] * s + p[2] * c]
}

pub(crate) fn cubic_bezier3(
    p0: [f32; 3],
    p1: [f32; 3],
    p2: [f32; 3],
    p3: [f32; 3],
    t: f32,
) -> [f32; 3] {
    let u = 1.0 - t;
    let b0 = u * u * u;
    let b1 = 3.0 * u * u * t;
    let b2 = 3.0 * u * t * t;
    let b3 = t * t * t;
    [
        b0 * p0[0] + b1 * p1[0] + b2 * p2[0] + b3 * p3[0],
        b0 * p0[1] + b1 * p1[1] + b2 * p2[1] + b3 * p3[1],
        b0 * p0[2] + b1 * p1[2] + b2 * p2[2] + b3 * p3[2],
    ]
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn push_oriented_frustum(
    verts: &mut Vec<Vertex>,
    indices: &mut Vec<u32>,
    pivot: [f32; 3],
    pivot_yaw: f32,
    offset: [f32; 3],
    size: [f32; 3],
    top_scale: [f32; 2],
    rot_xyz: [f32; 3],
    color: [f32; 3],
    meta: [f32; 4],
) {
    let hx = size[0] * 0.5;
    let hy = size[1] * 0.5;
    let hz = size[2] * 0.5;
    let thx = hx * top_scale[0];
    let thz = hz * top_scale[1];

    let local = [
        [-hx, -hy, -hz],  // 0
        [hx, -hy, -hz],   // 1
        [hx, -hy, hz],    // 2
        [-hx, -hy, hz],   // 3
        [-thx, hy, -thz], // 4
        [thx, hy, -thz],  // 5
        [thx, hy, thz],   // 6
        [-thx, hy, thz],  // 7
    ];

    let xform = |p: [f32; 3]| -> [f32; 3] {
        let p = rot_xyz_pt(p, rot_xyz);
        let rel = [p[0] + offset[0], p[1] + offset[1], p[2] + offset[2]];
        let (sy, cy) = pivot_yaw.sin_cos();
        [
            pivot[0] + rel[0] * cy + rel[2] * sy,
            pivot[1] + rel[1],
            pivot[2] - rel[0] * sy + rel[2] * cy,
        ]
    };

    let mut w = [[0.0f32; 3]; 8];
    for i in 0..8 {
        w[i] = xform(local[i]);
    }
    let center = xform([0.0, 0.0, 0.0]);

    let faces = [
        [w[3], w[2], w[6], w[7]], // +Z front
        [w[1], w[0], w[4], w[5]], // -Z back
        [w[2], w[1], w[5], w[6]], // +X right
        [w[0], w[3], w[7], w[4]], // -X left
        [w[7], w[6], w[5], w[4]], // +Y top
        [w[0], w[1], w[2], w[3]], // -Y bottom
    ];

    for q in faces {
        let mid = [
            (q[0][0] + q[1][0] + q[2][0] + q[3][0]) * 0.25,
            (q[0][1] + q[1][1] + q[2][1] + q[3][1]) * 0.25,
            (q[0][2] + q[1][2] + q[2][2] + q[3][2]) * 0.25,
        ];
        let n = face_normal(q[0], q[1], q[2]);
        let outward = sub3(mid, center);
        if dot3(n, outward) >= 0.0 {
            push_quad(verts, indices, q[0], q[1], q[2], q[3], n, color, meta);
        } else {
            let flipped_n = scale3(n, -1.0);
            push_quad(
                verts, indices, q[0], q[3], q[2], q[1], flipped_n, color, meta,
            );
        }
    }
}

fn rot_xyz_pt(p: [f32; 3], rot: [f32; 3]) -> [f32; 3] {
    let (sx, cx) = rot[0].sin_cos();
    let y1 = p[1] * cx - p[2] * sx;
    let z1 = p[1] * sx + p[2] * cx;

    let (sy, cy) = rot[1].sin_cos();
    let x2 = p[0] * cy + z1 * sy;
    let z2 = -p[0] * sy + z1 * cy;

    let (sz, cz) = rot[2].sin_cos();
    let x3 = x2 * cz - y1 * sz;
    let y3 = x2 * sz + y1 * cz;

    [x3, y3, z2]
}

pub(crate) fn push_tube(
    verts: &mut Vec<Vertex>,
    indices: &mut Vec<u32>,
    spine: &[[f32; 3]],
    radii: &[f32],
    sides: usize,
    color: [f32; 3],
    kind: f32,
) {
    if spine.len() < 2 || spine.len() != radii.len() || sides < 3 {
        return;
    }

    let n_pts = spine.len();
    let mut tangents = Vec::with_capacity(n_pts);
    for i in 0..n_pts {
        let t = if i == 0 {
            sub3(spine[1], spine[0])
        } else if i + 1 == n_pts {
            sub3(spine[i], spine[i - 1])
        } else {
            sub3(spine[i + 1], spine[i - 1])
        };
        tangents.push(normalize3(t));
    }

    let up_seed = if tangents[0][1].abs() < 0.9 {
        [0.0, 1.0, 0.0]
    } else {
        [1.0, 0.0, 0.0]
    };
    let mut n1 = normalize3(cross3(tangents[0], up_seed));
    let mut ring_starts = Vec::with_capacity(n_pts);
    let mut arc = 0.0_f32;

    for i in 0..n_pts {
        if i > 0 {
            let d = sub3(spine[i], spine[i - 1]);
            arc += dot3(d, d).sqrt();
            let ti = tangents[i];
            n1 = normalize3(sub3(n1, scale3(ti, dot3(n1, ti))));
        }
        let n2 = normalize3(cross3(tangents[i], n1));
        let ring_start = verts.len() as u32;
        ring_starts.push(ring_start);

        for j in 0..=sides {
            let frac = j as f32 / sides as f32;
            let theta = frac * TAU;
            let (sin_t, cos_t) = theta.sin_cos();
            let radial = normalize3([
                n1[0] * cos_t + n2[0] * sin_t,
                n1[1] * cos_t + n2[1] * sin_t,
                n1[2] * cos_t + n2[2] * sin_t,
            ]);
            let pos = [
                spine[i][0] + radial[0] * radii[i],
                spine[i][1] + radial[1] * radii[i],
                spine[i][2] + radial[2] * radii[i],
            ];
            verts.push(Vertex {
                pos,
                normal: radial,
                color,
                meta: [kind, arc, frac, 0.0],
            });
        }
    }

    for i in 0..(n_pts - 1) {
        let r0 = ring_starts[i];
        let r1 = ring_starts[i + 1];
        for j in 0..sides as u32 {
            let v00 = r0 + j;
            let v10 = r0 + j + 1;
            let v01 = r1 + j;
            let v11 = r1 + j + 1;
            let e1 = sub3(verts[v10 as usize].pos, verts[v00 as usize].pos);
            let e2 = sub3(verts[v01 as usize].pos, verts[v00 as usize].pos);
            if dot3(cross3(e1, e2), verts[v00 as usize].normal) >= 0.0 {
                indices.extend_from_slice(&[v00, v10, v11, v00, v11, v01]);
            } else {
                indices.extend_from_slice(&[v00, v01, v11, v00, v11, v10]);
            }
        }
    }

    let push_cap = |verts: &mut Vec<Vertex>,
                    indices: &mut Vec<u32>,
                    center: [f32; 3],
                    ring_start: u32,
                    cap_normal: [f32; 3]| {
        let c_idx = verts.len() as u32;
        verts.push(Vertex {
            pos: center,
            normal: cap_normal,
            color,
            meta: [kind, 0.0, 0.0, 0.0],
        });
        for j in 0..sides as u32 {
            let va = ring_start + j;
            let vb = ring_start + j + 1;
            let e1 = sub3(verts[va as usize].pos, center);
            let e2 = sub3(verts[vb as usize].pos, center);
            if dot3(cross3(e1, e2), cap_normal) >= 0.0 {
                indices.extend_from_slice(&[c_idx, va, vb]);
            } else {
                indices.extend_from_slice(&[c_idx, vb, va]);
            }
        }
    };

    push_cap(
        verts,
        indices,
        spine[0],
        ring_starts[0],
        scale3(tangents[0], -1.0),
    );
    push_cap(
        verts,
        indices,
        spine[n_pts - 1],
        ring_starts[n_pts - 1],
        tangents[n_pts - 1],
    );
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn push_beveled_cap(
    verts: &mut Vec<Vertex>,
    indices: &mut Vec<u32>,
    cx: f32,
    cz: f32,
    bw: f32,
    bd: f32,
    tw: f32,
    td: f32,
    yb: f32,
    ytf: f32,
    ytb: f32,
    side_col: [f32; 3],
    top_col: [f32; 3],
    slot_f: f32,
) {
    let p = [
        [cx - bw, yb, cz - bd],
        [cx + bw, yb, cz - bd],
        [cx + bw, yb, cz + bd],
        [cx - bw, yb, cz + bd],
        [cx - tw, ytb, cz - td],
        [cx + tw, ytb, cz - td],
        [cx + tw, ytf, cz + td],
        [cx - tw, ytf, cz + td],
    ];

    let top_n = normalize3([0.0, 1.0, (ytb - ytf) / (2.0 * td.max(0.05))]);
    push_quad(
        verts,
        indices,
        p[7],
        p[6],
        p[5],
        p[4],
        top_n,
        top_col,
        [2.0, slot_f, 0.0, 1.0],
    );

    let faces = [
        ([p[3], p[2], p[6], p[7]], side_col),
        ([p[1], p[0], p[4], p[5]], side_col),
        ([p[0], p[3], p[7], p[4]], side_col),
        ([p[2], p[1], p[5], p[6]], side_col),
    ];
    for (quad, col) in faces {
        let n = face_normal(quad[0], quad[1], quad[2]);
        let base = verts.len() as u32;
        verts.push(Vertex {
            pos: quad[0],
            normal: n,
            color: col,
            meta: [2.0, slot_f, 1.0, 0.0],
        });
        verts.push(Vertex {
            pos: quad[1],
            normal: n,
            color: col,
            meta: [2.0, slot_f, 1.0, 0.0],
        });
        verts.push(Vertex {
            pos: quad[2],
            normal: n,
            color: col,
            meta: [2.0, slot_f, 0.15, 0.8],
        });
        verts.push(Vertex {
            pos: quad[3],
            normal: n,
            color: col,
            meta: [2.0, slot_f, 0.15, 0.8],
        });
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
}

pub(crate) fn push_box(
    verts: &mut Vec<Vertex>,
    indices: &mut Vec<u32>,
    min: [f32; 3],
    max: [f32; 3],
    color: [f32; 3],
    meta: [f32; 4],
) {
    let [x0, y0, z0] = min;
    let [x1, y1, z1] = max;
    push_quad(
        verts,
        indices,
        [x0, y1, z1],
        [x1, y1, z1],
        [x1, y1, z0],
        [x0, y1, z0],
        [0.0, 1.0, 0.0],
        color,
        meta,
    );
    push_quad(
        verts,
        indices,
        [x0, y0, z0],
        [x1, y0, z0],
        [x1, y0, z1],
        [x0, y0, z1],
        [0.0, -1.0, 0.0],
        color,
        meta,
    );
    push_quad(
        verts,
        indices,
        [x0, y0, z1],
        [x1, y0, z1],
        [x1, y1, z1],
        [x0, y1, z1],
        [0.0, 0.0, 1.0],
        color,
        meta,
    );
    push_quad(
        verts,
        indices,
        [x1, y0, z0],
        [x0, y0, z0],
        [x0, y1, z0],
        [x1, y1, z0],
        [0.0, 0.0, -1.0],
        color,
        meta,
    );
    push_quad(
        verts,
        indices,
        [x1, y0, z1],
        [x1, y0, z0],
        [x1, y1, z0],
        [x1, y1, z1],
        [1.0, 0.0, 0.0],
        color,
        meta,
    );
    push_quad(
        verts,
        indices,
        [x0, y0, z0],
        [x0, y0, z1],
        [x0, y1, z1],
        [x0, y1, z0],
        [-1.0, 0.0, 0.0],
        color,
        meta,
    );
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn push_quad(
    verts: &mut Vec<Vertex>,
    indices: &mut Vec<u32>,
    p0: [f32; 3],
    p1: [f32; 3],
    p2: [f32; 3],
    p3: [f32; 3],
    normal: [f32; 3],
    color: [f32; 3],
    meta: [f32; 4],
) {
    let base = verts.len() as u32;
    for pos in [p0, p1, p2, p3] {
        verts.push(Vertex {
            pos,
            normal,
            color,
            meta,
        });
    }
    indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
}

pub(crate) fn linear_hex(hex: u32) -> [f32; 3] {
    rgb_to_linear([
        ((hex >> 16) & 0xFF) as u8,
        ((hex >> 8) & 0xFF) as u8,
        (hex & 0xFF) as u8,
    ])
}

pub(crate) fn face_normal(a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> [f32; 3] {
    let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    normalize3(cross3(u, v))
}

pub(crate) fn rgb_to_linear(rgb: [u8; 3]) -> [f32; 3] {
    [
        (f32::from(rgb[0]) / 255.0).powf(2.2),
        (f32::from(rgb[1]) / 255.0).powf(2.2),
        (f32::from(rgb[2]) / 255.0).powf(2.2),
    ]
}

fn hsv_to_linear(h: f32, s: f32, v: f32) -> [f32; 3] {
    let h6 = h.rem_euclid(1.0) * 6.0;
    let c = v * s;
    let x = c * (1.0 - ((h6 % 2.0) - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match h6 as i32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    [(r + m).powf(2.2), (g + m).powf(2.2), (b + m).powf(2.2)]
}

pub(crate) fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    let t = t.clamp(0.0, 1.0);
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

pub(crate) fn scale3(a: [f32; 3], s: f32) -> [f32; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}

pub(crate) fn sub3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

pub(crate) fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub(crate) fn cross3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

pub(crate) fn normalize3(a: [f32; 3]) -> [f32; 3] {
    let len = dot3(a, a).sqrt().max(1e-6);
    [a[0] / len, a[1] / len, a[2] / len]
}

pub(crate) fn look_at(eye: [f32; 3], target: [f32; 3], up: [f32; 3]) -> [[f32; 4]; 4] {
    let f = normalize3(sub3(target, eye));
    let s = normalize3(cross3(f, up));
    let u = cross3(s, f);
    [
        [s[0], u[0], -f[0], 0.0],
        [s[1], u[1], -f[1], 0.0],
        [s[2], u[2], -f[2], 0.0],
        [-dot3(s, eye), -dot3(u, eye), dot3(f, eye), 1.0],
    ]
}

pub(crate) fn perspective(fov_y_rad: f32, aspect: f32, near: f32, far: f32) -> [[f32; 4]; 4] {
    let f = 1.0 / (fov_y_rad * 0.5).tan();
    [
        [f / aspect, 0.0, 0.0, 0.0],
        [0.0, f, 0.0, 0.0],
        [0.0, 0.0, far / (near - far), -1.0],
        [0.0, 0.0, (near * far) / (near - far), 0.0],
    ]
}

pub(crate) fn mul_mat4(a: [[f32; 4]; 4], b: [[f32; 4]; 4]) -> [[f32; 4]; 4] {
    let mut out = [[0.0f32; 4]; 4];
    for c in 0..4 {
        for r in 0..4 {
            out[c][r] = (0..4).map(|k| a[k][r] * b[c][k]).sum();
        }
    }
    out
}

const SHADER_WGSL: &str = r#"
struct SceneUniforms {
    view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
    board_params: vec4<f32>,
    mascot_params: vec4<f32>,
    key_colors: array<vec4<f32>, 132>,
    key_motion: array<vec4<f32>, 132>,
};

@group(0) @binding(0)
var<uniform> u: SceneUniforms;

struct VsIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec3<f32>,
    @location(3) part_info: vec4<f32>,
};

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec3<f32>,
    @location(3) part_info: vec4<f32>,
};

@vertex
fn vs_main(vs_in: VsIn) -> VsOut {
    var local = vs_in.pos;
    var norm = vs_in.normal;
    let slot = i32(vs_in.part_info.y + 0.5);
    let kind = vs_in.part_info.x;

    // Hidden easter-egg chibi figures (kind 7 = Miku, kind 8 = Teto).
    if kind >= 6.9 {
        let is_teto = kind >= 7.5;
        let sc = select(u.mascot_params.x, u.mascot_params.y, is_teto);
        if sc <= 0.001 {
            var hidden: VsOut;
            hidden.clip = vec4<f32>(0.0, 0.0, -2.0, 1.0);
            hidden.world_pos = local;
            hidden.normal = norm;
            hidden.color = vs_in.color;
            hidden.part_info = vs_in.part_info;
            return hidden;
        }
        let age = select(u.mascot_params.z, u.mascot_params.w, is_teto);
        let pivot = select(vec3<f32>(1.48, -0.30, -1.82), vec3<f32>(-1.25, -0.30, -1.82), is_teto);
        let phase = select(0.0, 1.4, is_teto);
        let bounce = abs(sin(age * 4.4 + phase)) * 0.032;
        var rel = local - pivot;
        if vs_in.part_info.y > 0.25 {
            let sway = sin(age * 3.8 + phase + vs_in.part_info.z * 2.2) * 0.024 * vs_in.part_info.y;
            rel.x += sway;
            rel.y += sway * 0.35;
        }
        rel *= sc;
        rel.y += bounce * sc;
        local = pivot + rel;
    }

    // Depress keycaps & legend pads straight down along the local stem axis before tilting board.
    if slot >= 0 && slot < 132 && (kind == 2.0 || kind == 4.0) {
        local.y -= u.key_motion[slot].x;
    }

    // Apply ~5.4° keyboard chassis typing tilt around X for chassis/key parts (kind < 4.5).
    if kind < 4.5 {
        let s = u.board_params.x;
        let c = u.board_params.y;
        let py = local.y * c - local.z * s;
        let pz = local.y * s + local.z * c;
        local.y = py + 0.04;
        local.z = pz;

        let ny = norm.y * c - norm.z * s;
        let nz = norm.y * s + norm.z * c;
        norm = normalize(vec3<f32>(norm.x, ny, nz));
    }

    var vs_out: VsOut;
    vs_out.clip = u.view_proj * vec4<f32>(local, 1.0);
    vs_out.world_pos = local;
    vs_out.normal = norm;
    vs_out.color = vs_in.color;
    vs_out.part_info = vs_in.part_info;
    return vs_out;
}

fn matte_grain(p: vec2<f32>) -> f32 {
    let q = floor(p * 85.0);
    let h = fract(sin(dot(q, vec2<f32>(127.1, 311.7))) * 43758.5453);
    return h * 2.0 - 1.0;
}

@fragment
fn fs_main(frag_in: VsOut) -> @location(0) vec4<f32> {
    let kind = frag_in.part_info.x;
    let slot = i32(frag_in.part_info.y + 0.5);
    let n = normalize(frag_in.normal);
    let v = normalize(u.camera_pos.xyz - frag_in.world_pos);

    let key_dir = normalize(vec3<f32>(-0.45, 0.88, 0.55));
    let rim_dir = normalize(vec3<f32>(0.75, 0.45, -0.75));
    let diff = max(dot(n, key_dir), 0.0);
    let rim = max(dot(n, rim_dir), 0.0);
    let half_v = normalize(key_dir + v);
    let spec = pow(max(dot(n, half_v), 0.0), 32.0);

    var lit = vec3<f32>(0.0);
    if kind >= 6.9 {
        // Low-poly chibi figure vinyl/toon wrap shading with subtle hair/eye lift.
        let wrap_diff = clamp((dot(n, key_dir) + 0.35) / 1.35, 0.0, 1.0);
        let glow = frag_in.part_info.w;
        lit = frag_in.color * (0.42 + 0.58 * wrap_diff + 0.16 * rim + glow * 0.45)
            + vec3<f32>(0.06 * spec);
    } else if kind >= 4.5 && kind < 5.5 {
        // Paper-matte Regal Navy desk pad with soft contact shadow under keyboard, cable, and chibis.
        let dx = max(abs(frag_in.world_pos.x) - 2.95, 0.0);
        let dz = max(abs(frag_in.world_pos.z) - 1.12, 0.0);
        let d = sqrt(dx * dx + dz * dz);
        var shadow = smoothstep(0.0, 0.62, d);
        if frag_in.world_pos.z < -1.55 {
            let t_seg = clamp((-frag_in.world_pos.z - 2.45) / 1.75, 0.0, 1.0);
            let cable_cx = mix(-2.45, -2.96, t_seg * t_seg * (3.0 - 2.0 * t_seg));
            let c_dist = abs(frag_in.world_pos.x - cable_cx);
            let c_soft = smoothstep(0.015, 0.16, c_dist);
            let z_fade = smoothstep(-1.55, -2.20, frag_in.world_pos.z);
            shadow *= mix(1.0, 0.55 + 0.45 * c_soft, z_fade);
        }
        if u.mascot_params.x > 0.05 {
            let md = length(frag_in.world_pos.xz - vec2<f32>(1.48, -1.82));
            let ms = smoothstep(0.12, 0.48, md);
            shadow *= mix(1.0, 0.45 + 0.55 * ms, u.mascot_params.x);
        }
        if u.mascot_params.y > 0.05 {
            let td = length(frag_in.world_pos.xz - vec2<f32>(-1.25, -1.82));
            let ts = smoothstep(0.12, 0.48, td);
            shadow *= mix(1.0, 0.45 + 0.55 * ts, u.mascot_params.y);
        }
        let regal_ambient = vec3<f32>(0.008, 0.022, 0.052) * exp(-d * 1.15);
        let grain = matte_grain(frag_in.world_pos.xz) * 0.0022;
        lit = frag_in.color * (0.42 + 0.58 * shadow) + regal_ambient + vec3<f32>(grain);
    } else if kind >= 5.5 {
        // Braided Type-C cable jacket (`6.0`) and anodized cable organizer cuff (`6.5`).
        let arc = frag_in.part_info.y;
        let ring_u = frag_in.part_info.z;
        let w1 = sin((arc * 24.0 + ring_u * 6.0) * 6.28318);
        let w2 = sin((arc * 24.0 - ring_u * 6.0) * 6.28318);
        let braid = select(1.0 + 0.14 * w1 * w2, 1.0, kind >= 6.4);
        let spec_weight = select(0.11, 0.34, kind >= 6.4);
        lit = (frag_in.color * braid) * (0.28 + 0.70 * diff + 0.20 * rim) + vec3<f32>(spec_weight * spec);
    } else if kind == 3.0 && slot >= 0 && slot < 132 {
        let kc = u.key_colors[slot].rgb;
        let sel = u.key_motion[slot].y;
        let foc = u.key_motion[slot].z;
        lit = kc * 1.15 + vec3<f32>(0.015, 0.028, 0.052) * (0.25 + diff * 0.4);
        if sel > 0.5 {
            lit += vec3<f32>(0.10, 0.22, 0.38);
        }
        if foc > 0.5 {
            lit += vec3<f32>(0.34, 0.48, 0.68);
        }
    } else if kind == 4.0 && slot >= 0 && slot < 132 {
        let kc = u.key_colors[slot].rgb;
        let emit = frag_in.part_info.z;
        lit = frag_in.color * (0.18 + 0.55 * diff) + kc * (0.65 * emit + 0.12);
    } else {
        var base = frag_in.color;
        var bounce = vec3<f32>(0.0);
        if kind == 2.0 && slot >= 0 && slot < 132 {
            let kc = u.key_colors[slot].rgb;
            let sel = u.key_motion[slot].y;
            let foc = u.key_motion[slot].z;
            bounce = kc * (0.34 * frag_in.part_info.z);
            if sel > 0.5 && frag_in.part_info.w > 0.5 {
                base = mix(base, vec3<f32>(0.09, 0.18, 0.32), 0.32);
            }
            if foc > 0.5 && frag_in.part_info.w > 0.5 {
                base = mix(base, vec3<f32>(0.24, 0.38, 0.58), 0.38);
            }
        }
        let metallic_spec = select(0.11, 0.28, kind == 1.0) * spec;
        lit = base * (0.28 + 0.72 * diff + 0.18 * rim) + vec3<f32>(metallic_spec) + bounce;
    }

    let srgb = pow(clamp(lit, vec3<f32>(0.0), vec3<f32>(1.0)), vec3<f32>(1.0 / 2.2));
    return vec4<f32>(srgb, 1.0);
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evaluates_static_and_custom_key_colors() {
        let light = LightState {
            mode: 0,
            fg: [255, 0, 0],
            brightness: 4,
            power_on: true,
            ..Default::default()
        };

        let depths = [0.0f32; MATRIX_SLOTS];
        let sel = BTreeSet::new();
        let empty_set = BTreeSet::new();
        let scene = Kb3dSceneInput {
            light: &light,
            custom_colors: None,
            use_custom_colors: false,
            typing_only: false,
            impulses: &[],
            selected_keys: &sel,
            focused_key: None,
            live_depths_mm: &depths,
            time_s: 0.0,
            miku: MascotPose::default(),
            teto: MascotPose::default(),
            daemon_aqua_keys: &empty_set,
            daemon_aqua_pulsing: &empty_set,
        };
        let colors = evaluate_key_colors(&scene);
        assert!(colors[0][0] > 0.9);
        assert!(colors[0][1] < 0.01);

        let silent = evaluate_key_colors(&Kb3dSceneInput {
            typing_only: true,
            ..scene
        });
        assert!(silent[0][0] < 0.01);

        let ripple_light = LightState {
            mode: 8,
            fg: [255, 0, 0],
            brightness: 4,
            power_on: true,
            ..Default::default()
        };
        let ripple_silent = evaluate_key_colors(&Kb3dSceneInput {
            light: &ripple_light,
            custom_colors: None,
            use_custom_colors: false,
            typing_only: false,
            impulses: &[],
            selected_keys: &sel,
            focused_key: None,
            live_depths_mm: &depths,
            time_s: 1.0,
            miku: MascotPose::default(),
            teto: MascotPose::default(),
            daemon_aqua_keys: &empty_set,
            daemon_aqua_pulsing: &empty_set,
        });
        assert!(ripple_silent[0][0] < 0.01);

        // Fireworks (mode 16) persists > 1.5s locally around pressed key while Ripple (mode 8) already faded out.
        let w_key = WIN60HE_KEYS.iter().find(|k| k.index == 46).unwrap();
        let w_nx = ((w_key.x as f32 + w_key.width as f32 * 0.5) - 10.0) / 577.0;
        let w_ny = ((w_key.y as f32 + w_key.height as f32 * 0.5) - 10.0) / 207.0;
        let aged_imp = [KeyImpulse {
            index: 46,
            nx: w_nx,
            ny: w_ny,
            age_s: 1.8,
            strength: 1.0,
        }];
        let ripple_aged = evaluate_key_colors(&Kb3dSceneInput {
            light: &ripple_light,
            impulses: &aged_imp,
            ..scene
        });
        let fw_light = LightState {
            mode: 16,
            ..ripple_light
        };
        let fw_aged = evaluate_key_colors(&Kb3dSceneInput {
            light: &fw_light,
            impulses: &aged_imp,
            ..scene
        });
        let w_slot = WIN60HE_KEYS.iter().position(|k| k.index == 46).unwrap();
        let e_slot = WIN60HE_KEYS.iter().position(|k| k.name == "E").unwrap();
        let enter_slot = WIN60HE_KEYS.iter().position(|k| k.name == "Enter").unwrap();
        assert!(ripple_aged[w_slot][0] < 0.01, "Ripple should fade by 1.8s");
        assert!(
            fw_aged[w_slot][0] > 0.12,
            "Fireworks should still glow locally at 1.8s"
        );
        assert!(
            fw_aged[e_slot][0] > 0.02,
            "Fireworks should illuminate adjacent area around pressed key"
        );
        assert!(
            fw_aged[enter_slot][0] < 0.02,
            "Fireworks should stay localized near pressed area"
        );

        // Daemon memory indicator keys glow vivid Aqua Blue regardless of base lighting.
        let mut aqua_keys = BTreeSet::new();
        aqua_keys.insert(46);
        let aqua_out = evaluate_key_colors(&Kb3dSceneInput {
            daemon_aqua_keys: &aqua_keys,
            ..scene
        });
        assert!(aqua_out[w_slot][2] > 0.9 && aqua_out[w_slot][0] < 0.05);

        let mut custom = [[0u8; 3]; MATRIX_SLOTS];
        custom[46] = [0, 255, 128];
        let custom_out = evaluate_key_colors(&Kb3dSceneInput {
            light: &light,
            custom_colors: Some(&custom),
            use_custom_colors: true,
            typing_only: false,
            impulses: &[],
            selected_keys: &sel,
            focused_key: None,
            live_depths_mm: &depths,
            time_s: 0.0,
            miku: MascotPose::default(),
            teto: MascotPose::default(),
            daemon_aqua_keys: &empty_set,
            daemon_aqua_pulsing: &empty_set,
        });
        assert!(custom_out[w_slot][1] > 0.9);
    }

    #[test]
    fn model_registry_builds_keyboard_cable_and_mascots() {
        let model = find_model(Some("win60he"));
        assert_eq!(model.id, "win60he");
        let (verts, idx) = build_scene_mesh(model);
        assert!(verts.len() > 2000);
        assert_eq!(idx.len() % 3, 0);

        let cable_verts: Vec<&Vertex> = verts
            .iter()
            .filter(|v| v.meta[0] >= 5.5 && v.meta[0] < 6.9)
            .collect();
        assert!(!cable_verts.is_empty(), "expected Type-C cable vertices");
        assert!(
            cable_verts
                .iter()
                .all(|v| v.pos[1] >= -0.32 && v.pos[2] <= -1.4),
            "Type-C wiring should drape cleanly above desk pad rear-left"
        );

        let miku_verts: Vec<&Vertex> = verts
            .iter()
            .filter(|v| (v.meta[0] - 7.0).abs() < 0.1)
            .collect();
        let teto_verts: Vec<&Vertex> = verts
            .iter()
            .filter(|v| (v.meta[0] - 8.0).abs() < 0.1)
            .collect();
        assert!(!miku_verts.is_empty(), "expected low-poly Miku mesh");
        assert!(!teto_verts.is_empty(), "expected low-poly Teto mesh");
        assert!(
            miku_verts
                .iter()
                .all(|v| v.pos[1] >= -0.35 && v.pos[2] <= -1.35)
        );
        assert!(
            teto_verts
                .iter()
                .all(|v| v.pos[1] >= -0.35 && v.pos[2] <= -1.35)
        );

        const {
            assert!((KB3D_WIDTH * 4).is_multiple_of(256));
        }
    }

    #[test]
    fn renders_offscreen_3d_frame_when_gpu_available() {
        let renderer = match KeyboardRenderer3D::new_for_keyboard(None) {
            Ok(r) => r,
            Err(err) => {
                assert!(
                    !err.contains("validation"),
                    "wgpu shader/pipeline validation failed: {err}"
                );
                return;
            }
        };
        let cam = Kb3dCamera::default();
        let light = LightState::default();
        let depths = [0.0f32; MATRIX_SLOTS];
        let sel = BTreeSet::new();
        let empty = BTreeSet::new();
        let scene = Kb3dSceneInput {
            light: &light,
            custom_colors: None,
            use_custom_colors: false,
            typing_only: false,
            impulses: &[],
            selected_keys: &sel,
            focused_key: Some(46),
            live_depths_mm: &depths,
            time_s: 0.25,
            miku: MascotPose {
                scale: 1.0,
                age_s: 0.8,
            },
            teto: MascotPose {
                scale: 1.0,
                age_s: 0.4,
            },
            daemon_aqua_keys: &empty,
            daemon_aqua_pulsing: &empty,
        };
        let frame = renderer.render_frame(&cam, &scene).unwrap();
        let bytes = frame.as_bytes(0).unwrap();
        assert_eq!(bytes.len(), (KB3D_WIDTH * KB3D_HEIGHT * 4) as usize);
        assert!(bytes.iter().any(|&b| b != 0));
    }
}
