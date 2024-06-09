// the entity is the basic object on this "game engine project", it will have the values needed for our "GameObjects"
// the way we "render our objects its based on our object itself" so i will save that "render value" for later

// When i want to do other "element" i have to put this inside, since its the "shorter way" of adding the "basic position and dimensions data"
#[derive(Clone, Copy)]
pub struct GameObject {
    pub active: bool,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}