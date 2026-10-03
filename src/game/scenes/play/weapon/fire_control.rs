//! The player's fire control - where the selected missile's seeker is
//! looking, and what the pilot's locked it on.
//!
//! With a guided missile selected (see play::scene's weapon list):
//! - unlocked, the seeker rests on the nose (boresight - the diamond on the
//!   HUD);
//! - the pilot locks it (`lock`) on the enemy in its small field of view
//!   there, inside its range - the nearest, if there are several;
//! - the lock then holds wherever the target goes, inside the gimbal limit
//!   and the seeker's range - even going down in flames, its heat still
//!   there - until the pilot lets it go (`unlock`) or it's lost. Missiles
//!   already fired home on their own and don't care.
//!
//! Nothing selected (or a weapon without a seeker): nothing's locked.
//!
//! Plain numbers in and out - nothing of the scene's - so it's tested on
//! its own (below); play::scene feeds it the aircraft every frame.

use nalgebra::{UnitQuaternion, Vector3};

use super::weapon_spec::Guidance;

/// Where the player's jet is and how it's moving.
#[derive(Clone, Copy, Debug)]
pub struct OwnShip {
    pub position: Vector3<f32>,
    pub rotation: UnitQuaternion<f32>,
    pub velocity: Vector3<f32>,
}

impl OwnShip {
    /// Where its nose points.
    pub fn boresight(&self) -> Vector3<f32> {
        self.rotation * Vector3::z()
    }
}

/// An enemy aircraft the systems can see.
#[derive(Clone, Debug)]
pub struct Contact {
    pub name: String,
    pub position: Vector3<f32>,
    pub velocity: Vector3<f32>,
    /// Shot down and falling - still hot, so a lock holds on it, but none's
    /// taken.
    pub falling: bool,
}

/// How wide the seeker sees (degrees off where it's looking) - what's this
/// close to the nose can be locked.
pub const SEEKER_FIELD_DEG: f32 = 2.5;
/// How fast the seeker head turns (degrees a second).
const SEEKER_SLEW_DEG_S: f32 = 120.0;
/// Locked: the seeker's on its target, this close (degrees).
const ON_TARGET_DEG: f32 = 1.0;
/// Closer than this the missile can't arm and turn in time (m).
pub const MIN_RANGE: f32 = 300.0;

pub struct FireControl {
    /// Where the seeker's looking, a world direction.
    pub seeker_direction: Vector3<f32>,
    /// What the pilot's locked the seeker on, by name.
    pub seeker_target: Option<String>,
}

impl FireControl {
    pub fn new() -> Self {
        Self { seeker_direction: Vector3::z(), seeker_target: None }
    }

    /// Lets go of the lock - the pilot's unlocked, or another weapon was
    /// selected.
    pub fn drop_locks(&mut self) {
        self.seeker_target = None;
    }

    /// Locks the seeker (`seeker` - the selected missile's) on the enemy on
    /// the nose: inside its field of view there and its range, the nearest
    /// - not one already going down. Whether it found one.
    pub fn lock(&mut self, own: &OwnShip, contacts: &[Contact], seeker: &Guidance) -> bool {
        self.seeker_target = on_the_nose(own, contacts, seeker);
        self.seeker_target.is_some()
    }

    /// Whether the seeker is locked on - tracking a target and looking right
    /// at it. Its name.
    pub fn locked(&self, contacts: &[Contact], own: &OwnShip) -> Option<&str> {
        let name = self.seeker_target.as_deref()?;
        let contact = contacts.iter().find(|contact| contact.name == name)?;
        let line_of_sight = contact.position - own.position;
        (self.seeker_direction.angle(&line_of_sight).to_degrees() <= ON_TARGET_DEG).then_some(name)
    }

    /// One frame of the seeker (`seeker` - the selected missile's; `None`
    /// without one, and then nothing's locked) - `contacts` are the enemies.
    pub fn update(&mut self, own: &OwnShip, contacts: &[Contact], seeker: Option<&Guidance>, delta_time: f32) {
        let boresight = own.boresight();
        let Some(seeker) = seeker else {
            self.drop_locks();
            self.slew(boresight, delta_time);
            return;
        };
        // A lock holds while the target stays in view and in range -
        // otherwise it's lost.
        let target = self.seeker_target.as_ref().and_then(|name| contacts.iter().find(|contact| contact.name == *name));
        let in_view = |contact: &&Contact| {
            let line_of_sight = contact.position - own.position;
            boresight.angle(&line_of_sight).to_degrees() <= seeker.gimbal_limit_deg && line_of_sight.magnitude() <= seeker.lock_range
        };
        let target = target.filter(in_view);
        if target.is_none() {
            self.seeker_target = None;
        }
        // Where it looks: at its target, or the nose.
        let cue = target.map(|contact| (contact.position - own.position).normalize()).unwrap_or(boresight);
        self.slew(cue, delta_time);
    }

