/*
    Avionics: the electronics between the sensors and the pilot. For now the head-up display (see hud.rs);
    later the MFDs, the radar, the fire control...
*/

use crate::prefabs::aircraft::avionics::hud::Hud;
use crate::prefabs::aircraft::sensors::sensors::Sensors;

pub struct Avionics {
    pub hud: Hud,
}

impl Avionics {
    pub fn new(hud: Hud) -> Avionics {
        Avionics { hud }
    }

    // Every physics step, after the sensors read
    pub fn update(&mut self, sensors: &Sensors) {
        self.hud.update(sensors);
    }
}
