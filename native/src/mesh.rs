use std::collections::{BTreeMap, BTreeSet, VecDeque};

use glam::Vec3;
use serde::{Deserialize, Serialize};

pub const MAX_VERTICES: usize = 2048;
pub const MAX_FACES: usize = 2048;
pub const MAX_TRIANGLES: usize = 4096;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Mesh {
    pub points: Vec<Vec3>,
    pub faces: Vec<Vec<u32>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Primitive {
    Plane,
    Cube,
    Cylinder,
    Sphere,
    Ramp,
    Trough,
    Cup,
    Spoon,
}

impl Primitive {
    pub const ALL: [Self; 8] = [
        Self::Plane,
        Self::Cube,
        Self::Cylinder,
        Self::Sphere,
        Self::Ramp,
        Self::Trough,
        Self::Cup,
        Self::Spoon,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Plane => "Plane",
            Self::Cube => "Cube",
            Self::Cylinder => "Cylinder",
            Self::Sphere => "Sphere",
            Self::Ramp => "Ramp",
            Self::Trough => "Trough",
            Self::Cup => "Cup",
            Self::Spoon => "Spoon",
        }
    }

    pub fn half(self) -> Vec3 {
        match self {
            Self::Plane => Vec3::new(0.63, 0.18, 0.48),
            Self::Ramp => Vec3::new(0.66, 0.29, 0.43),
            Self::Trough => Vec3::new(0.80, 0.25, 0.46),
            Self::Spoon => Vec3::new(0.75, 0.16, 0.36),
            _ => Vec3::splat(0.43),
        }
    }

    pub fn geometry(self) -> Mesh {
        let p = |x, y, z| Vec3::new(x, y, z);
        let mut mesh = match self {
            Self::Plane => Mesh {
                points: vec![
                    p(-1., 0., -1.),
                    p(-1., 0., 1.),
                    p(1., 0., 1.),
                    p(1., 0., -1.),
                ],
                faces: vec![vec![0, 1, 2, 3]],
            },
            Self::Cube => Mesh {
                points: vec![
                    p(-1., -1., -1.),
                    p(1., -1., -1.),
                    p(1., -1., 1.),
                    p(-1., -1., 1.),
                    p(-1., 1., -1.),
                    p(1., 1., -1.),
                    p(1., 1., 1.),
                    p(-1., 1., 1.),
                ],
                faces: vec![
                    vec![0, 1, 2, 3],
                    vec![7, 6, 5, 4],
                    vec![0, 4, 5, 1],
                    vec![1, 5, 6, 2],
                    vec![2, 6, 7, 3],
                    vec![3, 7, 4, 0],
                ],
            },
            Self::Ramp => Mesh {
                points: vec![
                    p(-1., -1., -1.),
                    p(1., -1., -1.),
                    p(1., -1., 1.),
                    p(-1., -1., 1.),
                    p(-1., 1., -1.),
                    p(-1., 1., 1.),
                ],
                faces: vec![
                    vec![0, 1, 2, 3],
                    vec![0, 4, 1],
                    vec![1, 4, 5, 2],
                    vec![2, 5, 3],
                    vec![3, 5, 4, 0],
                ],
            },
            Self::Trough => Mesh {
                points: vec![
                    p(-1., 0.7, -1.),
                    p(-1., -0.4, -0.72),
                    p(-1., -0.4, 0.72),
                    p(-1., 0.7, 1.),
                    p(1., 0.45, -1.),
                    p(1., -0.65, -0.72),
                    p(1., -0.65, 0.72),
                    p(1., 0.45, 1.),
                ],
                faces: vec![vec![0, 4, 5, 1], vec![1, 5, 6, 2], vec![2, 6, 7, 3]],
            },
            Self::Cylinder => lathe(&[(-1.0, 1.0), (1.0, 1.0)], 24),
            Self::Cup => lathe(&[(-1.0, 0.81), (1.0, 1.0), (1.0, 0.82), (-0.75, 0.63)], 24),
            Self::Spoon => {
                let mut points = vec![p(-0.34, -0.56, 0.0)];
                let mut faces = Vec::new();
                let n = 24u32;
                for ring in 1..=3 {
                    let r = ring as f32 / 3.0;
                    for i in 0..n {
                        let angle = (i as f32 - 0.5) * std::f32::consts::TAU / n as f32;
                        points.push(p(
                            -0.34 + r * 0.64 * angle.cos(),
                            -0.56 + 0.61 * r * r,
                            r * 0.9 * angle.sin(),
                        ));
                    }
                }
                for i in 0..n {
                    let j = (i + 1) % n;
                    faces.push(vec![0, j + 1, i + 1]);
                    for ring in 0..2 {
                        let a = 1 + ring * n;
                        faces.push(vec![a + i, a + j, a + n + j, a + n + i]);
                    }
                }
                let end = points.len() as u32;
                points.extend([p(1.2, 0.22, -0.11), p(1.2, 0.22, 0.11)]);
                faces.push(vec![2 * n + 1, 2 * n + 2, end + 1, end]);
                Mesh { points, faces }
            }
            Self::Sphere => {
                let rings: Vec<_> = (1..12)
                    .map(|i| {
                        let angle = i as f32 * std::f32::consts::PI / 12.0;
                        (-angle.cos(), angle.sin())
                    })
                    .collect();
                let mut m = lathe(&rings, 24);
                let bottom = m.points.len() as u32;
                m.points.extend([Vec3::NEG_Y, Vec3::Y]);
                let last = m.faces.pop().unwrap();
                let first = m.faces.pop().unwrap();
                for (face, pole) in [(first, bottom), (last, bottom + 1)] {
                    for (a, b) in face.iter().zip(face.iter().cycle().skip(1)) {
                        m.faces.push(vec![*a, *b, pole]);
                    }
                }
                m
            }
        };
        mesh.recalculate();
        mesh
    }
}

