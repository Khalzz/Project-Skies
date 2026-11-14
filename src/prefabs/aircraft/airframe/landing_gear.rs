#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum State {
    Up,
    Extending,
    Down,
    Retracting,
}

pub struct LandingGear {
    state: State,
}

impl LandingGear {
    fn new() -> LandingGear {
        LandingGear {}
    }
}
