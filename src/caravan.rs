//! Caravans: when enough people of a city choose to settle away on the same
//! day, they leave together on the next day as a caravan, which travels in
//! search of a place to settle and founds a new city there.
//!
//! # Who leaves
//! Every morning, the adults of each city who chose `SettleAway` during the
//! previous day are counted. If there are at least `min_adults`, they leave
//! together, and their children under the founding `adult_age` follow their
//! mothers. A partner who did not choose to leave stays behind.
//!
//! # While travelling
//! Members are unavailable: their activity is `SettleAway` until
//! `u64::MAX`, so `decide` leaves them alone and `economy` and `tools` do not
//! count them as workers. Their home is the caravan itself, a [`CityId`] at
//! or above [`TRAVELLING`] that no city ever has (see [`Caravan::home`]).
//! That keeps them out of their old city's census without any special case,
//! and anyone born on the way belongs to the caravan from birth because a
//! child's home is its mother's. Their culture stays their old city's.
//!
//! The caravan eats the food it took along (one person-day per member and
//! day) and walks `km_per_day`, slower over hills and mountains and when it
//! fords a river; lakes and the sea stop it. Each day its scouts look over
//! the land within `scout_radius_km` for the best reachable site (see
//! [`site_value`]), less a penalty for the distance to it, away from every
//! other city. It keeps its goal until it gets there or the goal becomes
//! unusable. With no site in sight it heads for the reachable land farthest
//! from home. After `max_days` it gives up and goes back home.
//!
//! # Founding
//! On arrival the members found a city with the next [`NextCityId`]: the
//! origin's kind of people, what the members know between them, the food
//! they have left, and a name drawn from their seeds and the date. They
//! become available again.
//!
//! Everything here happens once a day, in the morning, and costs a pass over
//! the people plus a little per caravan; caravans are few.
//!
//! Tuning: the `caravan` section of `data/people.json`.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use bevy_ecs::prelude::*;

use crate::people::decide::Plan;
use bevy_ecs::schedule::ScheduleConfigs;
use bevy_ecs::system::ScheduleSystem;

use crate::biome::Biomes;
use crate::city::{City, CityId, NextCityId, Population, can_settle, cradle_score, too_close};
use crate::climate::Climate;
use crate::config::SimConfig;
use crate::economy::{Outlook, Stores};
use crate::events::{EventLog, SimEvent};
use crate::fertility::Fertility;
use crate::map::{Landform, Map, Position, Water};
use crate::people::founding::FoundingConfig;
use crate::people::rng::{PersonRng, mix};
use crate::people::status::StatusCatalog;
use crate::people::{ActionKind, Activity, Family, Need, Needs, Person, PersonId, Skills, Status};
use crate::rng::SimRng;
use crate::tech::{Knowledge, TechTree};
use crate::time::{TURNS_PER_DAY, Turn};

/// Homes at or above this id are caravans on the road, not cities.
pub const TRAVELLING: u32 = 0x8000_0000;

/// The `caravan` section of `data/people.json`.
#[derive(Resource, Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct CaravanConfig {
    /// Adults of one city who must choose to settle away on the same day
    /// for a caravan to leave.
    pub min_adults: u32,
    /// Distance walked per day on open plains.
    pub km_per_day: u32,
    /// Cost of entering a tile, per mille of its width: plains, hills and
    /// mountains.
    pub terrain_cost: TerrainCost,
    /// Extra cost of entering a river tile (fording), per mille.
    pub river_crossing: u32,
    /// How far ahead the scouts look.
    pub scout_radius_km: u32,
    /// Lowest [`site_value`] the caravan settles for.
    pub min_site_score: u32,
    /// Points of site value given up per 100 km of travel to reach it.
    pub distance_penalty_per_100km: u32,
    /// The caravan turns back after this many days on the road.
    pub max_days: u32,
    /// Most food taken along, in days per member.
    pub food_days: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct TerrainCost {
    pub plains: u32,
    pub hills: u32,
    pub mountains: u32,
}

