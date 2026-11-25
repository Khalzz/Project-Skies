// The landing gear handle: the pilot's request, the landing gear decides if it can follow it
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum GearLever {
    Up,
    #[default]
    Down,
}

impl GearLever {
    pub fn toggle(&mut self) {
        *self = match self {
            GearLever::Up => GearLever::Down,
            GearLever::Down => GearLever::Up,
        };
    }
}
