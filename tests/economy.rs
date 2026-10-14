//! Land and food: territories, farming through the seasons, overcrowding,
//! technologies, eating and hunger.

mod common;

use common::Fixture;
use societies::Simulation;
use societies::city::CityId;
use societies::economy::{
    self, EconomyConfig, Larder, Outlook, Stores, Territory, Work, claim_land,
};
use societies::map::Position;
use societies::people::{ActionKind, Activity, Need, Needs, Person};
use societies::time::{Season, Turn};

const SPRING: u64 = 0;
const SUMMER: u64 = 91;

/// Moves the calendar so that the next turn is the morning of `day`.
fn jump_to(sim: &mut Simulation, day: u64) {
    sim.world_mut().resource_mut::<Turn>().0 = day * 4 - 1;
}

/// Runs whole days (to the evening meal of the last one).
fn days(sim: &mut Simulation, n: u64) {
    sim.run(n * 4);
}

/// Sets every adult of `city` to `action` for the foreseeable future.
fn put_to_work(sim: &mut Simulation, city: CityId, action: ActionKind) {
    let (turn, tpy) = (sim.turn(), sim.config().turns_per_year());
    let world = sim.world_mut();
    // Keep them at it: no need ever becomes urgent enough to re-decide.
    world
        .resource_mut::<societies::people::decide::ActionCatalog>()
        .urgent = u16::MAX;
    let mut q = world.query::<(&Person, &mut Activity)>();
    for (person, mut activity) in q.iter_mut(world) {
        if person.home == city && person.age_years(turn, tpy) >= 12 {
            *activity = Activity {
                action,
                since: turn,
                until: u64::MAX - 1,
            };
        }
    }
}

fn larder(sim: &mut Simulation, city: CityId) -> Larder {
    let world = sim.world_mut();
    let mut q = world.query::<(&CityId, &Larder)>();
    q.iter(world)
        .find(|(id, _)| **id == city)
        .unwrap()
        .1
        .clone()
}

fn food(sim: &mut Simulation, city: CityId) -> u64 {
    let world = sim.world_mut();
    let mut q = world.query::<(&CityId, &Stores)>();
    q.iter(world).find(|(id, _)| **id == city).unwrap().1.food
}

fn outlook(sim: &mut Simulation, city: CityId) -> Outlook {
    let world = sim.world_mut();
    let mut q = world.query::<(&CityId, &Outlook)>();
    *q.iter(world).find(|(id, _)| **id == city).unwrap().1
}

fn territory(sim: &mut Simulation, city: CityId) -> Territory {
    let world = sim.world_mut();
    let mut q = world.query::<(&CityId, &Territory)>();
    q.iter(world)
        .find(|(id, _)| **id == city)
        .unwrap()
        .1
        .clone()
}

/// A city whose people farm and whose home tile has a growing season
/// that ends: the last season fields grow in (by the northern calendar)
/// and the one after it, when they do not.
fn farming_city(sim: &mut Simulation) -> (CityId, Season, Season) {
    sim.step();
    let tree = sim.technologies().clone();
    let cereals = tree.id("cereal_cultivation").unwrap();
    let config = EconomyConfig::builtin();
    let seasons = [
        Season::Spring,
        Season::Summer,
        Season::Autumn,
        Season::Winter,
    ];
    for c in sim.cities() {
        if !c.knowledge.knows(cereals) || c.population < 150 {
            continue;
        }
        let t = territory(sim, c.id);
        let home = &t.tiles[0];
        if sim.fertility().fertility(home.index as usize) < 300 {
            continue;
        }
        for k in 0..4 {
            let (grows, rests) = (seasons[k], seasons[(k + 1) % 4]);
            if economy::growing(&config, sim.climate(), home, grows)
                && !economy::growing(&config, sim.climate(), home, rests)
            {
                return (c.id, grows, rests);
            }
        }
    }
    panic!("no farming city with a season in which fields rest");
}

/// The first day of `season`.
fn first_day(season: Season) -> u64 {
    season as u64 * 91
}

/// A fresh simulation on the small fixture, starting on `day`.
fn sim_on(day: u64) -> Simulation {
    let mut sim = common::sim(Fixture::Small);
    if day > 0 {
        jump_to(&mut sim, day);
    }
    sim
}