impl CaravanConfig {
    pub fn builtin() -> Self {
        crate::people::section("caravan")
    }
}

/// Stable identifier of a caravan, in order of departure.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub struct CaravanId(pub u32);

/// The next [`CaravanId`] to hand out.
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct NextCaravanId(pub u32);

/// Where a caravan is heading.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Goal {
    pub index: u32,
    /// A site to settle on; otherwise just somewhere to look further from.
    pub settle: bool,
}

/// A group of people on the road, looking for a place to settle.
#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub struct Caravan {
    pub id: CaravanId,
    /// The city they left (and go back to if they give up).
    pub origin: CityId,
    /// Everyone travelling, in id order (kept up to date daily: deaths
    /// leave, births on the way join).
    pub members: Vec<PersonId>,
    pub position: Position,
    pub days: u32,
    /// Food carried, in person-days of eating.
    pub food: u64,
    /// Kilometers walked toward the next tile.
    pub progress_km: u32,
    pub goal: Option<Goal>,
}

impl Caravan {
    /// The home its members have while travelling: no city has it, so the
    /// census counts them nowhere.
    pub fn home(&self) -> CityId {
        CityId(TRAVELLING + self.id.0)
    }
}

/// Whether a person with this home is on the road with a caravan.
pub fn is_travelling(home: CityId) -> bool {
    home.0 >= TRAVELLING
}

/// Inserts this module's resources (its section of `data/people.json`).
pub fn install(world: &mut World) {
    world.insert_resource(CaravanConfig::builtin());
    world.init_resource::<NextCaravanId>();
}

/// This module's systems, in order, for its slot in the turn: once a day,
/// caravans on the road move (and perhaps settle), then new ones leave.
pub fn systems() -> ScheduleConfigs<ScheduleSystem> {
    (travel, depart).chain().run_if(new_day)
}

fn new_day(turn: Res<Turn>) -> bool {
    turn.0.is_multiple_of(TURNS_PER_DAY)
}

/// All caravans on the road, in id order.
pub fn caravans(world: &mut World) -> Vec<Caravan> {
    let mut query = world.query::<&Caravan>();
    let mut out: Vec<Caravan> = query.iter(world).cloned().collect();
    out.sort_by_key(|c| c.id);
    out
}

/// How good a place `index` is to settle: its [`cradle_score`] if it is a
/// cradle-like site, otherwise half its fertility (any settleable land
/// will do, but water and warmth count for much). 0 if it cannot be settled.
pub fn site_value(
    map: &Map,
    climate: &Climate,
    biomes: &Biomes,
    fertility: &Fertility,
    index: usize,
) -> u64 {
    if !can_settle(map, climate, index) {
        return 0;
    }
    match cradle_score(map, climate, biomes, fertility, index) {
        0 => u64::from(fertility.fertility(index)) / 2,
        score => score,
    }
}

/// Kilometers walked to enter tile `to` from its neighbor `from`, or None if
/// it cannot be entered (lake or sea). A tile is crossed at its width on the
/// globe (narrower east-west toward the poles), times its terrain cost.
pub fn step_km(
    map: &Map,
    config: &SimConfig,
    tuning: &CaravanConfig,
    from: usize,
    to: usize,
) -> Option<u32> {
    let tile = &map.tiles()[to];
    let terrain = match tile.landform {
        Landform::Water => return None,
        Landform::Plains => tuning.terrain_cost.plains,
        Landform::Hills => tuning.terrain_cost.hills,
        Landform::Mountains => tuning.terrain_cost.mountains,
    };
    let cost = match tile.water {
        Water::Lake | Water::Ocean => return None,
        Water::River => terrain + tuning.river_crossing,
        Water::Dry => terrain,
    };
    let (a, b) = (map.position_of(from as u32), map.position_of(to as u32));
    let circumference = u64::from(config.climate.planet_circumference_km);
    let ns = circumference / 2 / u64::from(map.height());
    let row = (a.y + b.y) / 2;
    let ew = (circumference / u64::from(map.width()) * map.area_weight(row) as u64 / 1000).max(1);
    // Diagonals: the longer side plus half the shorter, close to the true
    // diagonal without square roots.
    let km = match (a.x != b.x, a.y != b.y) {
        (true, true) => ns.max(ew) + ns.min(ew) / 2,
        (true, false) => ew,
        _ => ns,
    };
    Some((km * u64::from(cost) / 1000).max(1) as u32)
}

