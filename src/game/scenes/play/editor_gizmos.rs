//! What F10's editor draws over the world (see App::editor_lines): every
//! camera node's view (a pyramid out along where it looks, as wide as its
//! lens), the line from anything that faces a node to that node, the line
//! from every map node to its parent, the map's paths, and the selected
//! node's axes. Sizes are in screen pixels, so they read the same near and
//! far.

use nalgebra::Vector3;

use crate::engine::rendering::thick_lines::LineInstance;
use crate::engine::scene_manager::properties::{CameraLens, LookAt, Parent, Space, Transform3D};
use crate::engine::scene_manager::scene::Scene;

use super::map::MapSpec;
use super::tactical_map::{self, View};

const WIDTH: f32 = 2.0;
/// How long a camera's pyramid is, and the selected node's axes (px).
const FRUSTUM_PX: f32 = 70.0;
const AXES_PX: f32 = 45.0;

const CAMERA_COLOR: [f32; 4] = [0.45, 0.85, 1.0, 0.9];
const HIGHLIGHT_COLOR: [f32; 4] = [1.0, 0.82, 0.30, 1.0];
const LOOK_AT_COLOR: [f32; 4] = [0.95, 0.45, 0.95, 0.65];
const PARENT_COLOR: [f32; 4] = [0.75, 0.75, 0.75, 0.45];
const KEY_COLOR: [f32; 4] = [0.55, 1.0, 0.6, 0.85];
const GLOBAL_PARENT_COLOR: [f32; 4] = [0.75, 0.75, 0.75, 0.2];

fn line(lines: &mut Vec<LineInstance>, from: Vector3<f32>, to: Vector3<f32>, color: [f32; 4]) {
    lines.push(LineInstance { start: from.into(), end: to.into(), color, width: WIDTH });
}

/// Every line for this frame. `skip_camera`: the camera node being looked
/// through (the editor's own) - no pyramid round the eye. `highlighted`:
/// the selected node and the camera viewed through. `aspect`: the
/// screen's, for the pyramids' width.
#[allow(clippy::too_many_arguments)]
pub fn lines(scene: &Scene, map: &MapSpec, skip_camera: &str, selected: Option<&str>, highlighted: &[&str], view: &View, sea_level: f32, aspect: f32) -> Vec<LineInstance> {
    let mut lines = Vec::new();

    for path in &map.paths {
        tactical_map::flight_plan(&mut lines, &path.points, path.looped, None, sea_level, view);
    }

    for node in scene.content.nodes.iter() {
        let Some(transform) = node.get_property::<Transform3D>() else { continue };
        let position = transform.position;
        let lit = highlighted.contains(&node.id.as_str());

        if let (Some(lens), false) = (node.get_property::<CameraLens>(), node.id == skip_camera) {
            let length = view.meters_per_pixel(position) * FRUSTUM_PX;
            let half_height = (lens.fovy.to_radians() * 0.5).tan() * length;
            let half_width = half_height * aspect;
            let corner = |x: f32, y: f32| position + transform.rotation * Vector3::new(x * half_width, y * half_height, length);
            let corners = [corner(-1.0, -1.0), corner(1.0, -1.0), corner(1.0, 1.0), corner(-1.0, 1.0)];
            let color = if lit { HIGHLIGHT_COLOR } else { CAMERA_COLOR };
            for (index, &point) in corners.iter().enumerate() {
                line(&mut lines, position, point, color);
                line(&mut lines, point, corners[(index + 1) % 4], color);
            }
            // Which way's up - a little roof over the top edge.
            let roof = position + transform.rotation * Vector3::new(0.0, half_height * 1.5, length);
            line(&mut lines, corners[2], roof, color);
            line(&mut lines, corners[3], roof, color);
        }

        if let Some(target) = node.get_property::<LookAt>().and_then(|look_at| scene.world_transform(&look_at.target)) {
            line(&mut lines, position, target.position, LOOK_AT_COLOR);
        }

        if let (Some(parent), true) = (node.get_property::<Parent>(), map.node(&node.id).is_some()) {
            if let Some(parent_world) = scene.world_transform(&parent.id) {
                let color = if parent.space == Space::Global { GLOBAL_PARENT_COLOR } else { PARENT_COLOR };
                line(&mut lines, position, parent_world.position, color);
            }
        }

        if selected == Some(node.id.as_str()) {
            let length = view.meters_per_pixel(position) * AXES_PX;
            line(&mut lines, position, position + transform.rotation * Vector3::x() * length, [0.9, 0.33, 0.33, 1.0]);
            line(&mut lines, position, position + transform.rotation * Vector3::y() * length, [0.47, 0.78, 0.27, 1.0]);
            line(&mut lines, position, position + transform.rotation * Vector3::z() * length, [0.35, 0.55, 0.94, 1.0]);
        }
    }
    lines
}

/// An animation's camera key: a pyramid where it puts the camera, the way
/// it looks - lit when picked on the timeline.
pub fn camera_key(lines: &mut Vec<LineInstance>, position: Vector3<f32>, rotation: nalgebra::UnitQuaternion<f32>, fovy: f32, picked: bool, view: &View, aspect: f32) {
    let length = view.meters_per_pixel(position) * FRUSTUM_PX;
    let half_height = (fovy.to_radians() * 0.5).tan() * length;
    let half_width = half_height * aspect;
    let corner = |x: f32, y: f32| position + rotation * Vector3::new(x * half_width, y * half_height, length);
    let corners = [corner(-1.0, -1.0), corner(1.0, -1.0), corner(1.0, 1.0), corner(-1.0, 1.0)];
    let color = if picked { HIGHLIGHT_COLOR } else { KEY_COLOR };
    for (index, &point) in corners.iter().enumerate() {
        line(lines, position, point, color);
        line(lines, point, corners[(index + 1) % 4], color);
    }
}
