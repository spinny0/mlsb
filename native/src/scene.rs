use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};

pub const MAX_OBJECTS: usize = 16;
pub const WALL: f32 = 0.12;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    Emitter,
    Drain,
    Box,
    Tank,
    Light,
    Camera,
    Field,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Emitter => "Emitter",
            Self::Drain => "Drain",
            Self::Box => "Solid box",
            Self::Tank => "Glass tank",
            Self::Light => "Sun light",
            Self::Camera => "Camera",
            Self::Field => "Force Field",
        }
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Object {
    pub id: u32,
    pub name: String,
    pub kind: Kind,
    pub position: Vec3,
    pub rotation: Quat,
    pub half: Vec3,
    pub enabled: bool,
    #[serde(default = "yes")]
    pub visible: bool,
    #[serde(default = "yes")]
    pub selectable: bool,
    pub radius: f32,
    pub flow: f32,
    pub speed: f32,
    #[serde(default)]
    pub schedule: Schedule,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mesh: Option<crate::mesh::Mesh>,
    #[serde(default = "thickness")]
    pub thickness: f32,
    #[serde(default)]
    pub mesh_glass: bool,
    #[serde(default)]
    pub wetting: f32,
    #[serde(default)]
    pub oil: bool,
    #[serde(default)]
    pub drip: bool,
    #[serde(default)]
    pub motion: Option<ColliderMotion>,
    #[serde(default)]
    pub fluid: FluidRole,
    #[serde(default)]
    pub flow_mode: FlowMode,
    #[serde(default)]
    pub field: ForceField,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum FluidRole {
    #[default]
    Automatic,
    None,
    Domain,
    Flow,
    Effector,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum FlowMode {
    #[default]
    Inflow,
    Outflow,
    Geometry,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum FieldKind {
    #[default]
    Directional,
    Vortex,
    Radial,
}

#[derive(Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ForceField {
    pub kind: FieldKind,
    pub strength: f32,
    pub radius: f32,
}

impl Default for ForceField {
    fn default() -> Self {
        Self {
            kind: FieldKind::Directional,
            strength: 10.0,
            radius: 1.2,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Whitewater {
    pub spray: bool,
    pub foam: bool,
    pub bubbles: bool,
    pub amount: f32,
    pub lifetime: f32,
}

impl Default for Whitewater {
    fn default() -> Self {
        Self {
            spray: true,
            foam: true,
            bubbles: true,
            amount: 1.0,
            lifetime: 1.0,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ColliderMotion {
    pub start: u32,
    pub end: u32,
    pub offset: Vec3,
    pub tilt: f32,
}

impl Default for ColliderMotion {
    fn default() -> Self {
        Self {
            start: 90,
            end: 270,
            offset: Vec3::new(0.0, 0.45, 0.0),
            tilt: -20.0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Schedule {
    pub start: u32,
    pub stop: Option<u32>,
    pub ramp: f32,
    pub liters: Option<f32>,
    pub limited_drain: bool,
}

impl Schedule {
    pub fn strength(self, frame: u32) -> f32 {
        if frame < self.start || self.stop.is_some_and(|stop| frame >= stop) {
            return 0.0;
        }
        if self.ramp <= 0.0 {
            return 1.0;
        }
        ((frame - self.start) as f32 * crate::fluid::DT / self.ramp).min(1.0)
    }
}

impl Object {
    pub fn new(id: u32, kind: Kind, position: Vec3) -> Self {
        Self {
            id,
            kind,
            position,
            name: kind.label().into(),
            rotation: Quat::IDENTITY,
            half: if kind == Kind::Tank {
                Vec3::new(0.62, 0.45, 0.6)
            } else {
                Vec3::new(0.30, 0.13, 0.30)
            },
            enabled: true,
            visible: true,
            selectable: true,
            radius: 0.14,
            flow: 110.0,
            speed: 2.2,
            schedule: Schedule::default(),
            mesh: None,
            thickness: thickness(),
            mesh_glass: false,
            wetting: 0.0,
            oil: false,
            drip: false,
            motion: None,
            fluid: FluidRole::Automatic,
            flow_mode: if kind == Kind::Drain {
                FlowMode::Outflow
            } else {
                FlowMode::Inflow
            },
            field: ForceField::default(),
        }
    }

    pub fn flow_behavior(&self) -> Option<FlowMode> {
        if !self.enabled {
            return None;
        }
        match self.fluid {
            FluidRole::Flow => Some(self.flow_mode),
            FluidRole::Automatic => match self.kind {
                Kind::Emitter => Some(FlowMode::Inflow),
                Kind::Drain => Some(FlowMode::Outflow),
                _ => None,
            },
            _ => None,
        }
    }

    pub fn is_collider(&self) -> bool {
        self.enabled
            && matches!(self.kind, Kind::Box | Kind::Tank)
            && matches!(self.fluid, FluidRole::Automatic | FluidRole::Effector)
    }

    pub fn is_domain(&self) -> bool {
        self.enabled && self.fluid == FluidRole::Domain
    }

    pub fn geometry_boxes(&self, mut visit: impl FnMut(Vec3, Vec3)) {
        let h = self.half;
        if self.kind == Kind::Box && self.mesh.is_none() {
            visit(Vec3::ZERO, h);
        } else if self.kind == Kind::Tank {
            let w = WALL * 0.5;
            visit(Vec3::new(0.0, -h.y + w, 0.0), Vec3::new(h.x, w, h.z));
            for side in [-1.0, 1.0] {
                visit(
                    Vec3::new(side * (h.x - w), 0.0, 0.0),
                    Vec3::new(w, h.y, h.z),
                );
                visit(
                    Vec3::new(0.0, 0.0, side * (h.z - w)),
                    Vec3::new(h.x - WALL, h.y, w),
                );
            }
        }
    }

    pub fn boxes(&self, visit: impl FnMut(Vec3, Vec3)) {
        if self.is_collider() {
            self.geometry_boxes(visit);
        }
    }
}

fn yes() -> bool {
    true
}

fn thickness() -> f32 {
    0.068
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum EnvironmentMode {
    Studio,
    #[default]
    Garden,
    File,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Lighting {
    pub azimuth: f32,
    pub elevation: f32,
    pub power: f32,
    pub checker: bool,
    pub caustics: bool,
    pub environment: EnvironmentMode,
    pub panorama: String,
    pub environment_width: u32,
    pub environment_rotation: f32,
    pub environment_exposure: f32,
    pub environment_background: bool,
}

impl Default for Lighting {
    fn default() -> Self {
        Self {
            azimuth: -35.0,
            elevation: 67.0,
            power: 1.0,
            checker: false,
            caustics: true,
            environment: EnvironmentMode::Garden,
            panorama: String::new(),
            environment_width: 2048,
            environment_rotation: 0.0,
            environment_exposure: -0.3,
            environment_background: true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Preset {
    Empty,
    Station,
    Pour,
    Dam,
    Slosh,
    Honey,
    Wetting,
    Layers,
}

impl Preset {
    pub const ALL: [Self; 8] = [
        Self::Empty,
        Self::Station,
        Self::Pour,
        Self::Dam,
        Self::Slosh,
        Self::Honey,
        Self::Wetting,
        Self::Layers,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Empty => "Empty scene",
            Self::Station => "Live pouring station",
            Self::Pour => "Ramp pour",
            Self::Dam => "Dam break",
            Self::Slosh => "Slosh tank",
            Self::Honey => "Honey on a spoon",
            Self::Wetting => "Glass and wax",
            Self::Layers => "Oil over water",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Liquid {
    pub viscosity: f32,
    pub cohesion: f32,
    pub tint: [f32; 3],
    pub absorption: f32,
    pub ior: f32,
    #[serde(default)]
    pub surface_tension: f32,
}

impl Liquid {
    pub const WATER: Self = Self {
        viscosity: 0.025,
        cohesion: 0.12,
        tint: [0.85, 0.96, 0.985],
        absorption: 0.72,
        ior: 1.333,
        surface_tension: 0.25,
    };
    pub const OIL: Self = Self {
        viscosity: 0.28,
        cohesion: 0.22,
        tint: [0.94, 0.64, 0.23],
        absorption: 2.1,
        ior: 1.47,
        surface_tension: 0.20,
    };
    pub const SYRUP: Self = Self {
        viscosity: 0.89,
        cohesion: 0.35,
        tint: [0.67, 0.25, 0.09],
        absorption: 3.4,
        ior: 1.49,
        surface_tension: 0.53,
    };
    pub const HONEY: Self = Self {
        viscosity: 0.99,
        cohesion: 0.49,
        tint: [0.91, 0.52, 0.10],
        absorption: 3.7,
        ior: 1.49,
        surface_tension: 0.76,
    };
}

#[derive(Clone, Copy, Serialize, Deserialize)]
pub struct Fill {
    pub min: Vec3,
    pub max: Vec3,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Scene {
    pub format: u32,
    pub preset: Preset,
    pub seed: u32,
    pub liquid: Liquid,
    pub fill: Option<Fill>,
    #[serde(default)]
    pub fill_liters: Option<f32>,
    #[serde(default)]
    pub gate: Option<u32>,
    #[serde(default)]
    pub gate_frame: Option<u32>,
    pub objects: Vec<Object>,
    #[serde(default)]
    pub lighting: Lighting,
    #[serde(default = "gravity")]
    pub gravity: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_light: Option<u32>,
    #[serde(default)]
    pub gate_lift_frames: u32,
    #[serde(default)]
    pub whitewater: Whitewater,
}

fn gravity() -> f32 {
    9.81
}

impl Scene {
    pub fn assign_role(&mut self, id: u32, role: FluidRole) {
        if role == FluidRole::Domain {
            for object in &mut self.objects {
                if object.id != id && object.fluid == FluidRole::Domain {
                    object.fluid = FluidRole::None;
                }
            }
        }
        if let Some(object) = self.objects.iter_mut().find(|o| o.id == id) {
            object.fluid = role;
        }
    }

    pub fn preset(preset: Preset) -> Self {
        let mut scene = Self {
            format: 1,
            preset,
            seed: 71,
            liquid: Liquid::WATER,
            fill: None,
            fill_liters: None,
            gate: None,
            gate_frame: None,
            objects: Vec::new(),
            lighting: Lighting::default(),
            gravity: gravity(),
            active_light: None,
            gate_lift_frames: 0,
            whitewater: Whitewater::default(),
        };
        let mut add = |kind, name: &str, p, half, tilt| {
            let mut object = Object::new(scene.objects.len() as u32 + 1, kind, p);
            object.name = name.into();
            object.half = half;
            object.rotation = Quat::from_rotation_z(tilt);
            scene.objects.push(object);
        };
        match preset {
            Preset::Empty => {
                scene.lighting.environment = EnvironmentMode::Studio;
            }
            Preset::Station => {
                add(
                    Kind::Tank,
                    "Catch tank",
                    Vec3::new(0.25, 0.52, 0.0),
                    Vec3::new(1.52, 0.43, 0.94),
                    0.0,
                );
                add(
                    Kind::Box,
                    "Pouring cup",
                    Vec3::new(-0.63, 1.53, 0.0),
                    Vec3::new(0.37, 0.35, 0.37),
                    0.0,
                );
                add(
                    Kind::Box,
                    "Stirring spoon",
                    Vec3::new(0.70, 1.15, 0.27),
                    Vec3::new(0.44, 0.13, 0.23),
                    0.12,
                );
                add(
                    Kind::Field,
                    "Vortex",
                    Vec3::new(0.15, 0.48, 0.0),
                    Vec3::splat(0.15),
                    0.0,
                );
                add(
                    Kind::Emitter,
                    "Refill tap",
                    Vec3::new(0.19, 2.22, -0.28),
                    Vec3::splat(0.12),
                    0.0,
                );
                scene.objects[1].mesh = Some(crate::mesh::Primitive::Cup.geometry());
                scene.objects[1].mesh_glass = true;
                scene.objects[2].mesh = Some(crate::mesh::Primitive::Spoon.geometry());
                scene.objects[3].field = ForceField {
                    kind: FieldKind::Vortex,
                    strength: 0.0,
                    radius: 1.5,
                };
                scene.objects[4].enabled = false;
                scene.objects[4].flow = 160.0;
                scene.objects[4].speed = 2.8;
                scene.lighting.environment = EnvironmentMode::Studio;
                scene.fill = Some(Fill {
                    min: Vec3::new(-0.79, 1.34, -0.16),
                    max: Vec3::new(-0.47, 1.76, 0.16),
                });
            }
            Preset::Pour => {
                add(
                    Kind::Emitter,
                    "Pour nozzle",
                    Vec3::new(-0.63, 2.18, 0.0),
                    Vec3::splat(0.15),
                    0.08,
                );
                add(
                    Kind::Box,
                    "Tilted ramp",
                    Vec3::new(-0.70, 1.63, 0.0),
                    Vec3::new(0.90, 0.074, 0.50),
                    -0.39,
                );
                add(
                    Kind::Tank,
                    "Catch tank",
                    Vec3::new(0.73, 0.61, 0.0),
                    Vec3::new(0.93, 0.52, 0.72),
                    0.0,
                );
                scene.objects[0].flow = 170.0;
                scene.objects[0].speed = 1.8;
                scene.fill = Some(Fill {
                    min: Vec3::new(0.05, 0.24, -0.54),
                    max: Vec3::new(1.47, 0.48, 0.54),
                });
            }
            Preset::Dam => {
                add(
                    Kind::Tank,
                    "Channel",
                    Vec3::new(0.0, 0.69, 0.0),
                    Vec3::new(1.76, 0.60, 0.80),
                    0.0,
                );
                add(
                    Kind::Box,
                    "Dam gate",
                    Vec3::new(-0.02, 0.80, 0.0),
                    Vec3::new(0.075, 0.59, 0.69),
                    0.0,
                );
                scene.gate = Some(2);
                scene.fill = Some(Fill {
                    min: Vec3::new(-1.59, 0.26, -0.64),
                    max: Vec3::new(-0.15, 1.08, 0.64),
                });
            }
            Preset::Slosh => {
                add(
                    Kind::Tank,
                    "Tilting tank",
                    Vec3::new(0.0, 0.97, 0.0),
                    Vec3::new(1.29, 0.62, 0.76),
                    0.0,
                );
                scene.fill = Some(Fill {
                    min: Vec3::new(-1.12, 0.51, -0.58),
                    max: Vec3::new(1.12, 0.95, 0.58),
                });
            }
            Preset::Honey => {
                add(
                    Kind::Emitter,
                    "Honey nozzle",
                    Vec3::new(-0.39, 2.27, 0.0),
                    Vec3::splat(0.1),
                    0.0,
                );
                add(
                    Kind::Box,
                    "Spoon",
                    Vec3::new(-0.38, 1.12, 0.0),
                    Vec3::new(0.75, 0.16, 0.36),
                    0.0,
                );
                add(
                    Kind::Tank,
                    "Catch dish",
                    Vec3::new(0.18, 0.37, 0.0),
                    Vec3::new(1.29, 0.28, 0.67),
                    0.0,
                );
                scene.liquid = Liquid::HONEY;
                let nozzle = &mut scene.objects[0];
                nozzle.radius = 0.076;
                nozzle.flow = 28.0;
                nozzle.speed = 0.21;
                nozzle.schedule.stop = Some(235);
                let spoon = &mut scene.objects[1];
                spoon.mesh = Some(crate::mesh::Primitive::Spoon.geometry());
                spoon.wetting = 0.48;
                spoon.motion = Some(ColliderMotion {
                    start: 110,
                    end: 280,
                    offset: Vec3::new(0.36, 0.23, 0.0),
                    tilt: -78.0,
                });
            }
            Preset::Wetting => {
                for (x, name) in [(-0.87, "Clean glass"), (0.87, "Waxed plate")] {
                    add(
                        Kind::Emitter,
                        "Water tap",
                        Vec3::new(x - 0.16, 2.05, 0.0),
                        Vec3::splat(0.10),
                        0.0,
                    );
                    add(
                        Kind::Box,
                        name,
                        Vec3::new(x, 1.12, 0.0),
                        Vec3::new(0.74, 0.058, 0.58),
                        -0.47,
                    );
                }
                add(
                    Kind::Tank,
                    "Catch tray",
                    Vec3::new(0.0, 0.37, 0.0),
                    Vec3::new(1.81, 0.28, 0.72),
                    0.0,
                );
                for i in [0, 2] {
                    let tap = &mut scene.objects[i];
                    tap.radius = 0.054;
                    tap.flow = 5.0;
                    tap.speed = 0.20;
                    tap.drip = true;
                    tap.schedule.stop = Some(270);
                }
                scene.objects[1].wetting = 0.86;
                scene.objects[1].mesh = Some(crate::mesh::Primitive::Cube.geometry());
                scene.objects[1].mesh_glass = true;
            }
            Preset::Layers => {
                add(
                    Kind::Tank,
                    "Layer tank",
                    Vec3::new(0.0, 0.70, 0.0),
                    Vec3::new(1.18, 0.59, 0.64),
                    0.0,
                );
                add(
                    Kind::Emitter,
                    "Oil nozzle",
                    Vec3::new(0.0, 1.43, 0.0),
                    Vec3::splat(0.14),
                    0.0,
                );
                scene.fill = Some(Fill {
                    min: Vec3::new(-1.01, 0.26, -0.47),
                    max: Vec3::new(1.01, 0.72, 0.47),
                });
                let oil = &mut scene.objects[1];
                oil.oil = true;
                oil.radius = 0.22;
                oil.flow = 91.0;
                oil.speed = 0.6;
                oil.schedule.start = 36;
                oil.schedule.stop = Some(210);
            }
        }
        scene
    }

    pub fn initial_fill(&self) -> Option<Fill> {
        let mut fill = self.fill?;
        if let Some(liters) = self.fill_liters {
            let area = (fill.max.x - fill.min.x) * (fill.max.z - fill.min.z);
            if area <= 0.00001 {
                return None;
            }
            fill.max.y = (fill.min.y + liters * 0.001 / area).min(crate::fluid::END.y - 0.04);
        }
        (fill.max - fill.min)
            .cmpgt(Vec3::ZERO)
            .all()
            .then_some(fill)
    }

    pub fn domain_bounds(&self) -> (Vec3, Vec3) {
        use crate::fluid::{END, ORIGIN};
        let Some(domain) = self.objects.iter().find(|o| o.is_domain()) else {
            return (ORIGIN, END);
        };
        let (center, extent) = domain
            .mesh
            .as_ref()
            .map_or((Vec3::ZERO, Vec3::ONE), |mesh| {
                let low = mesh
                    .points
                    .iter()
                    .copied()
                    .reduce(Vec3::min)
                    .unwrap_or(-Vec3::ONE);
                let high = mesh
                    .points
                    .iter()
                    .copied()
                    .reduce(Vec3::max)
                    .unwrap_or(Vec3::ONE);
                ((low + high) * 0.5, (high - low) * 0.5)
            });
        let center = domain.position + domain.rotation * (center * domain.half);
        let rotation = glam::Mat3::from_quat(domain.rotation);
        let half = extent * domain.half;
        let extent = (rotation.x_axis.abs() * half.x
            + rotation.y_axis.abs() * half.y
            + rotation.z_axis.abs() * half.z)
            .max(Vec3::splat(0.15));
        let center = center.clamp(ORIGIN + Vec3::splat(0.15), END - Vec3::splat(0.15));
        ((center - extent).max(ORIGIN), (center + extent).min(END))
    }

    pub fn gate_is_open(&self, frame: u32) -> bool {
        self.gate.is_some()
            && self
                .gate_frame
                .is_some_and(|trigger| frame >= trigger.saturating_add(self.gate_lift_frames))
    }

    pub fn at_frame(&self, frame: u32) -> std::borrow::Cow<'_, Self> {
        if !self
            .objects
            .iter()
            .any(|o| o.motion.is_some_and(|m| frame > m.start))
            && !self
                .gate_frame
                .is_some_and(|start| self.gate.is_some() && frame >= start)
        {
            return std::borrow::Cow::Borrowed(self);
        }
        let mut scene = self.clone();
        let gate_open = self.gate_is_open(frame);
        for object in &mut scene.objects {
            if let Some(m) = object.motion {
                let t = (frame.saturating_sub(m.start) as f32
                    / m.end.saturating_sub(m.start).max(1) as f32)
                    .min(1.0);
                let t = t * t * (3.0 - 2.0 * t);
                object.position += m.offset * t;
                object.rotation = Quat::from_rotation_z(m.tilt.to_radians() * t) * object.rotation;
            }
            if self.gate == Some(object.id) {
                if gate_open {
                    object.enabled = false;
                } else if let Some(start) = self.gate_frame
                    && self.gate_lift_frames != 0
                {
                    object.position.y +=
                        frame.saturating_sub(start) as f32 / self.gate_lift_frames as f32 * 1.34;
                }
            }
        }
        std::borrow::Cow::Owned(scene)
    }

    pub fn within_mesh_budget(&self) -> bool {
        self.objects
            .iter()
            .filter_map(|o| o.mesh.as_ref())
            .flat_map(|m| &m.faces)
            .map(|f| f.len().saturating_sub(2))
            .sum::<usize>()
            <= crate::mesh::MAX_TRIANGLES
    }

    pub fn validate(&mut self) -> Result<(), &'static str> {
        if !self.within_mesh_budget() {
            return Err("The scene exceeds the mesh triangle budget.");
        }
        if self.format != 1 || self.objects.len() > MAX_OBJECTS {
            return Err("Unsupported scene format or object count");
        }
        let mut ids = std::collections::BTreeSet::new();
        for o in &mut self.objects {
            if o.id == 0
                || !ids.insert(o.id)
                || !o.position.is_finite()
                || !o.half.is_finite()
                || !o.rotation.is_finite()
                || !o.radius.is_finite()
                || !o.speed.is_finite()
                || !o.flow.is_finite()
            {
                return Err("An object has invalid geometry or an invalid ID");
            }
            o.position = o
                .position
                .clamp(Vec3::new(-1.90, 0.05, -1.30), Vec3::new(1.90, 2.72, 1.30));
            o.half = o.half.clamp(
                Vec3::splat(if o.kind == Kind::Tank { 0.20 } else { 0.045 }),
                Vec3::new(1.85, 1.20, 1.25),
            );
            o.rotation = if o.rotation.length_squared() < 0.00001 {
                Quat::IDENTITY
            } else {
                o.rotation.normalize()
            };
            o.radius = o.radius.clamp(0.04, 0.55);
            o.flow = o.flow.clamp(0.0, 450.0);
            o.speed = o.speed.clamp(0.1, 6.0);
            o.name = o.name.chars().take(48).collect();
            if !o.schedule.ramp.is_finite() || o.schedule.liters.is_some_and(|v| !v.is_finite()) {
                return Err("Invalid flow schedule");
            }
            o.schedule.ramp = o.schedule.ramp.clamp(0.0, 120.0);
            o.schedule.liters = o.schedule.liters.map(|v| v.clamp(0.0, 30_000.0));
            o.schedule.stop = o.schedule.stop.map(|frame| frame.max(o.schedule.start));
            if !o.thickness.is_finite()
                || o.mesh
                    .as_ref()
                    .is_some_and(|m| !m.valid() || o.kind != Kind::Box)
            {
                return Err("Invalid mesh collider");
            }
            o.thickness = o.thickness.clamp(0.016, 0.30);
            if !o.field.strength.is_finite() || !o.field.radius.is_finite() {
                return Err("Invalid force field");
            }
            o.field.strength = o.field.strength.clamp(-60.0, 60.0);
            o.field.radius = o.field.radius.clamp(0.1, 5.0);
            if !o.wetting.is_finite()
                || o.motion
                    .is_some_and(|m| !m.offset.is_finite() || !m.tilt.is_finite())
            {
                return Err("Invalid collider physics settings");
            }
            o.wetting = o.wetting.clamp(0.0, 1.0);
            if let Some(m) = &mut o.motion {
                m.offset = m.offset.clamp(Vec3::splat(-3.0), Vec3::splat(3.0));
                m.tilt = m.tilt.clamp(-180.0, 180.0);
                m.end = m.end.max(m.start.saturating_add(1));
            }
        }
        if !self.gravity.is_finite() || self.fill_liters.is_some_and(|v| !v.is_finite()) {
            return Err("Invalid liquid quantity or gravity");
        }
        self.gravity = self.gravity.clamp(0.0, 30.0);
        self.fill_liters = self.fill_liters.map(|v| v.clamp(0.0, 30_000.0));
        if let Some(fill) = &mut self.fill {
            if !fill.min.is_finite() || !fill.max.is_finite() || fill.min.cmpgt(fill.max).any() {
                return Err("The initial fill bounds are invalid");
            }
            fill.min = fill.min.clamp(crate::fluid::ORIGIN, crate::fluid::END);
            fill.max = fill.max.clamp(crate::fluid::ORIGIN, crate::fluid::END);
        }
        let m = &mut self.liquid;
        if ![
            m.viscosity,
            m.cohesion,
            m.absorption,
            m.ior,
            m.surface_tension,
        ]
        .into_iter()
        .chain(m.tint)
        .all(f32::is_finite)
        {
            return Err("Invalid liquid material");
        }
        m.viscosity = m.viscosity.clamp(0.0, 1.0);
        m.cohesion = m.cohesion.clamp(0.0, 1.0);
        m.absorption = m.absorption.clamp(0.0, 8.0);
        m.ior = m.ior.clamp(1.0, 1.55);
        m.surface_tension = m.surface_tension.clamp(0.0, 1.0);
        self.gate_lift_frames = self.gate_lift_frames.min(900);
        if !self.whitewater.amount.is_finite() || !self.whitewater.lifetime.is_finite() {
            return Err("Invalid secondary particle settings");
        }
        self.whitewater.amount = self.whitewater.amount.clamp(0.0, 3.0);
        self.whitewater.lifetime = self.whitewater.lifetime.clamp(0.25, 3.0);
        m.tint = m.tint.map(|c| c.clamp(0.02, 1.0));
        let light = &mut self.lighting;
        if ![
            light.azimuth,
            light.elevation,
            light.power,
            light.environment_rotation,
            light.environment_exposure,
        ]
        .into_iter()
        .all(f32::is_finite)
        {
            return Err("Invalid studio light");
        }
        light.azimuth = light.azimuth.clamp(-180.0, 180.0);
        light.elevation = light.elevation.clamp(20.0, 90.0);
        light.power = light.power.clamp(0.0, 3.0);
        light.environment_rotation = light.environment_rotation.rem_euclid(360.0);
        light.environment_exposure = light.environment_exposure.clamp(-8.0, 8.0);
        light.environment_width = match light.environment_width {
            1024 => 1024,
            4096 => 4096,
            _ => 2048,
        };
        Ok(())
    }
}