/// What `travel` reads and changes about people.
type Travellers<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static PersonId,
        &'static mut Person,
        &'static mut Activity,
        &'static mut Needs,
        &'static Skills,
        &'static PersonRng,
    ),
>;

/// The land and rules caravans travel and settle by.
type TheWorld<'w> = (
    Res<'w, Map>,
    Res<'w, Climate>,
    Res<'w, Biomes>,
    Res<'w, Fertility>,
    Res<'w, TechTree>,
    Res<'w, SimConfig>,
);

/// Moves caravans on the road, feeds them, and founds cities where they
/// arrive (or sends them home when they give up). A resolve step: caravans
/// in id order.
#[allow(clippy::too_many_arguments)]
fn travel(
    mut commands: Commands,
    turn: Res<Turn>,
    world: TheWorld,
    tuning: Res<CaravanConfig>,
    mut log: ResMut<EventLog>,
    mut next_city: ResMut<NextCityId>,
    mut people: Travellers,
    mut cities: Query<(&CityId, &City, &Position, &mut Stores)>,
    mut caravans: Query<(Entity, &mut Caravan)>,
) {
    if caravans.is_empty() {
        return;
    }
    let (map, climate, biomes, fertility, tree, config) = world;
    let now = turn.0;

    // Who is on the road with which caravan, read from their homes.
    let mut travellers: Vec<(CityId, PersonId, Entity)> = people
        .iter()
        .filter(|(_, _, p, ..)| is_travelling(p.home))
        .map(|(e, id, p, ..)| (p.home, *id, e))
        .collect();
    travellers.sort();

    let mut taken: Vec<Position> = cities.iter().map(|(_, _, p, _)| *p).collect();
    let mut order: Vec<(CaravanId, Entity)> = caravans.iter().map(|(e, c)| (c.id, e)).collect();
    order.sort();

    for (_, entity) in order {
        let mut caravan = caravans.get_mut(entity).unwrap().1;
        let home = caravan.home();
        let start = travellers.partition_point(|t| t.0 < home);
        let end = travellers.partition_point(|t| t.0 <= home);
        let members = &travellers[start..end];
        caravan.members = members.iter().map(|t| t.1).collect();
        if members.is_empty() {
            commands.entity(entity).despawn();
            continue;
        }

        // Everyone stays unavailable (newborns too), and eats from the
        // carried food while it lasts, the first in id order first.
        for &(_, _, e) in members {
            let (_, _, _, mut activity, mut needs, ..) = people.get_mut(e).unwrap();
            if activity.until != u64::MAX {
                *activity = Activity {
                    action: ActionKind::SettleAway,
                    since: now,
                    until: u64::MAX,
                };
            }
            if caravan.food > 0 {
                caravan.food -= 1;
                needs.set(Need::Food, 0);
            }
        }

        caravan.days += 1;
        if caravan.days > tuning.max_days {
            // Give up and go home with what food is left (the way back is
            // not simulated).
            let origin = caravan.origin;
            if let Some((.., mut stores)) = cities.iter_mut().find(|c| *c.0 == origin) {
                stores.food += caravan.food;
            }
            settle_members(&mut people, members, origin, now);
            commands.entity(entity).despawn();
            continue;
        }

        let here = map.index(caravan.position);
        let scout = Scout::run(&map, &config, &tuning, here);
        let usable = |index: usize, taken: &[Position]| {
            let pos = map.position_of(index as u32);
            can_settle(&map, &climate, index)
                && site_value(&map, &climate, &biomes, &fertility, index)
                    >= u64::from(tuning.min_site_score)
                && !taken.iter().any(|&t| too_close(&map, &config, pos, t))
        };

        // Keep a site that is still good; otherwise look for the best one
        // in sight, and failing that for land to look further from.
        let site = |i: usize| {
            usable(i, &taken).then(|| site_value(&map, &climate, &biomes, &fertility, i))
        };
        let reachable = |g: &Goal| scout.cost(g.index as usize).is_some();
        let old = caravan.goal;
        caravan.goal = old
            .filter(|g| g.settle && reachable(g) && usable(g.index as usize, &taken))
            .or_else(|| scout_site(&scout, &tuning, site))
            .or_else(|| old.filter(|g| !g.settle && reachable(g) && g.index as usize != here))
            .or_else(|| {
                let origin = city_position(&cities, caravan.origin).unwrap_or(caravan.position);
                farthest_land(&scout, &map, &config, origin, here)
            });

        // Walk along the cheapest path toward the goal.
        let Some(goal) = caravan.goal else {
            continue;
        };
        let path = scout.path_to(goal.index as usize);
        let mut budget = caravan.progress_km + tuning.km_per_day;
        let mut at = here;
        for &next in &path {
            let cost = scout.cost(next).unwrap() - scout.cost(at).unwrap();
            if cost > budget {
                break;
            }
            budget -= cost;
            at = next;
        }
        caravan.position = map.position_of(at as u32);
        caravan.progress_km = if at == goal.index as usize { 0 } else { budget };

        if at == goal.index as usize && goal.settle && usable(at, &taken) {
            let id = CityId(next_city.0);
            next_city.0 += 1;
            let origin = cities.iter().find(|c| *c.0 == caravan.origin);
            let kind = origin.map_or(crate::city::People::Barbarian, |c| c.1.people);
            let mut knowledge = Knowledge::new(&tree);
            let mut seed = mix(now, u64::from(id.0));
            for &(_, _, e) in members {
                let (_, _, _, _, _, skills, rng) = people.get(e).unwrap();
                for v in skills.0.variants() {
                    knowledge.learn(&tree, tree.variant_of(v), Some(v));
                }
                for t in skills.0.techs() {
                    knowledge.learn(&tree, t, None);
                }
                seed = mix(seed, rng.seed);
            }
            let name = crate::city::generate_name(&mut SimRng::from_seed(seed));
            let position = caravan.position;
            log.push(SimEvent::CityFounded {
                turn: now,
                id,
                name: name.clone(),
                position,
                people: kind,
            });
            commands.spawn((
                id,
                City {
                    name,
                    founded_on: now,
                    people: kind,
                },
                position,
                Population::new(members.len() as u64),
                knowledge,
                Stores {
                    food: caravan.food,
                    items: Vec::new(),
                },
                Outlook::default(),
            ));
            taken.push(position);
            settle_members(&mut people, members, id, now);
            commands.entity(entity).despawn();
        }
    }
}

