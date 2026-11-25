/*
    Ground surfaces defines the way the rubber of the wheels will interact with each surface, mostly
    thinking about the surface definition on actions "rolling, brake and lateral friction"
*/
#[derive(Clone, Copy, Debug)]
pub struct GroundSurface {
    pub rolling_resistance: f32,
    pub breakaway_resistance: f32, // what a stopped tyre resists before it starts rolling
    pub brake_friction: f32,
    pub lateral_friction: f32,
}

/*
    This implementation defines for now the "runway" as the only surface so everything will
    behave like a runway
*/
impl GroundSurface {
    pub const RUNWAY: Self = Self {
        rolling_resistance: 0.02,
        // Tuned so idle thrust can't move a stopped, loaded F-16
        breakaway_resistance: 0.05,
        brake_friction: 0.5,
        lateral_friction: 0.8,
    };
}