#[test]
fn every_tile_belongs_to_its_nearest_city() {
    let sim = common::sim(Fixture::Medium);
    let mut sim = sim;
    let cities: Vec<(CityId, Position)> = sim.cities().iter().map(|c| (c.id, c.position)).collect();
    let map = sim.map().clone();
    let config = sim.config().clone();
    let claims = claim_land(&map, &config, 1500, &cities);
    let mut owner = vec![None; map.tile_count() as usize];
    for (k, tiles) in claims.iter().enumerate() {
        let (id, pos) = cities[k];
        assert_eq!(tiles[0].0 as usize, map.index(pos), "own tile first");
        for &(i, area) in tiles {
            assert!(area > 0);
            assert!(owner[i as usize].is_none(), "tile {i} claimed twice");
            owner[i as usize] = Some(id);
        }
    }
    let claimed = owner.iter().filter(|o| o.is_some()).count();
    assert!(claimed > cities.len() * 4, "{claimed} tiles");

    // With the default radius, a city on this map holds its own tile.
    let small = claim_land(&map, &config, 120, &cities);
    assert!(small.iter().all(|t| !t.is_empty()));
}

#[test]
fn farmers_fill_the_stores_at_harvest() {
    // Late in the last growing season of the year.
    let (city, grows, _) = farming_city(&mut sim_on(0));
    let mut idle = sim_on(first_day(grows) + 45);
    let mut farming = sim_on(first_day(grows) + 45);
    idle.step();
    farming.step();
    put_to_work(&mut farming, city, ActionKind::Farm);
    put_to_work(&mut idle, city, ActionKind::Idle);

    // The crop grows in the fields; nothing reaches the stores yet.
    days(&mut farming, 20);
    days(&mut idle, 20);
    let growing = larder(&mut farming, city);
    let standing: u64 = growing.standing.values().sum();
    assert!(standing > 0, "{growing:?}");
    assert_eq!(growing.harvested_milli, 0);
    assert!(food(&mut farming, city) <= food(&mut idle, city) + 1);

    // At the end of the season it is harvested.
    days(&mut farming, 30);
    days(&mut idle, 30);
    let harvested = larder(&mut farming, city);
    assert!(harvested.standing.is_empty(), "{harvested:?}");
    assert!(harvested.harvested_milli >= standing);
    let (with, without) = (food(&mut farming, city), food(&mut idle, city));
    assert!(
        with > without + standing / 1000,
        "{with} with farmers, {without} without"
    );
}

#[test]
fn more_farmers_than_land_leaves_some_idle() {
    let crowd = |catchment: u32| {
        let (city, grows, _) = farming_city(&mut sim_on(0));
        let mut sim = sim_on(first_day(grows));
        sim.step();
        {
            let mut config = sim.world_mut().resource_mut::<EconomyConfig>();
            config.farm.catchment_km2 = catchment;
            config.herd.catchment_km2 = catchment;
        }
        put_to_work(&mut sim, city, ActionKind::Farm);
        days(&mut sim, 2);
        (larder(&mut sim, city), outlook(&mut sim, city))
    };
    let (crowded, crowded_outlook) = crowd(2);
    let (roomy, roomy_outlook) = crowd(80);
    let farm = Work::Farm as usize;
    assert!(crowded.last_day.workers[farm] > 0);
    assert!(
        crowded.last_day.idle[farm] > crowded.last_day.workers[farm],
        "{:?}",
        crowded.last_day
    );
    assert_eq!(crowded_outlook.free_land, 0);
    assert!(roomy.last_day.idle[farm] < crowded.last_day.idle[farm]);
    assert!(roomy_outlook.free_land > 0);
    assert!(roomy.produced_milli > crowded.produced_milli * 2);
}

#[test]
fn the_cold_season_yields_less() {
    let (city, grows, rests) = farming_city(&mut sim_on(0));
    for action in [ActionKind::Farm, ActionKind::Forage] {
        let produced = |day: u64| {
            let mut sim = sim_on(day);
            sim.step();
            put_to_work(&mut sim, city, action);
            let before = larder(&mut sim, city).produced_milli;
            days(&mut sim, 5);
            larder(&mut sim, city).produced_milli - before
        };
        let (summer, winter) = (
            produced(first_day(grows) + 30),
            produced(first_day(rests) + 30),
        );
        assert!(summer > 0, "{action:?}");
        assert!(
            winter < summer,
            "{action:?}: {winter} in the cold season, {summer} in the growing one"
        );
        if action == ActionKind::Farm {
            assert_eq!(winter, 0, "nothing grows in the fields in the cold season");
        }
    }
}

