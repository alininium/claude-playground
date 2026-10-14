//! Statuses and learning: positions unlocked by technologies, their slots,
//! how statuses steer actions and ease the Esteem need, children and adults
//! learning, and technologies dying out with their last knower.

mod common;

use std::collections::BTreeMap;

use common::Fixture;
use societies::Simulation;
use societies::city::CityId;
use societies::people::status::{StatusCatalog, StatusSchedule};
use societies::people::{
    ActionKind, Need, Needs, Person, PersonId, PersonSnapshot, Skills, StatusId,
};
use societies::tech::{Knowledge, TechTree};

fn catalog(sim: &Simulation) -> StatusCatalog {
    sim.world().resource::<StatusCatalog>().clone()
}

fn status(sim: &Simulation, name: &str) -> StatusId {
    catalog(sim).id(name).unwrap()
}

/// Makes every city review its statuses and recompute its knowledge on the
/// next turn.
fn make_due(sim: &mut Simulation) {
    let turn = sim.turn();
    let world = sim.world_mut();
    let mut q = world.query::<&mut StatusSchedule>();
    for mut s in q.iter_mut(world) {
        s.next_review = turn + 1;
        s.next_knowledge = turn + 1;
    }
}

/// Changes the skills of everyone in `city`.
fn edit_skills(sim: &mut Simulation, city: CityId, mut f: impl FnMut(&TechTree, &mut Knowledge)) {
    let tree = sim.technologies().clone();
    let world = sim.world_mut();
    let mut q = world.query::<(&Person, &mut Skills)>();
    for (p, mut s) in q.iter_mut(world) {
        if p.home == city {
            f(&tree, &mut s.0);
        }
    }
}

fn residents(sim: &mut Simulation, city: CityId) -> Vec<PersonSnapshot> {
    sim.people()
        .into_iter()
        .filter(|p| p.person.home == city)
        .collect()
}

fn holders(people: &[PersonSnapshot], s: StatusId) -> usize {
    people.iter().filter(|p| p.status == s).count()
}

fn tune(sim: &mut Simulation, f: impl FnOnce(&mut societies::people::status::StatusConfig)) {
    let mut c = catalog(sim);
    f(&mut c.config);
    sim.world_mut()
        .insert_resource(StatusCatalog::new(c.config.clone()));
}

#[test]
fn the_catalog_names_real_technologies() {
    let c = StatusCatalog::builtin();
    assert_eq!(c.unknown_technologies(&TechTree::builtin()), Vec::<String>::new());
    assert_eq!(c.id("member"), Some(StatusId(0)));
    for name in ["elder", "chief", "noble", "priest", "shaman", "scribe"] {
        assert!(c.id(name).is_some(), "{name}");
    }
    for name in ["artisan", "smith", "warrior", "king", "bonded"] {
        assert!(c.id(name).is_some(), "{name}");
    }
}

#[test]
fn statuses_appear_only_with_their_technologies() {
    let mut sim = common::sim(Fixture::Medium);
    sim.step();
    let c = catalog(&sim);
    let tree = sim.technologies().clone();
    let cities = sim.cities();
    let people = sim.people();
    let mut seen = BTreeMap::new();
    for p in &people {
        if p.status == StatusId(0) {
            continue;
        }
        let city = cities.iter().find(|c| c.id == p.person.home).unwrap();
        assert!(
            c.unlocked(&tree, &city.knowledge, p.status),
            "{} without its technology",
            c.status(p.status).unwrap().id
        );
        *seen.entry(p.status).or_insert(0) += 1;
    }
    // Kin leadership and ancestor cults are known everywhere at the start.
    for name in ["chief", "elder", "shaman"] {
        assert!(seen.contains_key(&status(&sim, name)), "no {name} anywhere");
    }
    assert!(!seen.contains_key(&status(&sim, "king")));

    // A city that forgets everything but fire keeps only plain members...
    let city = CityId(0);
    let fire = tree.id("fire").unwrap();
    edit_skills(&mut sim, city, |tree, k| {
        *k = Knowledge::new(tree);
        k.learn(tree, fire, None);
    });
    make_due(&mut sim);
    sim.step();
    let here = residents(&mut sim, city);
    assert!(here.iter().all(|p| p.status == StatusId(0)));

    // ...until it learns kin leadership again.
    let kin = tree.id("kin_leadership").unwrap();
    edit_skills(&mut sim, city, |tree, k| k.learn(tree, kin, None));
    make_due(&mut sim);
    sim.step();
    let here = residents(&mut sim, city);
    assert_eq!(holders(&here, status(&sim, "chief")), 1);
    assert!(holders(&here, status(&sim, "elder")) > 0);
}