    /// Turns the seeker toward `direction`, at its slew rate.
    fn slew(&mut self, direction: Vector3<f32>, delta_time: f32) {
        let angle = self.seeker_direction.angle(&direction);
        let step = (SEEKER_SLEW_DEG_S * delta_time).to_radians();
        if angle <= step || angle < 1e-5 {
            self.seeker_direction = direction;
            return;
        }
        let axis = self.seeker_direction.cross(&direction);
        let rotation = match nalgebra::Unit::try_new(axis, 1e-6) {
            Some(axis) => UnitQuaternion::from_axis_angle(&axis, step),
            // Straight behind: any way round.
            None => UnitQuaternion::from_axis_angle(&Vector3::y_axis(), step),
        };
        self.seeker_direction = (rotation * self.seeker_direction).normalize();
    }
}

/// The enemy in the seeker's field of view on the nose (the diamond) and in
/// its range - the nearest, not one already going down. What a lock takes,
/// and what a missile fired without one grabs coming off the rail.
pub fn on_the_nose(own: &OwnShip, contacts: &[Contact], seeker: &Guidance) -> Option<String> {
    let boresight = own.boresight();
    let range = |contact: &Contact| (contact.position - own.position).magnitude();
    contacts.iter()
        .filter(|contact| !contact.falling && range(contact) <= seeker.lock_range)
        .filter(|contact| boresight.angle(&(contact.position - own.position)).to_degrees() <= SEEKER_FIELD_DEG)
        .min_by(|a, b| range(a).total_cmp(&range(b)))
        .map(|contact| contact.name.clone())
}

/// How far (m) a missile fired now could still reach `target` - shorter
/// chasing a target running away, longer meeting one head on. A rough
/// launch zone, not a fly-out: some ground for every second it closes.
pub fn max_range(own: &OwnShip, target: &Contact, lock_range: f32) -> f32 {
    let line_of_sight = (target.position - own.position).normalize();
    let closing_speed = (own.velocity - target.velocity).dot(&line_of_sight);
    (3_500.0 + 12.0 * closing_speed).clamp(1_000.0, lock_range)
}

