/*
    The flaps are a small section of the wings designated to "generate lift at cost of more drag". In the cockpit
    they're only a request (0 up .. 1 fully down), whatever the plane's hardware is: a lever with detents, a button,
    or nothing at all when a flight computer sets them (the F-16, with its ALT FLAPS switch to force them down).
    How far that moves each surface is the surface's own travel.
*/

pub struct Flaps {
    pub request: f32,
    pub alt_flaps: bool,
}
