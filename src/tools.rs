//! Tools: items (stone, copper, bronze tools...) made by people who craft
//! them, such as a blacksmith, and kept in their city's
//! [`Stores`](crate::economy::Stores). People take tools from the stores
//! ([`Equipment`]), work better with them, and wear them out.
//!
//! Everything happens once a day, at midday (see [`daily_tools`]): whoever
//! is working then counts as having worked that day. City by city, in
//! `PersonId` order, workers first take a tool if they have none that helps
//! them, then wear the one they use, and then the crafters put their day's
//! labour into the item their city is shortest of. Someone who knows copper
//! smelting is a smith in effect: no status is needed, only the knowledge.
//! Ores are not on the map yet, so metal tools need only the technology.
//!
//! Tuning: the `tools` section of `data/people.json`.

use std::collections::BTreeMap;

use bevy_ecs::prelude::*;
use bevy_ecs::schedule::ScheduleConfigs;
use bevy_ecs::system::ScheduleSystem;

use crate::city::CityId;
use crate::economy::{Outlook, Stores};
use crate::people::{ActionKind, Activity, Person, PersonId, Skills};
use crate::tech::{TechId, TechTree};
use crate::time::{TURNS_PER_DAY, Turn};

/// How many kinds of action there are (the length of [`Item::work`]).
const ACTIONS: usize = ActionKind::SettleAway as usize + 1;

/// The actions a tool can help with; the people doing them are the
/// workers who take tools and wear them out.
const WORK: [ActionKind; 6] = [
    ActionKind::Forage,
    ActionKind::Farm,
    ActionKind::Herd,
    ActionKind::Fish,
    ActionKind::Build,
    ActionKind::Craft,
];

/// The turn of the day (0 morning, 1 day...) on which the day's work is
/// counted.
const WORK_TURN: u64 = 1;

/// Inserts this module's resources (its section of `data/people.json`).
pub fn install(world: &mut World) {
    world.insert_resource(ToolCatalog::builtin());
    world.init_resource::<Workshops>();
}

/// This module's systems, in order, for its slot in the turn.
pub fn systems() -> ScheduleConfigs<ScheduleSystem> {
    daily_tools.into_configs()
}

/// The `tools` section of `data/people.json`.
#[derive(Debug, Clone, serde::Deserialize)]
struct ToolsFile {
    items: Vec<ItemFile>,
    reserve_per_mille: u32,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct ItemFile {
    id: String,
    name: String,
    requires_any: Vec<String>,
    labour_days: u32,
    durability_days: u32,
    #[serde(default)]
    work: BTreeMap<ActionKind, u32>,
}

/// One kind of item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub id: String,
    pub name: String,
    /// Technologies (by id), any one of which lets a person make it. Names
    /// the technology tree does not know are ignored.
    pub requires_any: Vec<String>,
    /// Work to make one, in thousandths of a person-day.
    pub labour: u32,
    /// Working days before it breaks (at least 1).
    pub durability_days: u32,
    /// Output with this tool, per mille, by [`ActionKind`] (1000: no help).
    pub work: [u32; ACTIONS],
}

/// The item catalog, read by `economy` (and anyone else whose work tools
/// speed up). Items are listed from least to most valued; [`ItemId`] is the
/// index. An empty catalog (the default) makes tools no different.
#[derive(Resource, Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolCatalog {
    pub items: Vec<Item>,
    /// A city is short of an item while its stores hold fewer than this
    /// share (per mille) of the workers it would help.
    pub reserve_per_mille: u32,
}

impl ToolCatalog {
    /// The catalog in the built-in `data/people.json`.
    pub fn builtin() -> Self {
        let file: ToolsFile = crate::people::section("tools");
        Self {
            items: file
                .items
                .into_iter()
                .map(|i| {
                    let mut work = [1000; ACTIONS];
                    for (action, m) in i.work {
                        work[action as usize] = m;
                    }
                    Item {
                        id: i.id,
                        name: i.name,
                        requires_any: i.requires_any,
                        labour: i.labour_days.max(1) * 1000,
                        durability_days: i.durability_days.max(1),
                        work,
                    }
                })
                .collect(),
            reserve_per_mille: file.reserve_per_mille,
        }
    }

    /// The item called `name`.
    pub fn id(&self, name: &str) -> Option<ItemId> {
        self.items
            .iter()
            .position(|i| i.id == name)
            .map(|i| ItemId(i as u16))
    }

    pub fn item(&self, id: ItemId) -> &Item {
        &self.items[usize::from(id.0)]
    }

    /// How productive someone with `equipment` is at `action`, per mille
    /// (1000: as without tools).
    pub fn work_multiplier(&self, equipment: &Equipment, action: ActionKind) -> u32 {
        match equipment.tool {
            Some(id) if equipment.wear < 1000 => self
                .items
                .get(usize::from(id.0))
                .map_or(1000, |i| i.work[action as usize]),
            _ => 1000,
        }
    }