#[test]
fn positions_respect_their_slots() {
    let mut sim = common::sim(Fixture::Medium);
    let tree = sim.technologies().clone();
    // City 1 becomes a kingdom with bonded labour; the rest stay as they are.
    let kingdom = CityId(1);
    let names = ["ranked_society", "kingship", "bonded_labor", "organized_infantry"];
    edit_skills(&mut sim, kingdom, |tree, k| {
        for n in names {
            k.learn(tree, tree.id(n).unwrap(), None);
        }
    });
    let c = catalog(&sim);
    let (chief, king) = (status(&sim, "chief"), status(&sim, "king"));
    let turn_years = |sim: &Simulation, p: &PersonSnapshot| {
        p.person.age_years(sim.turn(), sim.config().turns_per_year())
    };
    let cfg = c.config.clone();
    for round in 0..3 {
        if round > 0 {
            make_due(&mut sim);
        }
        sim.step();
        let cities = sim.cities();
        for city in &cities {
            let here = residents(&mut sim, city.id);
            let adults = here
                .iter()
                .filter(|p| turn_years(&sim, p) >= cfg.adult_age)
                .count() as u64;
            assert!(holders(&here, chief) + holders(&here, king) <= 1);
            for (s, def) in cfg.statuses.iter().enumerate().skip(1) {
                let slots = def.count.map(u64::from).unwrap_or_else(|| {
                    (adults * u64::from(def.per_mille.unwrap()) + 500) / 1000
                });
                let n = holders(&here, StatusId(s as u16)) as u64;
                assert!(n <= slots, "{}: {n} of {slots} in city {}", def.id, city.id.0);
                for p in here.iter().filter(|p| p.status == StatusId(s as u16)) {
                    let age = turn_years(&sim, p);
                    assert!(age >= def.min_age.max(cfg.adult_age), "{} aged {age}", def.id);
                    assert!(def.max_age.is_none_or(|m| age <= m));
                }
            }
        }
        let here = residents(&mut sim, kingdom);
        assert_eq!(holders(&here, king), 1, "the kingdom has its king");
        assert_eq!(holders(&here, chief), 0, "and no chief");
        assert!(holders(&here, status(&sim, "noble")) > 0);
        assert!(holders(&here, status(&sim, "bonded")) > 0);
        assert!(holders(&here, status(&sim, "warrior")) > 0);
        let _ = &tree;
    }
}

#[test]
fn the_kings_child_inherits_the_throne() {
    let mut sim = common::sim(Fixture::Medium);
    let kingdom = CityId(1);
    edit_skills(&mut sim, kingdom, |tree, k| {
        for n in ["ranked_society", "kingship"] {
            k.learn(tree, tree.id(n).unwrap(), None);
        }
    });
    sim.step();
    let king = status(&sim, "king");
    let here = residents(&mut sim, kingdom);
    let old = here.iter().find(|p| p.status == king).unwrap().id;
    let tpy = sim.config().turns_per_year();
    let heirs: Vec<PersonId> = here
        .iter()
        .filter(|p| {
            (p.family.mother == Some(old) || p.family.father == Some(old))
                && p.person.age_years(sim.turn(), tpy) >= 16
        })
        .map(|p| p.id)
        .collect();
    if heirs.is_empty() {
        return; // A childless king; nothing to inherit.
    }
    let world = sim.world_mut();
    let mut q = world.query::<(bevy_ecs::entity::Entity, &PersonId)>();
    let e = q.iter(world).find(|(_, id)| **id == old).unwrap().0;
    world.despawn(e);
    make_due(&mut sim);
    sim.step();
    let here = residents(&mut sim, kingdom);
    let new = here.iter().find(|p| p.status == king).unwrap().id;
    assert!(heirs.contains(&new), "the throne went outside the family");
}

