//! The level's solid ground - land and runway - as triangles on the main
//! thread, for things that aren't physics bodies (missiles) to hit. The
//! physics thread has its own copy for the aircraft (see play::scene::
//! spawn_world); this one is built from the same meshes.
//!
//! Triangles are sorted into a grid of square cells across the map (seen
//! from above), each cell also knowing its highest point - so a segment
//! well up in the sky is turned away without looking at a single triangle.

use std::collections::HashMap;

use nalgebra::Vector3;

/// A cell's width (m).
const CELL_SIZE: f32 = 200.0;

#[derive(Default)]
pub struct WorldCollision {
    triangles: Vec<[Vector3<f32>; 3]>,
    /// The triangles touching each cell (by index), and the cell's highest
    /// point among them.
    cells: HashMap<(i32, i32), (Vec<u32>, f32)>,
}

fn cell_of(x: f32, z: f32) -> (i32, i32) {
    ((x / CELL_SIZE).floor() as i32, (z / CELL_SIZE).floor() as i32)
}

impl WorldCollision {
    /// Adds a mesh (its `vertices` and triangle `indices`, as loaded) placed
    /// at `position`, scaled by `scale` - like the node it's drawn on.
    pub fn add_mesh(&mut self, vertices: &[Vector3<f32>], indices: &[[u32; 3]], position: Vector3<f32>, scale: Vector3<f32>) {
        let place = |vertex: &Vector3<f32>| position + vertex.component_mul(&scale);
        for triangle in indices {
            let corners = triangle.map(|index| place(&vertices[index as usize]));
            self.add_triangle(corners);
        }
    }

    fn add_triangle(&mut self, corners: [Vector3<f32>; 3]) {
        let index = self.triangles.len() as u32;
        let min = cell_of(corners.iter().map(|c| c.x).fold(f32::MAX, f32::min), corners.iter().map(|c| c.z).fold(f32::MAX, f32::min));
        let max = cell_of(corners.iter().map(|c| c.x).fold(f32::MIN, f32::max), corners.iter().map(|c| c.z).fold(f32::MIN, f32::max));
        let top = corners.iter().map(|c| c.y).fold(f32::MIN, f32::max);
        for x in min.0..=max.0 {
            for z in min.1..=max.1 {
                let cell = self.cells.entry((x, z)).or_insert((Vec::new(), f32::MIN));
                cell.0.push(index);
                cell.1 = cell.1.max(top);
            }
        }
        self.triangles.push(corners);
    }

    /// Where the segment from `start` to `end` first meets the ground -
    /// `None` if it doesn't.
    pub fn segment_hit(&self, start: Vector3<f32>, end: Vector3<f32>) -> Option<Vector3<f32>> {
        let lowest = start.y.min(end.y);
        let min = cell_of(start.x.min(end.x), start.z.min(end.z));
        let max = cell_of(start.x.max(end.x), start.z.max(end.z));
        let direction = end - start;
        let mut first: Option<f32> = None;
        for x in min.0..=max.0 {
            for z in min.1..=max.1 {
                let Some((triangles, top)) = self.cells.get(&(x, z)) else { continue };
                if lowest > *top {
                    continue;
                }
                for &index in triangles {
                    if let Some(t) = segment_triangle(start, direction, &self.triangles[index as usize]) {
                        if first.is_none_or(|first| t < first) {
                            first = Some(t);
                        }
                    }
                }
            }
        }
        first.map(|t| start + direction * t)
    }
}

/// Where (0..1 along `direction` from `start`) a segment crosses a triangle,
/// from either side - Möller-Trumbore.
fn segment_triangle(start: Vector3<f32>, direction: Vector3<f32>, [a, b, c]: &[Vector3<f32>; 3]) -> Option<f32> {
    let edge1 = b - a;
    let edge2 = c - a;
    let p = direction.cross(&edge2);
    let determinant = edge1.dot(&p);
    if determinant.abs() < 1e-9 {
        return None;
    }
    let inverse = 1.0 / determinant;
    let offset = start - a;
    let u = offset.dot(&p) * inverse;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = offset.cross(&edge1);
    let v = direction.dot(&q) * inverse;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = edge2.dot(&q) * inverse;
    (0.0..=1.0).contains(&t).then_some(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A square hill top 1 km across, 300 m up.
    fn plateau() -> WorldCollision {
        let mut world = WorldCollision::default();
        let vertices = [Vector3::new(-0.5, 0.3, -0.5), Vector3::new(0.5, 0.3, -0.5), Vector3::new(0.5, 0.3, 0.5), Vector3::new(-0.5, 0.3, 0.5)];
        world.add_mesh(&vertices, &[[0, 1, 2], [0, 2, 3]], Vector3::zeros(), Vector3::new(1000.0, 1000.0, 1000.0));
        world
    }

    #[test]
    fn a_dive_into_it_hits_where_it_crosses() {
        let hit = plateau().segment_hit(Vector3::new(10.0, 305.0, 20.0), Vector3::new(10.0, 295.0, 22.0)).expect("hits");
        assert!((hit - Vector3::new(10.0, 300.0, 21.0)).norm() < 1e-3, "{hit:?}");
    }

    #[test]
    fn above_beside_or_short_of_it_misses() {
        let world = plateau();
        assert_eq!(world.segment_hit(Vector3::new(0.0, 400.0, 0.0), Vector3::new(4.0, 399.0, 0.0)), None);
        assert_eq!(world.segment_hit(Vector3::new(800.0, 305.0, 0.0), Vector3::new(800.0, 295.0, 0.0)), None);
        assert_eq!(world.segment_hit(Vector3::new(0.0, 310.0, 0.0), Vector3::new(0.0, 301.0, 0.0)), None);
    }
}
