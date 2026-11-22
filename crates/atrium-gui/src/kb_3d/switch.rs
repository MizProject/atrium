//! `wgpu` 3D cutaway Hall-effect switch mechanism visualizer for `atrium-gui` Diagnostics.
//!
//! Models a translucent cutaway housing, POM `+` cross stem, sculpted cutaway keycap shell,
//! dynamic compressing helical coil spring, moving N/S neodymium magnet, SMD Hall-effect
//! sensor IC with live magnetic flux glow, PCB LED, and actuation threshold guide rails.

use std::f32::consts::{PI, TAU};
use std::mem::size_of;
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use gpui_kit::RenderImage;

use super::{
    OffscreenTarget, Vertex, create_init_buffer, create_vertex_pipeline, create_wgpu_device,
    linear_hex, look_at, mul_mat4, perspective, push_box, push_oriented_frustum, push_tube,
    rgb_to_linear,
};

/// Offscreen viewport size (`448 * 4 = 1792`, an exact multiple of 256).
pub const SWITCH_3D_WIDTH: u32 = 448;
pub const SWITCH_3D_HEIGHT: u32 = 360;

const MAX_STROKE_WORLD: f32 = 0.62;
const SPRING_Y_BOT: f32 = -1.00;
const SPRING_Y_TOP_REST: f32 = -0.08;
const MAGNET_BOT_REST: f32 = -0.26;

const KIND_BASE_PCB: f32 = 0.0;
const KIND_HALL_IC: f32 = 1.0;
const KIND_FLUX_PAD: f32 = 1.5;
const KIND_PCB_LED: f32 = 1.8;
const KIND_BOT_HOUSING: f32 = 2.0;
const KIND_TOP_HOUSING: f32 = 3.0;
const KIND_ACT_MARKER: f32 = 4.0;
const KIND_STEM: f32 = 5.0;
const KIND_KEYCAP: f32 = 5.5;
const KIND_MAGNET_SOUTH: f32 = 6.0;
const KIND_MAGNET_NORTH: f32 = 6.2;
const KIND_SPRING: f32 = 7.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwitchCameraPreset {
    Iso,
    Cutaway,
    Side,
    Top,
}

#[derive(Debug, Clone, Copy)]
pub struct SwitchCamera {
    pub yaw: f32,
    pub pitch: f32,
    pub zoom: f32,
}

impl Default for SwitchCamera {
    fn default() -> Self {
        Self::from_preset(SwitchCameraPreset::Iso)
    }
}

impl SwitchCamera {
    pub fn from_preset(preset: SwitchCameraPreset) -> Self {
        match preset {
            SwitchCameraPreset::Iso => Self {
                yaw: 0.44,
                pitch: 0.34,
                zoom: 4.75,
            },
            SwitchCameraPreset::Cutaway => Self {
                yaw: 0.04,
                pitch: 0.18,
                zoom: 4.55,
            },
            SwitchCameraPreset::Side => Self {
                yaw: 1.24,
                pitch: 0.24,
                zoom: 4.65,
            },
            SwitchCameraPreset::Top => Self {
                yaw: 0.0,
                pitch: 1.38,
                zoom: 4.85,
            },
        }
    }

    pub fn orbit(&mut self, dx: f32, dy: f32) {
        self.yaw = (self.yaw + dx * 0.0095).clamp(-PI, PI);
        self.pitch = (self.pitch + dy * 0.0095).clamp(-0.08, 1.48);
    }

    pub fn adjust_zoom(&mut self, delta: f32) {
        self.zoom = (self.zoom + delta).clamp(2.9, 7.2);
    }
}

#[derive(Clone, Debug)]
pub struct SwitchSceneInput {
    pub switch_id: u8,
    pub travel_mm: f32,
    pub max_mm: f32,
    pub actuation_mm: f32,
    pub triggered: bool,
    pub rapid_trigger: bool,
    pub led_rgb: [u8; 3],
    pub time_s: f32,
}

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct SwitchUniforms {
    view_proj: [[f32; 4]; 4],
    camera_pos: [f32; 4],
    /// x = stroke_world, y = act_offset_world, z = flux_norm (0..1), w = triggered (0|1)
    motion: [f32; 4],
    /// rgb = POM stem linear color, w = rapid_trigger (0|1)
    stem_color: [f32; 4],
    /// rgb = PCB / key bounce LED linear color, w = time_s
    led_color: [f32; 4],
}