/// Whether a missile fired now at `target` should get there - in range, not
/// too close.
pub fn in_range(own: &OwnShip, target: &Contact, lock_range: f32) -> bool {
    let range = (target.position - own.position).magnitude();
    range >= MIN_RANGE && range <= max_range(own, target, lock_range)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::scenes::play::weapon::weapon_spec::WeaponSpec;

    fn seeker() -> Guidance {
        WeaponSpec::read("missiles/aim-9x").unwrap().guidance
    }

    fn own() -> OwnShip {
        OwnShip { position: Vector3::new(0.0, 3000.0, 0.0), rotation: UnitQuaternion::identity(), velocity: Vector3::new(0.0, 0.0, 250.0) }
    }

    fn bandit(name: &str, position: Vector3<f32>) -> Contact {
        Contact { name: name.into(), position, velocity: Vector3::zeros(), falling: false }
    }

    fn run(fire_control: &mut FireControl, contacts: &[Contact], seconds: f32) {
        let guidance = seeker();
        for _ in 0..(seconds * 60.0) as usize {
            fire_control.update(&own(), contacts, Some(&guidance), 1.0 / 60.0);
        }
    }

    #[test]
    fn nothing_selected_locks_nothing() {
        let mut fire_control = FireControl::new();
        let ahead = [bandit("bandit", Vector3::new(0.0, 3000.0, 2000.0))];
        assert!(fire_control.lock(&own(), &ahead, &seeker()));
        for _ in 0..60 {
            fire_control.update(&own(), &ahead, None, 1.0 / 60.0);
        }
        assert_eq!(fire_control.seeker_target, None);
    }

    #[test]
    fn nothing_is_locked_until_the_pilot_locks_it() {
        let mut fire_control = FireControl::new();
        let ahead = [bandit("bandit", Vector3::new(0.0, 3000.0, 2000.0))];
        run(&mut fire_control, &ahead, 1.0);
        assert_eq!(fire_control.seeker_target, None);
        assert!(fire_control.lock(&own(), &ahead, &seeker()));
        run(&mut fire_control, &ahead, 0.1);
        assert_eq!(fire_control.locked(&ahead, &own()), Some("bandit"));
        fire_control.drop_locks();
        run(&mut fire_control, &ahead, 0.1);
        assert_eq!(fire_control.seeker_target, None, "unlocked, it stays unlocked");
    }

    #[test]
    fn only_whats_on_the_nose_and_in_range_locks() {
        let mut fire_control = FireControl::new();
        // 30 degrees off the nose; dead ahead but past the seeker's range;
        // dead ahead but going down.
        let off_nose = bandit("off", Vector3::new(5000.0 * 30f32.to_radians().tan(), 3000.0, 5000.0));
        let far = bandit("far", Vector3::new(0.0, 3000.0, 30_000.0));
        let falling = Contact { falling: true, ..bandit("falling", Vector3::new(0.0, 3000.0, 2000.0)) };
        assert!(!fire_control.lock(&own(), &[off_nose, far, falling], &seeker()));
        // The nearer of two on the nose.
        let near = bandit("near", Vector3::new(0.0, 3000.0, 3000.0));
        let behind_it = bandit("behind", Vector3::new(0.0, 3000.0, 6000.0));
        assert!(fire_control.lock(&own(), &[behind_it, near], &seeker()));
        assert_eq!(fire_control.seeker_target.as_deref(), Some("near"));
    }

    #[test]
    fn fired_unlocked_it_takes_whats_in_the_diamond() {
        let ahead = bandit("ahead", Vector3::new(0.0, 3000.0, 2000.0));
        let off_nose = bandit("off", Vector3::new(2000.0 * 30f32.to_radians().tan(), 3000.0, 2000.0));
        assert_eq!(on_the_nose(&own(), &[off_nose.clone(), ahead], &seeker()).as_deref(), Some("ahead"));
        assert_eq!(on_the_nose(&own(), &[off_nose], &seeker()), None, "nothing in the diamond: it flies straight");
    }

    #[test]
    fn a_lock_holds_on_a_target_going_down() {
        let mut fire_control = FireControl::new();
        let ahead = bandit("bandit", Vector3::new(0.0, 3000.0, 2000.0));
        assert!(fire_control.lock(&own(), std::slice::from_ref(&ahead), &seeker()));
        let hit = [Contact { falling: true, ..ahead }];
        run(&mut fire_control, &hit, 0.5);
        assert_eq!(fire_control.locked(&hit, &own()), Some("bandit"));
    }

    #[test]
    fn a_lock_holds_off_the_nose_until_the_gimbal_limit() {
        let mut fire_control = FireControl::new();
        let ahead = [bandit("bandit", Vector3::new(0.0, 3000.0, 2000.0))];
        assert!(fire_control.lock(&own(), &ahead, &seeker()));
        run(&mut fire_control, &ahead, 1.0);
        // 80 degrees off - outside the radar's cone, still in the seeker's view.
        let wide = [bandit("bandit", Vector3::new(2000.0 * 80f32.to_radians().tan(), 3000.0, 2000.0))];
        run(&mut fire_control, &wide, 1.0);
        assert_eq!(fire_control.locked(&wide, &own()), Some("bandit"));
        run(&mut fire_control, &[bandit("bandit", Vector3::new(0.0, 3000.0, -2000.0))], 0.1);
        assert_eq!(fire_control.seeker_target, None);
    }

    #[test]
    fn the_launch_zone_is_longer_head_on() {
        let head_on = Contact { velocity: Vector3::new(0.0, 0.0, -250.0), ..bandit("a", Vector3::new(0.0, 3000.0, 6000.0)) };
        let running = Contact { velocity: Vector3::new(0.0, 0.0, 250.0), ..bandit("b", Vector3::new(0.0, 3000.0, 6000.0)) };
        assert!(in_range(&own(), &head_on, 18_000.0));
        assert!(!in_range(&own(), &running, 18_000.0));
        let too_close = bandit("c", Vector3::new(0.0, 3000.0, 100.0));
        assert!(!in_range(&own(), &too_close, 18_000.0));
    }
}
