/*
    Camera view its the "definitor of settings" for all the cameras surrounding a unit, units define their cameras
    in a list, then when the camera has that unit selected it will go through a selection of cameras and then it
    will handle which one of them to use
*/

use nalgebra::Vector3;

// Both points in the unit's own frame (nose -Z, +X right, +Y up)
#[derive(Clone, Copy)]
pub struct CameraView {
    pub offset: Vector3<f32>,
    pub look_at: Vector3<f32>,
    pub fov: f32,
}

// THIS STRUCT SHOULD ONLY BE USED TO SETUP THE CAMERAS OF EACH ELEMENT
pub struct Cameras {
    pub first_person: Vec<CameraView>,
    pub third_person: Vec<CameraView>,
    pub cinematic: Vec<CameraView>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CameraKind {
    FirstPerson,
    ThirdPerson,
    Cinematic,
}

impl Cameras {
    pub fn of(&self, kind: CameraKind) -> &[CameraView] {
        match kind {
            CameraKind::FirstPerson => &self.first_person,
            CameraKind::ThirdPerson => &self.third_person,
            CameraKind::Cinematic => &self.cinematic,
        }
    }
}