pub struct SwitchRenderer3D {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    pipeline: wgpu::RenderPipeline,
    uniform_buf: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    vertex_buf: wgpu::Buffer,
    index_buf: wgpu::Buffer,
    index_count: u32,
    target: OffscreenTarget,
}

impl SwitchRenderer3D {
    pub fn from_shared_device(
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
    ) -> Result<Self, String> {
        let (vertices, indices) = build_switch_mesh();
        let index_count = u32::try_from(indices.len()).map_err(|_| "switch mesh too large".to_string())?;

        let vertex_buf = create_init_buffer(
            &device,
            "sw3d-vb",
            bytemuck::cast_slice(&vertices),
            wgpu::BufferUsages::VERTEX,
        );
        let index_buf = create_init_buffer(
            &device,
            "sw3d-ib",
            bytemuck::cast_slice(&indices),
            wgpu::BufferUsages::INDEX,
        );
        let uniform_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sw3d-uniforms"),
            size: size_of::<SwitchUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sw3d-bind-layout"),
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
            label: Some("sw3d-bind-group"),
            layout: &bind_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buf.as_entire_binding(),
            }],
        });

        let pipeline = create_vertex_pipeline(&device, "sw3d", SWITCH_SHADER_WGSL, &bind_layout)?;
        let target = OffscreenTarget::new(&device, "sw3d", SWITCH_3D_WIDTH, SWITCH_3D_HEIGHT);

        Ok(Self {
            device,
            queue,
            pipeline,
            uniform_buf,
            bind_group,
            vertex_buf,
            index_buf,
            index_count,
            target,
        })
    }

    #[allow(dead_code)]
    pub fn new() -> Result<Self, String> {
        let (device, queue) = create_wgpu_device("atrium-switch3d-device")?;
        Self::from_shared_device(device, queue)
    }

    pub fn render_frame(
        &self,
        cam: &SwitchCamera,
        scene: &SwitchSceneInput,
    ) -> Result<Arc<RenderImage>, String> {
        let uniforms = build_switch_uniforms(cam, scene);
        self.queue
            .write_buffer(&self.uniform_buf, 0, bytemuck::bytes_of(&uniforms));

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("sw3d-encoder"),
            });

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("sw3d-pass"),
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

pub fn stem_color_hex_for_switch(switch_id: u8) -> u32 {
    match switch_id {
        1 => 0x38bdf8, // HM1 cool sky blue
        2 => 0x2dd4bf, // HH1 mint teal
        3 => 0xf59e0b, // CY1 amber gold
        5 => 0xa78bfa, // TC1 violet-lilac
        _ => 0x60a5fa,
    }
}

fn build_switch_uniforms(cam: &SwitchCamera, scene: &SwitchSceneInput) -> SwitchUniforms {
    let eye = [
        cam.zoom * cam.pitch.cos() * cam.yaw.sin(),
        cam.zoom * cam.pitch.sin(),
        cam.zoom * cam.pitch.cos() * cam.yaw.cos(),
    ];
    let target = [0.0, 0.02, 0.0];
    let view = look_at(eye, target, [0.0, 1.0, 0.0]);
    let proj = perspective(
        38.0_f32.to_radians(),
        SWITCH_3D_WIDTH as f32 / SWITCH_3D_HEIGHT as f32,
        0.1,
        24.0,
    );
    let view_proj = mul_mat4(proj, view);

    let max_mm = scene.max_mm.max(0.5);
    let travel_frac = (scene.travel_mm / max_mm).clamp(0.0, 1.0);
    let act_frac = (scene.actuation_mm / max_mm).clamp(0.02, 0.98);
    let stroke_world = travel_frac * MAX_STROKE_WORLD;
    let act_world = act_frac * MAX_STROKE_WORLD;
    // Hall sensor inverse-distance flux curve normalized cleanly to 0..1
    let flux_norm = ((0.22 + 0.78 * travel_frac) * travel_frac).clamp(0.0, 1.0);

    let stem_lin = linear_hex(stem_color_hex_for_switch(scene.switch_id));
    let led_lin = rgb_to_linear(scene.led_rgb);

    SwitchUniforms {
        view_proj,
        camera_pos: [eye[0], eye[1], eye[2], scene.time_s],
        motion: [
            stroke_world,
            act_world,
            flux_norm,
            if scene.triggered { 1.0 } else { 0.0 },
        ],
        stem_color: [
            stem_lin[0],
            stem_lin[1],
            stem_lin[2],
            if scene.rapid_trigger { 1.0 } else { 0.0 },
        ],
        led_color: [led_lin[0], led_lin[1], led_lin[2], scene.time_s],
    }
}

