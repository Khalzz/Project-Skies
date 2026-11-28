use std::f32::consts::{FRAC_PI_2, PI};

use nalgebra::UnitVector3;
use pankarta_software::physics::compute_principal_inertia;
use pankarta_software::prelude::*;

use crate::prefabs::aircraft::aircraft::Aircraft;
use crate::prefabs::aircraft::airframe::airframe::Mass;
use crate::prefabs::aircraft::airframe::fuselage::{Fuselage, FuselageAero};
use crate::prefabs::aircraft::airframe::landing_gear::LandingGear;
use crate::prefabs::aircraft::airframe::speedbrake::{Panel, Speedbrake};
use crate::prefabs::aircraft::flight_controls::controls::gear::GearLever;
use crate::prefabs::aircraft::airframe::wings::airfoil::Airfoil;
use crate::prefabs::aircraft::airframe::wings::body::{
    Hinge, Mix, Motion, Planform, Sections, SurfaceBody,
};
use crate::prefabs::aircraft::airframe::wings::surface::Surface;
use crate::prefabs::aircraft::airframe::Airframe;
use crate::prefabs::aircraft::avionics::avionics::Avionics;
use crate::prefabs::aircraft::avionics::hud::Hud;
use crate::prefabs::aircraft::flight_controls::control_system::ControlSystem;
use crate::prefabs::aircraft::flight_controls::fbw::FlyByWire;
use crate::prefabs::aircraft::flight_controls::flight_controls::FlightControls;
use crate::prefabs::aircraft::powerplant::powerplant::Powerplant;
use crate::prefabs::aircraft::powerplant::{Engine, EngineRole, Spool, ThrustMap, ThrustStage};
use crate::prefabs::aircraft::sensors::sensors::Sensors;
use crate::prefabs::camera::camera_view::{CameraView, Cameras};

