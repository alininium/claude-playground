use bevy_ecs::prelude::{ResMut, Resource};

/// The current turn number. Turn 0 is the initial state, before any turn ran.
#[derive(Resource, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Turn(pub u64);

pub fn advance_turn(mut turn: ResMut<Turn>) {
    turn.0 += 1;
}
