//! A turn-based, fully deterministic simulation of societies.
//!
//! The simulation is an ECS `World` advanced one turn at a time by a single
//! `Schedule`. Determinism rules every system must follow:
//!
//! - All randomness comes from the one [`rng::SimRng`] resource.
//! - Systems run on the single-threaded executor, in an explicit chained order.
//! - Never rely on query iteration order when it affects the outcome
//!   (RNG draws, tie-breaks, ...): sort by a stable id such as [`city::CityId`].
//! - No floating point in simulation state; use integer / fixed-point math.
//! - No wall-clock time, no `HashMap` iteration with random hashers.

pub mod city;
pub mod config;
pub mod events;
pub mod map;
pub mod rng;
pub mod turn;

use bevy_ecs::prelude::*;
use bevy_ecs::schedule::SingleThreadedExecutor;

use city::{City, CityId, NextCityId, Population};
use config::SimConfig;
use events::EventLog;
use map::{Map, Position};
use rng::SimRng;
use turn::Turn;

pub struct Simulation {
    world: World,
    schedule: Schedule,
}

/// Plain-data view of one city, ordered by id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CitySnapshot {
    pub id: CityId,
    pub name: String,
    pub position: Position,
    pub population: u64,
    pub founded_on: u64,
}

impl Simulation {
    pub fn new(config: SimConfig) -> Self {
        let mut world = World::new();
        world.insert_resource(Map::new(config.map_width, config.map_height));
        world.insert_resource(SimRng::from_seed(config.seed));
        world.insert_resource(Turn::default());
        world.insert_resource(NextCityId::default());
        world.insert_resource(EventLog::default());
        world.insert_resource(config);

        let mut schedule = Schedule::default();
        schedule.set_executor(SingleThreadedExecutor::new());
        schedule.add_systems((turn::advance_turn, city::grow_cities, city::spawn_cities).chain());

        Self { world, schedule }
    }

    /// Advances the simulation by exactly one turn.
    pub fn step(&mut self) {
        self.schedule.run(&mut self.world);
    }

    pub fn run(&mut self, turns: u64) {
        for _ in 0..turns {
            self.step();
        }
    }

    pub fn turn(&self) -> u64 {
        self.world.resource::<Turn>().0
    }

    pub fn map(&self) -> &Map {
        self.world.resource::<Map>()
    }

    pub fn events(&self) -> &EventLog {
        self.world.resource::<EventLog>()
    }

    pub fn world(&self) -> &World {
        &self.world
    }

    pub fn cities(&mut self) -> Vec<CitySnapshot> {
        let mut query = self
            .world
            .query::<(&CityId, &City, &Position, &Population)>();
        let mut cities: Vec<_> = query
            .iter(&self.world)
            .map(|(id, city, pos, pop)| CitySnapshot {
                id: *id,
                name: city.name.clone(),
                position: *pos,
                population: pop.count,
                founded_on: city.founded_on,
            })
            .collect();
        cities.sort_by_key(|c| c.id);
        cities
    }

    /// A stable fingerprint of the simulated state (FNV-1a). Two runs with the
    /// same config must produce the same digest at every turn.
    pub fn state_digest(&mut self) -> u64 {
        let mut h = Fnv1a::new();
        h.write_u64(self.turn());
        // Fold in the RNG position without advancing the real stream.
        h.write_u64(self.world.resource::<SimRng>().clone().next_u64());
        let mut query = self
            .world
            .query::<(&CityId, &City, &Position, &Population)>();
        let mut rows: Vec<_> = query.iter(&self.world).collect();
        rows.sort_by_key(|(id, ..)| **id);
        for (id, city, pos, pop) in rows {
            h.write_u64(u64::from(id.0));
            h.write(city.name.as_bytes());
            h.write_u64(city.founded_on);
            h.write_u64(u64::from(pos.x));
            h.write_u64(u64::from(pos.y));
            h.write_u64(pop.count);
            h.write_u64(pop.growth_remainder);
        }
        h.finish()
    }

    /// Renders the map as ASCII: `.` empty tile, `o` village, `O` town, `@` city.
    pub fn render_map(&mut self) -> String {
        let (w, h) = (self.map().width(), self.map().height());
        let mut grid = vec![vec!['.'; w as usize]; h as usize];
        for c in self.cities() {
            grid[c.position.y as usize][c.position.x as usize] = match c.population {
                0..100 => 'o',
                100..1000 => 'O',
                _ => '@',
            };
        }
        grid.into_iter()
            .map(|row| row.into_iter().collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

struct Fnv1a(u64);

impl Fnv1a {
    fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }
    fn write(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.0 ^= u64::from(*b);
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    fn write_u64(&mut self, v: u64) {
        self.write(&v.to_le_bytes());
    }
    fn finish(&self) -> u64 {
        self.0
    }
}