    /// For each item, the items that can stand in for it (itself
    /// included): those that help at least as much with the work it helps
    /// most. Copper tools stand in for stone ones, but not for a sickle,
    /// which reaps better.
    fn stand_ins(&self) -> Vec<Vec<usize>> {
        let n = self.items.len();
        (0..n)
            .map(|i| {
                let work = &self.items[i].work;
                let main = (0..ACTIONS).max_by_key(|&a| (work[a], a)).unwrap_or(0);
                (0..n)
                    .filter(|&j| self.items[j].work[main] >= work[main])
                    .collect()
            })
            .collect()
    }

    /// For each item, the technologies that let a person make it.
    fn makers(&self, tree: &TechTree) -> Vec<Vec<TechId>> {
        self.items
            .iter()
            .map(|i| i.requires_any.iter().filter_map(|t| tree.id(t)).collect())
            .collect()
    }
}

/// Index into the item catalog.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub struct ItemId(pub u16);

/// The tool a person carries, if any, and how worn it is (0 new, 1000
/// broken).
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Equipment {
    pub tool: Option<ItemId>,
    pub wear: u16,
}

/// Unfinished work in each city: for each item, the labour already put
/// into the next one, in thousandths of a person-day. Crafters of a city
/// pool their work, so a piece left half done is finished by whoever makes
/// that item next.
#[derive(Resource, Debug, Clone, Default, PartialEq, Eq)]
pub struct Workshops(pub BTreeMap<CityId, Vec<u32>>);

/// The day's tool work, run on the midday turn: for every city in id order,
/// its workers in id order take and wear tools, then its crafters craft.
/// Workers away with a caravan (`Activity::until == u64::MAX`) do nothing.
/// The cost is one pass over everyone a day plus a sort of the workers.
#[allow(clippy::type_complexity)]
pub fn daily_tools(
    turn: Res<Turn>,
    catalog: Res<ToolCatalog>,
    tree: Res<TechTree>,
    mut workshops: ResMut<Workshops>,
    mut people: Query<(
        Entity,
        &PersonId,
        &Person,
        &Activity,
        &Skills,
        &mut Equipment,
    )>,
    mut cities: Query<(&CityId, &mut Stores, &mut Outlook)>,
) {
    if turn.0 % TURNS_PER_DAY != WORK_TURN || catalog.items.is_empty() {
        return;
    }
    let day = turn.0 / TURNS_PER_DAY;
    let makers = catalog.makers(&tree);
    let stand_ins = catalog.stand_ins();
    let n = catalog.items.len();

    let mut workers: Vec<(CityId, PersonId, ActionKind, Entity)> = people
        .iter()
        .filter(|(_, _, _, a, _, _)| a.until != u64::MAX && WORK.contains(&a.action))
        .map(|(e, id, p, a, _, _)| (p.home, *id, a.action, e))
        .collect();
    workers.sort_unstable_by_key(|w| (w.0, w.1));

    let mut rows: Vec<_> = cities.iter_mut().collect();
    rows.sort_by_key(|(id, ..)| **id);
    let mut next = 0;
    let mut crafters: Vec<(Entity, u32)> = Vec::new();
    for (city, stores, outlook) in rows.iter_mut() {
        while next < workers.len() && workers[next].0 < **city {
            next += 1;
        }
        let start = next;
        while next < workers.len() && workers[next].0 == **city {
            next += 1;
        }
        let here = &workers[start..next];
        if stores.items.len() < n {
            stores.items.resize(n, 0);
        }

        // Tools first, so a crafter's own tool already speeds up today's
        // work.
        let mut by_action = [0u32; ACTIONS];
        crafters.clear();
        for &(_, _, action, entity) in here {
            by_action[action as usize] += 1;
            let Ok((.., mut equipment)) = people.get_mut(entity) else {
                continue;
            };
            take_tool(&catalog, &mut stores.items, &mut equipment, action);
            if action == ActionKind::Craft {
                crafters.push((entity, catalog.work_multiplier(&equipment, action)));
            }
            wear(&catalog, &mut equipment, action, day);
        }

        if !crafters.is_empty() {
            // How many of each item the city wants in reserve.
            let wanted: Vec<u32> = catalog
                .items
                .iter()
                .map(|item| {
                    let helped: u32 = (0..ACTIONS)
                        .filter(|&a| item.work[a] > 1000)
                        .map(|a| by_action[a])
                        .sum();
                    (helped * catalog.reserve_per_mille).div_ceil(1000)
                })
                .collect();
            let progress = workshops.0.entry(**city).or_default();
            progress.resize(n.max(progress.len()), 0);
            for &(entity, labour) in &crafters {
                let Ok((_, _, _, _, skills, _)) = people.get(entity) else {
                    continue;
                };
                let Some(i) = choose(&makers, &stand_ins, &skills.0, &wanted, &stores.items) else {
                    continue;
                };
                progress[i] += labour;
                let cost = catalog.items[i].labour;
                if progress[i] >= cost {
                    stores.items[i] += progress[i] / cost;
                    progress[i] %= cost;
                }
            }
        }

        let available = stores.items.iter().sum();
        if outlook.tools_available != available {
            outlook.tools_available = available;
        }
    }
}

