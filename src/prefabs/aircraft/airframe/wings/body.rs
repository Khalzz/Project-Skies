/*
    Body its the definition of the body of the wing, related to his sizing itself, and somehow defining his
    "control areas", body mosty will contain a shape but also "sections", subshapes, this subshapes contain
    a logic defined by the type they contain and their configurations


*/

use nalgebra::{UnitQuaternion, UnitVector3};
use pankarta_software::scene_manager::node::properties::Model;

use crate::prefabs::aircraft::flight_controls::flight_controls::Commands;

// The outline of a surface seen from above, measured in meters from the root
#[derive(Clone, Copy)]
pub struct Planform {
    pub root_chord: f32,
    pub tip_chord: f32,
    pub span: f32,
    pub sweep: f32, // leading edge angle, in degrees
}

// Defines the shape of the planform
impl Planform {
    pub fn corners(&self) -> [(f32, f32); 4] {
        let tip_back = self.span * self.sweep.to_radians().tan();
        [
            (0.0, 0.0),                             // root leading edge
            (self.span, tip_back),                  // tip leading edge
            (self.span, tip_back + self.tip_chord), // tip trailing edge
            (0.0, self.root_chord),                 // root trailing edge
        ]
    }

    pub fn area(&self) -> f32 {
        (self.root_chord + self.tip_chord) / 2.0 * self.span
    }

    // A panel on the fuselage acts like it continues into its mirror image, so it's counted as both
    pub fn aspect_ratio(&self) -> f32 {
        2.0 * self.span * self.span / self.area()
    }

    // Sweep of the line halfway back along the chord, in radians
    pub fn half_chord_sweep(&self) -> f32 {
        let taper = self.tip_chord / self.root_chord;
        let tan = self.sweep.to_radians().tan()
            - 2.0 / self.aspect_ratio() * (1.0 - taper) / (1.0 + taper);
        tan.atan()
    }

    pub fn chord_at(&self, along_span: f32) -> f32 {
        let t = (along_span / self.span).clamp(0.0, 1.0);
        self.root_chord + (self.tip_chord - self.root_chord) * t
    }

    // The chord the whole surface flies like, and where along the span it sits
    pub fn mean_chord(&self) -> (f32, f32) {
        let taper = self.tip_chord / self.root_chord;
        let chord = 2.0 / 3.0 * self.root_chord * (1.0 + taper + taper * taper) / (1.0 + taper);
        let along = self.span / 3.0 * (1.0 + 2.0 * taper) / (1.0 + taper);
        (chord, along)
    }

    // Where its lift acts: a quarter of the way back along the mean chord
    pub fn aerodynamic_center(&self) -> (f32, f32) {
        let (chord, along) = self.mean_chord();
        let leading_edge = along * self.sweep.to_radians().tan();
        (along, leading_edge + chord / 4.0)
    }

    // The average of the four corners
    pub fn middle(&self) -> (f32, f32) {
        let corners = self.corners();
        let (span, back) = corners.iter().fold((0.0, 0.0), |(span, back), corner| {
            (span + corner.0, back + corner.1)
        });
        (span / corners.len() as f32, back / corners.len() as f32)
    }
}
#[derive(Clone, Copy, Default)]
pub struct Mix {
    pub pitch: f32,
    pub roll: f32,
    pub yaw: f32,
    pub flaps: f32,
}

impl Mix {
    // The commands it follows, e.g. "roll + flaps"; "passive" when it follows none
    pub fn describe(&self) -> String {
        let followed: Vec<&str> = [
            ("pitch", self.pitch),
            ("roll", self.roll),
            ("yaw", self.yaw),
            ("flaps", self.flaps),
        ]
        .iter()
        .filter(|(_, weight)| *weight != 0.0)
        .map(|(name, _)| *name)
        .collect();

        if followed.is_empty() {
            "passive".to_string()
        } else {
            followed.join(" + ")
        }
    }

    pub fn is_passive(&self) -> bool {
        self.pitch == 0.0 && self.roll == 0.0 && self.yaw == 0.0 && self.flaps == 0.0
    }
}

const DEFAULT_ACTUATOR_RATE: f32 = 60.0; // deg/s, a fighter's stabilator or flaperon

#[derive(Clone)]
pub struct Hinge {
    pub mesh: Option<String>,
    pub travel: (f32, f32), // max (Down, up) in degrees angle of the section
    pub deflection: f32,
    pub mix: Mix,
    pub rate: f32, // degrees per second its actuator can move it
}

