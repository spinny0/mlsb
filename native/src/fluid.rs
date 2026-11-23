use std::collections::BTreeMap;

use glam::{Quat, Vec3};

use crate::collider::MeshCollider;
use crate::scene::{FieldKind, FlowMode, Kind, Liquid, Scene, Whitewater};

pub const ORIGIN: Vec3 = Vec3::new(-2.0, 0.0, -1.4);
pub const END: Vec3 = Vec3::new(2.0, 2.8, 1.4);
pub const DT: f32 = 1.0 / 90.0;
pub const MAX_PARTICLES: usize = 28_000;
pub const MAX_WHITEWATER: usize = 4096;
const H: f32 = 0.1;
const SPACING: f32 = 0.051;
const PARTICLE_VOLUME: f32 = SPACING * SPACING * SPACING;
const CONTACT: f32 = 0.019;
const GRID: [usize; 3] = [40, 28, 28];
const STRIDE: [usize; 3] = [1, GRID[0] + 1, (GRID[0] + 1) * (GRID[1] + 1)];
const CELLS: usize = STRIDE[2] * (GRID[2] + 1);
const OPEN: i16 = -1;
const DOMAIN: i16 = -2;
const DIRECTIONS: [Vec3; 3] = [Vec3::X, Vec3::Y, Vec3::Z];

#[derive(Clone, Copy, Debug)]
pub struct Quality {
    pub capacity: usize,
    pub pressure_iterations: usize,
}

pub const QUALITIES: [Quality; 3] = [
    Quality {
        capacity: 10_000,
        pressure_iterations: 14,
    },
    Quality {
        capacity: 18_000,
        pressure_iterations: 23,
    },
    Quality {
        capacity: MAX_PARTICLES,
        pressure_iterations: 36,
    },
];

#[derive(Clone, Copy)]
struct Particle {
    position: Vec3,
    velocity: Vec3,
    affine: [Vec3; 3],
    oil: bool,
}