/// Where city `id` is, if it still exists.
fn city_position(
    cities: &Query<(&CityId, &City, &Position, &mut Stores)>,
    id: CityId,
) -> Option<Position> {
    cities.iter().find(|c| *c.0 == id).map(|c| *c.2)
}

/// Makes the members residents of `city` and available again.
fn settle_members(
    people: &mut Travellers,
    members: &[(CityId, PersonId, Entity)],
    city: CityId,
    now: u64,
) {
    for &(_, _, e) in members {
        let (_, _, mut person, mut activity, ..) = people.get_mut(e).unwrap();
        person.home = city;
        *activity = Activity {
            action: ActionKind::Idle,
            since: now,
            until: now,
        };
    }
}

/// The best site the scouts found: the highest `value` less the distance
/// penalty; ties go to the lower tile index.
fn scout_site(
    scout: &Scout,
    tuning: &CaravanConfig,
    value: impl Fn(usize) -> Option<u64>,
) -> Option<Goal> {
    scout
        .reached()
        .filter_map(|(i, km)| {
            let v = value(i)?;
            let penalty = u64::from(km) * u64::from(tuning.distance_penalty_per_100km) / 100;
            Some((v.saturating_sub(penalty), Reverse(i)))
        })
        .max()
        .map(|(_, Reverse(i))| Goal {
            index: i as u32,
            settle: true,
        })
}