#[test]
fn children_learn_from_their_parents() {
    let mut sim = common::sim(Fixture::Medium);
    tune(&mut sim, |c| {
        c.learn_days = 1;
        c.child_learning = 1000;
    });
    let tpy = sim.config().turns_per_year();
    let before = sim.people();
    let age = |p: &PersonSnapshot, turn| p.person.age_years(turn, tpy);
    let turn0 = sim.turn();
    let children: Vec<&PersonSnapshot> = before
        .iter()
        .filter(|p| (6..12).contains(&age(p, turn0)) && p.family.mother.is_some())
        .collect();
    assert!(children.len() > 50);
    sim.run(4 * 12);
    let after: BTreeMap<PersonId, PersonSnapshot> =
        sim.people().into_iter().map(|p| (p.id, p)).collect();
    let (mut gained, mut total) = (0, 0);
    for child in &children {
        let now = &after[&child.id];
        let parents: Vec<&PersonSnapshot> = [child.family.mother, child.family.father]
            .into_iter()
            .flatten()
            .filter_map(|id| after.get(&id))
            .collect();
        total += 1;
        if now.skills.count() > child.skills.count() {
            gained += 1;
        }
        for t in now.skills.techs() {
            if !child.skills.knows(t) {
                assert!(
                    parents.iter().any(|p| p.skills.knows(t)),
                    "a child learned what no parent knows"
                );
            }
        }
    }
    assert!(gained * 2 > total, "only {gained} of {total} children learned");
}

#[test]
fn adults_learn_what_their_city_knows() {
    let mut sim = common::sim(Fixture::Medium);
    tune(&mut sim, |c| {
        c.learn_days = 1;
        c.adult_learning = 1000;
    });
    let before: BTreeMap<PersonId, usize> =
        sim.people().iter().map(|p| (p.id, p.skills.count())).collect();
    sim.run(4 * 10);
    let tree = sim.technologies().clone();
    let cities = sim.cities();
    let mut gained = 0;
    for p in sim.people() {
        let city = cities.iter().find(|c| c.id == p.person.home).unwrap();
        for t in p.skills.techs() {
            assert!(city.knowledge.knows(t), "learned beyond the city");
            assert!(tree.prerequisites_met(&p.skills, t), "{}", tree.tech(t).id);
        }
        if before.get(&p.id).is_some_and(|&n| p.skills.count() > n) {
            gained += 1;
        }
    }
    assert!(gained > 100, "{gained} people learned something");
}