impl Particle {
    fn new(position: Vec3, velocity: Vec3) -> Self {
        Self {
            position,
            velocity,
            affine: [Vec3::ZERO; 3],
            oil: false,
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
struct BoxShape {
    id: u32,
    center: Vec3,
    rotation: Quat,
    half: Vec3,
    bound: Vec3,
}

impl BoxShape {
    fn local(self, p: Vec3) -> Vec3 {
        self.rotation.conjugate() * (p - self.center)
    }

    fn contains(self, p: Vec3) -> bool {
        (p - self.center).abs().cmple(self.bound).all()
            && self.local(p).abs().cmple(self.half).all()
    }
}

#[derive(Clone, Copy, Default)]
struct Motion {
    linear: Vec3,
    angular: Vec3,
    center: Vec3,
    wetting: f32,
}

struct Stencil {
    first: usize,
    frac: Vec3,
}

impl Stencil {
    fn new(p: Vec3, axis: usize) -> Self {
        let offset = Vec3::splat(0.5) - DIRECTIONS[axis] * 0.5;
        Self::at(p, offset)
    }

    fn at(p: Vec3, offset: Vec3) -> Self {
        let g = ((p - ORIGIN) / H - offset).clamp(
            Vec3::ZERO,
            Vec3::new(GRID[0] as f32, GRID[1] as f32, GRID[2] as f32) - Vec3::splat(0.001),
        );
        let lo = g.floor();
        Self {
            first: index(lo.x as usize, lo.y as usize, lo.z as usize),
            frac: g - lo,
        }
    }

    fn each(&self, mut f: impl FnMut(usize, f32, Vec3, Vec3)) {
        let s = self.frac;
        let wx = [1.0 - s.x, s.x];
        let wy = [1.0 - s.y, s.y];
        let wz = [1.0 - s.z, s.z];
        for z in 0..2 {
            for y in 0..2 {
                for x in 0..2 {
                    let weight = wx[x] * wy[y] * wz[z];
                    let delta = (Vec3::new(x as f32, y as f32, z as f32) - s) * H;
                    let gradient = Vec3::new(
                        (x as f32 * 2.0 - 1.0) * wy[y] * wz[z],
                        (y as f32 * 2.0 - 1.0) * wx[x] * wz[z],
                        (z as f32 * 2.0 - 1.0) * wx[x] * wy[y],
                    ) / H;
                    f(
                        self.first + x + y * STRIDE[1] + z * STRIDE[2],
                        weight,
                        delta,
                        gradient,
                    );
                }
            }
        }
    }
}

fn index(x: usize, y: usize, z: usize) -> usize {
    x + y * STRIDE[1] + z * STRIDE[2]
}

fn cell_coordinates(p: Vec3) -> [usize; 3] {
    let g = ((p - ORIGIN) / H).floor().max(Vec3::ZERO);
    [
        (g.x as usize).min(GRID[0] - 1),
        (g.y as usize).min(GRID[1] - 1),
        (g.z as usize).min(GRID[2] - 1),
    ]
}

#[derive(Clone, Copy)]
struct Face {
    index: usize,
    left: Option<usize>,
    right: Option<usize>,
    position: Vec3,
}

struct Grid {
    faces: [Vec<Face>; 3],
    boundary: Vec<[i16; 3]>,
    solid: Vec<i16>,
    density: Vec<u16>,
    volume: Vec<f32>,
    oil: Vec<f32>,
    mobility: Vec<[f32; 3]>,
    layered: bool,
    velocity: Vec<[f32; 3]>,
    weight: Vec<[f32; 3]>,
    valid: Vec<[bool; 3]>,
    next_valid: Vec<[bool; 3]>,
    scratch: Vec<[f32; 3]>,
    pressure: Vec<f32>,
    active: Vec<(usize, u8, f32, f32)>,
}

impl Grid {
    fn new() -> Self {
        let faces = std::array::from_fn(|axis| {
            let mut faces = Vec::new();
            for z in 0..GRID[2] + usize::from(axis == 2) {
                for y in 0..GRID[1] + usize::from(axis == 1) {
                    for x in 0..GRID[0] + usize::from(axis == 0) {
                        let i = index(x, y, z);
                        let c = [x, y, z][axis];
                        faces.push(Face {
                            index: i,
                            left: (c != 0).then(|| i - STRIDE[axis]),
                            right: (c < GRID[axis]).then_some(i),
                            position: ORIGIN
                                + (Vec3::new(x as f32, y as f32, z as f32) + Vec3::splat(0.5)
                                    - DIRECTIONS[axis] * 0.5)
                                    * H,
                        });
                    }
                }
            }
            faces
        });
        Self {
            faces,
            boundary: vec![[OPEN; 3]; CELLS],
            solid: vec![OPEN; CELLS],
            density: vec![0; CELLS],
            volume: vec![0.0; CELLS],
            oil: vec![0.0; CELLS],
            mobility: vec![[1.0; 3]; CELLS],
            layered: false,
            velocity: vec![[0.0; 3]; CELLS],
            weight: vec![[0.0; 3]; CELLS],
            valid: vec![[false; 3]; CELLS],
            next_valid: vec![[false; 3]; CELLS],
            scratch: vec![[0.0; 3]; CELLS],
            pressure: vec![0.0; CELLS],
            active: Vec::new(),
        }
    }

    fn mask(&mut self, boxes: &[BoxShape], meshes: &[MeshCollider], bounds: (Vec3, Vec3)) {
        for z in 0..GRID[2] {
            for y in 0..GRID[1] {
                for x in 0..GRID[0] {
                    let p =
                        ORIGIN + (Vec3::new(x as f32, y as f32, z as f32) + Vec3::splat(0.5)) * H;
                    self.solid[index(x, y, z)] =
                        if p.cmplt(bounds.0).any() || p.cmpgt(bounds.1).any() {
                            DOMAIN
                        } else {
                            boxes
                                .iter()
                                .position(|b| b.contains(p))
                                .or_else(|| {
                                    meshes
                                        .iter()
                                        .position(|m| m.solid(p))
                                        .map(|i| i + boxes.len())
                                })
                                .map_or(OPEN, |i| i as i16)
                        };
                }
            }
        }
        for axis in 0..3 {
            for face in &self.faces[axis] {
                let left = face.left.map_or(DOMAIN, |i| self.solid[i]);
                let right = face.right.map_or(DOMAIN, |i| self.solid[i]);
                self.boundary[face.index][axis] = if left == DOMAIN || right == DOMAIN {
                    DOMAIN
                } else if left != OPEN {
                    left
                } else {
                    right
                };
            }
        }
    }

    fn scatter(&mut self, particles: &[Particle], particle_volume: f32) {
        self.density.fill(0);
        self.volume.fill(0.0);
        self.oil.fill(0.0);
        self.layered = particles.iter().any(|p| p.oil);
        self.velocity.fill([0.0; 3]);
        self.weight.fill([0.0; 3]);
        for p in particles {
            let [x, y, z] = cell_coordinates(p.position);
            let i = index(x, y, z);
            if self.solid[i] == OPEN {
                self.density[i] = self.density[i].saturating_add(1);
            }
            let volume = Stencil::at(p.position, Vec3::splat(0.5));
            let mut open = 0.0;
            volume.each(|i, w, _, _| {
                if self.solid[i] == OPEN {
                    open += w;
                }
            });
            let scale = particle_volume / (H * H * H * open.max(0.001));
            volume.each(|i, w, _, _| {
                if self.solid[i] == OPEN {
                    self.volume[i] += w * scale;
                    if p.oil {
                        self.oil[i] += w * scale;
                    }
                }
            });
            for axis in 0..3 {
                let stencil = Stencil::new(p.position, axis);
                stencil.each(|i, w, delta, _| {
                    self.velocity[i][axis] += w * (p.velocity[axis] + p.affine[axis].dot(delta));
                    self.weight[i][axis] += w;
                });
            }
        }
    }

    fn forces(&mut self, motions: &[Motion], liquid: Liquid, gravity: f32) {
        self.valid.fill([false; 3]);
        if self.layered {
            self.mobility.fill([1.0; 3]);
        }
        for axis in 0..3 {
            for f in &self.faces[axis] {
                let i = f.index;
                let owner = self.boundary[i][axis];
                let weight = self.weight[i][axis];
                if owner != OPEN {
                    let velocity = if owner == DOMAIN {
                        0.0
                    } else {
                        let m = motions[owner as usize];
                        (m.linear + m.angular.cross(f.position - m.center))[axis]
                    };
                    self.velocity[i][axis] = velocity;
                    self.valid[i][axis] = true;
                } else if weight > 0.00001 {
                    let left = f.left.map_or(0, |j| self.density[j]);
                    let right = f.right.map_or(0, |j| self.density[j]);
                    let (a, b) = (f.left.unwrap(), f.right.unwrap());
                    let volume = self.volume[a] + self.volume[b];
                    let oil = if self.layered {
                        ((self.oil[a] + self.oil[b]) / volume.max(0.001)).min(1.0)
                    } else {
                        0.0
                    };
                    if self.layered {
                        self.mobility[i][axis] = (1.0 - 0.17 * oil).recip();
                    }
                    let surface_pull = if left == 0 || right == 0 {
                        (f32::from(right) - f32::from(left)).clamp(-8.0, 8.0)
                            * liquid.cohesion
                            * 0.4
                            + (self.volume[b].min(1.0) - self.volume[a].min(1.0))
                                * liquid.surface_tension
                                * 1.8
                    } else {
                        0.0
                    };
                    self.velocity[i][axis] = (self.velocity[i][axis] / weight
                        + (surface_pull - if axis == 1 { gravity } else { 0.0 }) * DT)
                        .clamp(-9.0, 9.0);
                    self.valid[i][axis] = true;
                }
            }
        }
        if liquid.viscosity < 0.001 && !self.layered {
            return;
        }
        self.scratch.copy_from_slice(&self.velocity);
        for axis in 0..3 {
            for face in &self.faces[axis] {
                let i = face.index;
                if self.boundary[i][axis] != OPEN || !self.valid[i][axis] {
                    continue;
                }
                let mut sum = 0.0;
                let mut count = 0;
                for j in nearby(i).into_iter().flatten() {
                    if self.valid[j][axis] {
                        sum += self.scratch[j][axis];
                        count += 1;
                    }
                }
                if count > 0 {
                    let (a, b) = (face.left.unwrap(), face.right.unwrap());
                    let oil = if self.layered {
                        ((self.oil[a] + self.oil[b]) / (self.volume[a] + self.volume[b]).max(0.001))
                            .min(1.0)
                    } else {
                        0.0
                    };
                    let viscosity =
                        liquid.viscosity + (Liquid::OIL.viscosity - liquid.viscosity) * oil;
                    self.velocity[i][axis] += (sum / count as f32 - self.scratch[i][axis])
                        * viscosity
                        * (0.43 + 0.35 * viscosity.powi(3));
                }
            }
        }
    }

    fn project(&mut self, iterations: usize) {
        self.active.clear();
        for (i, &count) in self.density.iter().enumerate() {
            if count == 0 {
                self.pressure[i] = 0.0;
                continue;
            }
            let mut neighbors = 0u8;
            let mut div = 0.0;
            let mut diagonal = 0.0;
            for (axis, &s) in STRIDE.iter().enumerate() {
                if self.boundary[i][axis] == OPEN {
                    neighbors |= 1 << (axis * 2);
                    diagonal += if self.layered {
                        self.mobility[i][axis]
                    } else {
                        1.0
                    };
                }
                if self.boundary[i + s][axis] == OPEN {
                    neighbors |= 2 << (axis * 2);
                    diagonal += if self.layered {
                        self.mobility[i + s][axis]
                    } else {
                        1.0
                    };
                }
                div += self.velocity[i + s][axis] - self.velocity[i][axis];
            }
            if neighbors != 0 {
                // Velocity projection alone leaves density errors from previous
                // advection steps behind. Expand overfull cells toward their
                // rest volume, without drawing free surfaces inward.
                let crowding = (self.volume[i] - 1.08).max(0.0);
                div -= (crowding * H / DT * 0.29).min(0.34);
                self.active.push((i, neighbors, div, diagonal.recip()));
            } else {
                self.pressure[i] = 0.0;
            }
        }
        for _ in 0..iterations {
            for &(i, mask, div, inv) in &self.active {
                let mut sum = 0.0;
                for (axis, &s) in STRIDE.iter().enumerate() {
                    if mask & (1 << (2 * axis)) != 0 {
                        sum += self.pressure[i - s]
                            * if self.layered {
                                self.mobility[i][axis]
                            } else {
                                1.0
                            };
                    }
                    if mask & (2 << (2 * axis)) != 0 {
                        sum += self.pressure[i + s]
                            * if self.layered {
                                self.mobility[i + s][axis]
                            } else {
                                1.0
                            };
                    }
                }
                let q = &mut self.pressure[i];
                *q += 1.67 * ((sum - div) * inv - *q);
            }
        }
        for axis in 0..3 {
            for face in &self.faces[axis] {
                let i = face.index;
                if self.boundary[i][axis] != OPEN {
                    continue;
                }
                let a = face.left.unwrap();
                let b = face.right.unwrap();
                if self.density[a] > 0 || self.density[b] > 0 {
                    let mobility = if self.layered {
                        self.mobility[i][axis]
                    } else {
                        1.0
                    };
                    self.velocity[i][axis] -= (self.pressure[b] - self.pressure[a]) * mobility;
                    self.valid[i][axis] = true;
                }
            }
        }
        self.extrapolate();
    }

    fn extrapolate(&mut self) {
        for _ in 0..2 {
            self.next_valid.copy_from_slice(&self.valid);
            for axis in 0..3 {
                for face in &self.faces[axis] {
                    let i = face.index;
                    if self.valid[i][axis] {
                        continue;
                    }
                    let mut sum = 0.0;
                    let mut count = 0;
                    for j in nearby(i).into_iter().flatten() {
                        if self.valid[j][axis] && self.boundary[j][axis] == OPEN {
                            sum += self.velocity[j][axis];
                            count += 1;
                        }
                    }
                    if count > 0 {
                        self.velocity[i][axis] = sum / count as f32;
                        self.next_valid[i][axis] = true;
                    }
                }
            }
            std::mem::swap(&mut self.valid, &mut self.next_valid);
        }
    }

    fn sample(&self, position: Vec3) -> (Vec3, [Vec3; 3]) {
        let mut velocity = Vec3::ZERO;
        let mut affine = [Vec3::ZERO; 3];
        for axis in 0..3 {
            Stencil::new(position, axis).each(|i, w, _, gradient| {
                let v = self.velocity[i][axis];
                velocity[axis] += w * v;
                affine[axis] += gradient * v;
            });
            affine[axis] = affine[axis].clamp_length_max(42.0);
        }
        (velocity.clamp_length_max(7.8), affine)
    }
}

fn nearby(i: usize) -> [Option<usize>; 6] {
    let x = i % STRIDE[1];
    let y = (i % STRIDE[2]) / STRIDE[1];
    let z = i / STRIDE[2];
    [
        (x > 0).then(|| i - 1),
        (x < GRID[0]).then(|| i + 1),
        (y > 0).then(|| i - STRIDE[1]),
        (y < GRID[1]).then(|| i + STRIDE[1]),
        (z > 0).then(|| i - STRIDE[2]),
        (z < GRID[2]).then(|| i + STRIDE[2]),
    ]
}

pub struct Fluid {
    grid: Grid,
    particles: Vec<Particle>,
    boxes: Vec<BoxShape>,
    meshes: Vec<MeshCollider>,
    motions: Vec<Motion>,
    previous: BTreeMap<u32, (Vec3, Quat)>,
    flow_state: BTreeMap<u32, FlowState>,
    seed: u32,
    secondary_seed: u32,
    pub time: f32,
    pub emitted: u64,
    pub drained: u64,
    pub particle_volume: f32,
    pub ticks: u32,
    pub initial_liters: f32,
    bounds: (Vec3, Vec3),
    whitewater: Vec<SecondaryParticle>,
    whitewater_cursor: usize,
    whitewater_budget: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SecondaryKind {
    Spray,
    Foam,
    Bubble,
}

#[derive(Clone, Copy)]
struct SecondaryParticle {
    kind: SecondaryKind,
    position: Vec3,
    velocity: Vec3,
    life: f32,
}

#[derive(Default)]
struct FlowState {
    fraction: f32,
    passed: u64,
    pending_drop: usize,
}

impl Fluid {
    pub fn new(scene: &Scene, quality: Quality) -> Self {
        let mut fluid = Self {
            grid: Grid::new(),
            particles: Vec::with_capacity(MAX_PARTICLES),
            boxes: Vec::new(),
            meshes: Vec::new(),
            motions: Vec::new(),
            previous: BTreeMap::new(),
            flow_state: BTreeMap::new(),
            seed: scene.seed.max(1),
            secondary_seed: (scene.seed ^ 0x9e3779b9).max(1),
            time: 0.0,
            emitted: 0,
            drained: 0,
            particle_volume: PARTICLE_VOLUME,
            ticks: 0,
            initial_liters: 0.0,
            bounds: (ORIGIN, END),
            whitewater: Vec::with_capacity(MAX_WHITEWATER),
            whitewater_cursor: 0,
            whitewater_budget: 0.0,
        };
        fluid.update_boxes(scene);
        if let Some(fill) = scene.initial_fill() {
            let extent = fill.max - fill.min;
            let volume = extent.element_product();
            let spacing = SPACING.max((volume / (quality.capacity as f32 * 0.92)).cbrt());
            let mut count = (extent / spacing).round().as_uvec3().max(glam::UVec3::ONE);
            while count.x * count.y * count.z > quality.capacity as u32 {
                let axis = count.max_element();
                if count.x == axis {
                    count.x -= 1;
                } else if count.y == axis {
                    count.y -= 1;
                } else {
                    count.z -= 1;
                }
            }
            let cell = extent / count.as_vec3();
            fluid.particle_volume = volume / (count.x * count.y * count.z) as f32;
            for z in 0..count.z {
                for y in 0..count.y {
                    for x in 0..count.x {
                        let offset = Vec3::new(fluid.random(), fluid.random(), fluid.random())
                            - Vec3::splat(0.5);
                        let p = fill.min
                            + (Vec3::new(x as f32, y as f32, z as f32) + 0.5 + offset * 0.24)
                                * cell;
                        if !fluid.boxes.iter().any(|b| b.contains(p))
                            && !fluid.meshes.iter().any(|m| m.solid(p))
                            && p.cmpge(fluid.bounds.0 + Vec3::splat(CONTACT)).all()
                            && p.cmple(fluid.bounds.1 - Vec3::splat(CONTACT)).all()
                        {
                            fluid.particles.push(Particle::new(p, Vec3::ZERO));
                        }
                    }
                }
            }
        }
        for object in scene
            .objects
            .iter()
            .filter(|o| o.flow_behavior() == Some(FlowMode::Geometry))
        {
            fluid.seed_geometry(object, quality.capacity);
        }
        fluid.initial_liters = fluid.particles.len() as f32 * fluid.particle_volume * 1000.0;
        fluid
    }

    fn seed_geometry(&mut self, object: &crate::scene::Object, capacity: usize) {
        let spacing = self.particle_volume.cbrt().max(0.012);
        let mesh = object.mesh.as_ref().map(|_| MeshCollider::new(object));
        if mesh.as_ref().is_some_and(|m| !m.closed) {
            return;
        }
        let (lo, hi) = mesh
            .as_ref()
            .and_then(|m| m.tree.bounds())
            .unwrap_or_else(|| {
                let basis = glam::Mat3::from_quat(object.rotation);
                let half = basis.x_axis.abs() * object.half.x
                    + basis.y_axis.abs() * object.half.y
                    + basis.z_axis.abs() * object.half.z;
                (object.position - half, object.position + half)
            });
        let lo = lo.max(self.bounds.0 + Vec3::splat(CONTACT));
        let hi = hi.min(self.bounds.1 - Vec3::splat(CONTACT));
        let counts = ((hi - lo) / spacing).floor().max(Vec3::ZERO).as_uvec3();
        let inverse = object.rotation.conjugate();
        for z in 0..counts.z {
            for y in 0..counts.y {
                for x in 0..counts.x {
                    if self.particles.len() >= capacity {
                        return;
                    }
                    let p = lo + (Vec3::new(x as f32, y as f32, z as f32) + 0.5) * spacing;
                    let in_source = mesh.as_ref().map_or_else(
                        || {
                            (inverse * (p - object.position))
                                .abs()
                                .cmple(object.half)
                                .all()
                        },
                        |m| m.solid(p),
                    );
                    if in_source
                        && !self.boxes.iter().any(|b| b.contains(p))
                        && !self.meshes.iter().any(|m| m.solid(p))
                    {
                        let mut particle = Particle::new(p, Vec3::ZERO);
                        particle.oil = object.oil;
                        self.particles.push(particle);
                    }
                }
            }
        }
    }

    pub fn secondary_snapshot(&self) -> Vec<[f32; 4]> {
        self.whitewater
            .iter()
            .map(|p| {
                let kind = match p.kind {
                    SecondaryKind::Spray => 1.0,
                    SecondaryKind::Foam => 2.0,
                    SecondaryKind::Bubble => 3.0,
                };
                p.position
                    .extend(kind + 0.75 * (p.life / 0.25).clamp(0.02, 1.0))
                    .to_array()
            })
            .collect()
    }

    pub fn snapshot(&self) -> Vec<[f32; 4]> {
        self.particles
            .iter()
            .map(|p| {
                [
                    p.position.x,
                    p.position.y,
                    p.position.z,
                    if p.oil {
                        -p.velocity.length().max(0.00001)
                    } else {
                        p.velocity.length()
                    },
                ]
            })
            .collect()
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.particles.len()
    }

    fn random(&mut self) -> f32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 17;
        self.seed ^= self.seed << 5;
        (self.seed >> 8) as f32 / 16_777_216.0
    }

    fn update_boxes(&mut self, scene: &Scene) {
        let bounds = scene.domain_bounds();
        let bounds_changed = bounds != self.bounds;
        self.bounds = bounds;
        let mut next = Vec::with_capacity(self.boxes.len());
        let mut meshes = Vec::new();
        self.motions.clear();
        for object in &scene.objects {
            if scene.gate == Some(object.id) && scene.gate_is_open(self.ticks) {
                continue;
            }
            let q = object.rotation;
            let matrix = glam::Mat3::from_quat(q);
            let (last, last_rotation) = self
                .previous
                .get(&object.id)
                .copied()
                .unwrap_or((object.position, q));
            let mut delta = q * last_rotation.conjugate();
            if delta.w < 0.0 {
                delta = -delta;
            }
            let (axis, angle) = delta.to_axis_angle();
            let angular = (axis * (angle / DT)).clamp_length_max(9.0);
            let linear = ((object.position - last) / DT).clamp_length_max(4.5);
            if object.is_collider() && object.mesh.is_some() {
                meshes.push((
                    object,
                    Motion {
                        linear,
                        angular,
                        center: object.position,
                        wetting: object.wetting,
                    },
                ));
                continue;
            }
            object.boxes(|offset, half| {
                let offset_world = q * offset;
                let shape = BoxShape {
                    id: object.id,
                    center: object.position + offset_world,
                    rotation: q,
                    half,
                    bound: matrix.x_axis.abs() * half.x
                        + matrix.y_axis.abs() * half.y
                        + matrix.z_axis.abs() * half.z,
                };
                next.push(shape);
                self.motions.push(Motion {
                    linear: linear + angular.cross(offset_world),
                    angular,
                    center: shape.center,
                    wetting: object.wetting,
                });
            });
        }
        let meshes_changed = meshes.len() != self.meshes.len()
            || !self
                .meshes
                .iter()
                .zip(&meshes)
                .all(|(old, (object, _))| old.same(object));
        if meshes_changed {
            self.meshes = meshes.iter().map(|(o, _)| MeshCollider::new(o)).collect();
        }
        self.motions.extend(meshes.iter().map(|(_, m)| *m));
        if self.time == 0.0 || next != self.boxes || meshes_changed || bounds_changed {
            self.grid.mask(&next, &self.meshes, bounds);
            self.boxes = next;
        }
        self.previous.clear();
        self.previous.extend(
            scene
                .objects
                .iter()
                .map(|o| (o.id, (o.position, o.rotation))),
        );
    }

    fn emit(&mut self, scene: &Scene, capacity: usize) {
        for emitter in scene
            .objects
            .iter()
            .filter(|o| o.flow_behavior() == Some(FlowMode::Inflow))
        {
            let rate = emitter.schedule.strength(self.ticks);
            if rate <= 0.0 {
                continue;
            }
            let mut count = self.flow_count(emitter, rate, true);
            if emitter.drip {
                let state = self.flow_state.entry(emitter.id).or_default();
                state.pending_drop += count;
                let drop_size = (4.0 + emitter.radius * 36.0).round() as usize;
                count = (state.pending_drop / drop_size) * drop_size;
                if let Some(liters) = emitter.schedule.liters {
                    let remaining = ((liters * 0.001 / self.particle_volume).floor() as usize)
                        .saturating_sub(state.passed as usize);
                    if remaining < drop_size && state.pending_drop >= remaining {
                        count = remaining;
                    }
                    count = count.min(remaining);
                }
                state.pending_drop -= count;
            }
            let count = count.min(capacity.saturating_sub(self.particles.len()));
            let before = self.emitted;
            for _ in 0..count {
                let r =
                    self.random().sqrt() * emitter.radius * if emitter.drip { 0.82 } else { 1.0 };
                let phase = self.random() * std::f32::consts::TAU;
                let trail = self.random()
                    * if emitter.drip {
                        emitter.radius * 0.65
                    } else {
                        emitter.speed * DT
                    };
                let local = Vec3::new(r * phase.cos(), -0.055 - trail, r * phase.sin());
                let p = emitter.position + emitter.rotation * local;
                if p.cmpge(self.bounds.0 + Vec3::splat(CONTACT)).all()
                    && p.cmple(self.bounds.1 - Vec3::splat(CONTACT)).all()
                    && !self.boxes.iter().any(|b| b.contains(p))
                    && !self.meshes.iter().any(|m| m.solid(p))
                {
                    let mut particle =
                        Particle::new(p, emitter.rotation * Vec3::NEG_Y * emitter.speed);
                    particle.oil = emitter.oil;
                    self.particles.push(particle);
                    self.emitted += 1;
                }
            }
            self.flow_state.entry(emitter.id).or_default().passed += self.emitted - before;
        }
    }

    fn flow_count(&mut self, object: &crate::scene::Object, strength: f32, limited: bool) -> usize {
        let state = self.flow_state.entry(object.id).or_default();
        let limit = object.schedule.liters.map_or(usize::MAX, |liters| {
            ((liters * 0.001 / self.particle_volume).floor() as usize)
                .saturating_sub(state.passed as usize)
        });
        if !limited {
            return limit;
        }
        state.fraction += object.flow * strength * 0.001 / self.particle_volume * DT;
        let count = state.fraction.floor() as usize;
        state.fraction -= count as f32;
        limit.min(count)
    }

    pub fn step(&mut self, scene: &Scene, quality: Quality) {
        let evaluated = scene.at_frame(self.ticks.saturating_add(1));
        let scene = &*evaluated;
        self.update_boxes(scene);
        let inside = |p: Vec3| {
            p.cmpge(self.bounds.0 + Vec3::splat(CONTACT)).all()
                && p.cmple(self.bounds.1 - Vec3::splat(CONTACT)).all()
        };
        let fields: Vec<_> = scene
            .objects
            .iter()
            .filter(|o| o.enabled && o.kind == Kind::Field && o.field.strength.abs() > 0.001)
            .map(|o| (o.position, o.rotation * Vec3::Y, o.field))
            .collect();
        let lo = self.bounds.0 + Vec3::splat(CONTACT);
        let hi = self.bounds.1 - Vec3::splat(CONTACT);
        for particle in &mut self.particles {
            if !inside(particle.position) {
                particle.position = particle.position.clamp(lo, hi);
            }
            if !fields.is_empty() {
                let mut force = Vec3::ZERO;
                for &(center, axis, field) in &fields {
                    let delta = particle.position - center;
                    let radius_fraction = delta.length() / field.radius;
                    if radius_fraction >= 1.0 {
                        continue;
                    }
                    let direction = match field.kind {
                        FieldKind::Directional => axis,
                        FieldKind::Vortex => {
                            axis.cross(delta).normalize_or_zero() - delta.normalize_or_zero() * 0.22
                        }
                        FieldKind::Radial => delta.normalize_or_zero(),
                    };
                    force += direction * field.strength * (1.0 - radius_fraction).powi(2);
                }
                particle.velocity += force.clamp_length_max(60.0) * DT;
            }
        }
        self.emit(scene, quality.capacity);
        self.grid.scatter(&self.particles, self.particle_volume);
        self.grid.forces(&self.motions, scene.liquid, scene.gravity);
        self.grid.project(quality.pressure_iterations);

        for particle in &mut self.particles {
            let previous = particle.position;
            let [x, y, z] = cell_coordinates(previous);
            let i = index(x, y, z);
            let fill = self.grid.volume[i];
            let contact = ContactLiquid {
                material: if particle.oil {
                    Liquid::OIL
                } else {
                    scene.liquid
                },
                film_gravity: scene.gravity * (1.0 - 1.7 * fill).max(0.0),
            };
            let (v, affine) = self.grid.sample(particle.position);
            let midpoint = particle.position + v * (DT * 0.5);
            let (advect, _) = self.grid.sample(midpoint);
            particle.velocity = v;
            particle.affine = affine;
            particle.position += advect * DT;
            if self.grid.layered {
                let oil = (self.grid.oil[i] / fill.max(0.001)).min(1.0);
                // Retain the relative rise of lighter parcels at a mixed grid cell.
                // The phase-weighted correction sums to zero within a mixed volume.
                let relative = if particle.oil { 1.0 - oil } else { -oil };
                let lift = scene.gravity * 0.17 * relative * fill.min(1.0);
                particle.velocity.y += lift * DT;
                particle.position.y += lift * DT * DT * 0.5;
            }
            collide(particle, previous, &self.boxes, &self.motions, contact);
            collide_meshes(
                particle,
                previous,
                &self.meshes,
                &self.motions[self.boxes.len()..],
                contact,
            );
            let bounded = particle.position.clamp(lo, hi);
            for axis in 0..3 {
                if (bounded[axis] - particle.position[axis]) * particle.velocity[axis] < 0.0 {
                    particle.velocity[axis] = 0.0;
                    particle.affine[axis] = Vec3::ZERO;
                }
            }
            particle.position = bounded;
        }
        let before = self.particles.len();
        for drain in scene
            .objects
            .iter()
            .filter(|o| o.flow_behavior() == Some(FlowMode::Outflow))
        {
            let strength = drain.schedule.strength(self.ticks);
            if strength <= 0.0 {
                continue;
            }
            let max = self.flow_count(
                drain,
                strength,
                drain.schedule.limited_drain || drain.schedule.ramp > 0.0,
            );
            let mut removed = 0;
            let inverse = drain.rotation.conjugate();
            self.particles.retain(|p| {
                let q = inverse * (p.position - drain.position);
                if removed < max
                    && q.y.abs() <= drain.radius * 0.55
                    && q.x * q.x + q.z * q.z <= drain.radius.powi(2)
                {
                    removed += 1;
                    false
                } else {
                    true
                }
            });
            self.flow_state.entry(drain.id).or_default().passed += removed as u64;
        }
        self.drained += (before - self.particles.len()) as u64;
        self.advance_whitewater(scene.whitewater, scene.gravity, scene.liquid.viscosity);
        self.ticks = self.ticks.saturating_add(1);
        self.time = self.ticks as f32 * DT;
    }

    fn advance_whitewater(&mut self, settings: Whitewater, gravity: f32, viscosity: f32) {
        if settings.amount <= 0.0 || !(settings.spray || settings.foam || settings.bubbles) {
            self.whitewater.clear();
            self.whitewater_budget = 0.0;
            return;
        }
        let grid = &self.grid;
        let bounds = self.bounds;
        self.whitewater.retain_mut(|p| {
            p.life -= DT;
            let [x, y, z] = cell_coordinates(p.position);
            let fill = grid.volume[index(x, y, z)];
            let flow = grid.sample(p.position).0;
            match p.kind {
                SecondaryKind::Spray => {
                    p.velocity.y -= gravity * DT;
                    if p.velocity.y < 0.0 && fill > 0.49 {
                        if !settings.foam {
                            return false;
                        }
                        p.kind = SecondaryKind::Foam;
                        p.life = p.life.max(0.60 * settings.lifetime);
                    }
                }
                SecondaryKind::Foam => {
                    let lift = ((fill - 0.42) * 0.75).clamp(-0.40, 0.38);
                    p.velocity = p.velocity.lerp(flow * 0.66 + Vec3::Y * lift, 0.21);
                    if fill < 0.12 {
                        p.life -= 7.0 * DT;
                    }
                }
                SecondaryKind::Bubble => {
                    p.velocity = p.velocity.lerp(flow * 0.65 + Vec3::Y * 0.34, 0.19);
                    if fill < 0.20 {
                        if !settings.foam {
                            return false;
                        }
                        p.kind = SecondaryKind::Foam;
                        p.life = p.life.min(0.75 * settings.lifetime);
                    }
                }
            }
            let enabled = match p.kind {
                SecondaryKind::Spray => settings.spray,
                SecondaryKind::Foam => settings.foam,
                SecondaryKind::Bubble => settings.bubbles,
            };
            p.position += p.velocity * DT;
            enabled
                && p.life > 0.0
                && p.position.cmpge(bounds.0).all()
                && p.position.cmple(bounds.1).all()
                && !self.boxes.iter().any(|b| b.contains(p.position))
                && !self.meshes.iter().any(|m| m.solid(p.position))
        });
        if settings.amount <= 0.0
            || self.particles.is_empty()
            || !(settings.spray || settings.foam || settings.bubbles)
        {
            return;
        }
        self.whitewater_budget =
            (self.whitewater_budget + settings.amount * (1.0 - viscosity).powi(2) * 8.0).min(22.0);
        let remaining = MAX_WHITEWATER.saturating_sub(self.whitewater.len());
        let budget = (self.whitewater_budget as usize).min(remaining);
        if budget == 0 {
            return;
        }
        let len = self.particles.len();
        let mut spawned = 0;
        for _ in 0..len.min(360) {
            let index_in_fluid = self.whitewater_cursor % len;
            self.whitewater_cursor = self.whitewater_cursor.wrapping_add(1);
            let p = self.particles[index_in_fluid];
            let [x, y, z] = cell_coordinates(p.position);
            let i = index(x, y, z);
            let fill = self.grid.volume[i];
            let speed = p.velocity.length();
            if speed < 1.05 || fill < 0.24 {
                continue;
            }
            let below = y > 0 && self.grid.volume[i - STRIDE[1]] > 0.30;
            let above = y + 1 < GRID[1] && self.grid.volume[i + STRIDE[1]] > 0.31;
            let roll = unit_random(&mut self.secondary_seed);
            let lateral = Vec3::new(
                unit_random(&mut self.secondary_seed) - 0.5,
                0.0,
                unit_random(&mut self.secondary_seed) - 0.5,
            );
            let swirl = p
                .affine
                .iter()
                .map(|row| row.length_squared())
                .sum::<f32>()
                .sqrt();
            let surface = below && !above;
            let [u, v, w] = p.affine;
            let rotation = Vec3::new(w.y - v.z, u.z - w.x, v.x - u.y).length();
            let impact = rotation > 3.2 && swirl > 4.0 && (speed > 1.45 || p.velocity.y < -1.0);
            let (kind, velocity, life) = if settings.spray && surface && impact && roll < 0.10 {
                (
                    SecondaryKind::Spray,
                    p.velocity * 0.44 + lateral * 1.8 + Vec3::Y * (0.65 + speed * 0.38),
                    0.65,
                )
            } else if settings.foam && surface && impact && roll < 0.29 {
                (SecondaryKind::Foam, p.velocity * 0.63 + lateral * 0.18, 1.2)
            } else if settings.bubbles && above && fill > 0.50 && impact && roll < 0.13 {
                (
                    SecondaryKind::Bubble,
                    p.velocity * 0.58 + Vec3::Y * 0.2,
                    1.6,
                )
            } else {
                continue;
            };
            let life = life * settings.lifetime * (0.65 + unit_random(&mut self.secondary_seed));
            self.whitewater.push(SecondaryParticle {
                kind,
                position: p.position + lateral * 0.038,
                velocity,
                life,
            });
            spawned += 1;
            if spawned >= budget {
                break;
            }
        }
        self.whitewater_budget -= spawned as f32;
    }
}

fn unit_random(seed: &mut u32) -> f32 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 17;
    *seed ^= *seed << 5;
    (*seed >> 8) as f32 / 16_777_216.0
}

#[derive(Clone, Copy)]
struct ContactLiquid {
    material: Liquid,
    film_gravity: f32,
}

fn wet_contact(
    p: &mut Particle,
    motion: &Motion,
    point: Vec3,
    normal: Vec3,
    gap: f32,
    liquid: ContactLiquid,
) {
    if gap > 0.085 {
        return;
    }
    let viscosity = liquid.material.viscosity;
    if gap <= 0.012 && normal.y > 0.12 && normal.y < 0.995 {
        let slope = Vec3::NEG_Y + normal * normal.y;
        let slide = liquid.film_gravity * (1.0 - motion.wetting) * (1.0 - viscosity * 0.91);
        p.velocity += slope * (slide * DT);
        p.position += slope * (slide * DT * DT * 0.48);
    }
    if motion.wetting <= 0.0 {
        return;
    }
    let wall = motion.linear + motion.angular.cross(point - motion.center);
    let v = p.velocity - wall;
    let outward = v.dot(normal);
    let strength = motion.wetting
        * (1.0 - gap.max(0.0) / 0.085)
        * (1.0 - outward.max(0.0) / (1.2 + 2.8 * viscosity)).max(0.0);
    if strength <= 0.0 {
        return;
    }
    let tangent = v - normal * outward;
    let drag = (strength * (0.15 + 0.60 * viscosity)).min(0.88);
    let pull = strength * (4.5 + 23.0 * viscosity + 2.0 * liquid.material.surface_tension);
    p.velocity = wall + tangent * (1.0 - drag) + normal * (outward - DT * pull);
    p.position -= normal * (DT * DT * pull * 0.48).min(gap.max(0.0));
    for row in &mut p.affine {
        *row *= 1.0 - drag;
    }
}

fn collide_meshes(
    p: &mut Particle,
    previous: Vec3,
    meshes: &[MeshCollider],
    motions: &[Motion],
    liquid: ContactLiquid,
) {
    for (mesh, motion) in meshes.iter().zip(motions) {
        let shell = if mesh.closed {
            CONTACT
        } else {
            CONTACT + mesh.object.thickness * 0.5
        };
        let start =
            previous + (motion.linear + motion.angular.cross(previous - motion.center)) * DT;
        let travel = p.position - start;
        let distance = travel.length();
        let wet_reach = if motion.wetting > 0.0 { 0.085 } else { 0.0 };
        if !mesh.near(p.position, shell + distance + wet_reach) {
            continue;
        }
        let mut collision = None;
        if distance > 0.00001 {
            let dir = travel / distance;
            if let Some((t, n)) = mesh.tree.hit(start, dir, distance + 0.0001) {
                let normal = if n.dot(dir) < 0.0 { n } else { -n };
                let remaining = dir * (distance - t).max(0.0);
                let tangent = remaining - normal * remaining.dot(normal);
                collision = Some((start + dir * t + tangent, normal));
            }
        }
        if collision.is_none() {
            let inside = mesh.closed && mesh.solid(p.position);
            let reach = if inside { 10.0 } else { shell };
            if let Some((point, n)) = mesh.tree.nearest(p.position, reach) {
                let delta = (p.position - point).normalize_or_zero();
                let normal = if inside {
                    n
                } else if delta.length_squared() > 0.5 {
                    delta
                } else {
                    if n.dot(previous - point) >= 0.0 {
                        n
                    } else {
                        -n
                    }
                };
                collision = Some((point, normal));
            }
        }
        if let Some((point, normal)) = collision {
            p.position = (point + normal * (shell + 0.0002))
                .clamp(ORIGIN + Vec3::splat(CONTACT), END - Vec3::splat(CONTACT));
            let wall = motion.linear + motion.angular.cross(point - motion.center);
            let relative = p.velocity - wall;
            p.velocity = wall + (relative - normal * relative.dot(normal).min(0.0)) * 0.985;
            p.affine = [Vec3::ZERO; 3];
            wet_contact(p, motion, point, normal, 0.0, liquid);
        } else if wet_reach > 0.0
            && let Some((point, _)) = mesh.tree.nearest(p.position, shell + wet_reach)
        {
            let delta = p.position - point;
            wet_contact(
                p,
                motion,
                point,
                delta.normalize_or_zero(),
                delta.length() - shell,
                liquid,
            );
        }
    }
}

fn swept_box(start: Vec3, end: Vec3, half: Vec3) -> Option<(Vec3, Vec3)> {
    if start.abs().cmple(half).all() {
        return None;
    }
    let d = end - start;
    let inverse = d.map(|v| 1.0 / v.abs().max(0.000001).copysign(v));
    let near = (-half - start) * inverse;
    let far = (half - start) * inverse;
    let enter = near.min(far);
    let t = enter.max_element();
    if !(0.0..=1.0).contains(&t) || near.max(far).min_element() < t {
        return None;
    }
    let axis = if enter.x == t {
        0
    } else if enter.y == t {
        1
    } else {
        2
    };
    let n = DIRECTIONS[axis] * -d[axis].signum();
    Some((start + d * t, n))
}

fn collide(
    p: &mut Particle,
    previous: Vec3,
    boxes: &[BoxShape],
    motion: &[Motion],
    liquid: ContactLiquid,
) {
    for iteration in 0..2 {
        let bounds = p
            .position
            .clamp(ORIGIN + Vec3::splat(CONTACT), END - Vec3::splat(CONTACT));
        for axis in 0..3 {
            let correction = bounds[axis] - p.position[axis];
            if correction != 0.0 && p.velocity[axis] * correction < 0.0 {
                p.velocity[axis] = 0.0;
                p.affine[axis] = Vec3::ZERO;
            }
        }
        p.position = bounds;
        for (b, motion) in boxes.iter().zip(motion) {
            let wall_at_start = motion.linear + motion.angular.cross(previous - b.center);
            let start = previous + wall_at_start * DT;
            let sweep = (p.position - start).abs();
            let wet_range = if motion.wetting > 0.0 && iteration == 0 {
                0.085
            } else {
                0.0
            };
            if (p.position - b.center)
                .abs()
                .cmpgt(
                    b.bound
                        + Vec3::splat(CONTACT * 1.8 + wet_range)
                        + if iteration == 0 { sweep } else { Vec3::ZERO },
                )
                .any()
            {
                continue;
            }
            let local = b.local(p.position);
            if iteration == 0
                && let Some((point, n)) =
                    swept_box(b.local(start), local, b.half + Vec3::splat(CONTACT))
            {
                let normal = b.rotation * n;
                let remaining = local - point;
                let tangent = remaining - n * remaining.dot(n);
                p.position = b.center + b.rotation * (point + tangent + n * 0.0002);
                let wall = motion.linear + motion.angular.cross(p.position - b.center);
                let v = p.velocity - wall;
                p.velocity = wall + (v - normal * v.dot(normal).min(0.0)) * 0.985;
                p.affine = [Vec3::ZERO; 3];
                wet_contact(p, motion, p.position, normal, 0.0, liquid);
                continue;
            }
            let distance = b.half + Vec3::splat(CONTACT) - local.abs();
            if distance.min_element() <= 0.0 {
                if wet_range > 0.0 {
                    let point = local.clamp(-b.half, b.half);
                    let delta = local - point;
                    wet_contact(
                        p,
                        motion,
                        b.center + b.rotation * point,
                        b.rotation * delta.normalize_or_zero(),
                        delta.length() - CONTACT,
                        liquid,
                    );
                }
                continue;
            }
            let axis = if distance.x <= distance.y && distance.x <= distance.z {
                0
            } else if distance.y <= distance.z {
                1
            } else {
                2
            };
            let normal =
                b.rotation * DIRECTIONS[axis] * if local[axis] >= 0.0 { 1.0 } else { -1.0 };
            p.position += normal * (distance[axis] + 0.0001);
            let wall = motion.linear + motion.angular.cross(p.position - b.center);
            let v = p.velocity - wall;
            p.velocity = wall + (v - normal * v.dot(normal).min(0.0)) * 0.985;
            p.affine = [Vec3::ZERO; 3];
            if iteration == 0 {
                wet_contact(p, motion, p.position, normal, 0.0, liquid);
            }
        }
    }
    p.position = p
        .position
        .clamp(ORIGIN + Vec3::splat(CONTACT), END - Vec3::splat(CONTACT));
}
