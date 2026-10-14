//! Tools: crafters make the items their technologies allow, workers take
//! them from the stores and work better with them, and tools wear out.

mod common;

use common::Fixture;
use societies::Simulation;
use societies::city::CityId;
use societies::economy::{Outlook, Stores};
use societies::people::{ActionKind, Activity, Person, PersonId, Skills};
use societies::tech::{Knowledge, TechTree};
use societies::time::TURNS_PER_DAY;
use societies::tools::{Equipment, ItemId, ToolCatalog};

const CITY: CityId = CityId(0);

/// The first `count` adults of `CITY` (in id order) take up `action` for
/// good; everyone else there stays idle.
fn assign(sim: &mut Simulation, count: usize, action: ActionKind) -> Vec<PersonId> {
    let (turn, tpy) = (sim.turn(), sim.config().turns_per_year());
    let mut adults: Vec<PersonId> = sim
        .people()
        .iter()
        .filter(|p| p.person.home == CITY && p.person.age_years(turn, tpy) >= 16)
        .map(|p| p.id)
        .collect();
    adults.truncate(count);
    assert_eq!(adults.len(), count, "the city has enough adults");
    let world = sim.world_mut();
    // Keep them at it: no need ever becomes urgent enough to re-decide.
    world
        .resource_mut::<societies::people::decide::ActionCatalog>()
        .urgent = u16::MAX;
    let mut query = world.query::<(&PersonId, &Person, &mut Activity)>();
    for (id, person, mut activity) in query.iter_mut(world) {
        if person.home != CITY {
            continue;
        }
        *activity = Activity {
            action: if adults.contains(id) {
                action
            } else {
                ActionKind::Idle
            },
            since: turn,
            // Never re-decided during the test (u64::MAX would mean away).
            until: u64::MAX - 1,
        };
    }
    adults
}

/// Makes `who` know exactly the technologies `techs` (by id).
fn teach(sim: &mut Simulation, who: &[PersonId], techs: &[&str]) {
    let tree = sim.technologies().clone();
    let world = sim.world_mut();
    let mut query = world.query::<(&PersonId, &mut Skills)>();
    for (id, mut skills) in query.iter_mut(world) {
        if who.contains(id) {
            let mut k = Knowledge::new(&tree);
            for t in techs {
                k.learn(&tree, tree.id(t).unwrap(), None);
            }
            skills.0 = k;
        }
    }
}

fn stores(sim: &mut Simulation, city: CityId) -> (Vec<u32>, u32) {
    let world = sim.world_mut();
    let mut query = world.query::<(&CityId, &Stores, &Outlook)>();
    query
        .iter(world)
        .find(|(id, ..)| **id == city)
        .map(|(_, s, o)| (s.items.clone(), o.tools_available))
        .unwrap()
}

fn set_stores(sim: &mut Simulation, items: &[(ItemId, u32)]) {
    let n = catalog(sim).items.len();
    let world = sim.world_mut();
    let mut query = world.query::<(&CityId, &mut Stores)>();
    for (id, mut s) in query.iter_mut(world) {
        if *id == CITY {
            s.items = vec![0; n];
            for &(item, count) in items {
                s.items[usize::from(item.0)] = count;
            }
        }
    }
}

/// What the people of `CITY` carry. (Other cities run their own lives.)
fn equipment(sim: &mut Simulation) -> Vec<(PersonId, Equipment)> {
    let world = sim.world_mut();
    let mut query = world.query::<(&PersonId, &Person, &Equipment)>();
    let mut out: Vec<_> = query
        .iter(world)
        .filter(|(_, p, _)| p.home == CITY)
        .map(|(id, _, e)| (*id, *e))
        .collect();
    out.sort_by_key(|e| e.0);
    out
}

fn catalog(sim: &Simulation) -> ToolCatalog {
    sim.world().resource::<ToolCatalog>().clone()
}

fn item(sim: &Simulation, name: &str) -> ItemId {
    catalog(sim).id(name).unwrap()
}

fn days(sim: &mut Simulation, n: u64) {
    sim.run(n * TURNS_PER_DAY);
}

/// Items in the stores plus items carried, by item.
fn made(sim: &mut Simulation, name: &str) -> u32 {
    let id = item(sim, name);
    let held = equipment(sim)
        .iter()
        .filter(|(_, e)| e.tool == Some(id))
        .count() as u32;
    stores(sim, CITY).0[usize::from(id.0)] + held
}