#[test]
fn a_technology_dies_with_its_last_knower() {
    let mut sim = common::sim(Fixture::Medium);
    sim.step();
    let tree = sim.technologies().clone();
    let cities = sim.cities();
    // The rarest technology some city knows.
    let mut best = None;
    for city in &cities {
        let here = residents(&mut sim, city.id);
        for t in city.knowledge.techs() {
            let knowers = here.iter().filter(|p| p.skills.knows(t)).count();
            if knowers > 0 && knowers * 4 < here.len() && best.is_none_or(|(_, _, n)| knowers < n) {
                best = Some((city.id, t, knowers));
            }
        }
    }
    let (city, tech, _) = best.expect("some technology is known by few");
    let world = sim.world_mut();
    let mut q = world.query::<(bevy_ecs::entity::Entity, &Person, &Skills)>();
    let doomed: Vec<_> = q
        .iter(world)
        .filter(|(_, p, s)| p.home == city && s.0.knows(tech))
        .map(|(e, ..)| e)
        .collect();
    for e in doomed {
        world.despawn(e);
    }
    make_due(&mut sim);
    tune(&mut sim, |c| {
        c.learn_days = 1;
        c.adult_learning = 1000;
        c.child_learning = 1000;
    });
    sim.step();
    let known = |sim: &mut Simulation| {
        sim.cities()
            .into_iter()
            .find(|c| c.id == city)
            .unwrap()
            .knowledge
            .knows(tech)
    };
    assert!(!known(&mut sim), "{} outlived its knowers", tree.tech(tech).id);
    // Nobody can learn it again from the city.
    sim.run(4 * 10);
    assert!(residents(&mut sim, city).iter().all(|p| !p.skills.knows(tech)));
    assert!(!known(&mut sim));
}

#[test]
fn statuses_steer_actions() {
    let c = StatusCatalog::builtin();
    let id = |n| c.id(n).unwrap();
    assert!(c.leaning(id("smith"), ActionKind::Craft) > 0);
    assert!(c.leaning(id("artisan"), ActionKind::Craft) > 0);
    assert!(c.leaning(id("priest"), ActionKind::Worship) > 0);
    assert!(c.leaning(id("shaman"), ActionKind::Worship) > 0);
    assert!(c.leaning(id("warrior"), ActionKind::Farm) < 0);
    assert!(!c.allows(id("bonded"), ActionKind::SettleAway));
    assert!(c.allows(id("bonded"), ActionKind::Farm));
    assert!(!c.allows(id("king"), ActionKind::Farm));
    let member = StatusId(0);
    for a in [ActionKind::Farm, ActionKind::Craft, ActionKind::SettleAway] {
        assert!(c.allows(member, a));
        assert_eq!(c.leaning(member, a), 0);
    }
    // Unknown statuses, and the empty catalog, are neutral.
    assert!(c.allows(StatusId(999), ActionKind::Farm));
    assert!(StatusCatalog::default().allows(id("bonded"), ActionKind::SettleAway));
}

#[test]
fn standing_eases_the_need_for_esteem() {
    let mut sim = common::sim(Fixture::Small);
    sim.step();
    {
        let world = sim.world_mut();
        let mut q = world.query::<&mut Needs>();
        for mut n in q.iter_mut(world) {
            n.set(Need::Esteem, 1000);
        }
    }
    while sim.turn() % 4 != 3 {
        sim.step();
    }
    sim.step(); // a morning
    let chief = status(&sim, "chief");
    let people = sim.people();
    let esteem = |s: StatusId| {
        people
            .iter()
            .filter(|p| p.status == s)
            .map(|p| p.needs.get(Need::Esteem))
            .max()
            .unwrap()
    };
    assert!(esteem(chief) < esteem(StatusId(0)), "chiefs feel no better");
    assert!(esteem(StatusId(0)) < 1000);
}

#[test]
fn learning_is_deterministic_on_any_number_of_threads() {
    let fast = |threads| {
        let mut sim = common::sim(Fixture::Small);
        sim.set_threads(threads);
        tune(&mut sim, |c| {
            c.learn_days = 1;
            c.review_days = 2;
            c.knowledge_days = 3;
            c.adult_learning = 300;
            c.child_learning = 500;
        });
        sim
    };
    let (mut a, mut b, mut c) = (fast(1), fast(4), fast(1));
    for _ in 0..4 {
        a.run(10);
        b.run(10);
        c.run(10);
        let d = a.state_digest();
        assert_eq!(d, b.state_digest(), "turn {}", a.turn());
        assert_eq!(d, c.state_digest(), "turn {}", a.turn());
    }
}
