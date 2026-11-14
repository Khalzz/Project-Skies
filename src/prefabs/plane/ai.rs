//! A bot's behaviour - how it flies and fights without a player:
//! - `autopilot`: flies its path (or a scripted one) like a pilot would;
//! - `pilot_skill`: how well - one profile per skill level (`Difficulty`);
//! - `defense`: answering missiles fired at it with flares.

pub mod autopilot;
pub mod defense;
pub mod pilot_skill;