impl Hinge {
    pub fn new(mesh: Option<String>, travel: (f32, f32), mix: Mix) -> Hinge {
        Hinge {
            mesh,
            travel,
            deflection: 0.0,
            mix,
            rate: DEFAULT_ACTUATOR_RATE,
        }
    }

    pub fn with_rate(mut self, rate: f32) -> Hinge {
        self.rate = rate;
        self
    }

    pub fn command(&mut self, commands: &Commands, side: f32, dt: f32) {
        let m = self.mix;

        // Control first: pitch, roll and yaw always get the travel they ask for
        let control = (m.pitch * commands.pitch
            + m.roll * commands.roll * side
            + m.yaw * commands.yaw)
            .clamp(-1.0, 1.0);

        // Flaps only take the travel control left free, so a flaperon with full flaps can still roll
        // both ways instead of sitting on its limit (each weight: 1 = follows it fully, 0 = ignores it)
        let room = 1.0 - control.abs();
        let value = control + (m.flaps * commands.flaps).clamp(-room, room);

        // asymmetric travel: + uses the down limit, - uses the up limit
        let target = if value >= 0.0 {
            value * self.travel.0
        } else {
            value * self.travel.1
        };
        let step = self.rate * dt;
        self.deflection += (target - self.deflection).clamp(-step, step);
    }

    // `axis` is in the aircraft's space, so the turn goes on top of the mesh's own rotation
    pub fn place(&self, model: &mut Model, axis: UnitVector3<f32>) {
        let Some(name) = &self.mesh else { return };
        if let Some(mesh) = model.mesh_mut(name) {
            let base = mesh.original_transform().rotation;
            mesh.transform.rotation =
                UnitQuaternion::from_axis_angle(&axis, self.deflection.to_radians()) * base;
        }
    }
}

#[derive(Clone)]
pub enum Motion {
    Fixed {
        // when the surface its fixed (like a stabilizer or a wing) it will allow adding inner sections
        sections: Vec<Sections>,
    },
    Pivoting {
        hinge: Hinge,
    },
}

impl Motion {
    // The hinge, if it moves; None for a fixed surface
    pub fn hinge_mut(&mut self) -> Option<&mut Hinge> {
        match self {
            Motion::Pivoting { hinge } => Some(hinge),
            Motion::Fixed { .. } => None,
        }
    }

    pub fn with_mesh(mut self, mesh: &str) -> Motion {
        match self.hinge_mut() {
            Some(hinge) => hinge.mesh = Some(mesh.to_string()),
            None => panic!("a fixed surface has no hinge to give a mesh"),
        }
        self
    }
}

#[derive(Clone)]
pub struct Sections {
    root: (f32, f32), // where its root leading edge sits on its surface: (along the span, back)
    planform: Planform,
    pub motion: Motion,
}

impl Sections {
    pub fn new(root: (f32, f32), planform: Planform, motion: Motion) -> Sections {
        Sections {
            root,
            planform,
            motion,
        }
    }

    pub fn with_mesh(mut self, mesh: &str) -> Sections {
        self.motion = self.motion.with_mesh(mesh);
        self
    }

    // Its corners in its surface's (along the span, back)
    pub fn corners(&self) -> [(f32, f32); 4] {
        self.planform
            .corners()
            .map(|(span, back)| (self.root.0 + span, self.root.1 + back))
    }

    pub fn area(&self) -> f32 {
        self.planform.area()
    }

    // Where its share of the lift acts, in its surface's (along the span, back)
    pub fn center(&self) -> (f32, f32) {
        let (along, back) = self.planform.middle();
        (self.root.0 + along, self.root.1 + back)
    }

    // How deep it is compared to its surface's chord where it sits
    pub fn chord_ratio(&self, surface: &Planform) -> f32 {
        let (along, _) = self.center();
        let chord = self.area() / self.planform.span;
        (chord / surface.chord_at(along)).clamp(0.0, 1.0)
    }
}

/*
    A surface body its the main component that generates the lift,
    a wing, a stabilizer, a stabilator, a canard, etc...

    Some of them like the wings contain inner sections: (flaps, slats, ailerons)
    and others dont: (stabilators or cannards)

    So this setting its defined mainly in that way
*/

#[derive(Clone)]
pub struct SurfaceBody {
    pub planform: Planform,
    pub motion: Motion,
}

impl SurfaceBody {
    pub fn new(planform: Planform, motion: Motion) -> SurfaceBody {
        SurfaceBody { planform, motion }
    }

    pub fn with_section(mut self, section: Sections) -> SurfaceBody {
        match &mut self.motion {
            Motion::Fixed { sections } => sections.push(section),
            Motion::Pivoting { .. } => panic!("a pivoting surface can't hold sections"),
        }
        self
    }
}