fn lathe(rings: &[(f32, f32)], n: u32) -> Mesh {
    let mut mesh = Mesh {
        points: Vec::new(),
        faces: Vec::new(),
    };
    for &(y, r) in rings {
        for i in 0..n {
            let a = i as f32 * std::f32::consts::TAU / n as f32;
            mesh.points.push(Vec3::new(a.cos() * r, y, a.sin() * r));
        }
    }
    for ring in 0..rings.len() as u32 - 1 {
        for i in 0..n {
            let j = (i + 1) % n;
            mesh.faces.push(vec![
                ring * n + i,
                (ring + 1) * n + i,
                (ring + 1) * n + j,
                ring * n + j,
            ]);
        }
    }
    mesh.faces.push((0..n).rev().collect());
    let last = (rings.len() as u32 - 1) * n;
    mesh.faces.push((last..last + n).collect());
    mesh
}

pub fn edge(a: u32, b: u32) -> (u32, u32) {
    (a.min(b), a.max(b))
}

pub fn sides(face: &[u32]) -> impl Iterator<Item = (u32, u32)> + '_ {
    face.iter()
        .copied()
        .zip(face.iter().copied().cycle().skip(1))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CutSide {
    Both,
    Positive,
    Negative,
}

type Edge = (u32, u32);

pub struct LoopRing {
    strips: BTreeMap<usize, (Edge, Edge)>,
    edges: BTreeMap<Edge, Edge>,
}

impl LoopRing {
    pub fn segments(&self, mesh: &Mesh, count: usize, slide: f32) -> Vec<[Vec3; 2]> {
        let mut lines = Vec::new();
        for &(a, b) in self.strips.values() {
            for i in 1..=count {
                let t = loop_fraction(i, count, slide);
                lines.push(
                    [a, b].map(|(u, v)| mesh.points[u as usize].lerp(mesh.points[v as usize], t)),
                );
            }
        }
        lines
    }
}

fn loop_fraction(i: usize, count: usize, slide: f32) -> f32 {
    (i as f32 + slide.clamp(-0.95, 0.95)) / (count + 1) as f32
}