impl Aircraft {
    pub fn f16() -> Aircraft {
        // GE F110-GE-129: idle ~4 kN, military (max dry) ~76 kN, afterburner from ~88 kN to ~129 kN.
        // Dry thrust grows faster near the top of the lever, so it takes two stages.
        let f110 = ThrustMap::new(vec![
            ThrustStage {
                lever: (0.0, 0.5),
                thrust: (0.03, 0.18),
                afterburner: false,
            },
            ThrustStage {
                lever: (0.5, 0.85),
                thrust: (0.18, 0.59),
                afterburner: false,
            },
            ThrustStage {
                lever: (0.85, 1.0),
                thrust: (0.68, 1.0),
                afterburner: true,
            },
        ]);
        let engines = vec![Engine::new(
            EngineRole::Main,
            Vector3::new(0.0, 0.077, 3.53),
            129_000.0,
            f110,
            None,
        )
        .spool_rates(Spool {
            up: 51_000.0,
            afterburner: 106_000.0,
            down: 90_000.0,
        })
        .flame("Afterburner")];

        // Wing
        let planform = Planform {
            root_chord: 4.85,
            tip_chord: 1.28,
            span: 4.5,
            sweep: 38.3,
        };

        let flaperon = Sections::new(
            (1.035, 3.99),
            Planform {
                root_chord: 0.79,
                tip_chord: 0.4,
                span: 2.46,
                sweep: 8.3,
            },
            Motion::Pivoting {
                hinge: Hinge::new(
                    None,
                    (21.5, 21.5),
                    Mix {
                        flaps: 1.0,
                        roll: 1.0,
                        ..Default::default()
                    },
                ),
            },
        );

        let slat = Sections::new(
            (1.4, 1.11),
            Planform {
                root_chord: 0.55,
                tip_chord: 0.18,
                span: 3.1,
                sweep: 38.3,
            },
            // Leading edge flaps: 25 deg down, 2 up, scheduled by the flight computer; the actuator
            // answers in 0.136 s, at most 25 deg/s (NASA TP-1538)
            Motion::Pivoting {
                hinge: Hinge::new(
                    None,
                    (25.0, 2.0),
                    Mix {
                        leading_edge: 1.0,
                        ..Default::default()
                    },
                )
                .with_rate(25.0)
                .with_lag(0.136),
            },
        )
        .on_leading_edge();

        let wing = |flaperon_mesh: &str| {
            SurfaceBody::new(planform, Motion::Fixed { sections: vec![] })
                .with_section(flaperon.clone().with_mesh(flaperon_mesh))
                .with_section(slat.clone())
        };

        // 2D sections; each surface corrects them for its own planform
        let naca_64206 = Airfoil::new("assets/planes/f16/aero/naca64206.ron".to_string());
        let naca_0009 = Airfoil::new("assets/planes/f16/aero/naca0009.ron".to_string());

        // The strakes' vortex lift, sized so the wing reaches CL ~1.0 at 12 deg and ~1.6 at 25 deg
        let strakes = 5.7;

        let left_wing = Surface::new(
            "Left Wing",
            100.0,
            wing("flaperon-l"),
            Vector3::new(0.0, 0.25, -4.77),
            -Vector3::x_axis(),
            naca_64206.clone(),
        )
        .with_vortex_lift(strakes)
        .with_thickness(0.04);

        let right_wing = Surface::new(
            "Right Wing",
            100.0,
            wing("flaperon-r"),
            Vector3::new(0.0, 0.25, -4.77),
            Vector3::x_axis(),
            naca_64206,
        )
        .with_vortex_lift(strakes)
        .with_thickness(0.04);

        let stabilator = |mesh: &str| {
            SurfaceBody::new(
                Planform {
                    root_chord: 3.078,
                    tip_chord: 0.9,
                    span: 2.8,
                    sweep: 40.0,
                },
                Motion::Pivoting {
                    hinge: Hinge::new(
                        Some(mesh.to_string()),
                        (25.0, 25.0),
                        Mix {
                            pitch: -1.0,
                            roll: 0.2,
                            ..Default::default()
                        },
                    ),
                },
            )
        };

        let anhedral = 10.0_f32.to_radians();
        let (sin, cos) = anhedral.sin_cos();

        let left_stabilator = Surface::new(
            "Left Stabilator",
            40.0,
            stabilator("stabilator-l"),
            Vector3::new(0.0, 0.25, 0.261),
            UnitVector3::new_normalize(Vector3::new(-cos, -sin, 0.0)),
            naca_0009.clone(),
        );

        let right_stabilator = Surface::new(
            "Right Stabilator",
            40.0,
            stabilator("stabilator-r"),
            Vector3::new(0.0, 0.25, 0.261),
            UnitVector3::new_normalize(Vector3::new(cos, -sin, 0.0)),
            naca_0009.clone(),
        );

        let rudder = Sections::new(
            (0.22, 2.147),
            Planform {
                root_chord: 0.566,
                tip_chord: 0.289,
                span: 1.892,
                sweep: 33.4,
            },
            Motion::Pivoting {
                hinge: Hinge::new(
                    Some("rudder".to_string()),
                    (30.0, 30.0),
                    Mix {
                        yaw: 1.0,
                        ..Default::default()
                    },
                )
                .with_rate(120.0),
            },
        );

        let vertical_stabilizer = Surface::new(
            "Vertical Stabilizer",
            40.0,
            SurfaceBody::new(
                Planform {
                    root_chord: 2.6,
                    tip_chord: 1.2,
                    span: 2.2,
                    sweep: 49.0,
                },
                Motion::Fixed { sections: vec![] },
            )
            .with_section(rudder),
            Vector3::new(0.0, 1.0, 0.35),
            Vector3::y_axis(),
            naca_0009,
        );

        let surfaces = vec![
            left_wing,
            right_wing,
            left_stabilator,
            right_stabilator,
            vertical_stabilizer,
        ];

        let empty_mass = 11900.0_f32; // this later should be replaced by the empty weight + pilot and the other weights come from fuel and weapons
        let center_of_mass = Vector3::new(0.0, 0.029, -2.244);

        let round = ((0.0, 0.0), (1.0, 1.0));
        let body_sections: [(Vector3<f32>, &[(f32, f32, (f32, f32), (f32, f32))]); 2] = [
            (
                Vector3::new(0.0, 0.06, 3.65),
                &[
                    (0.0, 0.45, round.0, round.1),
                    (0.65, 0.53, round.0, round.1),
                    (1.4, 0.66, round.0, round.1),
                    (2.4, 0.75, round.0, round.1),
                    (3.9, 0.79, round.0, round.1),
                    (7.65, 0.85, round.0, round.1),
                    (9.8, 0.82, round.0, round.1),
                ],
            ),
            (
                Vector3::new(0.0, 0.0, -6.15),
                &[
                    (0.0, 0.74, (0.0, -0.215), (1.0, 0.59)),
                    (0.95, 0.68, (0.0, -0.19), (1.0, 0.65)),
                    (1.55, 0.62, (0.0, -0.135), (1.0, 0.7)),
                    (2.15, 0.54, (0.0, -0.06), (1.0, 0.74)),
                    (2.55, 0.48, (0.0, 0.0), (1.0, 0.74)),
                    (3.15, 0.32, (0.0, 0.08), (1.0, 0.72)),
                    (3.75, 0.12, (0.0, 0.165), (1.0, 0.71)),
                    (4.42, 0.0, (0.0, 0.185), (1.0, 1.0)),
                ],
            ),
        ];
        let along_the_nose = (UnitQuaternion::from_axis_angle(&Vector3::y_axis(), PI)
            * UnitQuaternion::from_axis_angle(&Vector3::x_axis(), FRAC_PI_2))
        .scaled_axis();

        let colliders =
            body_sections
                .iter()
                .fold(Colliders::new(), |colliders, &(tail_end, rings)| {
                    let profile = rings.iter().fold(
                        Mesh::profile_shape(16),
                        |profile, &(at, radius, offset, stretch)| {
                            profile
                                .ring(at, radius)
                                .offset(offset.0, offset.1)
                                .stretch(stretch.0, stretch.1)
                        },
                    );
                    colliders.add(
                        Collider::from_mesh(&profile, ModelShape::ConvexHull)
                            .translation(tail_end)
                            .rotation(along_the_nose)
                            .friction(0.8)
                            .density(0.0),
                    )
                });
        let section_boxes: Vec<ColliderBuilder> = body_sections
            .iter()
            .map(|&(tail_end, rings)| {
                let length = rings.last().map_or(0.0, |&(at, ..)| at);
                let (low, high) = rings.iter().fold(
                    (Vector3::repeat(f32::MAX), Vector3::repeat(f32::MIN)),
                    |(low, high), &(_, radius, offset, stretch)| {
                        let middle = Vector3::new(offset.0, -offset.1, 0.0);
                        let reach = Vector3::new(radius * stretch.0, radius * stretch.1, 0.0);
                        (low.inf(&(middle - reach)), high.sup(&(middle + reach)))
                    },
                );
                let half = (high - low) / 2.0;
                ColliderBuilder::cuboid(half.x, half.y, length / 2.0)
                    .translation(tail_end + (high + low) / 2.0 - Vector3::z() * (length / 2.0))
            })
            .collect();
        let inertia = compute_principal_inertia(empty_mass, center_of_mass, &section_boxes);

        // Speedbrakes: a clamshell each side of the nozzle, between it and the stabilators; the upper
        // petal opens up, the lower one down, 60 deg in about two seconds. ~1.4 m² between the four.
        // The model has no meshes for them yet, so they're only seen in the gizmos.
        let petal = |side: f32, up: bool| Panel {
            hinge: Vector3::new(side * 0.72, 0.0, 2.75),
            // Turning about -x lifts the trailing edge, about +x lowers it
            axis: if up { -Vector3::x_axis() } else { Vector3::x_axis() },
            chord: Vector3::new(0.0, 0.0, 0.85),
            width: 0.4,
            mesh: None,
        };
        let speedbrake = Speedbrake::new(
            vec![
                petal(-1.0, true),
                petal(-1.0, false),
                petal(1.0, true),
                petal(1.0, false),
            ],
            60.0,
            30.0,
        );

        let airframe = Airframe {
            landing_gear: LandingGear::new(GearLever::Down),
            surfaces,
            mass: Mass {
                empty: empty_mass,
                center_of_mass,
                inertia,
            },
            fuselage: Fuselage {
                colliders,
                // Drag of what the wings don't count: the whole jet's ~0.014 × 27.87 m² on top of the wings'
                aero: FuselageAero::from_rings(&body_sections, 0.4),
            },
            speedbrake,
            aero: vec![],
        };

        // Cameras
        let cameras = Cameras {
            first_person: vec![CameraView {
                offset: Vector3::new(0.0, 0.97, -6.485),
                look_at: Vector3::new(0.0, 0.97, -20.0), // straight ahead over the nose
                fov: 70.0,
            }],
            third_person: vec![CameraView {
                offset: Vector3::new(0.0, 3.85, 23.77),
                look_at: center_of_mass,
                fov: 45.0,
            }],
            cinematic: vec![
                // Low under the nose, off to the left, looking back up at the intake
                CameraView {
                    offset: Vector3::new(-2.5, -1.2, -12.0),
                    look_at: Vector3::new(0.0, -0.3, -6.0), // the intake
                    fov: 50.0,
                },
                // Just past the right wingtip, looking in at the canopy
                CameraView {
                    offset: Vector3::new(5.5, 0.6, -0.5),
                    look_at: Vector3::new(0.0, 0.97, -6.485), // the pilot's seat
                    fov: 55.0,
                },
                // Above and behind the tail, looking forward along the spine
                CameraView {
                    offset: Vector3::new(0.0, 4.0, 6.0),
                    look_at: Vector3::new(0.0, 0.5, -6.0), // down the spine to the canopy
                    fov: 50.0,
                },
                // Far out ahead and to the right, a flyby
                CameraView {
                    offset: Vector3::new(14.0, 5.0, -30.0),
                    look_at: center_of_mass,
                    fov: 35.0,
                },
            ],
        };
        // Cameras

        Aircraft {
            player: true,
            airframe,
            avionics: Avionics::new(Hud::f16()),
            powerplant: Powerplant::new(engines),
            flight_controls: FlightControls::new(
                0.0,
                // 9 g pulling, -3 pushing, 25 deg angle of attack, 300 deg/s roll
                ControlSystem::FlyByWire(FlyByWire::new((-3.0, 9.0), 25.0, 300f32.to_radians())),
            ),
            cameras,
            sensors: Sensors::default(),
            debug: false,
            chocks_placed: false,
        }
    }
}
