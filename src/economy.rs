//! Land and food: each city's territory and what its tiles can yield given
//! fertility, season and the technologies' `food_yield`-style effects; land
//! handed out to the people who chose food work (in id order); production
//! into the city's [`Stores`], eating, hunger, and the signal that there is
//! no usable land left (which pushes people to settle away).
//!
//! # Land
//! A city's [`Territory`] is the tiles within `territory_radius_km` of it
//! that no other city is nearer to (ties go to the lower [`CityId`]), and
//! always its own tile. The territory says what kinds of land the city has;
//! how much of it people can actually work is limited by walking distance:
//! each kind of food work has a catchment area (a few kilometers around the
//! fields for farmers, much more for herders and foragers), spread over the
//! territory's usable tiles in proportion to their area. A tile's capacity
//! for an action, in workers, is its share of the catchment times a density
//! of workers per km² times the tile's quality for that action (fertility
//! for fields, the biome's grazing for herds). Fishing grounds hold a fixed
//! number of fishers per kind of water (river, lake, sea).
//!
//! # Work
//! Every turn the people whose [`Activity`] is Forage, Farm, Herd or Fish
//! get places, city by city in [`PersonId`] order, best land first; who
//! finds no room produces nothing. A worker's output is a base yield (in
//! person-days of food per day of work) × the land's quality × the season ×
//! the technologies they personally know (`food_yield`, `herd_yield`,
//! `fish_yield`, with their `where` filters) × their tools
//! ([`ToolCatalog::work_multiplier`]). Herds, fish and wild food go straight
//! into the stores. Fields only grow in the growing season (warm enough and
//! not too dry: the tile's seasonal climate normals) and their crop stands
//! in the field until the season after which nothing grows, when it is
//! harvested into the stores all at once. That makes farming seasonal with
//! one number per tile, not a simulation of each field.
//!
//! # Eating
//! Once a day, in the evening: the harvest comes in, stored food spoils
//! (slowly up to what the city's storage technologies can keep, fast beyond
//! that), and everyone at home eats, children less. If there is not enough,
//! everyone gets the same share and their Food need falls only that much
//! (`needs` grows it again). Then [`Outlook`] is brought up to date.
//!
//! All of it works on per-city aggregates and one pass over the workers per
//! turn, so people who are not working cost almost nothing here.
//!
//! Tuning: the `economy` section of `data/people.json`.

use std::collections::BTreeMap;

use bevy_ecs::prelude::*;
use bevy_ecs::schedule::ScheduleConfigs;
use bevy_ecs::system::ScheduleSystem;

use crate::biome::{Biome, Biomes};
use crate::city::{CityId, Population};
use crate::climate::Climate;
use crate::climate::cryosphere::GLACIER;
use crate::config::SimConfig;
use crate::fertility::Fertility;
use crate::map::{Landform, Map, Position, Water};
use crate::people::{ActionKind, Activity, Need, Needs, Person, PersonId, Skills};
use crate::tech::{Effect, Knowledge, Site, TechId, TechTree, VariantId, WaterAccess};
use crate::time::{Date, Season, TURNS_PER_DAY, TimeOfDay, Turn};
use crate::tools::{Equipment, ToolCatalog};

/// Inserts this module's resources (its section of `data/people.json`).
pub fn install(world: &mut World) {
    world.insert_resource(EconomyConfig::builtin());
    world.init_resource::<EconomyState>();
}

/// This module's systems, in order, for its slot in the turn.
pub fn systems() -> ScheduleConfigs<ScheduleSystem> {
    (update_territories, work_the_land, daily_food).chain()
}

// ---------------------------------------------------------------------------
// Tuning
// ---------------------------------------------------------------------------

/// The `economy` section of `data/people.json`. Food is counted in
/// person-days (what one adult eats in a day); yields are in thousandths of
/// a person-day, shares and multipliers per mille.
#[derive(Resource, Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct EconomyConfig {
    /// How far a city's land reaches, in km (at least its own tile).
    pub territory_radius_km: u32,
    /// Turns of work that make a full day's work.
    pub work_turns_per_day: u32,
    pub farm: LandWork,
    pub herd: LandWork,
    pub forage: LandWork,
    pub fish: FishWork,
    /// A worker must know one of these to farm.
    pub farm_requires_any: Vec<String>,
    /// A worker must know one of these to herd.
    pub herd_requires_any: Vec<String>,
    /// Land that cannot be farmed without a `farmable` technology for it.
    pub unfarmable_landforms: Vec<Landform>,
    pub unfarmable_biomes: Vec<Biome>,
    /// Grazing quality of each biome, per mille (missing: none).
    pub pasture: BTreeMap<Biome, u32>,
    /// Wild food on barren-looking land: foraging quality is this plus the
    /// rest scaled by fertility, per mille.
    pub forage_floor: u32,
    pub growing_season: GrowingSeason,
    pub off_season: OffSeason,
    pub storage: Storage,
    pub eating: Eating,
    /// Days of food each city has in store when the world begins.
    pub initial_food_days: u32,
}

