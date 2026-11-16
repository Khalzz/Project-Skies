#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GearState {
    Up,
    Extending,
    Down,
    Retracting,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GearLever {
    Up,
    Down,
}

/*
    This allows me to define the .into so from gear lever we can get a landing state
    avoiding having the possibility of starting with the landing gear "extending or retracting"
*/
impl From<GearLever> for GearState {
    fn from(lever: GearLever) -> Self {
        match lever {
            GearLever::Up => GearState::Up,
            GearLever::Down => GearState::Down,
        }
    }
}

pub struct LandingGear {
    gear_state: GearState, // The real airframe actual state (up, ext, dwn, rtr)
    gear_lever: GearLever, // The landing gear lever inside the cockpit (the control)
    gear_counter: f32,
}

impl LandingGear {
    pub fn new(state: GearLever) -> LandingGear {
        LandingGear {
            gear_state: state.into(),
            gear_lever: state,
            gear_counter: 0.0,
        }
    }

    pub fn up(&mut self) {
        // If the landing gear its already up dont do anything
        if self.gear_lever == GearLever::Up {
            return;
        }

        self.gear_lever = GearLever::Up;
        self.gear_state = GearState::Retracting;
    }

    pub fn down(&mut self) {
        if self.gear_lever == GearLever::Down {
            return;
        }

        self.gear_lever = GearLever::Down;
        self.gear_state = GearState::Extending;
    }

    pub fn toggle(&mut self) {
        match self.gear_lever {
            GearLever::Up => {
                // If the landing gear its already up, extend it
                self.gear_state = GearState::Extending;
                self.gear_lever = GearLever::Down
            }
            GearLever::Down => {
                // If the landing gear its already down, retract it
                self.gear_state = GearState::Retracting;
                self.gear_lever = GearLever::Up
            }
        }
    }

    pub fn update(&mut self) {}
}
