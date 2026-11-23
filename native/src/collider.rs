use crate::scene::Object;
use glam::Vec3;

#[derive(Clone, Copy)]
pub struct Triangle {
    pub a: Vec3,
    pub b: Vec3,
    pub c: Vec3,
    pub selected: f32,
    pub glass_id: f32,
}

impl Triangle {
    fn center(self) -> Vec3 {
        (self.a + self.b + self.c) / 3.0
    }
    pub fn normal(self) -> Vec3 {
        (self.b - self.a).cross(self.c - self.a).normalize_or_zero()
    }

    fn ray(self, origin: Vec3, direction: Vec3) -> Option<f32> {
        let e = self.b - self.a;
        let f = self.c - self.a;
        let n = direction.cross(f);
        let det = e.dot(n);
        if det.abs() < 1e-8 {
            return None;
        }
        let delta = origin - self.a;
        let u = delta.dot(n) / det;
        let v = direction.dot(delta.cross(e)) / det;
        if u < 0.0 || v < 0.0 || u + v > 1.0 {
            return None;
        }
        Some(f.dot(delta.cross(e)) / det)
    }

    fn nearest(self, p: Vec3) -> Vec3 {
        let [a, b, c] = [self.a, self.b, self.c];
        let n = self.normal();
        let q = p - n * n.dot(p - a);
        if [(a, b), (b, c), (c, a)]
            .into_iter()
            .all(|(a, b)| (b - a).cross(q - a).dot(n) >= 0.0)
        {
            return q;
        }
        [(a, b), (b, c), (c, a)]
            .into_iter()
            .map(|(a, b)| {
                let v = b - a;
                a + v * ((p - a).dot(v) / v.length_squared().max(1e-12)).clamp(0.0, 1.0)
            })
            .min_by(|a, b| a.distance_squared(p).total_cmp(&b.distance_squared(p)))
            .unwrap()
    }
}

pub fn triangles(object: &Object, selected: bool) -> Vec<Triangle> {
    let Some(mesh) = &object.mesh else {
        return Vec::new();
    };
    mesh.triangles()
        .into_iter()
        .filter_map(|indices| {
            let [a, b, c] = indices.map(|i| {
                object.position + object.rotation * (mesh.points[i as usize] * object.half)
            });
            let triangle = Triangle {
                a,
                b,
                c,
                selected: f32::from(selected),
                glass_id: 0.0,
            };
            (triangle.normal().length_squared() > 0.5).then_some(triangle)
        })
        .collect()
}

#[derive(Clone, Copy)]
pub struct Node {
    pub low: Vec3,
    pub high: Vec3,
    pub start: usize,
    pub count: usize,
    pub skip: usize,
}

pub struct Tree {
    pub nodes: Vec<Node>,
    pub triangles: Vec<Triangle>,
}

impl Tree {
    pub fn build(mut triangles: Vec<Triangle>) -> Self {
        let mut nodes = Vec::new();
        if !triangles.is_empty() {
            Self::split(&mut nodes, &mut triangles, 0);
        }
        Self { nodes, triangles }
    }

    fn split(nodes: &mut Vec<Node>, triangles: &mut [Triangle], start: usize) {
        let mut low = Vec3::splat(f32::INFINITY);
        let mut high = Vec3::splat(f32::NEG_INFINITY);
        for t in triangles.iter() {
            low = low.min(t.a.min(t.b).min(t.c));
            high = high.max(t.a.max(t.b).max(t.c));
        }
        let i = nodes.len();
        nodes.push(Node {
            low,
            high,
            start,
            count: triangles.len(),
            skip: i + 1,
        });
        if triangles.len() <= 4 {
            return;
        }
        let sizes = high - low;
        let axis = if sizes.x >= sizes.y.max(sizes.z) {
            0
        } else if sizes.y >= sizes.z {
            1
        } else {
            2
        };
        let mid = triangles.len() / 2;
        triangles.select_nth_unstable_by(mid, |a, b| a.center()[axis].total_cmp(&b.center()[axis]));
        let (left, right) = triangles.split_at_mut(mid);
        Self::split(nodes, left, start);
        Self::split(nodes, right, start + mid);
        nodes[i].count = 0;
        nodes[i].skip = nodes.len();
    }

