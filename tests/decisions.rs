//! Needs and decisions: what people choose to do given how they feel, who
//! they are and what their city offers, and how their needs change with
//! what they did.

mod common;

use common::Fixture;
use societies::Simulation;
use societies::city::CityId;
use societies::economy::Outlook;
use societies::people::founding::FoundingConfig;
use societies::people::{ActionKind, Activity, Need, Needs, PersonSnapshot};
use societies::time::TimeOfDay;

const FOOD_WORK: [ActionKind; 4] = [
    ActionKind::Forage,
    ActionKind::Farm,
    ActionKind::Herd,
    ActionKind::Fish,
];

/// Sets everyone's needs with `f`.
fn set_needs(sim: &mut Simulation, f: impl Fn(&mut Needs)) {
    let world = sim.world_mut();
    let mut query = world.query::<&mut Needs>();
    for mut needs in query.iter_mut(world) {
        f(&mut needs);
    }
}

/// Gives every city the same outlook (nothing else writes it yet).
fn set_outlook(sim: &mut Simulation, free_land: u32, food_days: u32) {
    let world = sim.world_mut();
    let mut query = world.query::<&mut Outlook>();
    for mut outlook in query.iter_mut(world) {
        outlook.free_land = free_land;
        outlook.food_days = food_days;
    }
}

/// Needs all at `level`, except those given.
fn needs_at(level: u16, except: &[(Need, u16)]) -> impl Fn(&mut Needs) {
    let except = except.to_vec();
    move |n: &mut Needs| {
        for need in Need::ALL {
            n.set(need, level);
        }
        for &(need, v) in &except {
            n.set(need, v);
        }
    }
}

fn adult_age() -> u32 {
    FoundingConfig::builtin().adult_age
}

fn adults(sim: &mut Simulation) -> Vec<PersonSnapshot> {
    let (turn, tpy) = (sim.turn(), sim.config().turns_per_year());
    sim.people()
        .into_iter()
        .filter(|p| p.person.age_years(turn, tpy) >= adult_age())
        .collect()
}

/// Per mille of `people` doing one of `actions`.
fn share(people: &[PersonSnapshot], actions: &[ActionKind]) -> usize {
    let n = people
        .iter()
        .filter(|p| actions.contains(&p.activity.action))
        .count();
    n * 1000 / people.len().max(1)
}

#[test]
fn hungry_people_choose_food_work() {
    let mut hungry = common::sim(Fixture::Small);
    set_needs(&mut hungry, needs_at(0, &[(Need::Food, 1000)]));
    hungry.step();
    let mut fed = common::sim(Fixture::Small);
    set_needs(
        &mut fed,
        needs_at(0, &[(Need::Company, 400), (Need::Fun, 400)]),
    );
    fed.step();
    let (h, f) = (
        share(&adults(&mut hungry), &FOOD_WORK),
        share(&adults(&mut fed), &FOOD_WORK),
    );
    assert!(h > 900, "{h}‰ of hungry adults look for food");
    assert!(f < 400 && f + 500 < h, "{f}‰ of fed adults look for food");
}

#[test]
fn tired_people_sleep_or_rest() {
    let mut sim = common::sim(Fixture::Small);
    set_needs(&mut sim, needs_at(100, &[(Need::Rest, 1000)]));
    sim.step();
    let s = share(&adults(&mut sim), &[ActionKind::Sleep, ActionKind::Rest]);
    assert!(s > 900, "{s}‰ of tired adults sleep or rest");
}

#[test]
fn extraverts_socialize_more_than_introverts() {
    let mut sim = common::sim(Fixture::Small);
    set_needs(
        &mut sim,
        needs_at(
            100,
            &[(Need::Food, 300), (Need::Company, 300), (Need::Fun, 300)],
        ),
    );
    sim.step();
    let people = adults(&mut sim);
    let (outgoing, shy): (Vec<_>, Vec<_>) = people
        .into_iter()
        .filter(|p| p.traits.extraversion >= 160 || p.traits.extraversion <= 96)
        .partition(|p| p.traits.extraversion >= 160);
    assert!(outgoing.len() > 30 && shy.len() > 30);
    let (o, s) = (
        share(&outgoing, &[ActionKind::Socialize]),
        share(&shy, &[ActionKind::Socialize]),
    );
    assert!(o > s * 2 && o > s + 200, "extraverts {o}‰, introverts {s}‰");
}

#[test]
fn farming_needs_free_land() {
    for (land, expect_farmers) in [(0, false), (1000, true)] {
        let mut sim = common::sim(Fixture::Small);
        set_outlook(&mut sim, land, 30);
        set_needs(&mut sim, needs_at(0, &[(Need::Food, 1000)]));
        sim.step();
        let farmers = share(&adults(&mut sim), &[ActionKind::Farm, ActionKind::Herd]);
        assert_eq!(farmers > 0, expect_farmers, "free land {land}: {farmers}‰");
    }
}

