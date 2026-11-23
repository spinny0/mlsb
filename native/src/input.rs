use eframe::egui::{self, Key, Pos2, Vec2};
use glam::{Quat, Vec3};

use crate::{
    scene::{Kind, Object, Scene},
    viewport::Camera,
};

pub const AXES: [Vec3; 3] = [Vec3::X, Vec3::NEG_Z, Vec3::Y];

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    Move,
    Rotate,
    Scale,
}

pub struct Transform {
    pub before: Scene,
    pub original: Object,
    pub operation: Operation,
    pub axis: Option<usize>,
    pub local: bool,
    pub number: String,
    pub delta: Vec2,
    pub snap: bool,
    pub mesh_elements: bool,
}

impl Transform {
    pub fn new(scene: &Scene, id: u32, operation: Operation) -> Option<Self> {
        let original = scene.objects.iter().find(|o| o.id == id)?.clone();
        Some(Self {
            before: scene.clone(),
            original,
            operation,
            axis: None,
            local: false,
            number: String::new(),
            delta: Vec2::ZERO,
            snap: false,
            mesh_elements: false,
        })
    }

    pub fn axis_key(&mut self, axis: usize) {
        if self.axis == Some(axis) {
            self.local = !self.local;
        } else {
            self.axis = Some(axis);
            self.local = false;
        }
    }

    pub fn label(&self) -> String {
        let operation = match self.operation {
            Operation::Move => "Move",
            Operation::Rotate => "Rotate",
            Operation::Scale => "Scale",
        };
        let axis = self.axis.map_or("", |axis| [" X", " Y", " Z"][axis]);
        let space = if self.local { " local" } else { "" };
        format!(
            "{operation}{axis}{space}  {}     Enter confirm   Esc cancel",
            self.number
        )
    }

    pub fn apply(&self, object: &mut Object, camera: Camera, height: f32) {
        let (_, right, up, forward) = camera.basis();
        let axis = self.axis.map(|i| {
            let axis = AXES[i];
            if self.local {
                self.original.rotation * axis
            } else {
                axis
            }
        });
        let n = self.number.parse::<f32>().ok().filter(|v| v.is_finite());
        let snap = |value: f32, step: f32| {
            if self.snap && n.is_none() {
                (value / step).round() * step
            } else {
                value
            }
        };
        let delta =
            (right * self.delta.x - up * self.delta.y) * camera.distance * 0.75 / height.max(1.0);
        match self.operation {
            Operation::Move => {
                let default_axis = axis.unwrap_or(Vec3::X);
                let move_by = if let Some(n) = n {
                    default_axis * n
                } else if let Some(axis) = axis {
                    axis * snap(axis.dot(delta), 0.10)
                } else {
                    delta.map(|v| snap(v, 0.10))
                };
                let position = self.original.position + move_by;
                object.position = if self.mesh_elements {
                    position
                } else {
                    position.clamp(Vec3::new(-1.9, 0.05, -1.3), Vec3::new(1.9, 2.72, 1.3))
                };
            }
            Operation::Rotate => {
                let angle = snap(
                    n.map_or(self.delta.x * 0.51 - self.delta.y * 0.17, |x| x),
                    5.0,
                )
                .to_radians();
                object.rotation = (Quat::from_axis_angle(axis.unwrap_or(-forward), angle)
                    * self.original.rotation)
                    .normalize();
            }
            Operation::Scale => {
                let factor = snap(
                    n.unwrap_or((self.delta.x * 0.007 - self.delta.y * 0.003).exp()),
                    0.10,
                )
                .clamp(0.08, 12.0);
                if matches!(object.kind, Kind::Emitter | Kind::Drain) {
                    object.radius = (self.original.radius * factor).clamp(0.04, 0.55);
                } else {
                    let mut scale = Vec3::splat(factor);
                    if let Some(i) = self.axis {
                        scale = Vec3::ONE;
                        scale[[0, 2, 1][i]] = factor;
                    }
                    let minimum = if object.kind == Kind::Tank {
                        0.2
                    } else {
                        0.045
                    };
                    let half = self.original.half * scale;
                    object.half = if self.mesh_elements {
                        half
                    } else {
                        half.clamp(Vec3::splat(minimum), Vec3::new(1.85, 1.2, 1.25))
                    };
                }
            }
        }
    }
}

pub fn read_transform(transform: &mut Transform, ctx: &egui::Context) {
    ctx.input(|input| {
        for (key, axis) in [(Key::X, 0), (Key::Y, 1), (Key::Z, 2)] {
            if input.key_pressed(key) {
                transform.axis_key(axis);
            }
        }
        if input.key_pressed(Key::Backspace) {
            transform.number.pop();
        }
        for event in &input.events {
            if let egui::Event::Text(text) = event {
                for c in text.chars() {
                    if (c.is_ascii_digit() || c == '.' || c == '-') && transform.number.len() < 14 {
                        transform.number.push(c);
                    }
                }
            }
        }
    });
}

pub fn project(camera: Camera, rect: egui::Rect, point: Vec3) -> Option<Pos2> {
    let (matrix, _) = camera.projection(rect.width() / rect.height());
    let clip = matrix * point.extend(1.0);
    if clip.w <= 0.0 {
        return None;
    }
    let screen = clip.truncate() / clip.w;
    Some(egui::pos2(
        rect.left() + (screen.x * 0.5 + 0.5) * rect.width(),
        rect.top() + (0.5 - screen.y * 0.5) * rect.height(),
    ))
}

pub fn pick(scene: &Scene, camera: Camera, rect: egui::Rect, point: Pos2) -> Option<u32> {
    let uv = (point - rect.min) / rect.size();
    let (origin, ray) = camera.ray(glam::Vec2::new(uv.x, uv.y), rect.width() / rect.height());
    let hit = |o: &Object| {
        if o.mesh.is_some() {
            return crate::collider::Tree::build(crate::collider::triangles(o, false))
                .hit(origin, ray, 100.0)
                .map(|(distance, _)| distance);
        }
        let inverse = o.rotation.conjugate();
        let p = inverse * (origin - o.position);
        let r = inverse * ray;
        let inv = Vec3::ONE / (r.signum() * r.abs().max(Vec3::splat(0.00001)));
        let half = if matches!(o.kind, Kind::Emitter | Kind::Drain) {
            Vec3::splat(o.radius.max(0.12))
        } else if matches!(o.kind, Kind::Light | Kind::Camera) {
            Vec3::splat(0.12)
        } else {
            o.half
        };
        let a = (-half - p) * inv;
        let b = (half - p) * inv;
        let enter = a.min(b).max_element();
        let exit = a.max(b).min_element();
        (exit >= enter && enter >= 0.0).then_some(enter)
    };
    let closest = |glass| {
        scene
            .objects
            .iter()
            .filter(|o| o.enabled && o.visible && o.selectable && (o.kind == Kind::Tank) == glass)
            .filter_map(|o| hit(o).map(|t| (t, o.id)))
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, id)| id)
    };
    closest(false).or_else(|| closest(true))
}