impl Mesh {
    pub fn tank_shell(half: Vec3) -> Self {
        let wall = crate::scene::WALL;
        let w = Vec3::new(
            (1.0 - wall / half.x).max(0.12),
            -1.0 + wall / half.y,
            (1.0 - wall / half.z).max(0.12),
        );
        let mut points = Vec::new();
        for (x, y, z) in [
            (1.0, -1.0, 1.0),
            (1.0, 1.0, 1.0),
            (w.x, 1.0, w.z),
            (w.x, w.y, w.z),
        ] {
            points.extend([
                Vec3::new(-x, y, -z),
                Vec3::new(x, y, -z),
                Vec3::new(x, y, z),
                Vec3::new(-x, y, z),
            ]);
        }
        let mut faces = vec![vec![0, 1, 2, 3], vec![15, 14, 13, 12]];
        for ring in 0..3u32 {
            for i in 0..4u32 {
                let j = (i + 1) % 4;
                faces.push(vec![
                    ring * 4 + i,
                    (ring + 1) * 4 + i,
                    (ring + 1) * 4 + j,
                    ring * 4 + j,
                ]);
            }
        }
        let mut mesh = Self { points, faces };
        mesh.recalculate();
        mesh
    }

    pub fn valid(&self) -> bool {
        !self.points.is_empty()
            && !self.faces.is_empty()
            && self.points.len() <= MAX_VERTICES
            && self.faces.len() <= MAX_FACES
            && self
                .points
                .iter()
                .all(|p| p.is_finite() && p.abs().max_element() <= 32.0)
            && self.faces.iter().all(|f| {
                f.len() >= 3
                    && f.len() <= 128
                    && f.iter().all(|&i| (i as usize) < self.points.len())
            })
            && self.faces.iter().map(|f| f.len() - 2).sum::<usize>() <= MAX_TRIANGLES
    }

    pub fn edges(&self) -> BTreeSet<(u32, u32)> {
        self.faces
            .iter()
            .flat_map(|f| sides(f).map(|(a, b)| edge(a, b)))
            .collect()
    }

    pub fn closed(&self) -> bool {
        let mut counts = BTreeMap::new();
        for (a, b) in self.faces.iter().flat_map(|f| sides(f)) {
            *counts.entry(edge(a, b)).or_insert(0) += 1;
        }
        !counts.is_empty() && counts.values().all(|&n| n == 2)
    }

    pub fn center(&self, face: &[u32]) -> Vec3 {
        face.iter().map(|&i| self.points[i as usize]).sum::<Vec3>() / face.len().max(1) as f32
    }

    pub fn normal(&self, face: &[u32]) -> Vec3 {
        let c = self.center(face);
        sides(face)
            .map(|(a, b)| (self.points[a as usize] - c).cross(self.points[b as usize] - c))
            .sum::<Vec3>()
            .normalize_or_zero()
    }

    pub fn triangles(&self) -> Vec<[u32; 3]> {
        let mut triangles = Vec::new();
        for face in &self.faces {
            let mut ring = face.clone();
            let normal = self.normal(face);
            if normal.length_squared() < 0.5 {
                continue;
            }
            while ring.len() > 3 {
                let mut found = false;
                for i in 0..ring.len() {
                    let a = ring[(i + ring.len() - 1) % ring.len()];
                    let b = ring[i];
                    let c = ring[(i + 1) % ring.len()];
                    let [pa, pb, pc] = [a, b, c].map(|i| self.points[i as usize]);
                    if (pb - pa).cross(pc - pb).dot(normal) <= 0.000001 {
                        continue;
                    }
                    let inside = ring
                        .iter()
                        .filter(|&&i| i != a && i != b && i != c)
                        .any(|&i| {
                            let p = self.points[i as usize];
                            [(pa, pb), (pb, pc), (pc, pa)]
                                .into_iter()
                                .all(|(x, y)| (y - x).cross(p - x).dot(normal) >= -0.000001)
                        });
                    if inside {
                        continue;
                    }
                    triangles.push([a, b, c]);
                    ring.remove(i);
                    found = true;
                    break;
                }
                if !found {
                    if let Some(i) = (0..ring.len()).find(|&i| {
                        let a = self.points[ring[(i + ring.len() - 1) % ring.len()] as usize];
                        let b = self.points[ring[i] as usize];
                        let c = self.points[ring[(i + 1) % ring.len()] as usize];
                        (b - a).cross(c - b).length_squared() < 1e-10
                    }) {
                        ring.remove(i);
                    } else {
                        break;
                    }
                }
            }
            if ring.len() == 3 {
                triangles.push([ring[0], ring[1], ring[2]]);
            }
        }
        triangles
    }

