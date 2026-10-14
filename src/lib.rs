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
pub mod journal;
pub mod map;
pub mod render;
pub mod rng;
pub mod save;
pub mod tech;
pub mod time;
pub mod volcano;

use bevy_ecs::prelude::*;
use bevy_ecs::schedule::SingleThreadedExecutor;

use biome::Biomes;
use city::{City, CityId, People, Population};
use climate::Climate;
use config::SimConfig;
use events::EventLog;
use fertility::Fertility;
use journal::Journal;
use map::{Map, Position};
use rng::SimRng;
use save::WorldSave;
use tech::{Knowledge, TechTree};
use time::{Date, Turn};
use volcano::Volcanism;

pub struct Simulation {
    world: World,
    schedule: Schedule,
}

/// Seed of the random stream cities are founded from (kept for saving).
#[derive(Resource, Debug, Clone, Copy)]
struct SocietySeed(u64);

/// Plain-data view of one city, ordered by id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CitySnapshot {
    pub id: CityId,
    pub name: String,
    pub position: Position,
    pub population: u64,
    pub founded_on: u64,
    pub people: People,
    pub knowledge: Knowledge,
}

/// Mixed into the society seed for the stream that hands out starting
/// technologies.
const TECH_STREAM: u64 = 0x7EC4_0000_0000_0001;

impl Simulation {
    /// Generates a new world from `config` and founds its cities.
    pub fn new(config: SimConfig) -> Self {
        Self::from_world(Self::generate_world(&config), config)
    }

    /// Generates a world (geography, climate, volcanoes; no cities).
    pub fn generate_world(config: &SimConfig) -> WorldSave {
        let mut rng = SimRng::from_seed(config.seed);
        let map = Map::generate(
            config.map_width,
            config.map_height,
            &config.terrain,
            &mut rng,
        );
        let climate = Climate::new(&map, &config.climate, &mut rng);
        let (volcanism, volcanoes) = volcano::place_volcanoes(config, &map, &mut rng);
        let biomes = Biomes::new(&map, &climate, config);
        let fertility = Fertility::new(&map, &climate, &biomes, &volcanism, &mut rng);
        let society_seed = rng.next_u64();
        WorldSave {
            config: config.clone(),
            turn: 0,
            map,
            climate,
            biomes,
            fertility,
            volcanism,
            volcanoes,
            journal: Journal::default(),
            rng,
            society_seed,
        }
    }

    /// Loads a world saved with [`Simulation::save_world`].
    pub fn load_world(path: &std::path::Path) -> std::io::Result<WorldSave> {
        WorldSave::read(path)
    }

    /// Starts a simulation on a saved (or freshly generated) world and founds
    /// its cities. Everything that shaped the world comes from the save:
    /// seed, map size, terrain, the climate (except the oscillation's
    /// parameters) and where volcanoes are. Everything else comes from
    /// `config`: city settings, eruption rates and aerosol strength, and the
    /// oscillation's period, damping and amplitude.
    pub fn from_world(save: WorldSave, config: SimConfig) -> Self {
        Self::from_world_with_technologies(save, config, TechTree::builtin())
    }

    /// Like [`Simulation::from_world`], with a technology tree of your own
    /// (see [`TechTree::load`]) instead of the built-in one.
    pub fn from_world_with_technologies(
        save: WorldSave,
        config: SimConfig,
        tree: TechTree,
    ) -> Self {
        let world_config = save.config.clone();
        let config = SimConfig {
            seed: world_config.seed,
            map_width: world_config.map_width,
            map_height: world_config.map_height,
            terrain: world_config.terrain.clone(),
            climate: climate::ClimateConfig {
                oscillation: config.climate.oscillation.clone(),
                ..world_config.climate.clone()
            },
            volcanoes: volcano::VolcanoConfig {
                arcs_per_10k_tiles: world_config.volcanoes.arcs_per_10k_tiles,
                hotspots_per_10k_tiles: world_config.volcanoes.hotspots_per_10k_tiles,
                ..config.volcanoes.clone()
            },
            ..config
        };

        let mut world = World::new();
        world.insert_resource(EventLog::default());
        for (volcano, position) in save.volcanoes {
            world.spawn((volcano, position));
        }
        let mut society_rng = SimRng::from_seed(save.society_seed);
        let mut tech_rng = SimRng::from_seed(save.society_seed ^ TECH_STREAM);
        city::found_initial_cities(
            &mut world,
            save.turn,
            &config,
            &save.map,
            &save.climate,
            &save.biomes,
            &save.fertility,
            &tree,
            &mut society_rng,
            &mut tech_rng,
        );
        world.insert_resource(tree);
        world.insert_resource(save.map);
        world.insert_resource(save.climate);
        world.insert_resource(save.biomes);
        world.insert_resource(save.fertility);
        world.insert_resource(save.volcanism);
        world.insert_resource(save.journal);
        world.insert_resource(save.rng);
        world.insert_resource(SocietySeed(save.society_seed));
        world.insert_resource(Turn(save.turn));
        world.insert_resource(config);

        let mut schedule = Schedule::default();
        schedule.set_executor(SingleThreadedExecutor::new());
        schedule.add_systems(
            (
                time::advance_turn,
                volcano::volcanic_activity,
                climate::update_climate,
                journal::update_journal,
                biome::update_biomes,
                fertility::update_fertility,
                city::grow_cities,
            )
                .chain(),
        );

        Self { world, schedule }
    }

