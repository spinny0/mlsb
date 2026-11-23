use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU32, Ordering},
};

use bytemuck::{Pod, Zeroable};
use eframe::egui_wgpu::{self, wgpu};
use glam::{Mat4, Vec2, Vec3, Vec4};

use crate::{
    collider,
    environment::Environment,
    fluid::MAX_PARTICLES,
    scene::{Kind, Scene, WALL},
    surface::{self, Surface},
};

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const FIELD: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const VERTEX: [wgpu::VertexAttribute; 1] = wgpu::vertex_attr_array![0 => Float32x4];
pub const SHADER: &str = concat!(
    include_str!("liquid.wgsl"),
    "\n",
    include_str!("optics.wgsl")
);

#[derive(Clone, Copy, PartialEq)]
pub struct Camera {
    pub pivot: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub distance: f32,
    pub ortho: bool,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            pivot: Vec3::new(0.0, 1.03, 0.0),
            yaw: 0.29,
            pitch: 0.37,
            distance: 5.65,
            ortho: false,
        }
    }
}

impl Camera {
    pub fn basis(self) -> (Vec3, Vec3, Vec3, Vec3) {
        let offset = Vec3::new(
            self.yaw.sin() * self.pitch.cos(),
            self.pitch.sin(),
            self.yaw.cos() * self.pitch.cos(),
        );
        let forward = -offset;
        let right = Vec3::new(self.yaw.cos(), 0.0, -self.yaw.sin());
        let up = right.cross(forward);
        (self.pivot + offset * self.distance, right, up, forward)
    }

    pub fn projection(self, aspect: f32) -> (Mat4, Mat4) {
        let (eye, _, up, forward) = self.basis();
        let view = Mat4::look_to_rh(eye, forward, up);
        let projection = if self.ortho {
            let half_height = self.distance * 0.37;
            Mat4::orthographic_rh(
                -half_height * aspect,
                half_height * aspect,
                -half_height,
                half_height,
                0.05,
                36.0,
            )
        } else {
            Mat4::perspective_rh(41.0_f32.to_radians(), aspect, 0.05, 36.0)
        };
        let matrix = projection * view;
        (matrix, matrix.inverse())
    }