/// With no site in sight: the reachable land farthest from home, to look
/// further from. None if the caravan is already there.
fn farthest_land(
    scout: &Scout,
    map: &Map,
    config: &SimConfig,
    origin: Position,
    here: usize,
) -> Option<Goal> {
    let circumference = i64::from(config.climate.planet_circumference_km);
    let km2 = |i: usize| {
        let p = map.position_of(i as u32);
        let ns = i64::from(p.y.abs_diff(origin.y)) * circumference / 2 / i64::from(map.height());
        let ew = i64::from(p.x.abs_diff(origin.x)) * circumference / i64::from(map.width())
            * map.area_weight((p.y + origin.y) / 2)
            / 1000;
        ns * ns + ew * ew
    };
    let (best, i) = scout
        .reached()
        .map(|(i, _)| (km2(i), Reverse(i)))
        .max()
        .map(|(d, Reverse(i))| (d, i))?;
    (i != here && best > km2(here)).then_some(Goal {
        index: i as u32,
        settle: false,
    })
}

/// The cheapest walking distance from the caravan to every tile within
/// How many rows (and columns) around a caravan its scouts search, for rows
/// `per_row_km` tall: `scout_km`, but always at least half again the
/// minimum distance between cities (anything nearer home is too close to
/// it, so a smaller window could hold nothing to find), at least 2, and at
/// most 64, which bounds the daily search to 129 × 129 tiles.
pub fn scout_rows(per_row_km: u64, scout_km: u32, min_city_distance_km: u32) -> usize {
    let per_row = per_row_km.max(1);
    let wanted = u64::from(scout_km).div_ceil(per_row);
    let beyond_neighbors = (u64::from(min_city_distance_km) * 3 / 2).div_ceil(per_row) + 1;
    wanted.max(beyond_neighbors).clamp(2, 64) as usize
}

/// scouting range (Dijkstra over a small square window; ties by tile
/// index, so the result never depends on anything but the map).
struct Scout {
    width: usize,
    x0: usize,
    y0: usize,
    side_x: usize,
    side_y: usize,
    map_width: usize,
    cost: Vec<Option<u32>>,
    from: Vec<usize>,
}

impl Scout {
    fn run(map: &Map, config: &SimConfig, tuning: &CaravanConfig, here: usize) -> Self {
        let per_row =
            (u64::from(config.climate.planet_circumference_km) / 2 / u64::from(map.height()))
                .max(1);
        let radius = scout_rows(per_row, tuning.scout_radius_km, config.min_city_distance_km);
        let (w, h) = (map.width() as usize, map.height() as usize);
        let (x, y) = (here % w, here / w);
        let (x0, y0) = (x.saturating_sub(radius), y.saturating_sub(radius));
        let (x1, y1) = ((x + radius).min(w - 1), (y + radius).min(h - 1));
        let (side_x, side_y) = (x1 - x0 + 1, y1 - y0 + 1);
        let mut scout = Self {
            width: side_x,
            x0,
            y0,
            side_x,
            side_y,
            map_width: w,
            cost: vec![None; side_x * side_y],
            from: vec![usize::MAX; side_x * side_y],
        };
        let start = scout.local(here).unwrap();
        scout.cost[start] = Some(0);
        let mut heap = BinaryHeap::new();
        heap.push(Reverse((0u32, here)));
        while let Some(Reverse((km, i))) = heap.pop() {
            if scout.cost[scout.local(i).unwrap()] != Some(km) {
                continue;
            }
            for n in map.neighbors(i) {
                let Some(ln) = scout.local(n) else {
                    continue;
                };
                let Some(step) = step_km(map, config, tuning, i, n) else {
                    continue;
                };
                let total = km + step;
                if scout.cost[ln].is_none_or(|c| total < c) {
                    scout.cost[ln] = Some(total);
                    scout.from[ln] = i;
                    heap.push(Reverse((total, n)));
                }
            }
        }
        scout
    }

