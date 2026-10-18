//! Whether a missile's path sets its fuze off against an aircraft's
//! colliders. The sphere is `fuze.proximity_radius` around the missile,
//! measured to the closest point on each collider along the step; a path
//! that enters a collider detonates at the entry even when that sphere
//! would miss a thin wing between samples. The earlier of the two along
//! the step wins.

use nalgebra::{UnitQuaternion, Vector3};

use crate::engine::game_nodes::game_object::ColliderType;

/// A collider the fuze can test, in the aircraft's frame. Cuboids and the
/// wing hulls (`Wing` resolved to a convex hull when the plane loads).
/// Anything else is left out - a flare has none, so it can't set a warhead off.
#[derive(Clone, Debug)]
pub enum FuzeCollider {
    Cuboid { half_extents: Vector3<f32>, position: Vector3<f32> },
    Hull { points: Vec<Vector3<f32>>, faces: Vec<Face> },
}

/// How far any of `colliders` reaches from the aircraft's origin (m). A
/// missile farther than this, plus its proximity radius, cannot touch one.
pub fn reach_of(colliders: &[FuzeCollider]) -> f32 {
    colliders.iter().fold(0.0, |farthest, collider| farthest.max(collider.reach()))
}

impl FuzeCollider {
    /// The fuze shape of a plane collider, when it's one the fuze understands.
    pub fn from_collider(collider: &ColliderType) -> Option<Self> {
        match collider {
            ColliderType::Cuboid { half_extents, position } => Some(Self::Cuboid {
                half_extents: Vector3::new(half_extents.0, half_extents.1, half_extents.2),
                position: Vector3::new(position.0, position.1, position.2),
            }),
            ColliderType::ConvexHull { points, .. } if points.len() >= 4 => Some(Self::hull(points.clone())),
            _ => None,
        }
    }

    /// A wing hull. Its faces are built once, here, and reused for every
    /// fuze step.
    pub fn hull(points: Vec<Vector3<f32>>) -> Self {
        let faces = hull_faces(&points);
        Self::Hull { points, faces }
    }

    /// How far this collider reaches from the aircraft's origin (m).
    pub fn reach(&self) -> f32 {
        match self {
            Self::Cuboid { half_extents, position } => {
                let mut farthest = 0.0_f32;
                for x in [-half_extents.x, half_extents.x] {
                    for y in [-half_extents.y, half_extents.y] {
                        for z in [-half_extents.z, half_extents.z] {
                            farthest = farthest.max((position + Vector3::new(x, y, z)).magnitude());
                        }
                    }
                }
                farthest
            }
            Self::Hull { points, .. } => points.iter().fold(0.0, |farthest, point| farthest.max(point.magnitude())),
        }
    }

    /// `t` along the segment (0 at `a`, 1 at `b`) where the missile detonates
    /// against this collider, if the sphere of `radius` reaches it or the
    /// segment enters it. The earlier of those.
    fn detonation_t(&self, a: Vector3<f32>, b: Vector3<f32>, radius: f32) -> Option<f32> {
        let (distance, t_closest, entry) = match self {
            Self::Cuboid { half_extents, position } => {
                let (distance, t_closest) = segment_aabb_closest(a - position, b - position, *half_extents);
                let entry = segment_enters_aabb(a - position, b - position, *half_extents);
                (distance, t_closest, entry)
            }
            Self::Hull { points, faces } => {
                let entry = segment_enters_hull(a, b, faces);
                let (distance, t_closest) = if entry.is_some() {
                    (0.0, entry.unwrap_or(0.0))
                } else {
                    segment_hull_closest(a, b, faces, points)
                };
                (distance, t_closest, entry)
            }
        };
        let mut t = (distance <= radius).then_some(t_closest);
        if let Some(entry) = entry {
            t = Some(t.map(|t| t.min(entry)).unwrap_or(entry));
        }
        t
    }
}

/// `t` along the missile's step (0 at `missile_start`, 1 at `missile_end`)
/// where it detonates against `colliders`, which ride on an aircraft moving
/// from `track_start` to `track_end` at a fixed `rotation`. The earliest
/// collider wins. `None`: the sphere misses every one and the path enters none.
pub fn detonation_t(
    rotation: &UnitQuaternion<f32>,
    track_start: Vector3<f32>,
    track_end: Vector3<f32>,
    missile_start: Vector3<f32>,
    missile_end: Vector3<f32>,
    colliders: &[FuzeCollider],
    radius: f32,
) -> Option<f32> {
    let inv = rotation.inverse();
    let local_start = inv * (missile_start - track_start);
    let local_end = inv * (missile_end - track_end);
    colliders.iter().filter_map(|collider| collider.detonation_t(local_start, local_end, radius)).min_by(|a, b| a.total_cmp(b))
}