pub fn build_switch_mesh() -> (Vec<Vertex>, Vec<u32>) {
    let mut verts = Vec::with_capacity(6000);
    let mut indices = Vec::with_capacity(11000);

    // 1. Tester deck + FR-4 PCB + brass pins + copper trace accents.
    let base_col = linear_hex(0x091321);
    push_box(
        &mut verts,
        &mut indices,
        [-1.65, -1.34, -1.55],
        [1.65, -1.24, 1.55],
        base_col,
        [KIND_BASE_PCB, 0.0, 0.0, 0.0],
    );

    let pcb_col = linear_hex(0x0f2537);
    let pcb_edge = linear_hex(0x1e3f5c);
    push_box(
        &mut verts,
        &mut indices,
        [-1.25, -1.22, -1.20],
        [1.25, -1.10, 1.20],
        pcb_col,
        [KIND_BASE_PCB, 0.0, 0.0, 0.0],
    );
    push_box(
        &mut verts,
        &mut indices,
        [-1.26, -1.16, 1.16],
        [1.26, -1.10, 1.21],
        pcb_edge,
        [KIND_BASE_PCB, 0.0, 0.0, 0.25],
    );

    let gold = linear_hex(0xd6a84f);
    for (px, pz) in [
        (-1.05, -0.98),
        (1.05, -0.98),
        (-1.05, 0.98),
        (1.05, 0.98),
        (-0.48, -0.42),
        (0.48, -0.42),
    ] {
        push_box(
            &mut verts,
            &mut indices,
            [px - 0.08, -1.105, pz - 0.08],
            [px + 0.08, -1.088, pz + 0.08],
            gold,
            [KIND_BASE_PCB, 0.0, 0.12, 0.45],
        );
    }
    // Two underside mechanical mounting legs / alignment dowels passing through PCB
    push_box(
        &mut verts,
        &mut indices,
        [-0.56, -1.25, -0.10],
        [-0.44, -1.08, 0.02],
        gold,
        [KIND_BASE_PCB, 0.0, 0.1, 0.5],
    );
    push_box(
        &mut verts,
        &mut indices,
        [0.44, -1.25, -0.10],
        [0.56, -1.08, 0.02],
        gold,
        [KIND_BASE_PCB, 0.0, 0.1, 0.5],
    );

    // 2. Centered SMD Hall-Effect Sensor IC + gull-wing leads + Magnetic Flux Glow Pad.
    let ic_col = linear_hex(0x151923);
    let tin = linear_hex(0xb8c6d8);
    push_box(
        &mut verts,
        &mut indices,
        [-0.14, -1.10, -0.12],
        [0.14, -1.01, 0.12],
        ic_col,
        [KIND_HALL_IC, 0.0, 0.0, 0.1],
    );
    for lz in [-0.075_f32, 0.0, 0.075] {
        push_box(
            &mut verts,
            &mut indices,
            [-0.20, -1.10, lz - 0.018],
            [-0.14, -1.04, lz + 0.018],
            tin,
            [KIND_HALL_IC, 0.0, 0.0, 0.6],
        );
        push_box(
            &mut verts,
            &mut indices,
            [0.14, -1.10, lz - 0.018],
            [0.20, -1.04, lz + 0.018],
            tin,
            [KIND_HALL_IC, 0.0, 0.0, 0.6],
        );
    }
    // Flux sensing glowing perimeter pad & trace ring surrounding Hall IC
    let flux_pad_col = linear_hex(0x164e63);
    push_box(
        &mut verts,
        &mut indices,
        [-0.29, -1.098, -0.27],
        [0.29, -1.084, 0.27],
        flux_pad_col,
        [KIND_FLUX_PAD, 0.0, 1.0, 0.0],
    );

    // 3. Per-key SMD RGB LED mounted toward rear PCB (-Z).
    let led_housing = linear_hex(0xd6dee8);
    let led_die = linear_hex(0x60a5fa);
    push_box(
        &mut verts,
        &mut indices,
        [-0.18, -1.10, -0.62],
        [0.18, -1.03, -0.44],
        led_housing,
        [KIND_BASE_PCB, 0.0, 0.0, 0.0],
    );
    push_box(
        &mut verts,
        &mut indices,
        [-0.13, -1.04, -0.59],
        [0.13, -1.00, -0.47],
        led_die,
        [KIND_PCB_LED, 0.0, 1.0, 0.0],
    );

    // 4. Cutaway Nylon Bottom Housing (`+Z` front open cutaway so spring & magnet stay unobstructed).
    let bot_col = linear_hex(0x1a273b);
    let rail_col = linear_hex(0x283c59);
    // Floor frame
    push_box(
        &mut verts,
        &mut indices,
        [-0.86, -1.08, -0.86],
        [0.86, -1.00, -0.32],
        bot_col,
        [KIND_BOT_HOUSING, 0.0, 0.0, 0.0],
    );
    push_box(
        &mut verts,
        &mut indices,
        [-0.86, -1.08, -0.32],
        [-0.44, -1.00, 0.56],
        bot_col,
        [KIND_BOT_HOUSING, 0.0, 0.0, 0.0],
    );
    push_box(
        &mut verts,
        &mut indices,
        [0.44, -1.08, -0.32],
        [0.86, -1.00, 0.56],
        bot_col,
        [KIND_BOT_HOUSING, 0.0, 0.0, 0.0],
    );
    // Back wall (-Z)
    push_box(
        &mut verts,
        &mut indices,
        [-0.86, -1.00, -0.86],
        [0.86, -0.05, -0.70],
        bot_col,
        [KIND_BOT_HOUSING, 0.0, 0.0, 0.0],
    );
    // Left wall (-X) & Right wall (+X) stopping cleanly at z = +0.54 cutaway plane
    push_box(
        &mut verts,
        &mut indices,
        [-0.86, -1.00, -0.70],
        [-0.70, -0.05, 0.54],
        bot_col,
        [KIND_BOT_HOUSING, 0.0, 0.0, 0.0],
    );
    push_box(
        &mut verts,
        &mut indices,
        [0.70, -1.00, -0.70],
        [0.86, -0.05, 0.54],
        bot_col,
        [KIND_BOT_HOUSING, 0.0, 0.0, 0.0],
    );
    // Inner left & right guide rail tracks for stem side sliders
    push_box(
        &mut verts,
        &mut indices,
        [-0.70, -0.96, -0.22],
        [-0.58, -0.06, -0.04],
        rail_col,
        [KIND_BOT_HOUSING, 0.0, 0.0, 0.15],
    );
    push_box(
        &mut verts,
        &mut indices,
        [-0.70, -0.96, 0.18],
        [-0.58, -0.06, 0.34],
        rail_col,
        [KIND_BOT_HOUSING, 0.0, 0.0, 0.15],
    );
    push_box(
        &mut verts,
        &mut indices,
        [0.58, -0.96, -0.22],
        [0.70, -0.06, -0.04],
        rail_col,
        [KIND_BOT_HOUSING, 0.0, 0.0, 0.15],
    );
    push_box(
        &mut verts,
        &mut indices,
        [0.58, -0.96, 0.18],
        [0.70, -0.06, 0.34],
        rail_col,
        [KIND_BOT_HOUSING, 0.0, 0.0, 0.15],
    );

    // Shallow spring seat centering collar ring around Hall IC (back/left/right horseshoe)
    let seat_col = linear_hex(0x334155);
    push_box(
        &mut verts,
        &mut indices,
        [-0.31, -1.02, -0.34],
        [0.31, -0.93, -0.27],
        seat_col,
        [KIND_BOT_HOUSING, 0.0, 0.0, 0.1],
    );
    push_box(
        &mut verts,
        &mut indices,
        [-0.34, -1.02, -0.27],
        [-0.27, -0.93, 0.24],
        seat_col,
        [KIND_BOT_HOUSING, 0.0, 0.0, 0.1],
    );
    push_box(
        &mut verts,
        &mut indices,
        [0.27, -1.02, -0.27],
        [0.34, -0.93, 0.24],
        seat_col,
        [KIND_BOT_HOUSING, 0.0, 0.0, 0.1],
    );

    // 5. Cutaway Polycarbonate Top Housing (`+Z` front open cutaway).
    let top_col = linear_hex(0x647d9e);
    let latch_col = linear_hex(0x88a4c7);
    // Tapered rear & side top-housing shell (inset slightly so keycap sleeves cleanly outside)
    push_oriented_frustum(
        &mut verts,
        &mut indices,
        [0.0, -0.05, 0.0],
        0.0,
        [0.0, 0.22, -0.69],
        [1.58, 0.44, 0.20],
        [0.80, 0.80],
        [0.0, 0.0, 0.0],
        top_col,
        [KIND_TOP_HOUSING, 0.0, 0.0, 0.2],
    );
    push_oriented_frustum(
        &mut verts,
        &mut indices,
        [0.0, -0.05, 0.0],
        0.0,
        [-0.70, 0.22, -0.10],
        [0.19, 0.44, 1.12],
        [0.80, 0.86],
        [0.0, 0.0, 0.0],
        top_col,
        [KIND_TOP_HOUSING, 0.0, 0.0, 0.2],
    );
    push_oriented_frustum(
        &mut verts,
        &mut indices,
        [0.0, -0.05, 0.0],
        0.0,
        [0.70, 0.22, -0.10],
        [0.19, 0.44, 1.12],
        [0.80, 0.86],
        [0.0, 0.0, 0.0],
        top_col,
        [KIND_TOP_HOUSING, 0.0, 0.0, 0.2],
    );
    // Top chimney collar rear/sides around stem opening
    push_box(
        &mut verts,
        &mut indices,
        [-0.54, 0.37, -0.54],
        [0.54, 0.48, -0.32],
        latch_col,
        [KIND_TOP_HOUSING, 0.0, 0.0, 0.35],
    );
    push_box(
        &mut verts,
        &mut indices,
        [-0.54, 0.37, -0.32],
        [-0.34, 0.48, 0.30],
        latch_col,
        [KIND_TOP_HOUSING, 0.0, 0.0, 0.35],
    );
    push_box(
        &mut verts,
        &mut indices,
        [0.34, 0.37, -0.32],
        [0.54, 0.48, 0.30],
        latch_col,
        [KIND_TOP_HOUSING, 0.0, 0.0, 0.35],
    );
    // Side housing snap latches
    for sx in [-0.88_f32, 0.88_f32] {
        for sz in [-0.45_f32, 0.22_f32] {
            push_box(
                &mut verts,
                &mut indices,
                [sx - 0.03, -0.20, sz - 0.06],
                [sx + 0.03, -0.02, sz + 0.06],
                latch_col,
                [KIND_TOP_HOUSING, 0.0, 0.0, 0.3],
            );
        }
    }

    // 6. Floating Actuation Plane Laser Brackets (`y` centered around MAGNET_BOT_REST, shifted in shader by act_world).
    let act_col = linear_hex(0x38bdf8);
    push_box(
        &mut verts,
        &mut indices,
        [-0.78, MAGNET_BOT_REST - 0.018, 0.52],
        [-0.34, MAGNET_BOT_REST + 0.018, 0.64],
        act_col,
        [KIND_ACT_MARKER, 0.0, 1.0, 0.0],
    );
    push_box(
        &mut verts,
        &mut indices,
        [0.34, MAGNET_BOT_REST - 0.018, 0.52],
        [0.78, MAGNET_BOT_REST + 0.018, 0.64],
        act_col,
        [KIND_ACT_MARKER, 0.0, 1.0, 0.0],
    );

    // 7. Moving POM Stem (`+` cross mount, slider body, side guide fins) + cutaway magnet cavity.
    let stem_base = linear_hex(0x60a5fa);
    // Slider core above magnet cavity (`y = -0.04 .. 0.62`)
    push_box(
        &mut verts,
        &mut indices,
        [-0.25, -0.04, -0.26],
        [0.25, 0.62, 0.20],
        stem_base,
        [KIND_STEM, 0.0, 0.0, 0.25],
    );
    // Twin side guide wings riding in bottom housing rails (`x = ±0.26 .. ±0.60`)
    for side in [-1.0_f32, 1.0] {
        let x0 = if side < 0.0 { -0.60 } else { 0.25 };
        let x1 = if side < 0.0 { -0.25 } else { 0.60 };
        push_box(
            &mut verts,
            &mut indices,
            [x0, -0.10, -0.02],
            [x1, 0.42, 0.14],
            stem_base,
            [KIND_STEM, 0.0, 0.0, 0.2],
        );
    }
    // Magnet shroud rear wall (`-Z` only so `+Z` front clearly exposes N/S magnet poles)
    push_box(
        &mut verts,
        &mut indices,
        [-0.18, -0.26, -0.18],
        [0.18, -0.04, -0.12],
        stem_base,
        [KIND_STEM, 0.0, 0.0, 0.2],
    );
    // Authentic MX Cross (`+`) mount sticking up into keycap socket (`y = 0.62 .. 0.98`)
    push_box(
        &mut verts,
        &mut indices,
        [-0.22, 0.62, -0.055],
        [0.22, 0.98, 0.055],
        stem_base,
        [KIND_STEM, 0.0, 0.0, 0.4],
    );
    push_box(
        &mut verts,
        &mut indices,
        [-0.055, 0.62, -0.22],
        [0.055, 0.98, 0.22],
        stem_base,
        [KIND_STEM, 0.0, 0.0, 0.4],
    );

    // 8. Neodymium Magnet (`S` polished chrome top half, `N` crimson lower pole approaching Hall IC).
    let mag_south = linear_hex(0xcbd5e1);
    let mag_north = linear_hex(0xef4444);
    push_box(
        &mut verts,
        &mut indices,
        [-0.13, MAGNET_BOT_REST + 0.10, -0.10],
        [0.13, MAGNET_BOT_REST + 0.20, 0.12],
        mag_south,
        [KIND_MAGNET_SOUTH, 0.0, 0.0, 0.6],
    );
    push_box(
        &mut verts,
        &mut indices,
        [-0.13, MAGNET_BOT_REST, -0.10],
        [0.13, MAGNET_BOT_REST + 0.10, 0.12],
        mag_north,
        [KIND_MAGNET_NORTH, 0.0, 0.0, 0.3],
    );

    // 9. Sculpted cutaway keycap shell (`y = 0.90 .. 1.46`) sleeves outside top housing collar.
    let cap_wall = linear_hex(0x16243a);
    let cap_top = linear_hex(0x22385a);
    // Back skirt
    push_oriented_frustum(
        &mut verts,
        &mut indices,
        [0.0, 0.90, 0.0],
        0.0,
        [0.0, 0.26, -0.82],
        [1.92, 0.52, 0.16],
        [0.86, 0.86],
        [0.0, 0.0, 0.0],
        cap_wall,
        [KIND_KEYCAP, 0.0, 0.0, 0.1],
    );
    // Left cutaway skirt
    push_oriented_frustum(
        &mut verts,
        &mut indices,
        [0.0, 0.90, 0.0],
        0.0,
        [-0.88, 0.26, -0.14],
        [0.16, 0.52, 1.24],
        [0.86, 0.88],
        [0.0, 0.0, 0.0],
        cap_wall,
        [KIND_KEYCAP, 0.0, 0.0, 0.1],
    );
    // Right cutaway skirt
    push_oriented_frustum(
        &mut verts,
        &mut indices,
        [0.0, 0.90, 0.0],
        0.0,
        [0.88, 0.26, -0.14],
        [0.16, 0.52, 1.24],
        [0.86, 0.88],
        [0.0, 0.0, 0.0],
        cap_wall,
        [KIND_KEYCAP, 0.0, 0.0, 0.1],
    );
    // Keycap sculpted crown roof (back/mid slice `z in [-0.78, 0.34]`) so stem tip stays visible from front
    push_box(
        &mut verts,
        &mut indices,
        [-0.82, 1.32, -0.76],
        [0.82, 1.44, 0.34],
        cap_top,
        [KIND_KEYCAP, 0.0, 0.8, 0.4],
    );
    // Cutaway stem boss ring around `+` cross inside keycap ceiling
    push_box(
        &mut verts,
        &mut indices,
        [-0.18, 0.94, -0.18],
        [0.18, 1.32, -0.08],
        cap_wall,
        [KIND_KEYCAP, 0.0, 0.2, 0.2],
    );

    // 10. Dynamic Helical Coil Spring (`7.5` turns around central axis, compressing in shader).
    let spring_gold = linear_hex(0xd8b468);
    let steps = 150usize;
    let turns = 7.5_f32;
    let coil_r = 0.24_f32;
    let wire_r = 0.019_f32;
    let mut spine = Vec::with_capacity(steps + 1);
    let mut radii = Vec::with_capacity(steps + 1);
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let theta = t * turns * TAU;
        let y = SPRING_Y_BOT + (SPRING_Y_TOP_REST - SPRING_Y_BOT) * t;
        spine.push([coil_r * theta.cos(), y, coil_r * theta.sin()]);
        radii.push(wire_r);
    }
    let spring_vert_start = verts.len();
    push_tube(
        &mut verts,
        &mut indices,
        &spine,
        &radii,
        8,
        spring_gold,
        KIND_SPRING,
    );
    for v in &mut verts[spring_vert_start..] {
        v.meta[1] =
            ((v.pos[1] - SPRING_Y_BOT) / (SPRING_Y_TOP_REST - SPRING_Y_BOT)).clamp(0.0, 1.0);
    }

    (verts, indices)
}