/// How much land one kind of food work needs, and what it yields.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct LandWork {
    /// The land within walking distance that people work, km².
    pub catchment_km2: u32,
    /// Workers the best land (quality 1000) can take, per 100 km².
    pub workers_per_100_km2: u32,
    /// Food from one day's work on the best land, thousandths of a
    /// person-day.
    pub daily_yield: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct FishWork {
    /// Fishers each kind of water can take.
    pub workers: FishingGrounds,
    pub daily_yield: u32,
    /// Fishers who know none of these catch `without_gear` per mille.
    pub gear: Vec<String>,
    pub without_gear: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct FishingGrounds {
    pub river: u32,
    pub lake: u32,
    pub coast: u32,
}

/// When fields grow: a season (from the tile's climate normals) at least
/// this warm and this wet. Land on a river or lake can be watered, so dry
/// seasons do not stop it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct GrowingSeason {
    /// Mean temperature, hundredths of a degree C.
    pub min_temperature: i32,
    /// Precipitation over the season, tenths of a millimeter.
    pub min_precipitation: i32,
}

/// Output outside the growing season, per mille.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct OffSeason {
    pub herd: u32,
    pub forage: u32,
    /// Fishing when the season's mean is below `freezing`.
    pub fish_frozen: u32,
    /// Hundredths of a degree C.
    pub freezing: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct Storage {
    /// Months of food a city can store well without storage technologies
    /// (baskets, pits); `food_storage` effects add to it.
    pub base_months: u32,
    /// Daily loss of well-stored food, per 100 000.
    pub stored_loss: u32,
    /// Daily loss of food beyond what can be stored well, per 100 000.
    pub open_loss: u32,
    /// The most that `preservation` effects can cut losses, per mille.
    pub max_preservation: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct Eating {
    /// Below this age a person eats `child_ration`.
    pub child_age: u32,
    /// Per mille of an adult's ration.
    pub child_ration: u32,
    /// How much a full day's food lowers the Food need.
    pub full_meal: u16,
}

impl EconomyConfig {
    pub fn builtin() -> Self {
        crate::people::section("economy")
    }

    /// The land needs of `action`. Fishing is not measured in land (see
    /// [`FishWork`]); it is never asked for.
    fn work(&self, action: Work) -> &LandWork {
        match action {
            Work::Farm => &self.farm,
            Work::Herd => &self.herd,
            Work::Forage | Work::Fish => &self.forage,
        }
    }
}

// ---------------------------------------------------------------------------
// Components
// ---------------------------------------------------------------------------

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
    /// Of that, room on fields (for farmers) and on pasture (for herders);
    /// `decide` checks the one that fits the work.
    pub free_farm: u32,
    pub free_pasture: u32,
    /// Days of food in the stores per inhabitant.
    pub food_days: u32,
    /// Tools in the stores that people could take.
    pub tools_available: u32,
}

/// The four kinds of food work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Work {
    Forage,
    Farm,
    Herd,
    Fish,
}

impl Work {
    pub const ALL: [Work; 4] = [Work::Forage, Work::Farm, Work::Herd, Work::Fish];

    pub fn of(action: ActionKind) -> Option<Work> {
        match action {
            ActionKind::Forage => Some(Work::Forage),
            ActionKind::Farm => Some(Work::Farm),
            ActionKind::Herd => Some(Work::Herd),
            ActionKind::Fish => Some(Work::Fish),
            _ => None,
        }
    }
}

/// The land a city works, nearest tile first (its own tile is always the
/// first). Recomputed when cities are founded, move or disappear, and at
/// the start of every year (biomes change).
#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub struct Territory {
    pub tiles: Vec<TerritoryTile>,
    /// `food_storage` and `preservation` effects at the city's own tile.
    storage: Vec<Additive>,
    preservation: Vec<Additive>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerritoryTile {
    pub index: u32,
    pub area_km2: u32,
    pub site: Site,
    /// Yield effects that apply on this tile, by kind.
    food: Vec<Bonus>,
    herd: Vec<Bonus>,
    fish: Vec<Bonus>,
    farmable: Vec<Bonus>,
}

/// One technology's effect of one kind on a tile: its own (`own`) and that
/// of each of its variants. Someone who knows the technology gets `own`
/// times the best variant they know.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Bonus {
    tech: TechId,
    own: u32,
    variants: Vec<(VariantId, u32)>,
}

/// An additive effect (months of storage, per mille of preservation).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Additive {
    tech: TechId,
    variant: Option<VariantId>,
    value: u32,
}

