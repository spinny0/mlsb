use glam::{IVec3, Mat3, Vec3};

use crate::scene::Scene;

pub const SIZE: [u32; 3] = [96, 68, 68];
pub const BLOCKS: [u32; 3] = [24, 17, 17];
pub const START: Vec3 = Vec3::new(-2.16, -0.12, -1.53);
pub const CELL: f32 = 0.045;
pub const ISO: f32 = 0.30;
const SUPPORT: f32 = 0.117;
const LAST: IVec3 = IVec3::new(SIZE[0] as i32 - 1, SIZE[1] as i32 - 1, SIZE[2] as i32 - 1);

pub struct Surface {
    values: Vec<f32>,
    oil: Vec<f32>,
    pub density: Vec<[u8; 4]>,
    pub has_oil: bool,
    pub occupancy: Vec<u8>,
    pub min: Vec3,
    pub max: Vec3,
}

fn index(x: i32, y: i32, z: i32) -> usize {
    (x + SIZE[0] as i32 * (y + SIZE[1] as i32 * z)) as usize
}

fn lattice(p: Vec3) -> Vec3 {
    (p - START) / CELL - 0.5
}

fn grid_bounds(p: Vec3, radius: Vec3) -> (IVec3, IVec3) {
    (
        lattice(p - radius).floor().as_ivec3().max(IVec3::ZERO),
        lattice(p + radius).ceil().as_ivec3().min(LAST),
    )
}

impl Surface {
    pub fn new() -> Self {
        let count = SIZE.into_iter().product::<u32>() as usize;
        Self {
            values: vec![0.0; count],
            oil: vec![0.0; count],
            density: vec![[0; 4]; count],
            has_oil: false,
            occupancy: vec![0; BLOCKS.into_iter().product::<u32>() as usize],
            min: Vec3::ZERO,
            max: Vec3::ZERO,
        }
    }

    #[cfg(test)]
    pub fn rebuild(&mut self, scene: &Scene, points: &[[f32; 4]]) {
        self.rebuild_with_foam(scene, points, &[]);
    }