/// Gives a worker the best tool in `items` for `action` if the one they
/// carry (if any) does not help with it. The tool they put down goes back
/// into the stores; how worn it was is forgotten (a simplification: such
/// swaps are rare, because the general tools help with all work).
fn take_tool(
    catalog: &ToolCatalog,
    items: &mut [u32],
    equipment: &mut Equipment,
    action: ActionKind,
) {
    if catalog.work_multiplier(equipment, action) > 1000 {
        return;
    }
    let best = (0..items.len())
        .filter(|&i| items[i] > 0 && catalog.items[i].work[action as usize] > 1000)
        .max_by_key(|&i| (catalog.items[i].work[action as usize], i));
    let Some(best) = best else {
        return;
    };
    if let Some(old) = equipment.tool
        && let Some(count) = items.get_mut(usize::from(old.0))
    {
        *count += 1;
    }
    items[best] -= 1;
    *equipment = Equipment {
        tool: Some(ItemId(best as u16)),
        wear: 0,
    };
}

/// Wears the worker's tool by one working day if it helped with today's
/// work, and throws it away once broken. A day's wear is
/// `1000 / durability`, with the remainder spread over the days by the day
/// number (as in Bresenham's line algorithm), so that any run of
/// `durability` working days in a row adds exactly 1000.
fn wear(catalog: &ToolCatalog, equipment: &mut Equipment, action: ActionKind, day: u64) {
    let Some(tool) = equipment.tool else {
        return;
    };
    if catalog.work_multiplier(equipment, action) <= 1000 {
        return;
    }
    let d = u64::from(catalog.item(tool).durability_days);
    let r = 1000 % d;
    let step = 1000 / d + ((day + 1) * r / d - day * r / d);
    let worn = u64::from(equipment.wear) + step;
    *equipment = if worn >= 1000 {
        Equipment::default()
    } else {
        Equipment {
            tool: Some(tool),
            wear: worn as u16,
        }
    };
}

/// What a crafter makes: the most valued item they know how to make that
/// the city is short of; if it is short of none, the most valued one that
/// helps anyone working there; otherwise the most valued they know.
///
/// The city is short of an item while it holds fewer than `wanted` of it
/// and of the items that stand in for it (see [`ToolCatalog::stand_ins`]):
/// a smith with copper tools in reserve does not go back to making stone
/// ones.
fn choose(
    makers: &[Vec<TechId>],
    stand_ins: &[Vec<usize>],
    skills: &crate::tech::Knowledge,
    wanted: &[u32],
    stock: &[u32],
) -> Option<usize> {
    let known = || {
        (0..makers.len())
            .rev()
            .filter(|&i| makers[i].iter().any(|&t| skills.knows(t)))
    };
    let held = |i: usize| stand_ins[i].iter().map(|&j| stock[j]).sum::<u32>();
    known()
        .find(|&i| held(i) < wanted[i])
        .or_else(|| known().find(|&i| wanted[i] > 0))
        .or_else(|| known().next())
}

/// Feeds this module's state (stores of items, tools carried, unfinished
/// work), in id order, to `write` (for digests).
pub fn digest(world: &mut World, write: &mut impl FnMut(u64)) {
    let mut cities: Vec<(CityId, Vec<u32>, u32)> = world
        .query::<(&CityId, &Stores, &Outlook)>()
        .iter(world)
        .map(|(id, s, o)| (*id, s.items.clone(), o.tools_available))
        .collect();
    cities.sort_by_key(|c| c.0);
    for (id, items, available) in cities {
        write(u64::from(id.0));
        items.iter().for_each(|&c| write(u64::from(c)));
        write(u64::from(available));
    }
    for (id, progress) in &world.resource::<Workshops>().0 {
        write(u64::from(id.0));
        progress.iter().for_each(|&p| write(u64::from(p)));
    }
    let mut carried: Vec<(PersonId, Equipment)> = world
        .query::<(&PersonId, &Equipment)>()
        .iter(world)
        .map(|(id, e)| (*id, *e))
        .collect();
    carried.sort_by_key(|c| c.0);
    for (id, e) in carried {
        write(id.0);
        write(e.tool.map_or(u64::MAX, |t| u64::from(t.0)));
        write(u64::from(e.wear));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tool_lasts_exactly_its_durability() {
        let catalog = ToolCatalog::builtin();
        for (i, item) in catalog.items.iter().enumerate() {
            for start in [0, 5, 1234] {
                let mut e = Equipment {
                    tool: Some(ItemId(i as u16)),
                    wear: 0,
                };
                let action = WORK
                    .into_iter()
                    .find(|&a| item.work[a as usize] > 1000)
                    .unwrap();
                let mut days = 0;
                while e.tool.is_some() {
                    wear(&catalog, &mut e, action, start + days);
                    days += 1;
                }
                assert_eq!(days, u64::from(item.durability_days), "{}", item.id);
            }
        }
    }

    #[test]
    fn idle_people_do_not_wear_tools() {
        let catalog = ToolCatalog::builtin();
        let mut e = Equipment {
            tool: Some(ItemId(0)),
            wear: 10,
        };
        wear(&catalog, &mut e, ActionKind::Idle, 3);
        assert_eq!(e.wear, 10);
    }
}