fn point_aabb_distance(point: Vector3<f32>, half: Vector3<f32>) -> f32 {
    Vector3::new(
        (point.x.abs() - half.x).max(0.0),
        (point.y.abs() - half.y).max(0.0),
        (point.z.abs() - half.z).max(0.0),
    ).magnitude()
}

/// First `t` in 0..1 where the segment enters the box centered at the
/// origin. 0 when it starts inside.
fn segment_enters_aabb(a: Vector3<f32>, b: Vector3<f32>, half: Vector3<f32>) -> Option<f32> {
    let direction = b - a;
    let mut t_min = 0.0_f32;
    let mut t_max = 1.0_f32;
    for axis in 0..3 {
        if direction[axis].abs() < 1e-8 {
            if a[axis].abs() > half[axis] {
                return None;
            }
            continue;
        }
        let inverse = 1.0 / direction[axis];
        let mut near = (-half[axis] - a[axis]) * inverse;
        let mut far = (half[axis] - a[axis]) * inverse;
        if near > far {
            std::mem::swap(&mut near, &mut far);
        }
        t_min = t_min.max(near);
        t_max = t_max.min(far);
        if t_min > t_max {
            return None;
        }
    }
    if t_max < 0.0 || t_min > 1.0 {
        return None;
    }
    Some(t_min.clamp(0.0, 1.0))
}

/// Distance from the segment to the box, and the `t` of the closest point
/// on the segment. 0 at the entry when the segment hits the box.
fn segment_aabb_closest(a: Vector3<f32>, b: Vector3<f32>, half: Vector3<f32>) -> (f32, f32) {
    if let Some(t) = segment_enters_aabb(a, b, half) {
        return (0.0, t);
    }
    let direction = b - a;
    let mut splits = vec![0.0_f32, 1.0];
    for axis in 0..3 {
        if direction[axis].abs() < 1e-8 {
            continue;
        }
        for plane in [-half[axis], half[axis]] {
            let t = (plane - a[axis]) / direction[axis];
            if (0.0..=1.0).contains(&t) {
                splits.push(t);
            }
        }
    }
    splits.sort_by(|left, right| left.total_cmp(right));
    let mut unique = Vec::new();
    for t in splits {
        if unique.last().is_none_or(|last: &f32| (t - last).abs() > 1e-5) {
            unique.push(t);
        }
    }

    let mut best_distance = f32::MAX;
    let mut best_t = 0.0;
    let mut consider = |t: f32| {
        let t = t.clamp(0.0, 1.0);
        let distance = point_aabb_distance(a + direction * t, half);
        if distance < best_distance {
            best_distance = distance;
            best_t = t;
        }
    };
    for t in &unique {
        consider(*t);
    }
    for pair in unique.windows(2) {
        let mid = (pair[0] + pair[1]) * 0.5;
        let at_mid = a + direction * mid;
        consider(mid);
        let mut numerator = 0.0;
        let mut denominator = 0.0;
        for axis in 0..3 {
            if at_mid[axis].abs() > half[axis] {
                let sign = at_mid[axis].signum();
                numerator += direction[axis] * (a[axis] - sign * half[axis]);
                denominator += direction[axis] * direction[axis];
            }
        }
        if denominator > 1e-12 {
            let t = -numerator / denominator;
            if (pair[0]..=pair[1]).contains(&t) {
                consider(t);
            }
        }
    }
    (best_distance, best_t)
}

#[derive(Clone, Debug)]
pub struct Face {
    a: Vector3<f32>,
    b: Vector3<f32>,
    c: Vector3<f32>,
    /// Outward. Not a unit vector.
    normal: Vector3<f32>,
}