/// A city's food accounts. The stores are kept here in thousandths of a
/// person-day (mirrored, rounded down, in [`Stores::food`]; if another
/// module changes `Stores::food`, that change wins).
#[derive(Component, Debug, Clone, PartialEq, Eq, Default)]
pub struct Larder {
    pub food_milli: u64,
    /// Crop standing in the fields of each territory tile, by tile index.
    pub standing: BTreeMap<u32, u64>,
    /// Workers and those who found no land, per [`Work`], most in any turn
    /// so far today and during the last full day.
    pub today: Workday,
    pub last_day: Workday,
    /// Totals since the city began, thousandths of a person-day. `produced`
    /// counts crops when they grow, `harvested` when they come in.
    pub produced_milli: u64,
    pub harvested_milli: u64,
    pub eaten_milli: u64,
    pub spoiled_milli: u64,
    /// People at home at the last meal.
    pub residents: u32,
    /// Share of a full ration they got, per mille.
    pub ration: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Workday {
    /// Those who got a place to work.
    pub workers: [u32; 4],
    /// Those who produced nothing: no place was left, or they lacked the
    /// skill (farming or herding technologies).
    pub idle: [u32; 4],
}

/// What this module remembers between turns: the cities its territories
/// were drawn for, and the technologies it looks for.
#[derive(Resource, Debug, Clone, Default)]
pub struct EconomyState {
    cities: Vec<(CityId, Position)>,
    year: u64,
    started: bool,
    farm_techs: Vec<TechId>,
    herd_techs: Vec<TechId>,
    fish_gear: Vec<TechId>,
}

// ---------------------------------------------------------------------------
// Territories
// ---------------------------------------------------------------------------

/// Draws every city's territory when the set of cities changed or a year
/// began, and gives new cities their accounts (with starting stores if the
/// world is just beginning).
fn update_territories(world: &mut World) {
    let turn = world.resource::<Turn>().0;
    let days = world.resource::<SimConfig>().climate.days_per_year;
    let year = Date::from_turn(turn, days).year;
    let mut query = world.query::<(&CityId, &Position)>();
    let mut cities: Vec<(CityId, Position)> =
        query.iter(world).map(|(id, pos)| (*id, *pos)).collect();
    cities.sort_by_key(|c| c.0);

    let first = !world.resource::<EconomyState>().started;
    {
        let state = world.resource::<EconomyState>();
        if !first && state.cities == cities && state.year == year {
            return;
        }
    }
    if first {
        let tree = world.resource::<TechTree>();
        let config = world.resource::<EconomyConfig>();
        let ids = |names: &[String]| -> Vec<TechId> {
            names
                .iter()
                .map(|n| {
                    tree.id(n).unwrap_or_else(|| {
                        panic!("data/people.json, section economy: unknown technology {n}")
                    })
                })
                .collect()
        };
        let (farm, herd, gear) = (
            ids(&config.farm_requires_any),
            ids(&config.herd_requires_any),
            ids(&config.fish.gear),
        );
        let mut state = world.resource_mut::<EconomyState>();
        state.farm_techs = farm;
        state.herd_techs = herd;
        state.fish_gear = gear;
    }

    let territories = {
        let map = world.resource::<Map>();
        let sim = world.resource::<SimConfig>();
        let biomes = world.resource::<Biomes>();
        let tree = world.resource::<TechTree>();
        let radius = world.resource::<EconomyConfig>().territory_radius_km;
        let tiles = claim_land(map, sim, radius, &cities);
        tiles
            .into_iter()
            .map(|tiles| territory(map, biomes, tree, &tiles))
            .collect::<Vec<_>>()
    };

    let initial_days = u64::from(world.resource::<EconomyConfig>().initial_food_days);
    let mut query = world.query::<(Entity, &CityId)>();
    let mut entities: Vec<(CityId, Entity)> = query.iter(world).map(|(e, id)| (*id, e)).collect();
    entities.sort_by_key(|c| c.0);
    for ((_, entity), territory) in entities.into_iter().zip(territories) {
        let mut city = world.entity_mut(entity);
        if !city.contains::<Stores>() {
            city.insert(Stores::default());
        }
        if !city.contains::<Outlook>() {
            city.insert(Outlook::default());
        }
        let keep: BTreeMap<u32, ()> = territory.tiles.iter().map(|t| (t.index, ())).collect();
        if let Some(mut larder) = city.get_mut::<Larder>() {
            // Crops on land the city no longer holds are brought in early.
            let lost: Vec<u32> = larder
                .standing
                .keys()
                .copied()
                .filter(|i| !keep.contains_key(i))
                .collect();
            for i in lost {
                let crop = larder.standing.remove(&i).unwrap_or(0);
                larder.food_milli += crop;
                larder.harvested_milli += crop;
            }
        } else {
            let food = if first {
                let residents = city.get::<Population>().map_or(0, |p| p.count);
                residents * initial_days
            } else {
                city.get::<Stores>().map_or(0, |s| s.food)
            };
            if let Some(mut stores) = city.get_mut::<Stores>() {
                stores.food = food;
            }
            city.insert(Larder {
                food_milli: food * 1000,
                ration: 1000,
                ..Larder::default()
            });
        }
        city.insert(territory);
    }

    let mut state = world.resource_mut::<EconomyState>();
    state.cities = cities;
    state.year = year;
    state.started = true;
}

/// The tiles each city holds, with their areas in km², nearest first: those
/// within `radius_km` that no other city is nearer to (ties to the lower
/// id), and always the city's own tile.
pub fn claim_land(
    map: &Map,
    config: &SimConfig,
    radius_km: u32,
    cities: &[(CityId, Position)],
) -> Vec<Vec<(u32, u32)>> {
    let circumference = i64::from(config.climate.planet_circumference_km);
    let ns_km = (circumference / 2 / i64::from(map.height())).max(1);
    let radius = i64::from(radius_km);
    let rows = (radius / ns_km + 1) as u32;
    cities
        .iter()
        .map(|&(id, here)| {
            let mut tiles: Vec<(i64, u32, u32)> = Vec::new();
            let y0 = here.y.saturating_sub(rows);
            let y1 = (here.y + rows + 1).min(map.height());
            for y in y0..y1 {
                let ew_km =
                    (circumference * map.area_weight(y) / i64::from(map.width()) / 1000).max(1);
                let cols = (radius / ew_km + 1).min(i64::from(map.width())) as u32;
                let x0 = here.x.saturating_sub(cols);
                let x1 = (here.x + cols + 1).min(map.width());
                for x in x0..x1 {
                    let there = Position { x, y };
                    let d = distance_sq_km(map, config, here, there);
                    if there != here && d > radius * radius {
                        continue;
                    }
                    let nearer = cities.iter().any(|&(other, pos)| {
                        other != id && {
                            let e = distance_sq_km(map, config, pos, there);
                            e < d || (e == d && other < id)
                        }
                    });
                    if there == here || !nearer {
                        let area = (ns_km * ew_km).min(i64::from(u32::MAX)) as u32;
                        tiles.push((d, map.index(there) as u32, area));
                    }
                }
            }
            tiles.sort();
            tiles.into_iter().map(|(_, i, a)| (i, a)).collect()
        })
        .collect()
}

/// Squared distance in km² on the globe, measured as `city::too_close`
/// does: north-south rows, east-west columns scaled by cos(latitude).
fn distance_sq_km(map: &Map, config: &SimConfig, a: Position, b: Position) -> i64 {
    let circumference = i64::from(config.climate.planet_circumference_km);
    let (dx, dy) = (a.x.abs_diff(b.x), a.y.abs_diff(b.y));
    let row = (a.y + b.y) / 2;
    let ns = i64::from(dy) * circumference / 2 / i64::from(map.height());
    let ew = i64::from(dx) * circumference / i64::from(map.width()) * map.area_weight(row) / 1000;
    ns * ns + ew * ew
}

fn territory(map: &Map, biomes: &Biomes, tree: &TechTree, tiles: &[(u32, u32)]) -> Territory {
    let tiles: Vec<TerritoryTile> = tiles
        .iter()
        .map(|&(index, area_km2)| {
            let site = Site::of(map, biomes, index as usize);
            TerritoryTile {
                index,
                area_km2,
                site,
                food: bonuses(tree, &site, "food_yield", Combine::Product),
                herd: bonuses(tree, &site, "herd_yield", Combine::Product),
                fish: bonuses(tree, &site, "fish_yield", Combine::Product),
                farmable: bonuses(tree, &site, "farmable", Combine::Max),
            }
        })
        .collect();
    let home = tiles[0].site;
    Territory {
        storage: additives(tree, &home, "food_storage"),
        preservation: additives(tree, &home, "preservation"),
        tiles,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Combine {
    /// Multipliers: several effects multiply; none is 1000.
    Product,
    /// Shares (`farmable`): the best counts; none is 0.
    Max,
}

/// Every technology with an effect of `kind` that applies at `site`.
fn bonuses(tree: &TechTree, site: &Site, kind: &str, combine: Combine) -> Vec<Bonus> {
    let fold = |effects: &[Effect]| -> Option<u32> {
        let mut hit: Option<u32> = None;
        for e in effects
            .iter()
            .filter(|e| e.kind == kind && e.at.matches(site))
        {
            let v = e.value.clamp(0, i64::from(u32::MAX)) as u32;
            hit = Some(match (combine, hit) {
                (_, None) => v,
                (Combine::Product, Some(h)) => (u64::from(h) * u64::from(v) / 1000) as u32,
                (Combine::Max, Some(h)) => h.max(v),
            });
        }
        hit
    };
    let none = match combine {
        Combine::Product => 1000,
        Combine::Max => 0,
    };
    let mut out = Vec::new();
    for t in 0..tree.len() {
        let tech = TechId(t as u16);
        let own = fold(&tree.tech(tech).effects);
        let variants: Vec<(VariantId, Option<u32>)> = tree
            .variants(tech)
            .map(|v| (v, fold(&tree.variant(v).effects)))
            .collect();
        if own.is_none() && variants.iter().all(|(_, x)| x.is_none()) {
            continue;
        }
        out.push(Bonus {
            tech,
            own: own.unwrap_or(none),
            variants: variants
                .into_iter()
                .map(|(v, x)| (v, x.unwrap_or(none)))
                .collect(),
        });
    }
    out
}

fn additives(tree: &TechTree, site: &Site, kind: &str) -> Vec<Additive> {
    let sum = |effects: &[Effect]| -> u32 {
        effects
            .iter()
            .filter(|e| e.kind == kind && e.at.matches(site))
            .map(|e| e.value.clamp(0, i64::from(u32::MAX)) as u32)
            .sum()
    };
    let mut out = Vec::new();
    for t in 0..tree.len() {
        let tech = TechId(t as u16);
        let value = sum(&tree.tech(tech).effects);
        if value > 0 {
            out.push(Additive {
                tech,
                variant: None,
                value,
            });
        }
        for v in tree.variants(tech) {
            let value = sum(&tree.variant(v).effects);
            if value > 0 {
                out.push(Additive {
                    tech,
                    variant: Some(v),
                    value,
                });
            }
        }
    }
    out
}

/// The product of the yield effects someone who knows `knowledge` gets,
/// per mille.
fn multiplier(bonuses: &[Bonus], knowledge: &Knowledge) -> u64 {
    let mut m = 1000u64;
    for b in bonuses.iter().filter(|b| knowledge.knows(b.tech)) {
        let best = b
            .variants
            .iter()
            .filter(|(v, _)| knowledge.knows_variant(*v))
            .map(|&(_, x)| x)
            .max()
            .unwrap_or(1000);
        m = m * u64::from(b.own) / 1000 * u64::from(best) / 1000;
    }
    m
}

/// The best `farmable` share someone who knows `knowledge` gets, per mille.
fn farmable(bonuses: &[Bonus], knowledge: &Knowledge) -> u32 {
    bonuses
        .iter()
        .filter(|b| knowledge.knows(b.tech))
        .map(|b| {
            let variant = b
                .variants
                .iter()
                .filter(|(v, _)| knowledge.knows_variant(*v))
                .map(|&(_, x)| x)
                .max()
                .unwrap_or(0);
            b.own.max(variant)
        })
        .max()
        .unwrap_or(0)
}

fn additive(list: &[Additive], knowledge: &Knowledge) -> u32 {
    list.iter()
        .filter(|a| match a.variant {
            None => knowledge.knows(a.tech),
            Some(v) => knowledge.knows_variant(v),
        })
        .map(|a| a.value)
        .sum()
}

fn knows_any(knowledge: &Knowledge, techs: &[TechId]) -> bool {
    techs.iter().any(|&t| knowledge.knows(t))
}

// ---------------------------------------------------------------------------
// Land quality and seasons
// ---------------------------------------------------------------------------

/// Everything the land systems read about the world.
struct Land<'a> {
    map: &'a Map,
    climate: &'a Climate,
    fertility: &'a Fertility,
    biomes: &'a Biomes,
    config: &'a EconomyConfig,
    state: &'a EconomyState,
}

impl Land<'_> {
    fn is_land(&self, i: usize) -> bool {
        let tile = &self.map.tiles()[i];
        tile.landform != Landform::Water
            && matches!(tile.water, Water::Dry | Water::River)
            && self.climate.cryosphere().tile(i).glacier < GLACIER
    }

    /// Whether fields grow on tile `t` in `season`.
    fn growing(&self, t: &TerritoryTile, season: Season) -> bool {
        growing(self.config, self.climate, t, season)
    }

    /// How good tile `t` is for `work` for someone knowing `knowledge`
    /// (per mille; 0: unusable).
    fn quality(&self, work: Work, t: &TerritoryTile, knowledge: &Knowledge) -> u32 {
        let i = t.index as usize;
        if !self.is_land(i) {
            return 0;
        }
        let fertility = u32::from(self.fertility.fertility(i));
        match work {
            Work::Farm => {
                let natural = !self.config.unfarmable_landforms.contains(&t.site.landform)
                    && !self
                        .config
                        .unfarmable_biomes
                        .contains(&self.biomes.biome(i));
                let share = if natural {
                    1000
                } else {
                    farmable(&t.farmable, knowledge)
                };
                fertility * share / 1000
            }
            Work::Herd => self
                .config
                .pasture
                .get(&self.biomes.biome(i))
                .copied()
                .unwrap_or(0)
                .min(1000),
            Work::Forage => {
                let floor = self.config.forage_floor.min(1000);
                floor + fertility * (1000 - floor) / 1000
            }
            Work::Fish => 1000,
        }
    }

    /// Output in `season`, per mille of the best season.
    fn season(&self, work: Work, t: &TerritoryTile, season: Season) -> u32 {
        let off = &self.config.off_season;
        match work {
            Work::Farm => {
                if self.growing(t, season) {
                    1000
                } else {
                    0
                }
            }
            Work::Herd if !self.growing(t, season) => off.herd,
            Work::Forage if !self.growing(t, season) => off.forage,
            Work::Fish
                if self.climate.normal(t.index as usize).seasonal_mean[season as usize]
                    < off.freezing =>
            {
                off.fish_frozen
            }
            _ => 1000,
        }
    }

    /// The places a city offers for `work`, best first: (tile, capacity in
    /// workers, quality). The city's knowledge decides which land counts
    /// as farmable and whether it farms or herds at all.
    fn plots(&self, work: Work, territory: &Territory, city: &Knowledge) -> Vec<Plot> {
        let requires = match work {
            Work::Farm => &self.state.farm_techs,
            Work::Herd => &self.state.herd_techs,
            _ => &Vec::new(),
        };
        if !requires.is_empty() && !knows_any(city, requires) {
            return Vec::new();
        }
        let mut plots = Vec::new();
        if work == Work::Fish {
            let grounds = &self.config.fish.workers;
            for (access, workers) in [
                (WaterAccess::Coast, grounds.coast),
                (WaterAccess::Lake, grounds.lake),
                (WaterAccess::River, grounds.river),
            ] {
                if let Some(tile) = territory
                    .tiles
                    .iter()
                    .position(|t| t.site.has(access) && self.is_land(t.index as usize))
                {
                    plots.push(Plot {
                        tile,
                        capacity: workers,
                        quality: 1000,
                    });
                }
            }
            return plots;
        }
        let work_config = self.config.work(work);
        let qualities: Vec<u32> = territory
            .tiles
            .iter()
            .map(|t| self.quality(work, t, city))
            .collect();
        let usable: u64 = territory
            .tiles
            .iter()
            .zip(&qualities)
            .filter(|(_, q)| **q > 0)
            .map(|(t, _)| u64::from(t.area_km2))
            .sum();
        for (tile, (t, &quality)) in territory.tiles.iter().zip(&qualities).enumerate() {
            if quality == 0 {
                continue;
            }
            let area = u64::from(t.area_km2);
            let worked = (u64::from(work_config.catchment_km2) * area / usable.max(1)).min(area);
            let capacity = worked * u64::from(work_config.workers_per_100_km2) / 100
                * u64::from(quality)
                / 1000;
            plots.push(Plot {
                tile,
                capacity: capacity.min(u64::from(u32::MAX)) as u32,
                quality,
            });
        }
        plots.sort_by_key(|p| std::cmp::Reverse(p.quality));
        plots
    }
}

/// Whether fields grow on tile `t` in `season`: the tile's long-term mean
/// for that season is at least `growing_season.min_temperature`, and it
/// gets `min_precipitation` or can be watered from a river or lake.
pub fn growing(
    config: &EconomyConfig,
    climate: &Climate,
    t: &TerritoryTile,
    season: Season,
) -> bool {
    let normal = climate.normal(t.index as usize);
    let s = season as usize;
    let rules = &config.growing_season;
    normal.seasonal_mean[s] >= rules.min_temperature
        && (normal.seasonal_precipitation[s] >= rules.min_precipitation
            || t.site.river
            || t.site.lake)
}

#[derive(Debug, Clone, Copy)]
struct Plot {
    /// Index into the territory's tiles.
    tile: usize,
    capacity: u32,
    quality: u32,
}

/// Food from one turn of `work`, thousandths of a person-day.
fn output(
    config: &EconomyConfig,
    work: Work,
    quality: u32,
    season: u32,
    tech: u64,
    tool: u32,
) -> u64 {
    let daily = match work {
        Work::Fish => config.fish.daily_yield,
        w => config.work(w).daily_yield,
    };
    u64::from(daily) / u64::from(config.work_turns_per_day.max(1)) * u64::from(quality) / 1000
        * u64::from(season)
        / 1000
        * tech
        / 1000
        * u64::from(tool)
        / 1000
}

// ---------------------------------------------------------------------------
// Work
// ---------------------------------------------------------------------------

/// The resolve step for food work, every turn: city by city, the workers in
/// id order take the best place left for their work; those who find none
/// produce nothing.
#[allow(clippy::too_many_arguments)]
fn work_the_land(
    turn: Res<Turn>,
    sim: Res<SimConfig>,
    config: Res<EconomyConfig>,
    state: Res<EconomyState>,
    map: Res<Map>,
    climate: Res<Climate>,
    fertility: Res<Fertility>,
    biomes: Res<Biomes>,
    tools: Res<ToolCatalog>,
    people: Query<(&PersonId, &Person, &Activity, &Skills, &Equipment)>,
    mut cities: Query<(&CityId, &Territory, &Knowledge, &mut Larder, &mut Stores)>,
) {
    let season = Date::from_turn(turn.0, sim.climate.days_per_year).season;
    let land = Land {
        map: &map,
        climate: &climate,
        fertility: &fertility,
        biomes: &biomes,
        config: &config,
        state: &state,
    };

    let mut workers: Vec<(CityId, Work, PersonId, &Skills, &Equipment)> = people
        .iter()
        .filter(|(_, _, a, _, _)| a.until != u64::MAX)
        .filter_map(|(id, p, a, s, e)| Work::of(a.action).map(|w| (p.home, w, *id, s, e)))
        .collect();
    workers.sort_by_key(|w| (w.0, w.1, w.2));

    let mut cities: Vec<_> = cities.iter_mut().collect();
    cities.sort_by_key(|c| *c.0);

    let mut start = 0;
    while start < workers.len() {
        let (city_id, work) = (workers[start].0, workers[start].1);
        let end = start
            + workers[start..]
                .iter()
                .take_while(|w| w.0 == city_id && w.1 == work)
                .count();
        let group = &workers[start..end];
        start = end;
        let Ok(c) = cities.binary_search_by_key(&city_id, |c| *c.0) else {
            continue;
        };
        let (_, territory, knowledge, larder, stores) = &mut cities[c];
        sync(larder, stores);

        let plots = land.plots(work, territory, knowledge);
        let mut left: Vec<u32> = plots.iter().map(|p| p.capacity).collect();
        let (mut busy, mut idle) = (0u32, 0u32);
        for &(_, _, _, skills, equipment) in group {
            let skills = &skills.0;
            let qualified = match work {
                Work::Farm => knows_any(skills, &state.farm_techs),
                Work::Herd => knows_any(skills, &state.herd_techs),
                _ => true,
            };
            if !qualified {
                idle += 1;
                continue;
            }
            // Only farm land can be open to some and not others (terraces,
            // new crops), so only farmers ever look past the first open plot.
            let place = (0..plots.len()).find(|&j| {
                left[j] > 0
                    && (work != Work::Farm
                        || land.quality(work, &territory.tiles[plots[j].tile], skills) > 0)
            });
            let Some(j) = place else {
                idle += 1;
                continue;
            };
            left[j] -= 1;
            busy += 1;
            let tile = &territory.tiles[plots[j].tile];
            let quality = land.quality(work, tile, skills);
            let tech = match work {
                Work::Farm => multiplier(&tile.food, skills),
                Work::Herd => multiplier(&tile.herd, skills),
                Work::Fish => {
                    let gear = if knows_any(skills, &state.fish_gear) {
                        1000
                    } else {
                        u64::from(config.fish.without_gear)
                    };
                    multiplier(&tile.fish, skills) * gear / 1000
                }
                Work::Forage => 1000,
            };
            let tool = tools.work_multiplier(equipment, action_of(work));
            let food = output(
                &config,
                work,
                quality,
                land.season(work, tile, season),
                tech,
                tool,
            );
            larder.produced_milli += food;
            if work == Work::Farm {
                if food > 0 {
                    *larder.standing.entry(tile.index).or_default() += food;
                }
            } else {
                larder.food_milli += food;
            }
        }
        let k = work as usize;
        larder.today.workers[k] = larder.today.workers[k].max(busy);
        larder.today.idle[k] = larder.today.idle[k].max(idle);
        stores.food = larder.food_milli / 1000;
    }
}

fn action_of(work: Work) -> ActionKind {
    match work {
        Work::Forage => ActionKind::Forage,
        Work::Farm => ActionKind::Farm,
        Work::Herd => ActionKind::Herd,
        Work::Fish => ActionKind::Fish,
    }
}

/// Takes over a change another module made to `Stores::food`.
fn sync(larder: &mut Larder, stores: &Stores) {
    if stores.food != larder.food_milli / 1000 {
        larder.food_milli = stores.food * 1000;
    }
}

// ---------------------------------------------------------------------------
// The daily round: harvest, spoilage, meals, outlook
// ---------------------------------------------------------------------------

/// Once a day, in the evening: brings in the harvest at the end of a
/// growing season, lets stored food spoil, feeds everyone at home, and
/// updates each city's [`Outlook`].
#[allow(clippy::too_many_arguments)]
fn daily_food(
    turn: Res<Turn>,
    sim: Res<SimConfig>,
    config: Res<EconomyConfig>,
    state: Res<EconomyState>,
    map: Res<Map>,
    climate: Res<Climate>,
    fertility: Res<Fertility>,
    biomes: Res<Biomes>,
    mut people: Query<(&Person, &Activity, &mut Needs)>,
    mut cities: Query<(
        &CityId,
        &Territory,
        &Knowledge,
        &mut Larder,
        &mut Stores,
        &mut Outlook,
    )>,
) {
    let days = sim.climate.days_per_year;
    let date = Date::from_turn(turn.0, days);
    if date.time_of_day != TimeOfDay::Evening {
        return;
    }
    // The crop of a season is brought in on the first evening of the next,
    // when the last turn of work on it is done.
    let previous = Date::from_turn(turn.0.saturating_sub(TURNS_PER_DAY), days).season;
    let season_began = turn.0 >= TURNS_PER_DAY && previous != date.season;
    let land = Land {
        map: &map,
        climate: &climate,
        fertility: &fertility,
        biomes: &biomes,
        config: &config,
        state: &state,
    };
    let tpy = sim.turns_per_year();

    // Who eats where: people and rations (thousandths of an adult's), by
    // city. People away with a caravan eat from its supplies.
    let slots = cities.iter().map(|c| c.0.0 as usize + 1).max().unwrap_or(0);
    let mut residents = vec![0u32; slots];
    let mut demand = vec![0u64; slots];
    let ration_of = |person: &Person| -> u64 {
        if person.age_years(turn.0, tpy) < config.eating.child_age {
            u64::from(config.eating.child_ration)
        } else {
            1000
        }
    };
    for (person, activity, _) in &people {
        if activity.until == u64::MAX {
            continue;
        }
        if let Some(d) = demand.get_mut(person.home.0 as usize) {
            *d += ration_of(person);
            residents[person.home.0 as usize] += 1;
        }
    }

    let mut share = vec![0u32; slots];
    for (id, territory, knowledge, mut larder, mut stores, mut outlook) in &mut cities {
        let c = id.0 as usize;
        sync(&mut larder, &stores);

        if season_began {
            let all_year = [
                Season::Spring,
                Season::Summer,
                Season::Autumn,
                Season::Winter,
            ];
            for t in &territory.tiles {
                let ends = land.growing(t, previous)
                    && (!land.growing(t, date.season)
                        || all_year.iter().all(|&s| land.growing(t, s)));
                if ends && let Some(crop) = larder.standing.remove(&t.index) {
                    larder.food_milli += crop;
                    larder.harvested_milli += crop;
                }
            }
        }

        // Spoilage: what fits the city's storage keeps well, the rest rots.
        let storage = &config.storage;
        let months = u64::from(storage.base_months + additive(&territory.storage, knowledge));
        let capacity = months * 30 * u64::from(residents[c]) * 1000;
        let kept = larder.food_milli.min(capacity);
        let open = larder.food_milli - kept;
        let preservation =
            additive(&territory.preservation, knowledge).min(storage.max_preservation);
        let loss = (kept * u64::from(storage.stored_loss) + open * u64::from(storage.open_loss))
            / 100_000
            * u64::from(1000 - preservation.min(1000))
            / 1000;
        larder.food_milli -= loss;
        larder.spoiled_milli += loss;

        // Meals: everyone gets the same share of a full ration.
        let ration = (larder.food_milli * 1000)
            .checked_div(demand[c])
            .map_or(1000, |r| r.min(1000) as u32);
        let eaten = demand[c] * u64::from(ration) / 1000;
        larder.food_milli -= eaten;
        larder.eaten_milli += eaten;
        larder.residents = residents[c];
        larder.ration = ration;
        share[c] = ration;

        // Outlook: room left on farm and pasture land (or, for those who
        // neither farm nor herd, foraging grounds), and days of food.
        larder.last_day = larder.today;
        larder.today = Workday::default();
        let room = |work: Work| -> u32 {
            let capacity: u32 = land
                .plots(work, territory, knowledge)
                .iter()
                .map(|p| p.capacity)
                .sum();
            capacity.saturating_sub(larder.last_day.workers[work as usize])
        };
        let farms = knows_any(knowledge, &state.farm_techs);
        let herds = knows_any(knowledge, &state.herd_techs);
        let free_farm = if farms { room(Work::Farm) } else { 0 };
        let free_pasture = if herds { room(Work::Herd) } else { 0 };
        let free = if farms || herds {
            free_farm + free_pasture
        } else {
            room(Work::Forage)
        };
        stores.food = larder.food_milli / 1000;
        let food_days = (stores.food / u64::from(residents[c].max(1))).min(u64::from(u32::MAX));
        let next = Outlook {
            free_land: free,
            free_farm,
            free_pasture,
            food_days: food_days as u32,
            tools_available: outlook.tools_available,
        };
        if *outlook != next {
            *outlook = next;
        }
    }

    let full = u64::from(config.eating.full_meal);
    for (person, activity, mut needs) in &mut people {
        if activity.until == u64::MAX {
            continue;
        }
        let ration = share.get(person.home.0 as usize).copied().unwrap_or(0);
        let relief = (full * u64::from(ration) / 1000) as u16;
        if relief > 0 && needs.get(Need::Food) > 0 {
            needs.satisfy(Need::Food, relief);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_builtin_tuning_parses_and_names_real_technologies() {
        let config = EconomyConfig::builtin();
        let tree = TechTree::builtin();
        for name in config
            .farm_requires_any
            .iter()
            .chain(&config.herd_requires_any)
            .chain(&config.fish.gear)
        {
            assert!(tree.id(name).is_some(), "unknown technology {name}");
        }
        assert!(config.work_turns_per_day > 0);
        assert!(config.pasture[&Biome::Grassland] > config.pasture[&Biome::BorealForest]);
    }

    #[test]
    fn variants_count_once_and_the_best_known_wins() {
        let tree = TechTree::builtin();
        let site = Site {
            biome: Biome::Grassland,
            landform: Landform::Plains,
            soil: crate::map::Soil::Loam,
            river: false,
            lake: false,
            coast: false,
        };
        let food = bonuses(&tree, &site, "food_yield", Combine::Product);
        let cereals = tree.id("cereal_cultivation").unwrap();
        let legumes = tree.id("legumes").unwrap();
        let mut k = Knowledge::new(&tree);
        assert_eq!(multiplier(&food, &k), 1000);
        k.learn(&tree, cereals, tree.variant_id("millet"));
        assert_eq!(multiplier(&food, &k), 900);
        k.learn(&tree, cereals, tree.variant_id("wheat_barley"));
        assert_eq!(multiplier(&food, &k), 1000);
        k.learn(&tree, legumes, None);
        assert_eq!(multiplier(&food, &k), 1080);
        // Rice only pays on a river or lake.
        let mut rice = Knowledge::new(&tree);
        rice.learn(&tree, cereals, tree.variant_id("rice"));
        assert_eq!(multiplier(&food, &rice), 1000);
        let wet = Site {
            river: true,
            ..site
        };
        let food = bonuses(&tree, &wet, "food_yield", Combine::Product);
        assert_eq!(multiplier(&food, &rice), 1300);
    }
}