#[test]
fn technologies_raise_yields() {
    let produced = |learn: bool| {
        let (city, grows, _) = farming_city(&mut sim_on(0));
        let mut sim = sim_on(first_day(grows) + 10);
        sim.step();
        let tree = sim.technologies().clone();
        let techs: Vec<_> = ["legumes", "manuring", "calendar"]
            .iter()
            .map(|t| tree.id(t).unwrap())
            .collect();
        if learn {
            let world = sim.world_mut();
            let mut q = world.query::<(&Person, &mut societies::people::Skills)>();
            for (person, mut skills) in q.iter_mut(world) {
                if person.home == city {
                    for &t in &techs {
                        skills.0.learn(&tree, t, None);
                    }
                }
            }
        }
        put_to_work(&mut sim, city, ActionKind::Farm);
        days(&mut sim, 3);
        larder(&mut sim, city).produced_milli
    };
    let (plain, learned) = (produced(false), produced(true));
    assert!(plain > 0);
    // Each of the three adds 5-8% for those who did not know it yet.
    assert!(learned * 100 > plain * 103, "{learned} vs {plain}");
    assert!(learned * 100 < plain * 125, "{learned} vs {plain}");
}

fn hunger(sim: &mut Simulation, city: CityId) -> Vec<u16> {
    let world = sim.world_mut();
    let mut q = world.query::<(&Person, &Needs)>();
    q.iter(world)
        .filter(|(p, _)| p.home == city)
        .map(|(_, n)| n.get(Need::Food))
        .collect()
}

fn make_hungry(sim: &mut Simulation, city: CityId) {
    let world = sim.world_mut();
    let mut q = world.query::<(&Person, &mut Needs)>();
    for (person, mut needs) in q.iter_mut(world) {
        if person.home == city {
            needs.set(Need::Food, 800);
        }
    }
}

#[test]
fn people_eat_when_there_is_food_and_go_hungry_when_there_is_none() {
    let mut fed = sim_on(0);
    let mut starving = sim_on(0);
    fed.step();
    starving.step();
    let city = CityId(0);
    // Nobody works: what they eat comes from the stores alone.
    put_to_work(&mut fed, city, ActionKind::Idle);
    put_to_work(&mut starving, city, ActionKind::Idle);
    make_hungry(&mut fed, city);
    make_hungry(&mut starving, city);
    {
        let world = starving.world_mut();
        let mut q = world.query::<(&CityId, &mut Stores)>();
        for (id, mut stores) in q.iter_mut(world) {
            if *id == city {
                stores.food = 0;
            }
        }
    }
    let before = larder(&mut fed, city);
    days(&mut fed, 1);
    days(&mut starving, 1);

    // Eating clears the hunger; a night then adds a day's worth (needs).
    let fed_hunger = hunger(&mut fed, city);
    assert!(fed_hunger.iter().all(|&h| h <= 300), "{fed_hunger:?}");
    let starving_hunger = hunger(&mut starving, city);
    assert!(
        starving_hunger.iter().all(|&h| h >= 800),
        "{:?}",
        starving_hunger
            .iter()
            .filter(|&&h| h < 800)
            .collect::<Vec<_>>()
    );
    assert_eq!(larder(&mut starving, city).ration, 0);
    assert_eq!(food(&mut starving, city), 0);

    // Everyone at home ate: adults a full ration, children less.
    let after = larder(&mut fed, city);
    let eaten = after.eaten_milli - before.eaten_milli;
    let residents = u64::from(after.residents);
    assert!(
        eaten < residents * 1000 && eaten > residents * 600,
        "{eaten}"
    );
    assert_eq!(after.ration, 1000);
    assert!(food(&mut fed, city) < before.food_milli / 1000);
}

#[test]
fn travellers_neither_work_nor_eat_at_home() {
    let (city, grows, _) = farming_city(&mut sim_on(0));
    let mut sim = sim_on(first_day(grows));
    sim.step();
    let (turn, tpy) = (sim.turn(), sim.config().turns_per_year());
    {
        let world = sim.world_mut();
        let mut q = world.query::<(&Person, &mut Activity)>();
        for (person, mut activity) in q.iter_mut(world) {
            if person.home == city && person.age_years(turn, tpy) >= 12 {
                *activity = Activity {
                    action: ActionKind::Farm,
                    since: turn,
                    until: u64::MAX,
                };
            }
        }
    }
    // Measure from now: on the first turn they may already have worked.
    let produced = larder(&mut sim, city).produced_milli;
    days(&mut sim, 2);
    let l = larder(&mut sim, city);
    assert_eq!(l.last_day.workers, [0; 4]);
    assert_eq!(l.produced_milli, produced);
    let children = hunger(&mut sim, city).len() as u32 - adults(&mut sim, city);
    assert_eq!(l.residents, children);
}