    pub fn inset(&mut self, id: usize, fraction: f32, height: f32) -> Option<usize> {
        let face = self.faces.get(id)?.clone();
        let normal = self.normal(&face);
        let center = self.center(&face);
        let ring: Vec<_> = face
            .iter()
            .map(|&i| {
                let p = self.points[i as usize].lerp(center, fraction) + normal * height;
                self.push(p)
            })
            .collect();
        self.faces[id] = ring.clone();
        for (i, (a, b)) in sides(&face).enumerate() {
            self.faces
                .push(vec![a, b, ring[(i + 1) % ring.len()], ring[i]]);
        }
        Some(id)
    }

    pub fn extrude(&mut self, selected: &BTreeSet<usize>, offset: Vec3) -> bool {
        if selected.is_empty() || !selected.iter().all(|&i| i < self.faces.len()) {
            return false;
        }
        let mut vertices = BTreeSet::new();
        let mut uses = BTreeMap::<(u32, u32), (usize, (u32, u32))>::new();
        for &i in selected {
            let f = &self.faces[i];
            vertices.extend(f.iter().copied());
            for (a, b) in sides(f) {
                uses.entry(edge(a, b)).or_insert((0, (a, b))).0 += 1;
            }
        }
        if !uses.values().any(|(n, _)| *n == 1) {
            return false;
        }
        let mut moved = BTreeMap::new();
        for v in vertices {
            moved.insert(v, self.push(self.points[v as usize] + offset));
        }
        for &i in selected {
            for v in &mut self.faces[i] {
                *v = moved[v];
            }
        }
        for (n, (a, b)) in uses.into_values() {
            if n == 1 {
                self.faces.push(vec![a, b, moved[&b], moved[&a]]);
            }
        }
        true
    }

    pub fn knife(&mut self, face: usize, cuts: [((u32, u32), f32); 2]) -> bool {
        let Some(f) = self.faces.get(face) else {
            return false;
        };
        let e = cuts.map(|(e, _)| edge(e.0, e.1));
        if e[0] == e[1] || !e.iter().all(|k| sides(f).any(|(a, b)| edge(a, b) == *k)) {
            return false;
        }
        let cuts = cuts.map(|((a, b), t)| {
            (
                edge(a, b),
                self.points[a as usize].lerp(self.points[b as usize], t.clamp(0.06, 0.94)),
            )
        });
        let mut points = BTreeMap::new();
        for (e, p) in cuts {
            points.insert(e, self.push(p));
        }
        self.insert_edges(&points);
        self.split_face(face, points[&e[0]], points[&e[1]])
    }

    pub fn loop_ring(&self, start: Edge) -> Option<LoopRing> {
        let mut edges = BTreeMap::from([(edge(start.0, start.1), start)]);
        let mut queue = VecDeque::from([start]);
        let mut strips = BTreeMap::new();
        while let Some(current) = queue.pop_front() {
            for (i, f) in self.faces.iter().enumerate().filter(|(_, f)| f.len() == 4) {
                if strips.contains_key(&i) {
                    continue;
                }
                if let Some(k) =
                    sides(f).position(|(a, b)| edge(a, b) == edge(current.0, current.1))
                {
                    let side = (f[k], f[(k + 1) % 4]);
                    let opposite = (f[(k + 3) % 4], f[(k + 2) % 4]);
                    let other = if side == current {
                        opposite
                    } else {
                        (opposite.1, opposite.0)
                    };
                    let key = edge(other.0, other.1);
                    if let Some(&oriented) = edges.get(&key) {
                        if oriented != other {
                            return None;
                        }
                    } else {
                        edges.insert(key, other);
                        queue.push_back(other);
                    }
                    strips.insert(i, (current, other));
                }
            }
        }
        (!strips.is_empty()).then_some(LoopRing { strips, edges })
    }