/// Runs `days` days and returns, for every morning decision, the adults
/// who chose to settle away (by `PersonId`, with their boldness).
fn settlers_over(sim: &mut Simulation, days: u64) -> Vec<Vec<(u64, u8)>> {
    let mut out = Vec::new();
    for _ in 0..days {
        // From night to the next morning's decisions.
        while sim.date().time_of_day != TimeOfDay::Morning {
            sim.step();
        }
        let chose: Vec<_> = adults(sim)
            .iter()
            .filter(|p| p.activity.action == ActionKind::SettleAway)
            .map(|p| (p.id.0, p.traits.boldness))
            .collect();
        out.push(chose);
        sim.step();
    }
    out
}

#[test]
fn hunger_without_land_sends_bold_adults_away() {
    // Nothing feeds anyone and the outlook stays at its default: no free
    // land, no food in store.
    let mut sim = common::sim(Fixture::Small);
    let days = settlers_over(&mut sim, 12);
    // Not on the first days: people leave only after going hungry a while.
    for (day, settlers) in days.iter().enumerate().take(5) {
        assert!(settlers.is_empty(), "day {day}: {} left", settlers.len());
    }
    let people = adults(&mut sim);
    let last: &Vec<(u64, u8)> = days.last().unwrap();
    assert!(!last.is_empty(), "nobody thinks of leaving");
    assert!(
        last.len() * 3 < people.len(),
        "{} of {} adults leave at once",
        last.len(),
        people.len()
    );
    let mean = |b: &mut dyn Iterator<Item = u8>| {
        let v: Vec<u64> = b.map(u64::from).collect();
        v.iter().sum::<u64>() / v.len().max(1) as u64
    };
    let bold = mean(&mut last.iter().map(|&(_, b)| b));
    let all = mean(&mut people.iter().map(|p| p.traits.boldness));
    assert!(
        bold > all + 25,
        "settlers' boldness {bold}, everyone's {all}"
    );
}

#[test]
fn nobody_leaves_while_there_is_land() {
    let mut sim = common::sim(Fixture::Small);
    set_outlook(&mut sim, 1000, 0);
    let days = settlers_over(&mut sim, 12);
    assert!(days.iter().all(|d| d.is_empty()));
    // And nobody leaves a city with food in store either.
    let mut sim = common::sim(Fixture::Small);
    set_outlook(&mut sim, 0, 60);
    let days = settlers_over(&mut sim, 12);
    assert!(days.iter().all(|d| d.is_empty()));
}

#[test]
fn children_follow_their_mothers() {
    let mut sim = common::sim(Fixture::Small);
    let (start_turn, tpy) = (sim.turn(), sim.config().turns_per_year());
    let child = |p: &PersonSnapshot| p.person.age_years(start_turn, tpy) < adult_age() - 1;
    let counters: Vec<(u64, u64)> = sim
        .people()
        .iter()
        .filter(|p| child(p))
        .map(|p| (p.id.0, p.rng.counter))
        .collect();
    assert!(counters.len() > 50);
    let allowed = [
        ActionKind::Idle,
        ActionKind::Sleep,
        ActionKind::Rest,
        ActionKind::Socialize,
        ActionKind::Worship,
    ];
    let mut following = 0;
    for _ in 0..12 {
        sim.step();
        let people = sim.people();
        let by_id: std::collections::HashMap<_, _> = people.iter().map(|p| (p.id, p)).collect();
        for p in people.iter().filter(|p| child(p)) {
            assert!(allowed.contains(&p.activity.action), "{:?}", p.activity);
            if let Some(m) = p.family.mother.and_then(|m| by_id.get(&m))
                && ![ActionKind::Idle, ActionKind::Sleep].contains(&p.activity.action)
            {
                assert_eq!(p.activity.action, m.activity.action);
                following += 1;
            }
        }
        // Children draw no random numbers: they do not decide.
        for &(id, counter) in &counters {
            let p = people.iter().find(|p| p.id.0 == id).unwrap();
            assert_eq!(p.rng.counter, counter, "child {id} decided");
        }
    }
    assert!(following > 0, "no child ever joined their mother");
}