    /// Snapshot of the world (everything but the cities), for saving.
    pub fn world_save(&mut self) -> WorldSave {
        let volcanoes = self.volcanoes();
        WorldSave {
            config: self.config().clone(),
            turn: self.turn(),
            map: self.map().clone(),
            climate: self.climate().clone(),
            biomes: self.biomes().clone(),
            fertility: self.fertility().clone(),
            volcanism: self.volcanism().clone(),
            volcanoes,
            journal: self.journal().clone(),
            rng: self.world.resource::<SimRng>().clone(),
            society_seed: self.world.resource::<SocietySeed>().0,
        }
    }

    /// Saves the world (everything but the cities) to a file.
    pub fn save_world(&mut self, path: &std::path::Path) -> std::io::Result<()> {
        self.world_save().write(path)
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

    /// The climate journal: notable weather and volcanic events by severity.
    pub fn journal(&self) -> &Journal {
        self.world.resource::<Journal>()
    }

    pub fn volcanism(&self) -> &Volcanism {
        self.world.resource::<Volcanism>()
    }

    /// All volcanoes, ordered by id.
    pub fn volcanoes(&mut self) -> Vec<(volcano::Volcano, Position)> {
        let mut query = self.world.query::<(&volcano::Volcano, &Position)>();
        let mut list: Vec<_> = query
            .iter(&self.world)
            .map(|(v, p)| (v.clone(), *p))
            .collect();
        list.sort_by_key(|(v, _)| v.id);
        list
    }

    pub fn map(&self) -> &Map {
        self.world.resource::<Map>()
    }

    /// The technology tree the cities' knowledge refers to.
    pub fn technologies(&self) -> &TechTree {
        self.world.resource::<TechTree>()
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
            .query::<(&CityId, &City, &Position, &Population, &Knowledge)>();
        let mut cities: Vec<_> = query
            .iter(&self.world)
            .map(|(id, city, pos, pop, knowledge)| CitySnapshot {
                id: *id,
                name: city.name.clone(),
                position: *pos,
                population: pop.count,
                founded_on: city.founded_on,
                people: city.people,
                knowledge: knowledge.clone(),
            })
            .collect();
        cities.sort_by_key(|c| c.id);
        cities
    }

    /// A stable fingerprint of the simulated state (FNV-1a). Two runs with the
    /// same config must produce the same digest at every turn.
    pub fn state_digest(&mut self) -> u64 {
        let mut h = Fnv1a(self.world_digest());
        self.cities_digest(&mut h);
        h.finish()
    }

    /// A stable fingerprint of the world without the cities: map, climate,
    /// volcanoes, journal and the main random stream.
    pub fn world_digest(&mut self) -> u64 {
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
        self.volcanism().digest(&mut |v| h.write_u64(v));
        self.journal().digest(&mut |v| h.write_u64(v));
        for (v, p) in self.volcanoes() {
            h.write_u64(u64::from(v.id.0) << 32 | u64::from(p.x) << 16 | u64::from(p.y));
            h.write_u64(v.magma);
            h.write_u64(v.last_eruption.unwrap_or(u64::MAX));
        }
        for &f in self.fertility().values() {
            h.write(&f.to_le_bytes());
        }
        // Fold in the RNG position without advancing the real stream.
        h.write_u64(self.world.resource::<SimRng>().clone().next_u64());
        h.finish()
    }

    /// Fingerprint of the cities alone.
    fn cities_digest(&mut self, h: &mut Fnv1a) {
        let mut query = self
            .world
            .query::<(&CityId, &City, &Position, &Population, &Knowledge)>();
        let mut rows: Vec<_> = query.iter(&self.world).collect();
        rows.sort_by_key(|(id, ..)| **id);
        for (id, city, pos, pop, knowledge) in rows {
            h.write_u64(u64::from(id.0));
            h.write(city.name.as_bytes());
            h.write_u64(city.founded_on);
            h.write_u64(city.people as u64);
            h.write_u64(u64::from(pos.x));
            h.write_u64(u64::from(pos.y));
            h.write_u64(pop.count);
            h.write_u64(pop.growth_remainder);
            knowledge.digest(&mut |v| h.write_u64(v));
        }
    }

    /// Renders the map and cities as ASCII. See [`render::ascii`].
    pub fn render_map(&mut self) -> String {
        let cities = self.cities();
        let volcanoes: Vec<Position> = self.volcanoes().into_iter().map(|(_, p)| p).collect();
        render::ascii(
            self.map(),
            self.climate(),
            self.biomes(),
            &volcanoes,
            &cities,
        )
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