    pub fn cut_ring(
        &mut self,
        ring: &LoopRing,
        count: usize,
        slide: f32,
    ) -> Option<BTreeSet<Edge>> {
        if !(1..=8).contains(&count)
            || self.points.len() + ring.edges.len() * count > MAX_VERTICES
            || self.faces.len() + ring.strips.len() * count > MAX_FACES
        {
            return None;
        }
        let mut inserted = BTreeMap::new();
        for (&key, &(a, b)) in &ring.edges {
            let mut chain = Vec::with_capacity(count + 2);
            chain.push(a);
            for i in 1..=count {
                let p = self.points[a as usize]
                    .lerp(self.points[b as usize], loop_fraction(i, count, slide));
                chain.push(self.push(p));
            }
            chain.push(b);
            inserted.insert(key, chain);
        }
        let chain = |(a, b): Edge| {
            let points = &inserted[&edge(a, b)];
            if points[0] == a {
                points.clone()
            } else {
                points.iter().rev().copied().collect()
            }
        };
        let mut result = BTreeSet::new();
        let mut faces = Vec::new();
        for (i, f) in self.faces.iter().enumerate() {
            if let Some(&(a, b)) = ring.strips.get(&i) {
                let [a, b] = [chain(a), chain(b)];
                for j in 0..=count {
                    faces.push(vec![a[j], a[j + 1], b[j + 1], b[j]]);
                }
                for j in 1..=count {
                    result.insert(edge(a[j], b[j]));
                }
            } else {
                let mut face = Vec::new();
                for (a, b) in sides(f) {
                    face.push(a);
                    if inserted.contains_key(&edge(a, b)) {
                        face.extend_from_slice(&chain((a, b))[1..=count]);
                    }
                }
                faces.push(face);
            }
        }
        self.faces = faces;
        self.recalculate();
        Some(result)
    }

    pub fn bevel_edge(&mut self, selected: Edge, amount: f32, segments: usize) -> bool {
        let [a, b] = [selected.0, selected.1];
        let sides_with_edge: Vec<_> = self
            .faces
            .iter()
            .enumerate()
            .filter_map(|(i, f)| {
                sides(f)
                    .position(|(u, v)| edge(u, v) == selected)
                    .map(|j| (i, j))
            })
            .collect();
        if sides_with_edge.len() != 2 || segments == 0 || segments > 8 {
            return false;
        }
        let a_point = self.points[a as usize];
        let b_point = self.points[b as usize];
        let axis = (b_point - a_point).normalize_or_zero();
        let mut directions = [Vec3::ZERO; 2];
        let mut limit = (b_point - a_point).length() * 0.4;
        for (side, &(i, j)) in sides_with_edge.iter().enumerate() {
            let f = &self.faces[i];
            let forward = f[j] == a;
            directions[side] = self
                .normal(f)
                .cross(if forward { axis } else { -axis })
                .normalize_or_zero();
            for &v in f.iter().filter(|&&v| v != a && v != b) {
                let distance = (self.points[v as usize] - a_point).dot(directions[side]);
                if distance > 0.001 {
                    limit = limit.min(distance * 0.48);
                }
            }
        }
        if directions[0].dot(directions[1]).abs() > 0.997 || limit < 0.0001 {
            return false;
        }
        let width = amount.clamp(0.0001, limit);
        let mut rails = Vec::new();
        for j in 0..=segments {
            let t = j as f32 / segments as f32;
            let inset = (directions[0] * (1.0 - t).powi(2) + directions[1] * t.powi(2)) * width;
            rails.push([self.push(a_point + inset), self.push(b_point + inset)]);
        }
        for (side, &(i, j)) in sides_with_edge.iter().enumerate() {
            let [x, y] = rails[if side == 0 { 0 } else { segments }];
            let forward = self.faces[i][j] == a;
            self.faces[i].splice(j + 1..j + 1, if forward { [x, y] } else { [y, x] });
        }
        for pair in rails.windows(2) {
            let [x, y] = pair[0];
            let [u, v] = pair[1];
            self.faces
                .extend([vec![x, y, v, u], vec![a, x, u], vec![b, v, y]]);
        }
        self.recalculate();
        true
    }