#[test]
fn smiths_with_copper_make_copper_tools() {
    let mut sim = common::sim(Fixture::Small);
    let smiths = assign(&mut sim, 8, ActionKind::Craft);
    teach(&mut sim, &smiths, &["fire", "stone_tools", "native_copper"]);
    days(&mut sim, 12);
    let copper = item(&sim, "copper_tools");
    let n = made(&mut sim, "copper_tools");
    // 8 smiths for 12 days at 4 person-days a tool, a little faster once
    // they carry tools of their own.
    assert!((20..=30).contains(&n), "{n} copper tools");
    assert_eq!(
        made(&mut sim, "stone_tools"),
        0,
        "nobody else works, so no stone"
    );
    // The smiths carry copper tools of their own now.
    for (id, e) in equipment(&mut sim) {
        if smiths.contains(&id) {
            assert_eq!(e.tool, Some(copper));
        }
    }
    let (items, available) = stores(&mut sim, CITY);
    assert_eq!(available, items.iter().sum::<u32>());
    assert!(available > 0);
}

#[test]
fn without_the_technology_there_is_no_such_tool() {
    let mut sim = common::sim(Fixture::Small);
    let crafters = assign(&mut sim, 8, ActionKind::Craft);
    teach(&mut sim, &crafters, &["fire", "stone_tools"]);
    days(&mut sim, 10);
    assert_eq!(made(&mut sim, "copper_tools"), 0);
    assert_eq!(made(&mut sim, "bronze_tools"), 0);
    assert!(made(&mut sim, "stone_tools") >= 70);

    // Knowing nothing at all, they make nothing.
    let mut sim = common::sim(Fixture::Small);
    let crafters = assign(&mut sim, 8, ActionKind::Craft);
    teach(&mut sim, &crafters, &["fire"]);
    days(&mut sim, 5);
    let (items, available) = stores(&mut sim, CITY);
    assert_eq!(available, 0, "{items:?}");
}

#[test]
fn crafters_make_what_the_workers_need() {
    // Farmers want sickles; the crafter knows both sickles and stone tools.
    let mut sim = common::sim(Fixture::Small);
    let people = assign(&mut sim, 21, ActionKind::Farm);
    teach(
        &mut sim,
        &people[..1],
        &["fire", "stone_tools", "cereal_cultivation"],
    );
    let world = sim.world_mut();
    let mut query = world.query::<(&PersonId, &mut Activity)>();
    for (id, mut a) in query.iter_mut(world) {
        if *id == people[0] {
            a.action = ActionKind::Craft;
        }
    }
    days(&mut sim, 10);
    let sickles = made(&mut sim, "flint_sickle");
    assert!(sickles >= 8, "{sickles} sickles");
    let farmers_with_sickles = equipment(&mut sim)
        .iter()
        .filter(|(id, e)| people[1..].contains(id) && e.tool == Some(item(&sim, "flint_sickle")))
        .count();
    assert!(farmers_with_sickles >= 8, "{farmers_with_sickles}");
}

#[test]
fn workers_take_the_best_tool_and_work_better() {
    let mut sim = common::sim(Fixture::Small);
    let farmers = assign(&mut sim, 3, ActionKind::Farm);
    let (stone, copper) = (item(&sim, "stone_tools"), item(&sim, "copper_tools"));
    set_stores(&mut sim, &[(stone, 5), (copper, 1)]);
    let cat = catalog(&sim);
    let before = equipment(&mut sim);
    for (_, e) in &before {
        assert_eq!(cat.work_multiplier(e, ActionKind::Farm), 1000);
    }
    days(&mut sim, 1);
    let after = equipment(&mut sim);
    let tool = |id: PersonId| after.iter().find(|e| e.0 == id).unwrap().1;
    // The first in id order gets the copper tool, the others stone.
    assert_eq!(tool(farmers[0]).tool, Some(copper));
    assert_eq!(tool(farmers[1]).tool, Some(stone));
    assert_eq!(tool(farmers[2]).tool, Some(stone));
    for &f in &farmers {
        assert!(cat.work_multiplier(&tool(f), ActionKind::Farm) > 1000);
    }
    assert!(
        cat.work_multiplier(&tool(farmers[0]), ActionKind::Farm)
            > cat.work_multiplier(&tool(farmers[1]), ActionKind::Farm)
    );
    // Idle people took nothing.
    let carried = after.iter().filter(|(_, e)| e.tool.is_some()).count();
    assert_eq!(carried, 3);
    let (items, available) = stores(&mut sim, CITY);
    assert_eq!(items[usize::from(stone.0)], 3);
    assert_eq!(items[usize::from(copper.0)], 0);
    assert_eq!(available, 3);
}