const SWITCH_SHADER_WGSL: &str = r#"
struct SwitchUniforms {
    view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
    motion: vec4<f32>,
    stem_color: vec4<f32>,
    led_color: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> u: SwitchUniforms;

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
    let kind = vs_in.part_info.x;
    let stroke = u.motion.x;
    let act_offset = u.motion.y;

    // Actuation indicator rails track exact configured trigger threshold depth.
    if abs(kind - 4.0) < 0.05 {
        local.y -= act_offset;
    }
    // Moving stem, cutaway keycap shell, and neodymium magnet translate rigidly by key travel.
    else if kind >= 4.9 && kind <= 6.5 {
        local.y -= stroke;
    }
    // Helical coil spring compresses linearly between fixed PCB base (t=0) and moving stem (t=1).
    else if kind >= 6.9 {
        local.y -= vs_in.part_info.y * stroke;
    }

    var vs_out: VsOut;
    vs_out.clip = u.view_proj * vec4<f32>(local, 1.0);
    vs_out.world_pos = local;
    vs_out.normal = vs_in.normal;
    vs_out.color = vs_in.color;
    vs_out.part_info = vs_in.part_info;
    return vs_out;
}

@fragment
fn fs_main(frag_in: VsOut) -> @location(0) vec4<f32> {
    let kind = frag_in.part_info.x;
    let n = normalize(frag_in.normal);
    let v = normalize(u.camera_pos.xyz - frag_in.world_pos);

    let key_dir = normalize(vec3<f32>(-0.42, 0.85, 0.68));
    let rim_dir = normalize(vec3<f32>(0.78, 0.42, -0.72));
    let diff = max(dot(n, key_dir), 0.0);
    let rim = max(dot(n, rim_dir), 0.0);
    let half_v = normalize(key_dir + v);
    let spec = pow(max(dot(n, half_v), 0.0), 36.0);
    let fresnel = pow(1.0 - max(dot(n, v), 0.0), 2.5);

    let flux = u.motion.z;
    let triggered = u.motion.w;
    let rapid = u.stem_color.w;

    var lit = vec3<f32>(0.0);
    if abs(kind - 1.5) < 0.05 {
        // Magnetic flux ring around Hall IC: ramps from dim cobalt -> cyan -> emerald on actuation.
        let idle_c = vec3<f32>(0.02, 0.08, 0.16);
        let approach_c = vec3<f32>(0.06, 0.48, 0.82);
        let trig_c = vec3<f32>(0.05, 0.82, 0.56);
        let glow = mix(idle_c, approach_c, smoothstep(0.02, 0.75, flux));
        let active_glow = mix(glow, trig_c, triggered * 0.85);
        lit = active_glow * (0.65 + 0.85 * flux + 0.55 * triggered);
    } else if abs(kind - 1.8) < 0.05 {
        // PCB SMD RGB LED.
        lit = u.led_color.rgb * 1.15 + vec3<f32>(0.12 * diff + 0.15 * spec);
    } else if abs(kind - 4.0) < 0.05 {
        // Actuation plane laser guide fins (sky blue / rapid-trigger violet -> emerald when crossed).
        let guide_idle = mix(vec3<f32>(0.12, 0.55, 0.92), vec3<f32>(0.55, 0.28, 0.95), rapid);
        let guide_trig = vec3<f32>(0.10, 0.86, 0.56);
        let guide_col = mix(guide_idle, guide_trig, triggered);
        lit = guide_col * (0.72 + 0.45 * triggered) + vec3<f32>(0.25 * spec);
    } else if abs(kind - 3.0) < 0.05 {
        // Frosty polycarbonate cutaway top housing with subtle edge Fresnel shimmer.
        let poly = frag_in.color * (0.34 + 0.55 * diff + 0.22 * rim);
        lit = poly + vec3<f32>(0.12, 0.22, 0.36) * fresnel + vec3<f32>(0.28 * spec);
    } else if abs(kind - 5.0) < 0.05 {
        // POM switch stem tinted cleanly by the active switch profile (HM1/HH1/CY1/TC1).
        lit = u.stem_color.rgb * (0.34 + 0.70 * diff + 0.20 * rim) + vec3<f32>(0.18 * spec);
    } else if abs(kind - 5.5) < 0.05 {
        // Sculpted keycap cutaway receiving PCB LED glow from underneath.
        let bounce = u.led_color.rgb * (0.28 * (1.0 - max(n.y, 0.0)));
        lit = frag_in.color * (0.32 + 0.72 * diff + 0.20 * rim) + bounce + vec3<f32>(0.14 * spec);
    } else if kind >= 6.9 {
        // Brushed metallic spring coils.
        lit = frag_in.color * (0.30 + 0.72 * diff + 0.25 * rim) + vec3<f32>(0.42 * spec);
    } else {
        let shine = frag_in.part_info.w;
        lit = frag_in.color * (0.30 + 0.72 * diff + 0.18 * rim) + vec3<f32>((0.10 + 0.25 * shine) * spec);
    }

    let srgb = pow(clamp(lit, vec3<f32>(0.0), vec3<f32>(1.0)), vec3<f32>(1.0 / 2.2));
    return vec4<f32>(srgb, 1.0);
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_complete_switch_cutaway_mesh() {
        let (verts, indices) = build_switch_mesh();
        assert!(verts.len() > 800);
        assert_eq!(indices.len() % 3, 0);
        assert!(verts.iter().all(|v| v.pos.iter().all(|c| c.is_finite())));

        let has_kind = |target: f32| verts.iter().any(|v| (v.meta[0] - target).abs() < 0.05);
        assert!(has_kind(KIND_HALL_IC), "expected Hall IC mesh");
        assert!(has_kind(KIND_MAGNET_NORTH), "expected N magnet pole");
        assert!(has_kind(KIND_STEM), "expected POM cross stem");
        assert!(has_kind(KIND_ACT_MARKER), "expected actuation guide");
        assert!(has_kind(KIND_SPRING), "expected helical spring");

        const {
            assert!((SWITCH_3D_WIDTH * 4).is_multiple_of(256));
        }
    }

    #[test]
    fn renders_offscreen_switch_cutaway_when_gpu_available() {
        let renderer = match SwitchRenderer3D::new() {
            Ok(r) => r,
            Err(err) => {
                assert!(
                    !err.contains("validation"),
                    "switch wgpu shader/pipeline validation failed: {err}"
                );
                return;
            }
        };
        let cam = SwitchCamera::default();
        let scene = SwitchSceneInput {
            switch_id: 1,
            travel_mm: 2.15,
            max_mm: 3.40,
            actuation_mm: 1.70,
            triggered: true,
            rapid_trigger: false,
            led_rgb: [56, 189, 248],
            time_s: 0.4,
        };
        let frame = renderer.render_frame(&cam, &scene).unwrap();
        let bytes = frame.as_bytes(0).unwrap();
        assert_eq!(bytes.len(), (SWITCH_3D_WIDTH * SWITCH_3D_HEIGHT * 4) as usize);
        assert!(bytes.iter().any(|&b| b != 0));

        if let Ok(out_path) = std::env::var("ATRIUM_DUMP_SWITCH_PREVIEW") {
            let mut rgba = bytes.to_vec();
            for px in rgba.chunks_exact_mut(4) {
                px.swap(0, 2);
            }
            let _ = image::save_buffer(
                &out_path,
                &rgba,
                SWITCH_3D_WIDTH,
                SWITCH_3D_HEIGHT,
                image::ColorType::Rgba8,
            );
        }
    }
}