/// Triangles of the convex hull. A triplet is a face when every other point
/// is on one side of its plane (or on it).
fn hull_faces(points: &[Vector3<f32>]) -> Vec<Face> {
    let mut faces = Vec::new();
    let count = points.len();
    for i in 0..count {
        for j in (i + 1)..count {
            for k in (j + 1)..count {
                let (a, b, c) = (points[i], points[j], points[k]);
                let mut normal = (b - a).cross(&(c - a));
                if normal.norm_squared() < 1e-10 {
                    continue;
                }
                let mut positive = 0;
                let mut negative = 0;
                for (index, point) in points.iter().enumerate() {
                    if index == i || index == j || index == k {
                        continue;
                    }
                    let side = normal.dot(&(point - a));
                    if side > 1e-4 {
                        positive += 1;
                    } else if side < -1e-4 {
                        negative += 1;
                    }
                }
                if positive > 0 && negative > 0 {
                    continue;
                }
                if positive > 0 {
                    normal = -normal;
                    faces.push(Face { a, b: c, c: b, normal });
                } else {
                    faces.push(Face { a, b, c, normal });
                }
            }
        }
    }
    faces
}

fn inside_hull(point: Vector3<f32>, faces: &[Face]) -> bool {
    !faces.is_empty() && faces.iter().all(|face| face.normal.dot(&(point - face.a)) <= 1e-3)
}

fn segment_enters_hull(a: Vector3<f32>, b: Vector3<f32>, faces: &[Face]) -> Option<f32> {
    if inside_hull(a, faces) {
        return Some(0.0);
    }
    let direction = b - a;
    let mut best = None;
    for face in faces {
        if direction.dot(&face.normal) >= 0.0 {
            continue;
        }
        let Some(t) = ray_triangle(a, direction, face.a, face.b, face.c) else { continue };
        if (0.0..=1.0).contains(&t) {
            best = Some(best.map(|earlier: f32| earlier.min(t)).unwrap_or(t));
        }
    }
    best
}

fn segment_hull_closest(a: Vector3<f32>, b: Vector3<f32>, faces: &[Face], points: &[Vector3<f32>]) -> (f32, f32) {
    if faces.is_empty() {
        return points.iter().map(|point| segment_point(a, b, *point)).min_by(|left, right| left.0.total_cmp(&right.0)).unwrap_or((f32::MAX, 0.0));
    }
    let mut best = (f32::MAX, 0.0);
    for face in faces {
        let (distance, t) = segment_triangle(a, b, face.a, face.b, face.c);
        if distance < best.0 {
            best = (distance, t);
        }
    }
    best
}

fn segment_point(a: Vector3<f32>, b: Vector3<f32>, point: Vector3<f32>) -> (f32, f32) {
    let direction = b - a;
    let length_sq = direction.norm_squared();
    let t = if length_sq > 1e-12 { (point - a).dot(&direction) / length_sq } else { 0.0 }.clamp(0.0, 1.0);
    ((a + direction * t - point).magnitude(), t)
}

/// Distance from the segment to the triangle, and `t` on the segment.
fn segment_triangle(a: Vector3<f32>, b: Vector3<f32>, p0: Vector3<f32>, p1: Vector3<f32>, p2: Vector3<f32>) -> (f32, f32) {
    let direction = b - a;
    if let Some(t) = ray_triangle(a, direction, p0, p1, p2).filter(|t| (0.0..=1.0).contains(t)) {
        return (0.0, t);
    }
    let mut best_distance_sq = f32::MAX;
    let mut best_t = 0.0;
    for (point, t) in [(a, 0.0), (b, 1.0)] {
        let closest = closest_point_on_triangle(point, p0, p1, p2);
        let distance_sq = (point - closest).norm_squared();
        if distance_sq < best_distance_sq {
            best_distance_sq = distance_sq;
            best_t = t;
        }
    }
    for (edge_start, edge_end) in [(p0, p1), (p1, p2), (p2, p0)] {
        let (distance_sq, t) = segment_segment(a, b, edge_start, edge_end);
        if distance_sq < best_distance_sq {
            best_distance_sq = distance_sq;
            best_t = t;
        }
    }
    (best_distance_sq.sqrt(), best_t)
}

/// Möller–Trumbore. `t` is along `direction` (not normalized), so a segment
/// `a` to `a + direction` hits when `t` is in 0..1.
fn ray_triangle(origin: Vector3<f32>, direction: Vector3<f32>, v0: Vector3<f32>, v1: Vector3<f32>, v2: Vector3<f32>) -> Option<f32> {
    let edge1 = v1 - v0;
    let edge2 = v2 - v0;
    let p = direction.cross(&edge2);
    let determinant = edge1.dot(&p);
    if determinant.abs() < 1e-8 {
        return None;
    }
    let inverse = 1.0 / determinant;
    let to_origin = origin - v0;
    let u = to_origin.dot(&p) * inverse;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = to_origin.cross(&edge1);
    let v = direction.dot(&q) * inverse;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = edge2.dot(&q) * inverse;
    (t >= 0.0).then_some(t)
}