    /// Index into the window, if the tile is in it.
    fn local(&self, index: usize) -> Option<usize> {
        let (x, y) = (index % self.map_width, index / self.map_width);
        (x >= self.x0 && y >= self.y0 && x < self.x0 + self.side_x && y < self.y0 + self.side_y)
            .then(|| (y - self.y0) * self.width + (x - self.x0))
    }

    fn cost(&self, index: usize) -> Option<u32> {
        self.cost[self.local(index)?]
    }

    /// Every reachable tile and its walking distance, in tile order.
    fn reached(&self) -> impl Iterator<Item = (usize, u32)> + '_ {
        self.cost.iter().enumerate().filter_map(|(l, c)| {
            let (x, y) = (self.x0 + l % self.width, self.y0 + l / self.width);
            c.map(|km| (y * self.map_width + x, km))
        })
    }

    /// The tiles walked through to reach `target`, in order, excluding the
    /// start.
    fn path_to(&self, target: usize) -> Vec<usize> {
        let mut path = Vec::new();
        let mut at = target;
        while let Some(l) = self.local(at) {
            if self.from[l] == usize::MAX {
                break;
            }
            path.push(at);
            at = self.from[l];
        }
        path.reverse();
        path
    }
}

/// What `depart` needs to know about someone who might leave.
type Leaver = (
    Entity,
    &'static PersonId,
    &'static mut Person,
    &'static mut Activity,
    &'static mut Family,
    Option<&'static mut Plan>,
    &'static mut Status,
);