    fn insert_edges(&mut self, cuts: &BTreeMap<(u32, u32), u32>) {
        for f in &mut self.faces {
            *f = sides(f)
                .flat_map(|(a, b)| {
                    [Some(a), cuts.get(&edge(a, b)).copied()]
                        .into_iter()
                        .flatten()
                })
                .collect();
        }
    }

    fn split_face(&mut self, id: usize, a: u32, b: u32) -> bool {
        let face = &self.faces[id];
        let Some(ia) = face.iter().position(|&v| v == a) else {
            return false;
        };
        let Some(ib) = face.iter().position(|&v| v == b) else {
            return false;
        };
        let (a, b) = (ia.min(ib), ia.max(ib));
        if b - a <= 1 || b - a >= face.len() - 1 {
            return false;
        }
        let first = face[a..=b].to_vec();
        let second = face[b..].iter().chain(&face[..=a]).copied().collect();
        self.faces[id] = first;
        self.faces.push(second);
        true
    }

    pub fn bisect(&mut self, normal: Vec3, offset: f32, side: CutSide, cap: bool) -> bool {
        let signed: Vec<_> = self
            .points
            .iter()
            .map(|p| normal.dot(*p) - offset)
            .collect();
        if !signed.iter().any(|&d| d > 0.00001) || !signed.iter().any(|&d| d < -0.00001) {
            return false;
        }
        let old = std::mem::take(&mut self.faces);
        let mut cuts = BTreeMap::new();
        for (a, b) in old.iter().flat_map(|f| sides(f)) {
            let (da, db) = (signed[a as usize], signed[b as usize]);
            let e = edge(a, b);
            if da * db < -1e-10 && !cuts.contains_key(&e) {
                let p = self.points[a as usize].lerp(self.points[b as usize], da / (da - db));
                cuts.insert(e, self.push(p));
            }
        }
        for face in old {
            let has_pos = face.iter().any(|&i| signed[i as usize] > 0.00001);
            let has_neg = face.iter().any(|&i| signed[i as usize] < -0.00001);
            if !has_pos && !has_neg {
                self.faces.push(face);
                continue;
            }
            for sign in [1.0, -1.0] {
                if (sign > 0.0 && (side == CutSide::Negative || !has_pos))
                    || (sign < 0.0 && (side == CutSide::Positive || !has_neg))
                {
                    continue;
                }
                let mut out = Vec::new();
                for (a, b) in sides(&face) {
                    if signed[a as usize] * sign >= -0.00001 {
                        out.push(a);
                    }
                    if let Some(&cut) = cuts.get(&edge(a, b)) {
                        out.push(cut);
                    }
                }
                if out.len() >= 3 {
                    self.faces.push(out);
                }
            }
        }
        if side != CutSide::Both && cap {
            let wanted = if side == CutSide::Positive {
                -normal
            } else {
                normal
            };
            self.cap(normal, offset, wanted);
        }
        self.cleanup();
        true
    }

    fn cap(&mut self, plane: Vec3, offset: f32, normal: Vec3) {
        let mut counts = BTreeMap::new();
        for (a, b) in self.faces.iter().flat_map(|f| sides(f)) {
            if [a, b]
                .iter()
                .all(|&i| (plane.dot(self.points[i as usize]) - offset).abs() < 0.0001)
            {
                *counts.entry(edge(a, b)).or_insert(0usize) += 1;
            }
        }
        let mut boundary: BTreeSet<_> = counts
            .into_iter()
            .filter_map(|(e, n)| (n == 1).then_some(e))
            .collect();
        while let Some((a, b)) = boundary.pop_first() {
            let mut ring = vec![a, b];
            let mut last = b;
            let mut closed = false;
            while let Some(&(u, v)) = boundary.iter().find(|&&(u, v)| u == last || v == last) {
                boundary.remove(&(u, v));
                last = if u == last { v } else { u };
                if last == a {
                    closed = true;
                    break;
                }
                ring.push(last);
            }
            if closed && ring.len() >= 3 {
                if self.normal(&ring).dot(normal) < 0.0 {
                    ring.reverse();
                }
                self.faces.push(ring);
            }
        }
    }

