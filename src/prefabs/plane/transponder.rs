//! What an aircraft answers when a radar interrogates it (IFF) - the coalition it flies for.
//! Put on every plane the flight manager spawns.

/// The aircraft's IFF reply: the name of its coalition ("Blue", "Red"...).
#[derive(Clone, Debug)]
pub struct Transponder {
    pub coalition: String,
}
