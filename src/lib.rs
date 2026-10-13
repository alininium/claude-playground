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

pub mod biome;
pub mod city;
pub mod climate;
pub mod config;
pub mod events;
pub mod fertility;
pub mod map;
pub mod render;
pub mod rng;
pub mod time;

use bevy_ecs::prelude::*;
use bevy_ecs::schedule::SingleThreadedExecutor;

use biome::Biomes;
use city::{City, CityId, Population};
use climate::Climate;
use config::SimConfig;
use events::EventLog;
use fertility::Fertility;
use map::{Map, Position};
use rng::SimRng;
use time::{Date, Turn};

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
        let mut rng = SimRng::from_seed(config.seed);
        let map = Map::generate(
            config.map_width,
            config.map_height,
            &config.terrain,
            &mut rng,
        );
        let climate = Climate::new(&map, &config.climate, &mut rng);
        let biomes = Biomes::new(&map, &climate, &config);
        let fertility = Fertility::new(&map, &climate, &biomes, &mut rng);
        world.insert_resource(EventLog::default());
        city::found_initial_cities(&mut world, &config, &map, &climate, &fertility, &mut rng);
        world.insert_resource(map);
        world.insert_resource(climate);
        world.insert_resource(biomes);
        world.insert_resource(fertility);
        world.insert_resource(rng);
        world.insert_resource(Turn::default());
        world.insert_resource(config);

        let mut schedule = Schedule::default();
        schedule.set_executor(SingleThreadedExecutor::new());
        schedule.add_systems(
            (
                time::advance_turn,
                climate::update_climate,
                biome::update_biomes,
                fertility::update_fertility,
                city::grow_cities,
            )
                .chain(),
        );

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

    pub fn date(&self) -> Date {
        Date::from_turn(
            self.turn(),
            self.world.resource::<SimConfig>().climate.days_per_year,
        )
    }

    pub fn config(&self) -> &SimConfig {
        self.world.resource::<SimConfig>()
    }

    pub fn climate(&self) -> &Climate {
        self.world.resource::<Climate>()
    }

    pub fn biomes(&self) -> &Biomes {
        self.world.resource::<Biomes>()
    }

    pub fn fertility(&self) -> &Fertility {
        self.world.resource::<Fertility>()
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
        for tile in self.map().tiles() {
            h.write_u64(tile.altitude as u64);
            h.write_u64(tile.surface as u64);
            h.write(&[tile.landform as u8, tile.soil as u8, tile.water as u8]);
            h.write_u64(u64::from(tile.moisture));
            h.write_u64(u64::from(tile.flow));
            h.write_u64(tile.downstream.map_or(u64::MAX, u64::from));
        }
        self.climate().digest(&mut |v| h.write_u64(v));
        for &b in self.biomes().all() {
            h.write(&[b as u8]);
        }
        for &f in self.fertility().values() {
            h.write(&f.to_le_bytes());
        }
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

    /// Renders the map and cities as ASCII. See [`render::ascii`].
    pub fn render_map(&mut self) -> String {
        let cities = self.cities();
        render::ascii(self.map(), self.climate(), self.biomes(), &cities)
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