pub fn closest_point_on_triangle(point: Vector3<f32>, a: Vector3<f32>, b: Vector3<f32>, c: Vector3<f32>) -> Vector3<f32> {
    let ab = b - a;
    let ac = c - a;
    let ap = point - a;
    let d1 = ab.dot(&ap);
    let d2 = ac.dot(&ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    let bp = point - b;
    let d3 = ab.dot(&bp);
    let d4 = ac.dot(&bp);
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let v = d1 / (d1 - d3);
        return a + ab * v;
    }
    let cp = point - c;
    let d5 = ab.dot(&cp);
    let d6 = ac.dot(&cp);
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let w = d2 / (d2 - d6);
        return a + ac * w;
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        return b + (c - b) * w;
    }
    let inverse = 1.0 / (va + vb + vc);
    let v = vb * inverse;
    let w = vc * inverse;
    a + ab * v + ac * w
}

fn segment_segment(p1: Vector3<f32>, q1: Vector3<f32>, p2: Vector3<f32>, q2: Vector3<f32>) -> (f32, f32) {
    let d1 = q1 - p1;
    let d2 = q2 - p2;
    let r = p1 - p2;
    let a = d1.dot(&d1);
    let e = d2.dot(&d2);
    let f = d2.dot(&r);
    let (s, t) = if a <= 1e-8 && e <= 1e-8 {
        (0.0, 0.0)
    } else if a <= 1e-8 {
        (0.0, (f / e).clamp(0.0, 1.0))
    } else {
        let c = d1.dot(&r);
        if e <= 1e-8 {
            ((-c / a).clamp(0.0, 1.0), 0.0)
        } else {
            let b = d1.dot(&d2);
            let denom = a * e - b * b;
            let mut s = if denom > 1e-8 { ((b * f - c * e) / denom).clamp(0.0, 1.0) } else { 0.0 };
            let mut t = (b * s + f) / e;
            if t < 0.0 {
                t = 0.0;
                s = (-c / a).clamp(0.0, 1.0);
            } else if t > 1.0 {
                t = 1.0;
                s = ((b - c) / a).clamp(0.0, 1.0);
            }
            (s, t)
        }
    };
    let closest = p1 + d1 * s;
    let other = p2 + d2 * t;
    ((closest - other).norm_squared(), s)
}

/// Distance from `point` to the filled triangle.
pub fn point_triangle_distance(point: Vector3<f32>, a: Vector3<f32>, b: Vector3<f32>, c: Vector3<f32>) -> f32 {
    (point - closest_point_on_triangle(point, a, b, c)).magnitude()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_segment_passing_a_box_reports_the_gap() {
        let half = Vector3::new(1.0, 1.0, 1.0);
        let (distance, t) = segment_aabb_closest(Vector3::new(10.0, 0.0, -5.0), Vector3::new(10.0, 0.0, 5.0), half);
        // The gap stays 9 m for the whole stretch beside the box. The fuze
        // takes the first moment it gets that close.
        assert!((distance - 9.0).abs() < 1e-3, "{distance}");
        assert!((t - 0.4).abs() < 1e-3, "{t}");
    }

    #[test]
    fn a_segment_through_a_box_enters_at_the_face() {
        let half = Vector3::new(1.0, 1.0, 1.0);
        let t = segment_enters_aabb(Vector3::new(-10.0, 0.0, 0.0), Vector3::new(10.0, 0.0, 0.0), half).unwrap();
        assert!((t - 0.45).abs() < 1e-3, "{t}");
    }

    #[test]
    fn a_segment_through_a_thin_hull_detonates_on_the_near_face() {
        // A slab 0.2 m thick on z = 0. The step jumps from z = -2 to z = 2,
        // more than the slab, so an endpoint check would miss it.
        let mut points = Vec::new();
        for x in [-2.0_f32, 2.0] {
            for y in [-1.0_f32, 1.0] {
                for z in [-0.1_f32, 0.1] {
                    points.push(Vector3::new(x, y, z));
                }
            }
        }
        let collider = FuzeCollider::hull(points);
        let t = collider.detonation_t(Vector3::new(0.0, 0.0, -2.0), Vector3::new(0.0, 0.0, 2.0), 0.01).unwrap();
        let z = -2.0 + 4.0 * t;
        assert!((z + 0.1).abs() < 0.05, "entered at z {z}, t {t}");
    }
}