    pub fn bounds(&self) -> Option<(Vec3, Vec3)> {
        self.nodes.first().map(|n| (n.low, n.high))
    }

    pub fn hit(&self, origin: Vec3, direction: Vec3, limit: f32) -> Option<(f32, Vec3)> {
        let mut closest = limit;
        let mut normal = Vec3::ZERO;
        self.visit_ray(origin, direction, limit, |distance, triangle| {
            if distance < closest {
                closest = distance;
                normal = triangle.normal();
            }
        });
        (closest < limit).then_some((closest, normal))
    }

    fn visit_ray(
        &self,
        origin: Vec3,
        direction: Vec3,
        limit: f32,
        mut visit: impl FnMut(f32, Triangle),
    ) {
        let inv = direction.map(|v| v.signum() / v.abs().max(1e-7));
        let mut i = 0;
        while i < self.nodes.len() {
            let n = self.nodes[i];
            let a = (n.low - origin) * inv;
            let b = (n.high - origin) * inv;
            let low = a.min(b).max_element();
            let high = a.max(b).min_element();
            if low > high || high < 0.00001 || low > limit {
                i = n.skip;
                continue;
            }
            for &t in &self.triangles[n.start..n.start + n.count] {
                if let Some(d) = t.ray(origin, direction)
                    && d > 0.00001
                    && d < limit
                {
                    visit(d, t);
                }
            }
            i += 1;
        }
    }

    pub fn inside(&self, point: Vec3) -> bool {
        let mut hits = 0usize;
        self.visit_ray(point, Vec3::new(1.0, 0.126813, 0.073397), 20.0, |_, _| {
            hits += 1
        });
        hits % 2 == 1
    }

    pub fn nearest(&self, p: Vec3, radius: f32) -> Option<(Vec3, Vec3)> {
        let mut dist = radius * radius;
        let mut result = None;
        let mut i = 0;
        while i < self.nodes.len() {
            let n = self.nodes[i];
            if p.clamp(n.low, n.high).distance_squared(p) > dist {
                i = n.skip;
                continue;
            }
            for &t in &self.triangles[n.start..n.start + n.count] {
                let q = t.nearest(p);
                let d = q.distance_squared(p);
                if d < dist {
                    dist = d;
                    result = Some((q, t.normal()));
                }
            }
            i += 1;
        }
        result
    }
}

pub struct MeshCollider {
    pub object: Object,
    pub tree: Tree,
    pub closed: bool,
}

impl MeshCollider {
    pub fn new(object: &Object) -> Self {
        let closed = object.mesh.as_ref().is_some_and(|m| m.closed());
        let tree = Tree::build(triangles(object, false));
        Self {
            object: object.clone(),
            tree,
            closed,
        }
    }

    pub fn near(&self, p: Vec3, margin: f32) -> bool {
        let Some((a, b)) = self.tree.bounds() else {
            return false;
        };
        p.cmpge(a - Vec3::splat(margin)).all() && p.cmple(b + Vec3::splat(margin)).all()
    }

    pub fn same(&self, other: &Object) -> bool {
        let old = &self.object;
        old.id == other.id
            && old.position == other.position
            && old.rotation == other.rotation
            && old.half == other.half
            && old.thickness == other.thickness
            && old.mesh == other.mesh
    }

    pub fn solid(&self, p: Vec3) -> bool {
        let shell = self.object.thickness * 0.5;
        self.near(p, shell)
            && (if self.closed {
                self.tree.inside(p)
            } else {
                self.tree.nearest(p, shell).is_some()
            })
    }
}