    pub fn ray(self, uv: Vec2, aspect: f32) -> (Vec3, Vec3) {
        let (_, inverse) = self.projection(aspect);
        let (eye, _, _, forward) = self.basis();
        let ndc = uv * Vec2::new(2.0, -2.0) + Vec2::new(-1.0, 1.0);
        let unproject = |depth| {
            let point = inverse * Vec4::new(ndc.x, ndc.y, depth, 1.0);
            point.truncate() / point.w
        };
        if self.ortho {
            (unproject(0.0), forward)
        } else {
            (eye, (unproject(1.0) - eye).normalize_or_zero())
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ObjectGpu {
    center_kind: [f32; 4],
    half_selected: [f32; 4],
    rotation: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Frame {
    matrix: [[f32; 4]; 4],
    inverse: [[f32; 4]; 4],
    eye: [f32; 4],
    right: [f32; 4],
    up: [f32; 4],
    forward: [f32; 4],
    size_radius: [f32; 4],
    material: [f32; 4],
    settings: [f32; 4],
    light: [f32; 4],
    bounds_min: [f32; 4],
    bounds_max: [f32; 4],
    environment: [f32; 4],
    geometry: [f32; 4],
    clip_plane: [f32; 4],
    objects: [ObjectGpu; 128],
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Display {
    Surface,
    Particles,
    RayTraced,
    Solid,
}

impl Display {
    pub fn label(self) -> &'static str {
        match self {
            Self::Surface => "Surface Preview",
            Self::Particles => "Particles",
            Self::RayTraced => "Ray Traced",
            Self::Solid => "Solid",
        }
    }
}

#[derive(Clone, Copy)]
pub struct ViewSettings {
    pub camera: Camera,
    pub selected: Option<u32>,
    pub mode: Display,
    pub detail: usize,
    pub surface_revision: u64,
}

struct Pipeline {
    pipeline: wgpu::RenderPipeline,
}

impl Pipeline {
    fn new(
        device: &wgpu::Device,
        shader: &wgpu::ShaderModule,
        fragment: &str,
        target: wgpu::TextureFormat,
        particle: bool,
        depth: bool,
    ) -> Self {
        let vertices = wgpu::VertexBufferLayout {
            array_stride: 16,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &VERTEX,
        };
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(fragment),
            layout: None,
            vertex: wgpu::VertexState {
                module: shader,
                entry_point: Some(if particle { "splat" } else { "full_screen" }),
                compilation_options: Default::default(),
                buffers: if particle {
                    std::slice::from_ref(&vertices)
                } else {
                    &[]
                },
            },
            fragment: Some(wgpu::FragmentState {
                module: shader,
                entry_point: Some(fragment),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: if particle {
                    wgpu::PrimitiveTopology::TriangleStrip
                } else {
                    wgpu::PrimitiveTopology::TriangleList
                },
                ..Default::default()
            },
            depth_stencil: depth.then_some(wgpu::DepthStencilState {
                format: DEPTH,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });
        Self { pipeline }
    }

    fn group(
        &self,
        device: &wgpu::Device,
        uniform: &wgpu::Buffer,
        inputs: &[(u32, wgpu::BindingResource<'_>)],
    ) -> wgpu::BindGroup {
        let mut entries = Vec::with_capacity(inputs.len() + 1);
        entries.push(wgpu::BindGroupEntry {
            binding: 0,
            resource: uniform.as_entire_binding(),
        });
        entries.extend(
            inputs
                .iter()
                .map(|(binding, resource)| wgpu::BindGroupEntry {
                    binding: *binding,
                    resource: resource.clone(),
                }),
        );
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.pipeline.get_bind_group_layout(0),
            entries: &entries,
        })
    }

    fn pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        output: &wgpu::TextureView,
        depth: Option<&wgpu::TextureView>,
        group: &wgpu::BindGroup,
        particles: Option<(&wgpu::Buffer, u32)>,
    ) {
        let color = Some(wgpu::RenderPassColorAttachment {
            view: output,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                store: wgpu::StoreOp::Store,
            },
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[color],
            depth_stencil_attachment: depth.map(|view| wgpu::RenderPassDepthStencilAttachment {
                view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Discard,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, group, &[]);
        if let Some((buffer, count)) = particles {
            pass.set_vertex_buffer(0, buffer.slice(..));
            pass.draw(0..4, 0..count);
        } else {
            pass.draw(0..3, 0..1);
        }
    }
}

struct Targets {
    size: [u32; 2],
    _texture: wgpu::Texture,
    output: wgpu::TextureView,
    depth: wgpu::TextureView,
    raw: wgpu::TextureView,
    color: wgpu::TextureView,
    optical: wgpu::BindGroup,
    resolve: wgpu::BindGroup,
    id: eframe::egui::TextureId,
}

pub struct Renderer {
    rs: egui_wgpu::RenderState,
    uniform: wgpu::Buffer,
    particles: wgpu::Buffer,
    raw: Pipeline,
    raw_group: wgpu::BindGroup,
    optical: Pipeline,
    resolve: Pipeline,
    focus: wgpu::ComputePipeline,
    focus_group: wgpu::BindGroup,
    density: wgpu::Texture,
    occupancy: wgpu::Texture,
    density_view: wgpu::TextureView,
    occupancy_view: wgpu::TextureView,
    sampler: wgpu::Sampler,
    caustics: wgpu::Buffer,
    nodes: wgpu::Buffer,
    triangles: wgpu::Buffer,
    mesh_objects: Vec<crate::scene::Object>,
    mesh_selection: Option<u32>,
    node_count: usize,
    environment: Environment,
    surface: Surface,
    revision: Option<u64>,
    colliders: Vec<(u32, Vec3, glam::Quat, Vec3, f32)>,
    surface_tension: f32,
    surface_has_secondary: bool,
    targets: Option<Targets>,
    pending: Arc<AtomicBool>,
    last_frame_ms: Arc<AtomicU32>,
    secondary: wgpu::Buffer,
    pub clip_plane: Option<(Vec3, f32)>,
    pub selection_outline: bool,
}

impl Renderer {
    pub fn new(rs: egui_wgpu::RenderState) -> Self {
        let device = &rs.device;
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Volume surface and light transport"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let buffer = |name, bytes, usage| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(name),
                size: bytes,
                usage: usage | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let uniform = buffer(
            "Viewport parameters",
            std::mem::size_of::<Frame>() as u64,
            wgpu::BufferUsages::UNIFORM,
        );
        let particles = buffer(
            "Debug particles",
            (MAX_PARTICLES * 16) as u64,
            wgpu::BufferUsages::VERTEX,
        );
        let secondary = buffer(
            "Live spray, foam and bubbles",
            (crate::fluid::MAX_WHITEWATER * 16) as u64,
            wgpu::BufferUsages::VERTEX,
        );
        let caustics = buffer(
            "Focused light on receivers",
            128 * 96 * 2 * 4,
            wgpu::BufferUsages::STORAGE,
        );
        let nodes = buffer(
            "Mesh hierarchy",
            (crate::mesh::MAX_TRIANGLES * 2 * 32) as u64,
            wgpu::BufferUsages::STORAGE,
        );
        let triangles = buffer(
            "Edited mesh triangles",
            (crate::mesh::MAX_TRIANGLES * 48) as u64,
            wgpu::BufferUsages::STORAGE,
        );
        let environment = Environment::new(device, &rs.queue);
        let volume = |name, [width, height, depth_or_array_layers]: [u32; 3], format| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(name),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D3,
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            })
        };
        let density = volume(
            "Liquid density and oil fraction",
            surface::SIZE,
            wgpu::TextureFormat::Rgba8Unorm,
        );
        let occupancy = volume(
            "Liquid occupancy blocks",
            surface::BLOCKS,
            wgpu::TextureFormat::R8Unorm,
        );
        let density_view = density.create_view(&Default::default());
        let occupancy_view = occupancy.create_view(&Default::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            min_filter: wgpu::FilterMode::Linear,
            mag_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let raw = Pipeline::new(device, &shader, "closest", FIELD, true, true);
        let raw_group = raw.group(device, &uniform, &[]);
        let optical = Pipeline::new(device, &shader, "optical_view", FORMAT, false, false);
        let resolve = Pipeline::new(device, &shader, "resolve_view", FORMAT, false, false);
        let focus = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Trace light onto floor and glass tank bases"),
            layout: None,
            module: &shader,
            entry_point: Some("focus_light"),
            compilation_options: Default::default(),
            cache: None,
        });
        let focus_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &focus.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&density_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&occupancy_view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: caustics.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: nodes.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 9,
                    resource: triangles.as_entire_binding(),
                },
            ],
        });
        Self {
            rs,
            uniform,
            particles,
            secondary,
            clip_plane: None,
            selection_outline: false,
            raw,
            raw_group,
            optical,
            resolve,
            focus,
            focus_group,
            density,
            occupancy,
            density_view,
            occupancy_view,
            sampler,
            caustics,
            nodes,
            triangles,
            mesh_objects: Vec::new(),
            mesh_selection: None,
            node_count: 0,
            environment,
            surface: Surface::new(),
            revision: None,
            colliders: Vec::new(),
            surface_tension: -1.0,
            surface_has_secondary: false,
            targets: None,
            pending: Arc::new(AtomicBool::new(false)),
            last_frame_ms: Arc::new(AtomicU32::new(0)),
        }
    }

    pub fn ready(&self) -> bool {
        let _ = self.rs.device.poll(wgpu::PollType::Poll);
        !self.pending.load(Ordering::Acquire)
    }

    pub fn last_frame_ms(&self) -> u32 {
        self.last_frame_ms.load(Ordering::Relaxed)
    }

    fn optical_group(&self, raw: &wgpu::TextureView) -> wgpu::BindGroup {
        self.optical.group(
            &self.rs.device,
            &self.uniform,
            &[
                (1, wgpu::BindingResource::TextureView(raw)),
                (2, wgpu::BindingResource::TextureView(&self.density_view)),
                (3, wgpu::BindingResource::TextureView(&self.occupancy_view)),
                (4, wgpu::BindingResource::Sampler(&self.sampler)),
                (5, self.caustics.as_entire_binding()),
                (8, self.nodes.as_entire_binding()),
                (9, self.triangles.as_entire_binding()),
                (
                    10,
                    wgpu::BindingResource::TextureView(&self.environment.view),
                ),
                (
                    11,
                    wgpu::BindingResource::Sampler(&self.environment.sampler),
                ),
            ],
        )
    }

    pub fn poll_environment(&mut self, light: &crate::scene::Lighting) -> bool {
        let (changed, uploaded) = self
            .environment
            .poll(&self.rs.device, &self.rs.queue, light);
        if uploaded && let Some(t) = &self.targets {
            let group = self.optical_group(&t.raw);
            self.targets.as_mut().unwrap().optical = group;
        }
        changed
    }

    pub fn panorama_status(&self) -> &str {
        &self.environment.status
    }
    pub fn panorama_loading(&self) -> bool {
        self.environment.loading()
    }

    pub fn resize(&mut self, size: [u32; 2]) -> bool {
        if self.targets.as_ref().is_some_and(|t| t.size == size) {
            return false;
        }
        if let Some(t) = self.targets.take() {
            self.rs.renderer.write().free_texture(&t.id);
        }
        let device = &self.rs.device;
        let create = |label, format, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: size[0],
                    height: size[1],
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | usage,
                view_formats: &[],
            })
        };
        let sample = wgpu::TextureUsages::TEXTURE_BINDING;
        let texture = create(
            "Liquid display",
            FORMAT,
            sample | wgpu::TextureUsages::COPY_SRC,
        );
        let output = texture.create_view(&Default::default());
        let depth = create("Debug particle depth", DEPTH, wgpu::TextureUsages::empty())
            .create_view(&Default::default());
        let raw = create("Debug particle color and depth", FIELD, sample)
            .create_view(&Default::default());
        let color = create("Optical color before edge filtering", FORMAT, sample)
            .create_view(&Default::default());
        let optical = self.optical_group(&raw);
        let resolve = self.resolve.group(
            device,
            &self.uniform,
            &[
                (4, wgpu::BindingResource::Sampler(&self.sampler)),
                (7, wgpu::BindingResource::TextureView(&color)),
            ],
        );
        let id = self.rs.renderer.write().register_native_texture(
            device,
            &output,
            wgpu::FilterMode::Linear,
        );
        self.targets = Some(Targets {
            size,
            _texture: texture,
            output,
            depth,
            raw,
            color,
            optical,
            resolve,
            id,
        });
        true
    }

    pub fn texture_id(&self) -> Option<eframe::egui::TextureId> {
        self.targets.as_ref().map(|t| t.id)
    }

    #[cfg(test)]
    pub fn output(&self) -> &wgpu::Texture {
        &self.targets.as_ref().unwrap()._texture
    }

    fn rebuild(
        &mut self,
        scene: &Scene,
        particles: &[[f32; 4]],
        secondary: &[[f32; 4]],
        revision: u64,
    ) {
        let colliders = scene
            .objects
            .iter()
            .filter(|o| o.is_collider())
            .map(|o| (o.id, o.position, o.rotation, o.half, o.thickness));
        if self.revision == Some(revision)
            && self.surface_tension == scene.liquid.surface_tension
            && self.surface_has_secondary == !secondary.is_empty()
            && self.colliders.iter().copied().eq(colliders.clone())
        {
            return;
        }
        self.colliders.clear();
        self.colliders.extend(colliders);
        self.revision = Some(revision);
        self.surface_tension = scene.liquid.surface_tension;
        self.surface_has_secondary = !secondary.is_empty();
        self.surface.rebuild_with_foam(scene, particles, secondary);
        let upload = |texture: &wgpu::Texture,
                      [width, height, depth_or_array_layers]: [u32; 3],
                      bytes: &[u8],
                      stride| {
            self.rs.queue.write_texture(
                texture.as_image_copy(),
                bytes,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(width * stride),
                    rows_per_image: Some(height),
                },
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers,
                },
            );
        };
        upload(
            &self.density,
            surface::SIZE,
            bytemuck::cast_slice(&self.surface.density),
            4,
        );
        upload(&self.occupancy, surface::BLOCKS, &self.surface.occupancy, 1);
    }

    #[cfg(test)]
    pub fn render(&mut self, scene: &Scene, particles: &[[f32; 4]], view: ViewSettings) {
        self.render_live(scene, particles, &[], view);
    }

    pub fn render_live(
        &mut self,
        scene: &Scene,
        particles: &[[f32; 4]],
        secondary: &[[f32; 4]],
        view: ViewSettings,
    ) {
        let start = std::time::Instant::now();
        let meshes = scene.objects.iter().filter(|o| {
            o.visible && o.mesh.is_some() && !o.is_domain() && o.flow_behavior().is_none()
        });
        let changed = !meshes.clone().eq(self.mesh_objects.iter());
        if changed || self.mesh_selection != view.selected {
            self.mesh_selection = view.selected;
            self.mesh_objects = meshes.cloned().collect();
            let geometry = collider::Tree::build(
                self.mesh_objects
                    .iter()
                    .enumerate()
                    .filter(|(_, o)| o.visible)
                    .flat_map(|(slot, o)| {
                        collider::triangles(o, view.selected == Some(o.id))
                            .into_iter()
                            .map(move |mut t| {
                                t.glass_id = if o.mesh_glass {
                                    128.0 + slot as f32
                                } else {
                                    0.0
                                };
                                t
                            })
                    })
                    .collect(),
            );
            let nodes: Vec<_> = geometry
                .nodes
                .iter()
                .map(|n| {
                    let a = if n.count == 0 {
                        n.skip as f32
                    } else {
                        -1.0 - n.start as f32
                    };
                    [
                        n.low.extend(a).to_array(),
                        n.high.extend(n.count as f32).to_array(),
                    ]
                })
                .collect();
            let triangles: Vec<_> = geometry
                .triangles
                .iter()
                .map(|t| {
                    [
                        t.a.extend(t.selected).to_array(),
                        t.b.extend(t.glass_id).to_array(),
                        t.c.extend(0.0).to_array(),
                    ]
                })
                .collect();
            self.node_count = nodes.len();
            if !nodes.is_empty() {
                self.rs
                    .queue
                    .write_buffer(&self.nodes, 0, bytemuck::cast_slice(&nodes));
                self.rs
                    .queue
                    .write_buffer(&self.triangles, 0, bytemuck::cast_slice(&triangles));
            }
            if changed {
                self.revision = None;
            }
        }
        if view.mode != Display::Particles {
            self.rebuild(scene, particles, secondary, view.surface_revision);
        }
        let Some(target) = &self.targets else {
            return;
        };
        let camera = view.camera;
        let [width, height] = target.size;
        let (matrix, inverse) = camera.projection(width as f32 / height as f32);
        let (eye, right, up, forward) = camera.basis();
        let mut frame = Frame::zeroed();
        frame.matrix = matrix.to_cols_array_2d();
        frame.inverse = inverse.to_cols_array_2d();
        frame.eye = eye.extend(f32::from(camera.ortho)).to_array();
        frame.right = right.extend(0.0).to_array();
        frame.up = up.extend(0.0).to_array();
        frame.forward = forward.extend(0.0).to_array();
        frame.size_radius = [width as f32, height as f32, 0.024, 0.0];
        if let Some((normal, offset)) = self.clip_plane {
            frame.clip_plane = normal.extend(offset).to_array();
        }
        let draw_secondary =
            !secondary.is_empty() && matches!(view.mode, Display::Surface | Display::RayTraced);
        frame.size_radius[3] = f32::from(draw_secondary);
        frame.material = Vec3::from_array(scene.liquid.tint)
            .extend(scene.liquid.absorption)
            .to_array();
        let lighting = &scene.lighting;
        frame.geometry[2] = f32::from(
            lighting.environment == crate::scene::EnvironmentMode::Studio
                || view.mode == Display::Solid,
        );
        frame.environment = [
            f32::from(self.environment.active(lighting)),
            lighting.environment_rotation.to_radians(),
            lighting.environment_exposure,
            f32::from(lighting.environment_background),
        ];
        frame.geometry[0] = self.node_count as f32;
        frame.geometry[1] = f32::from(self.surface.has_oil);
        frame.geometry[3] = f32::from(self.selection_outline && view.selected.is_some());
        let a = lighting.azimuth.to_radians();
        let e = lighting.elevation.to_radians();
        let default_direction = Vec3::new(a.sin() * e.cos(), e.sin(), a.cos() * e.cos());
        let direction = scene
            .objects
            .iter()
            .find(|o| o.kind == Kind::Light && o.enabled && scene.active_light == Some(o.id))
            .map_or(default_direction, |o| {
                let mut direction = o.rotation * Vec3::Y;
                direction.y = direction.y.max(0.12);
                direction.normalize_or_zero()
            });
        frame.light = direction.extend(lighting.power).to_array();
        let caustics =
            lighting.caustics && view.mode == Display::RayTraced && !particles.is_empty();
        frame.bounds_min = self
            .surface
            .min
            .extend(f32::from(!particles.is_empty()))
            .to_array();
        frame.bounds_max = self.surface.max.extend(f32::from(caustics)).to_array();
        let mut count = 0;
        for o in scene.objects.iter().filter(|o| {
            o.enabled && o.visible && !o.is_domain() && o.fluid != crate::scene::FluidRole::Flow
        }) {
            let selected = f32::from(view.selected == Some(o.id));
            let mut write = |center: Vec3, half: Vec3, kind| {
                frame.objects[count] = ObjectGpu {
                    center_kind: center.extend(kind).to_array(),
                    half_selected: half.extend(selected).to_array(),
                    rotation: o.rotation.to_array(),
                };
                count += 1;
            };
            match o.kind {
                Kind::Tank => o.geometry_boxes(|local, half| {
                    let base = half.y <= WALL * 0.5 + 0.001;
                    write(
                        o.position + o.rotation * local,
                        half,
                        if base { 3.0 } else { 4.0 },
                    );
                }),
                Kind::Box if o.mesh.is_none() => write(o.position, o.half, 1.0),
                Kind::Box => {}
                Kind::Emitter => write(
                    o.position + o.rotation * Vec3::Y * 0.045,
                    Vec3::new(o.radius * 1.08, 0.065, o.radius * 1.08),
                    2.0,
                ),
                Kind::Drain => write(
                    o.position - o.rotation * Vec3::Y * 0.07,
                    Vec3::new(o.radius, 0.025, o.radius),
                    2.0,
                ),
                Kind::Camera | Kind::Light | Kind::Field => {}
            }
        }
        if lighting.checker {
            frame.objects[count] = ObjectGpu {
                center_kind: [0.0, 1.2, -1.51, 6.0],
                half_selected: [2.03, 1.19, 0.024, 0.0],
                rotation: glam::Quat::IDENTITY.to_array(),
            };
            count += 1;
        }
        frame.settings = [
            count as f32,
            match view.mode {
                Display::Surface => 0.0,
                Display::Particles => 1.0,
                Display::RayTraced => 2.0,
                Display::Solid => 3.0,
            },
            [0.041, 0.026, 0.017][view.detail],
            scene.liquid.ior,
        ];
        let queue = &self.rs.queue;
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&frame));
        let mut encoder = self.rs.device.create_command_encoder(&Default::default());
        if caustics {
            encoder.clear_buffer(&self.caustics, 0, None);
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&self.focus);
            pass.set_bind_group(0, &self.focus_group, &[]);
            pass.dispatch_workgroups(32, 24, 1);
        }
        if view.mode == Display::Particles {
            let slice = &particles[..particles.len().min(MAX_PARTICLES)];
            if !slice.is_empty() {
                queue.write_buffer(&self.particles, 0, bytemuck::cast_slice(slice));
            }
            self.raw.pass(
                &mut encoder,
                &target.raw,
                Some(&target.depth),
                &self.raw_group,
                Some((&self.particles, slice.len() as u32)),
            );
        } else if draw_secondary {
            let slice = &secondary[..secondary.len().min(crate::fluid::MAX_WHITEWATER)];
            queue.write_buffer(&self.secondary, 0, bytemuck::cast_slice(slice));
            self.raw.pass(
                &mut encoder,
                &target.raw,
                Some(&target.depth),
                &self.raw_group,
                Some((&self.secondary, slice.len() as u32)),
            );
        }
        self.optical
            .pass(&mut encoder, &target.color, None, &target.optical, None);
        self.resolve
            .pass(&mut encoder, &target.output, None, &target.resolve, None);
        self.pending.store(true, Ordering::Release);
        queue.submit([encoder.finish()]);
        let pending = self.pending.clone();
        let duration = self.last_frame_ms.clone();
        queue.on_submitted_work_done(move || {
            duration.store(
                start.elapsed().as_millis().min(1000) as u32,
                Ordering::Relaxed,
            );
            pending.store(false, Ordering::Release);
        });
    }
}