#[test]
fn night_is_sleep_and_travellers_are_left_alone() {
    let mut sim = common::sim(Fixture::Small);
    let travelling: Vec<u64> = adults(&mut sim).iter().take(20).map(|p| p.id.0).collect();
    {
        let world = sim.world_mut();
        let mut query = world.query::<(&societies::people::PersonId, &mut Activity)>();
        for (id, mut activity) in query.iter_mut(world) {
            if travelling.contains(&id.0) {
                *activity = Activity {
                    action: ActionKind::SettleAway,
                    since: 0,
                    until: u64::MAX,
                };
            }
        }
    }
    let before: Vec<_> = sim
        .people()
        .into_iter()
        .filter(|p| travelling.contains(&p.id.0))
        .collect();
    for _ in 0..8 {
        sim.step();
        let people = sim.people();
        for p in &people {
            if travelling.contains(&p.id.0) {
                let b = before.iter().find(|b| b.id == p.id).unwrap();
                assert_eq!(p.activity, b.activity);
                assert_eq!(p.rng, b.rng);
            } else if sim.date().time_of_day == TimeOfDay::Night {
                assert_eq!(p.activity.action, ActionKind::Sleep);
            }
        }
    }
}

#[test]
fn an_urgent_need_interrupts_the_day() {
    let mut sim = common::sim(Fixture::Small);
    set_needs(&mut sim, needs_at(0, &[(Need::Company, 300)]));
    sim.step(); // turn 1: everyone decides for the rest of the day
    let before = share(&adults(&mut sim), &[ActionKind::Sleep, ActionKind::Rest]);
    set_needs(&mut sim, needs_at(0, &[(Need::Rest, 1000)]));
    sim.step(); // turn 2: still day, but now they are exhausted
    let after = share(&adults(&mut sim), &[ActionKind::Sleep, ActionKind::Rest]);
    assert!(
        before < 200 && after > 900,
        "before {before}‰, after {after}‰"
    );
}

#[test]
fn company_and_rest_follow_what_people_did() {
    let mut sim = common::sim(Fixture::Small);
    set_needs(&mut sim, needs_at(500, &[]));
    sim.run(2); // to the evening: the day's actions are chosen
    let day: Vec<(u64, ActionKind)> = sim
        .people()
        .iter()
        .map(|p| (p.id.0, p.activity.action))
        .collect();
    sim.step(); // nightfall: needs are brought up to date
    let people = sim.people();
    let mean = |action: ActionKind, need: Need, did: bool| {
        let v: Vec<u64> = people
            .iter()
            .filter(|p| {
                day.iter()
                    .any(|&(id, a)| id == p.id.0 && (a == action) == did)
            })
            .map(|p| u64::from(p.needs.get(need)))
            .collect();
        assert!(!v.is_empty(), "nobody chose {action:?} == {did}");
        v.iter().sum::<u64>() / v.len() as u64
    };
    let social = mean(ActionKind::Socialize, Need::Company, true);
    let others = mean(ActionKind::Socialize, Need::Company, false);
    assert!(social + 300 < others, "company {social} vs {others}");
    // A night's sleep nearly makes up for a waking day.
    let rested = mean(ActionKind::Sleep, Need::Rest, false);
    assert!((500..=600).contains(&rested), "rest after a night {rested}");
}

#[test]
fn cold_nights_make_people_want_warmth() {
    let mut sim = common::sim(Fixture::Small);
    set_needs(&mut sim, needs_at(0, &[]));
    sim.run(3); // to the first night
    assert_eq!(sim.date().time_of_day, TimeOfDay::Night);
    let cities = sim.cities();
    let people = sim.people();
    let temperature = |id: CityId| {
        let c = cities.iter().find(|c| c.id == id).unwrap();
        let i = sim.map().index(c.position);
        sim.climate().temperature(i)
    };
    let mut by_city: Vec<(i16, u64)> = cities
        .iter()
        .filter_map(|c| {
            let warmth: Vec<u64> = people
                .iter()
                .filter(|p| p.person.home == c.id)
                .map(|p| u64::from(p.needs.get(Need::Warmth)))
                .collect();
            (!warmth.is_empty()).then(|| {
                (
                    temperature(c.id),
                    warmth.iter().sum::<u64>() / warmth.len() as u64,
                )
            })
        })
        .collect();
    by_city.sort();
    let (coldest, warmest) = (by_city[0], by_city[by_city.len() - 1]);
    assert!(
        warmest.0 - coldest.0 >= 50,
        "fixture cities too alike: {by_city:?}"
    );
    assert!(coldest.1 > warmest.1, "{by_city:?}");
}

#[test]
fn decisions_are_the_same_on_any_number_of_threads() {
    let mut one = common::sim(Fixture::Small);
    let mut many = common::sim(Fixture::Small);
    one.set_threads(1);
    many.set_threads(4);
    for sim in [&mut one, &mut many] {
        set_needs(sim, needs_at(400, &[(Need::Food, 900)]));
    }
    for _ in 0..12 {
        one.step();
        many.step();
        assert_eq!(
            one.state_digest(),
            many.state_digest(),
            "turn {}",
            one.turn()
        );
    }
}