#[test]
fn tools_wear_out_and_break() {
    let mut sim = common::sim(Fixture::Small);
    let stone = item(&sim, "stone_tools");
    sim.world_mut().resource_mut::<ToolCatalog>().items[usize::from(stone.0)].durability_days = 6;
    let farmer = assign(&mut sim, 1, ActionKind::Farm)[0];
    set_stores(&mut sim, &[(stone, 2)]);
    let mut wear = Vec::new();
    for _ in 0..13 {
        days(&mut sim, 1);
        let e = equipment(&mut sim)
            .into_iter()
            .find(|e| e.0 == farmer)
            .unwrap()
            .1;
        wear.push((e.tool.is_some(), e.wear));
    }
    // A six-day tool: worn on each of its six days, then gone; the second
    // one is taken the next day; after that the stores are empty.
    let worn: Vec<u16> = wear[..5].iter().map(|w| w.1).collect();
    assert!(worn.windows(2).all(|w| w[0] < w[1]), "{wear:?}");
    assert!(wear[..5].iter().all(|w| w.0), "{wear:?}");
    assert_eq!(wear[5], (false, 0), "broken on day six: {wear:?}");
    assert!(
        wear[6].0 && wear[10].0 && !wear[11].0 && !wear[12].0,
        "{wear:?}"
    );
    assert_eq!(stores(&mut sim, CITY).1, 0);
}

#[test]
fn people_away_take_no_tools() {
    let mut sim = common::sim(Fixture::Small);
    let farmers = assign(&mut sim, 2, ActionKind::Craft);
    teach(&mut sim, &farmers, &["fire", "stone_tools"]);
    let world = sim.world_mut();
    let mut query = world.query::<(&PersonId, &mut Activity)>();
    for (_, mut a) in query.iter_mut(world) {
        if a.action == ActionKind::Craft {
            a.until = u64::MAX;
        }
    }
    let stone = item(&sim, "stone_tools");
    set_stores(&mut sim, &[(stone, 4)]);
    days(&mut sim, 3);
    assert!(equipment(&mut sim).iter().all(|(_, e)| e.tool.is_none()));
    assert_eq!(stores(&mut sim, CITY).0[usize::from(stone.0)], 4);
}

#[test]
fn tool_work_is_deterministic() {
    let run = |threads| {
        let mut sim = common::sim(Fixture::Small);
        sim.set_threads(threads);
        let smiths = assign(&mut sim, 6, ActionKind::Craft);
        teach(
            &mut sim,
            &smiths[..3],
            &["fire", "stone_tools", "copper_smelting"],
        );
        let world = sim.world_mut();
        let mut query = world.query::<(&PersonId, &Person, &mut Activity)>();
        let mut n = 0;
        for (_, p, mut a) in query.iter_mut(world) {
            if p.home == CITY && a.action == ActionKind::Idle && n < 20 {
                a.action = [ActionKind::Farm, ActionKind::Fish, ActionKind::Build][n % 3];
                a.until = u64::MAX - 1;
                n += 1;
            }
        }
        days(&mut sim, 8);
        (
            sim.state_digest(),
            equipment(&mut sim),
            stores(&mut sim, CITY),
        )
    };
    let one = run(1);
    assert!(one.2.1 > 0 || one.1.iter().any(|e| e.1.tool.is_some()));
    assert_eq!(run(1), one);
    assert_eq!(run(4), one);
}

#[test]
fn the_catalog_matches_the_technologies() {
    let tree = TechTree::builtin();
    let catalog = ToolCatalog::builtin();
    assert!(!catalog.items.is_empty());
    for item in &catalog.items {
        assert!(!item.requires_any.is_empty(), "{}", item.id);
        for t in &item.requires_any {
            assert!(tree.id(t).is_some(), "{}: unknown technology {t}", item.id);
        }
        assert!(
            item.work.iter().any(|&m| m > 1000),
            "{} helps nothing",
            item.id
        );
    }
    // Better metal, better tools.
    let farm = |name: &str| {
        let e = Equipment {
            tool: catalog.id(name),
            wear: 0,
        };
        catalog.work_multiplier(&e, ActionKind::Farm)
    };
    assert!(farm("stone_tools") < farm("copper_tools"));
    assert!(farm("copper_tools") < farm("bronze_tools"));
    assert_eq!(
        catalog.work_multiplier(&Equipment::default(), ActionKind::Farm),
        1000
    );
}