/// Sends out a caravan from every city where at least `min_adults` adults
/// chose to settle away during the previous day. A resolve step: cities in
/// id order.
#[allow(clippy::too_many_arguments)]
fn depart(
    mut commands: Commands,
    turn: Res<Turn>,
    config: Res<SimConfig>,
    tuning: Res<CaravanConfig>,
    founding: Res<FoundingConfig>,
    mut next_caravan: ResMut<NextCaravanId>,
    statuses: Res<StatusCatalog>,
    mut people: Query<Leaver>,
    mut cities: Query<(&CityId, &Position, &Population, &mut Stores)>,
) {
    let now = turn.0;
    let tpy = config.turns_per_year();
    let adult = founding.adult_age;
    // What someone did yesterday is what they went to sleep doing:
    // `decide` records it in `Plan::last` every night.
    let mut leaving: Vec<(CityId, PersonId, Entity)> = people
        .iter()
        .filter(|(_, _, p, a, _, plan, _)| {
            plan.is_some_and(|plan| plan.last == ActionKind::SettleAway)
                && a.until != u64::MAX
                && !is_travelling(p.home)
                && p.age_years(now, tpy) >= adult
        })
        .map(|(e, id, p, ..)| (p.home, *id, e))
        .collect();
    if leaving.len() < tuning.min_adults as usize {
        return;
    }
    leaving.sort();

    // Group by city; keep the cities with enough adults.
    let mut groups: Vec<(CityId, Vec<(PersonId, Entity)>)> = Vec::new();
    for (city, id, e) in leaving {
        match groups.last_mut() {
            Some((c, g)) if *c == city => g.push((id, e)),
            _ => groups.push((city, vec![(id, e)])),
        }
    }
    groups.retain(|(_, g)| g.len() >= tuning.min_adults as usize);
    if groups.is_empty() {
        return;
    }

    // Children under the adult age follow their mothers.
    let children: Vec<(CityId, PersonId, PersonId, Entity)> = people
        .iter()
        .filter(|(_, _, p, a, f, _, _)| {
            f.mother.is_some()
                && a.until != u64::MAX
                && !is_travelling(p.home)
                && p.age_years(now, tpy) < adult
        })
        .map(|(e, id, p, _, f, _, _)| (p.home, f.mother.unwrap(), *id, e))
        .collect();

    for (city, adults) in groups {
        let mut members = adults.clone();
        for &(home, mother, id, e) in &children {
            if home == city && adults.binary_search_by_key(&mother, |a| a.0).is_ok() {
                members.push((id, e));
            }
        }
        members.sort();

        let id = CaravanId(next_caravan.0);
        next_caravan.0 += 1;
        let Some((_, &position, population, mut stores)) = cities.iter_mut().find(|c| *c.0 == city)
        else {
            continue;
        };
        // Their share of the stores, as much as they can carry.
        let n = members.len() as u64;
        let share = stores.food * n / population.count.max(n);
        let food = share.min(n * u64::from(tuning.food_days));
        stores.food -= food;

        let caravan = Caravan {
            id,
            origin: city,
            members: members.iter().map(|m| m.0).collect(),
            position,
            days: 0,
            food,
            progress_km: 0,
            goal: None,
        };
        // A couple splits when only one of them leaves: both links are
        // cleared, so each may find someone new where they live.
        let staying: Vec<(PersonId, PersonId)> = members
            .iter()
            .filter_map(|&(id, e)| {
                let partner = people.get(e).ok()?.4.partner?;
                members
                    .binary_search_by_key(&partner, |m| m.0)
                    .is_err()
                    .then_some((id, partner))
            })
            .collect();
        if !staying.is_empty() {
            let entities: Vec<(PersonId, Entity)> = {
                let mut v: Vec<_> = people.iter().map(|(e, id, ..)| (*id, e)).collect();
                v.sort();
                v
            };
            for (leaver, partner) in staying {
                for id in [leaver, partner] {
                    if let Ok(i) = entities.binary_search_by_key(&id, |x| x.0)
                        && let Ok((_, _, _, _, mut family, _, _)) = people.get_mut(entities[i].1)
                    {
                        family.partner = None;
                    }
                }
            }
        }
        for &(_, e) in &members {
            let (_, _, mut person, mut activity, _, plan, mut status) = people.get_mut(e).unwrap();
            person.home = caravan.home();
            // Positions stay with the city: only a status held for life
            // goes along (a chief who leaves is no longer its chief).
            if !statuses.status(status.0).is_some_and(|s| s.for_life) {
                *status = Status::default();
            }
            // The choice is spent: once settled (or back home) they decide
            // afresh instead of leaving again.
            if let Some(mut plan) = plan {
                plan.last = ActionKind::Idle;
            }
            *activity = Activity {
                action: ActionKind::SettleAway,
                since: now,
                until: u64::MAX,
            };
        }
        commands.spawn(caravan);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scouts_always_see_past_the_nearest_allowed_site() {
        // Rows on a 40 000 km planet: 24, 64, 128 and 512 rows tall.
        let tuning = CaravanConfig::builtin();
        let min_km = SimConfig::default().min_city_distance_km;
        for height in [24u64, 64, 128, 512] {
            let per_row = 20_000 / height;
            let rows = scout_rows(per_row, tuning.scout_radius_km, min_km);
            let reach_km = rows as u64 * per_row;
            assert!(
                reach_km > u64::from(min_km),
                "{height} rows: scouts see {reach_km} km, sites start at {min_km} km"
            );
            assert!((2..=64).contains(&rows));
        }
        // The configured range wins when it is larger.
        assert_eq!(scout_rows(312, 1500, 800), 5);
        // A tiny range is widened past the minimum distance.
        assert!(scout_rows(39, 100, 800) * 39 > 800);
    }

    #[test]
    fn the_builtin_tuning_parses() {
        let c = CaravanConfig::builtin();
        assert!(c.min_adults >= 2);
        assert!(c.terrain_cost.plains < c.terrain_cost.hills);
        assert!(c.terrain_cost.hills < c.terrain_cost.mountains);
        assert!(c.km_per_day > 0 && c.max_days > 0);
    }

    #[test]
    fn caravan_homes_are_never_cities() {
        let c = Caravan {
            id: CaravanId(3),
            origin: CityId(1),
            members: Vec::new(),
            position: Position { x: 0, y: 0 },
            days: 0,
            food: 0,
            progress_km: 0,
            goal: None,
        };
        assert!(is_travelling(c.home()));
        assert!(!is_travelling(CityId(1)));
    }
}