    pub fn rebuild_with_foam(
        &mut self,
        scene: &Scene,
        points: &[[f32; 4]],
        secondary: &[[f32; 4]],
    ) {
        self.values.fill(0.0);
        self.oil.fill(0.0);
        self.has_oil = points.iter().any(|p| p[3] < 0.0);
        self.occupancy.fill(0);
        let mut low = Vec3::splat(f32::INFINITY);
        let mut high = Vec3::splat(f32::NEG_INFINITY);
        for p in points {
            let oil = p[3] < 0.0;
            let p = Vec3::from_slice(&p[..3]);
            low = low.min(p);
            high = high.max(p);
            let (a, b) = grid_bounds(p, Vec3::splat(SUPPORT));
            let c = lattice(p);
            for z in a.z..=b.z {
                let dz = (z as f32 - c.z) * (CELL / SUPPORT);
                for y in a.y..=b.y {
                    let dy = (y as f32 - c.y) * (CELL / SUPPORT);
                    let base = 1.0 - dy * dy - dz * dz;
                    if base <= 0.0 {
                        continue;
                    }
                    for x in a.x..=b.x {
                        let dx = (x as f32 - c.x) * (CELL / SUPPORT);
                        let w = (base - dx * dx).max(0.0);
                        let i = index(x, y, z);
                        let weight = 0.143 * w * w * w;
                        self.values[i] += weight;
                        if oil {
                            self.oil[i] += weight;
                        }
                    }
                }
            }
        }
        // Remove density inside solid material. This keeps a wet ramp and the
        // inside of a tank from becoming part of the liquid's optical volume.
        for object in &scene.objects {
            if object.is_collider() && object.mesh.is_some() {
                let mesh = crate::collider::MeshCollider::new(object);
                if let Some((low, high)) = mesh.tree.bounds() {
                    let (a, b) = grid_bounds(
                        (low + high) * 0.5,
                        (high - low) * 0.5 + Vec3::splat(object.thickness),
                    );
                    for z in a.z..=b.z {
                        for y in a.y..=b.y {
                            for x in a.x..=b.x {
                                let i = index(x, y, z);
                                if self.values[i] <= 0.0 {
                                    continue;
                                }
                                let world =
                                    START + (Vec3::new(x as f32, y as f32, z as f32) + 0.5) * CELL;
                                if mesh.solid(world) {
                                    self.values[i] = 0.0;
                                    self.oil[i] = 0.0;
                                }
                            }
                        }
                    }
                }
                continue;
            }
            let q = object.rotation;
            let m = Mat3::from_quat(q);
            object.boxes(|local, half| {
                let center = object.position + q * local;
                let extent =
                    m.x_axis.abs() * half.x + m.y_axis.abs() * half.y + m.z_axis.abs() * half.z;
                let (a, b) = grid_bounds(center, extent + Vec3::splat(0.01));
                let inverse = q.conjugate();
                for z in a.z..=b.z {
                    for y in a.y..=b.y {
                        for x in a.x..=b.x {
                            let i = index(x, y, z);
                            if self.values[i] <= 0.0 {
                                continue;
                            }
                            let world =
                                START + (Vec3::new(x as f32, y as f32, z as f32) + 0.5) * CELL;
                            let clearance =
                                ((inverse * (world - center)).abs() - half).max_element();
                            let t = ((clearance + 0.021) / 0.025).clamp(0.0, 1.0);
                            let clearance = t * t * (3.0 - 2.0 * t);
                            self.values[i] *= clearance;
                            self.oil[i] *= clearance;
                        }
                    }
                }
            });
        }
        for (i, (&value, bytes)) in self.values.iter().zip(&mut self.density).enumerate() {
            let oil = self.oil[i] / value.max(0.0001);
            let boost = 1.0 + 0.85 * scene.liquid.surface_tension * (1.0 - (value / 0.46).min(1.0));
            let value = value * boost;
            *bytes = [
                (value.min(1.0) * 255.0).round() as u8,
                (oil.min(1.0) * 255.0).round() as u8,
                0,
                0,
            ];
            if value < ISO - 0.008 {
                continue;
            }
            let plane = SIZE[0] as usize * SIZE[1] as usize;
            let x = (i % SIZE[0] as usize) as i32;
            let y = ((i % plane) / SIZE[0] as usize) as i32;
            let z = (i / plane) as i32;
            // A voxel influences samples half a cell across a block boundary.
            for bz in (z - 1).max(0) / 4..=(z + 1).min(LAST.z) / 4 {
                for by in (y - 1).max(0) / 4..=(y + 1).min(LAST.y) / 4 {
                    for bx in (x - 1).max(0) / 4..=(x + 1).min(LAST.x) / 4 {
                        self.occupancy
                            [(bx + BLOCKS[0] as i32 * (by + BLOCKS[1] as i32 * bz)) as usize] = 255;
                    }
                }
            }
        }
        let radius = 0.092;
        for particle in secondary.iter().filter(|p| (2.0..3.0).contains(&p[3])) {
            let point = Vec3::from_slice(&particle[..3]);
            let (low, high) = grid_bounds(point, Vec3::splat(radius));
            let center = lattice(point);
            let opacity = ((particle[3] - 2.0) / 0.75).min(1.0);
            for z in low.z..=high.z {
                for y in low.y..=high.y {
                    for x in low.x..=high.x {
                        let voxel = &mut self.density[index(x, y, z)];
                        if voxel[0] < 22 {
                            continue;
                        }
                        let distance = (Vec3::new(x as f32, y as f32, z as f32) - center)
                            .length_squared()
                            * (CELL / radius).powi(2);
                        let coverage = ((1.0 - distance).max(0.0).powi(2) * opacity * 39.0) as u8;
                        voxel[2] = voxel[2].saturating_add(coverage);
                    }
                }
            }
        }
        if points.is_empty() {
            self.min = Vec3::ZERO;
            self.max = Vec3::ZERO;
        } else {
            self.min = low - Vec3::splat(SUPPORT);
            self.max = high + Vec3::splat(SUPPORT);
        }
    }
}
