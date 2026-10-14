//! Land and food: each city's territory and what its tiles can yield given
//! fertility, season and the technologies' `food_yield`-style effects; land
//! handed out to the people who chose food work (in id order); production
//! into the city's [`Stores`], eating, hunger, and the signal that there is
//! no usable land left (which pushes people to settle away).
//!
//! Tuning: the `economy` section of `data/people.json`.

use bevy_ecs::prelude::*;
use bevy_ecs::schedule::ScheduleConfigs;
use bevy_ecs::system::ScheduleSystem;

/// Inserts this module's resources (its section of `data/people.json`).
pub fn install(_world: &mut World) {}

/// This module's systems, in order, for its slot in the turn.
pub fn systems() -> ScheduleConfigs<ScheduleSystem> {
    nothing_yet.into_configs()
}

fn nothing_yet() {}

/// A city's common inventory.
#[derive(Component, Debug, Clone, PartialEq, Eq, Default)]
pub struct Stores {
    /// Food, in person-days of eating.
    pub food: u64,
    /// Count of each item, indexed by `tools::ItemId`.
    pub items: Vec<u32>,
}

/// What a city offers its people, kept up to date by this module (and
/// `tools` for `tools_available`) and read by `people::decide`.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Outlook {
    /// How many more people could take up food work on the city's land
    /// (0: no usable land left, which pushes people to settle away).
    pub free_land: u32,
    /// Days of food in the stores per inhabitant.
    pub food_days: u32,
    /// Tools in the stores that people could take.
    pub tools_available: u32,
}