    pub fn fill(&mut self, selection: &BTreeSet<u32>) -> bool {
        if !(3..=128).contains(&selection.len()) {
            return false;
        }
        let mut ring: Vec<_> = selection.iter().copied().collect();
        let center = self.center(&ring);
        let x = (self.points[ring[0] as usize] - center).normalize_or_zero();
        let normal = ring
            .iter()
            .skip(1)
            .map(|&i| x.cross(self.points[i as usize] - center))
            .max_by(|a, b| a.length_squared().total_cmp(&b.length_squared()))
            .unwrap()
            .normalize_or_zero();
        if normal.length_squared() < 0.5 {
            return false;
        }
        let y = normal.cross(x);
        let angle = |v| {
            let p = self.points[v as usize] - center;
            p.dot(y).atan2(p.dot(x))
        };
        ring.sort_by(|&a, &b| angle(a).total_cmp(&angle(b)));
        if self
            .faces
            .iter()
            .any(|f| f.len() == selection.len() && f.iter().all(|v| selection.contains(v)))
        {
            return false;
        }
        self.faces.push(ring);
        self.recalculate();
        true
    }

    pub fn merge(&mut self, selection: &BTreeSet<u32>) -> bool {
        if selection.len() < 2 {
            return false;
        }
        let first = *selection.first().unwrap();
        self.points[first as usize] = selection
            .iter()
            .map(|&i| self.points[i as usize])
            .sum::<Vec3>()
            / selection.len() as f32;
        for face in &mut self.faces {
            for v in face {
                if selection.contains(v) {
                    *v = first;
                }
            }
        }
        self.cleanup();
        true
    }

    pub fn dissolve(&mut self, selection: &BTreeSet<u32>) -> bool {
        if selection.is_empty() {
            return false;
        }
        for face in &mut self.faces {
            face.retain(|v| !selection.contains(v));
        }
        self.cleanup();
        !self.faces.is_empty()
    }

    pub fn recalculate(&mut self) {
        let mut links = BTreeMap::<(u32, u32), Vec<usize>>::new();
        for (i, f) in self.faces.iter().enumerate() {
            for (a, b) in sides(f) {
                links.entry(edge(a, b)).or_default().push(i);
            }
        }
        let mut visited = vec![false; self.faces.len()];
        for start in 0..self.faces.len() {
            if visited[start] {
                continue;
            }
            let mut component = Vec::new();
            let mut queue = VecDeque::from([start]);
            visited[start] = true;
            let mut closed = true;
            while let Some(i) = queue.pop_front() {
                component.push(i);
                let directed: Vec<_> = sides(&self.faces[i]).collect();
                for (a, b) in directed {
                    let neighbors = &links[&edge(a, b)];
                    closed &= neighbors.len() == 2;
                    for &j in neighbors {
                        if visited[j] {
                            continue;
                        }
                        if sides(&self.faces[j]).any(|(u, v)| u == a && v == b) {
                            self.faces[j].reverse();
                        }
                        visited[j] = true;
                        queue.push_back(j);
                    }
                }
            }
            if closed {
                let volume = component
                    .iter()
                    .map(|&i| {
                        let f = &self.faces[i];
                        let o = self.points[f[0] as usize];
                        f[1..]
                            .windows(2)
                            .map(|e| {
                                o.dot(self.points[e[0] as usize].cross(self.points[e[1] as usize]))
                            })
                            .sum::<f32>()
                    })
                    .sum::<f32>();
                if volume < 0.0 {
                    for i in component {
                        self.faces[i].reverse();
                    }
                }
            }
        }
    }

    fn push(&mut self, point: Vec3) -> u32 {
        let id = self.points.len() as u32;
        self.points.push(point);
        id
    }

    fn cleanup(&mut self) {
        for f in &mut self.faces {
            let mut seen = BTreeSet::new();
            f.retain(|&v| seen.insert(v));
        }
        self.faces.retain(|f| f.len() >= 3);
        let used: BTreeSet<_> = self.faces.iter().flatten().copied().collect();
        let mut map = BTreeMap::new();
        self.points = used
            .into_iter()
            .enumerate()
            .map(|(i, v)| {
                map.insert(v, i as u32);
                self.points[v as usize]
            })
            .collect();
        for face in &mut self.faces {
            for v in face {
                *v = map[v];
            }
        }
    }
}
