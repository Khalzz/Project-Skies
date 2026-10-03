//! Bots defending themselves against missiles - for now, heat seekers
//! (Fox 2) answered with flares.
//!
//! A bot doesn't know the moment a missile's fired at it: it notices once
//! the missile's been coming for its difficulty's `reaction_seconds`, then
//! fires off a flare burst (see play::weapon::countermeasures). A beginner
//! does that once for each missile and then forgets about it; better pilots
//! keep bursting, every `burst_every`, for as long as one's still coming.
//!
//! Plain names and seconds in and out - nothing of the scene's - so it's
//! tested on its own (below); play::scene feeds it the missiles every frame.

use std::collections::{HashMap, HashSet};

use super::map::Difficulty;

impl Difficulty {
    /// How long a missile's been coming before it's noticed (s).
    pub fn reaction_seconds(self) -> f32 {
        match self {
            Difficulty::Beginner => 2.5,
            Difficulty::Medium => 1.2,
            Difficulty::Ace => 0.4,
        }
    }

    /// Once it's reacted, how often it bursts again while a missile's still
    /// coming (s) - `None`: once for each missile.
    pub fn burst_every(self) -> Option<f32> {
        match self {
            Difficulty::Beginner => None,
            Difficulty::Medium => Some(3.0),
            Difficulty::Ace => Some(1.5),
        }
    }
}

/// One bot's state.
#[derive(Default)]
struct Defending {
    /// Each missile coming at it, and how long it's been coming (s).
    incoming: HashMap<String, f32>,
    /// The missiles it's already answered (a beginner's once each).
    answered: HashSet<String>,
    /// Until it can burst again (s).
    cooldown: f32,
}

#[derive(Default)]
pub struct AiDefense {
    bots: HashMap<String, Defending>,
}

impl AiDefense {
    /// One frame for the bot `name` flown at `difficulty` - `incoming`: the
    /// heat-seeking missiles homing on it, by id. Whether it bursts flares
    /// now.
    pub fn update(&mut self, name: &str, difficulty: Difficulty, incoming: &[String], delta_time: f32) -> bool {
        let state = self.bots.entry(name.to_owned()).or_default();
        state.incoming.retain(|id, _| incoming.contains(id));
        state.answered.retain(|id| incoming.contains(id));
        for id in incoming {
            *state.incoming.entry(id.clone()).or_insert(0.0) += delta_time;
        }
        state.cooldown = (state.cooldown - delta_time).max(0.0);

        // The missiles it's noticed by now.
        let noticed: Vec<&String> = state.incoming.iter()
            .filter(|(_, seconds)| **seconds >= difficulty.reaction_seconds())
            .map(|(id, _)| id)
            .collect();
        if noticed.is_empty() || state.cooldown > 0.0 {
            return false;
        }
        match difficulty.burst_every() {
            // Once for each missile it notices.
            None => {
                let fresh: Vec<String> = noticed.into_iter().filter(|id| !state.answered.contains(*id)).cloned().collect();
                if fresh.is_empty() {
                    return false;
                }
                state.answered.extend(fresh);
                true
            }
            // Again and again while they keep coming.
            Some(every) => {
                state.cooldown = every;
                true
            }
        }
    }

    /// Forgets the bot `name` - it's gone, or flying again from scratch.
    pub fn forget(&mut self, name: &str) {
        self.bots.remove(name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs the bot for `seconds` with `incoming` coming at it - how many
    /// bursts, and when the first was (s).
    fn run(defense: &mut AiDefense, difficulty: Difficulty, incoming: &[String], seconds: f32) -> (usize, Option<f32>) {
        let dt = 1.0 / 60.0;
        let (mut bursts, mut first) = (0, None);
        for frame in 0..(seconds / dt) as usize {
            if defense.update("bot", difficulty, incoming, dt) {
                bursts += 1;
                first.get_or_insert(frame as f32 * dt);
            }
        }
        (bursts, first)
    }

    #[test]
    fn the_better_the_pilot_the_quicker_it_reacts() {
        let missile = vec!["missile:1".to_owned()];
        let first = |difficulty| run(&mut AiDefense::default(), difficulty, &missile, 5.0).1.expect("it reacts");
        assert!(first(Difficulty::Ace) < first(Difficulty::Medium));
        assert!(first(Difficulty::Medium) < first(Difficulty::Beginner));
        assert!((first(Difficulty::Ace) - Difficulty::Ace.reaction_seconds()).abs() < 0.05);
    }

    #[test]
    fn a_beginner_bursts_once_better_pilots_keep_going() {
        let missile = vec!["missile:1".to_owned()];
        assert_eq!(run(&mut AiDefense::default(), Difficulty::Beginner, &missile, 10.0).0, 1);
        assert!(run(&mut AiDefense::default(), Difficulty::Ace, &missile, 10.0).0 > run(&mut AiDefense::default(), Difficulty::Medium, &missile, 10.0).0);
        // A second missile gets the beginner going again.
        let mut defense = AiDefense::default();
        run(&mut defense, Difficulty::Beginner, &missile, 5.0);
        let two = vec!["missile:1".to_owned(), "missile:2".to_owned()];
        assert_eq!(run(&mut defense, Difficulty::Beginner, &two, 5.0).0, 1);
    }

    #[test]
    fn nothing_coming_nothing_done() {
        assert_eq!(run(&mut AiDefense::default(), Difficulty::Ace, &[], 5.0), (0, None));
    }
}