fn adults(sim: &mut Simulation, city: CityId) -> u32 {
    let (turn, tpy) = (sim.turn(), sim.config().turns_per_year());
    let world = sim.world_mut();
    let mut q = world.query::<&Person>();
    q.iter(world)
        .filter(|p| p.home == city && p.age_years(turn, tpy) >= 12)
        .count() as u32
}

#[test]
fn food_beyond_what_can_be_stored_rots() {
    let loss = |days_of_food: u64| {
        let mut sim = sim_on(SUMMER);
        sim.step();
        let city = CityId(0);
        let residents = sim.cities()[0].population.max(1);
        {
            let world = sim.world_mut();
            let mut q = world.query::<(&CityId, &mut Stores)>();
            for (id, mut stores) in q.iter_mut(world) {
                if *id == city {
                    stores.food = days_of_food * residents;
                }
            }
        }
        let before = larder(&mut sim, city).spoiled_milli;
        days(&mut sim, 10);
        let spoiled = larder(&mut sim, city).spoiled_milli - before;
        spoiled * 100_000 / (days_of_food * residents * 1000)
    };
    // Per 100 000 of the stores lost in ten days.
    let (little, glut) = (loss(20), loss(2000));
    assert!(glut > little * 5, "{glut} vs {little}");
}

#[test]
fn the_same_seed_feeds_the_same_people() {
    let run = |threads: usize| {
        let mut sim = sim_on(SPRING + 60);
        sim.set_threads(threads);
        sim.step();
        for c in 0..4 {
            let action = [
                ActionKind::Farm,
                ActionKind::Herd,
                ActionKind::Fish,
                ActionKind::Forage,
            ][c];
            put_to_work(&mut sim, CityId(c as u32), action);
        }
        days(&mut sim, 40);
        let ledgers: Vec<Larder> = (0..6).map(|c| larder(&mut sim, CityId(c))).collect();
        (ledgers, sim.state_digest())
    };
    let one = run(1);
    assert_eq!(one, run(1));
    assert_eq!(one, run(4));
    assert!(one.0.iter().any(|l| l.harvested_milli > 0));
}

#[test]
fn outlook_reports_land_and_food() {
    let mut sim = sim_on(SPRING);
    days(&mut sim, 1);
    for c in sim.cities() {
        let o = outlook(&mut sim, c.id);
        let l = larder(&mut sim, c.id);
        // Within a day: births and deaths come after the economy's step.
        let expected = food(&mut sim, c.id) / u64::from(l.residents.max(1));
        assert!(
            (o.food_days as u64).abs_diff(expected) <= 1,
            "{} vs {expected}",
            o.food_days
        );
        assert!(o.food_days > 100, "{o:?}");
        assert!(o.free_land > 0, "{o:?}");
    }
}

/// The module's own cost: one simulated day of land, food and meals for
/// about 10 000 people, every adult at food work. Timing only means
/// something in optimized builds: `cargo test --release --test economy`.
#[test]
#[cfg_attr(debug_assertions, ignore = "timing needs `cargo test --release`")]
fn a_day_of_food_for_ten_thousand_people_is_cheap() {
    use bevy_ecs::schedule::Schedule;
    let mut sim = common::sim_with(Fixture::Medium, |c| {
        c.initial_cities = 30;
        c.initial_barbarians = 20;
        c.min_city_distance_km = 300;
        c.city_population = [300, 500];
    });
    sim.step();
    let people = sim.people().len();
    assert!(people >= 9_000, "{people} people");
    for c in sim.cities() {
        let work = [ActionKind::Farm, ActionKind::Forage, ActionKind::Herd][c.id.0 as usize % 3];
        put_to_work(&mut sim, c.id, work);
    }
    let mut schedule = Schedule::default();
    schedule.add_systems(economy::systems());
    let world = sim.world_mut();
    let start = std::time::Instant::now();
    for _ in 0..4 {
        world.resource_mut::<Turn>().0 += 1;
        schedule.run(world);
    }
    let took = start.elapsed();
    println!("one day of food for {people} people: {took:?}");
    assert!(took < std::time::Duration::from_millis(10), "{took:?}");
}
