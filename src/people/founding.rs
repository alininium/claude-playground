//! The first people of each settlement, created with the world.
//!
//! Ages follow a pre-modern age pyramid (`founding.age_table` in
//! `data/people.json`). Women and men of marriageable age are paired up, and
//! children are given to mothers of a plausible age, so every settlement
//! starts as a set of households.

use bevy_ecs::prelude::*;

use super::rng::PersonRng;
use super::{
    Activity, Family, Need, Needs, NextPersonId, Person, PersonBundle, PersonId, Sex, Skills,
    Status, Traits,
};
use crate::city::CityId;
use crate::rng::SimRng;
use crate::tech::{Category, Knowledge, TechTree};

/// The `founding` section of `data/people.json`.
#[derive(Resource, Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct FoundingConfig {
    /// `[lowest age of a five-year band, per mille of people in it]`.
    pub age_table: Vec<[u32; 2]>,
    pub adult_age: u32,
    pub skill_share: u32,
    pub marriage_age: MarriageAge,
    pub max_husband_age_gap: u32,
    pub mother_age_at_birth: [u32; 2],
    pub initial_needs_max: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct MarriageAge {
    pub female: u32,
    pub male: u32,
}

impl FoundingConfig {
    pub fn builtin() -> Self {
        super::section("founding")
    }
}

/// Everything a settlement's founding needs to know about the world.
pub struct Settlement<'a> {
    pub city: CityId,
    pub size: u32,
    /// What the settlement knows; its people know parts of it.
    pub knowledge: &'a Knowledge,
    pub turn: u64,
    pub turns_per_year: u64,
}

/// Spawns `settlement.size` people. Each person's seed is drawn from `rng`;
/// everything else about them comes from their own stream.
pub fn spawn_settlement(
    world: &mut World,
    settlement: &Settlement,
    tree: &TechTree,
    config: &FoundingConfig,
    rng: &mut SimRng,
) {
    let tpy = settlement.turns_per_year as i64;
    let total_weight: u64 = config.age_table.iter().map(|b| u64::from(b[1])).sum();

    struct Draft {
        id: PersonId,
        rng: PersonRng,
        sex: Sex,
        age: u32,
        born_on: i64,
        traits: Traits,
        needs: Needs,
        family: Family,
        skills: Knowledge,
    }

    let mut drafts: Vec<Draft> = (0..settlement.size)
        .map(|_| {
            let id = world.resource_mut::<NextPersonId>().take();
            let mut r = PersonRng::new(rng.next_u64());
            let sex = if r.below(2) == 0 {
                Sex::Female
            } else {
                Sex::Male
            };
            let mut pick = r.below(total_weight.max(1));
            let band = config
                .age_table
                .iter()
                .find(|b| {
                    if pick < u64::from(b[1]) {
                        true
                    } else {
                        pick -= u64::from(b[1]);
                        false
                    }
                })
                .unwrap_or(&[0, 0]);
            let age = band[0] + r.below(5) as u32;
            let born_on =
                settlement.turn as i64 - i64::from(age) * tpy - r.below(tpy as u64) as i64;
            let traits = Traits::random(&mut r);
            let mut needs = Needs::default();
            for need in Need::ALL {
                needs.set(
                    need,
                    r.below(u64::from(config.initial_needs_max) + 1) as u16,
                );
            }
            let skills = starting_skills(
                settlement.knowledge,
                tree,
                age >= config.adult_age,
                config,
                &mut r,
            );
            Draft {
                id,
                rng: r,
                sex,
                age,
                born_on,
                traits,
                needs,
                family: Family::default(),
                skills,
            }
        })
        .collect();

    // Couples: each woman of marriageable age, in id order, takes the
    // unmarried man closest to four years older, within the allowed gap.
    for w in 0..drafts.len() {
        if drafts[w].sex != Sex::Female || drafts[w].age < config.marriage_age.female {
            continue;
        }
        let wa = drafts[w].age;
        let husband = (0..drafts.len())
            .filter(|&m| {
                let d = &drafts[m];
                d.sex == Sex::Male
                    && d.family.partner.is_none()
                    && d.age >= config.marriage_age.male
                    && d.age >= wa
                    && d.age <= wa + config.max_husband_age_gap
            })
            .min_by_key(|&m| ((drafts[m].age as i64 - wa as i64 - 4).abs(), drafts[m].id));
        if let Some(m) = husband {
            let (wid, mid) = (drafts[w].id, drafts[m].id);
            drafts[w].family.partner = Some(mid);
            drafts[m].family.partner = Some(wid);
        }
    }

    // Children go to a mother of a plausible age, married ones first, then
    // the one with the fewest children so far.
    let [lo, hi] = config.mother_age_at_birth;
    for c in 0..drafts.len() {
        if drafts[c].age >= config.adult_age {
            continue;
        }
        let ca = drafts[c].age;
        let mother = (0..drafts.len())
            .filter(|&m| {
                let d = &drafts[m];
                d.sex == Sex::Female && d.age >= ca + lo && d.age <= ca + hi
            })
            .min_by_key(|&m| {
                let d = &drafts[m];
                (d.family.partner.is_none(), d.family.births, d.id)
            });
        if let Some(m) = mother {
            let (mid, father) = (drafts[m].id, drafts[m].family.partner);
            drafts[m].family.births += 1;
            drafts[c].family.mother = Some(mid);
            drafts[c].family.father = father;
            if let Some(f) = father
                && let Some(fi) = drafts.iter().position(|d| d.id == f)
            {
                drafts[fi].family.births += 1;
            }
        }
    }

    for d in drafts {
        world.spawn(PersonBundle {
            equipment: Default::default(),
            id: d.id,
            person: Person {
                sex: d.sex,
                born_on: d.born_on,
                home: settlement.city,
                culture: settlement.city,
            },
            traits: d.traits,
            needs: d.needs,
            family: d.family,
            skills: Skills(d.skills),
            status: Status::default(),
            activity: Activity {
                since: settlement.turn,
                until: settlement.turn,
                ..Activity::default()
            },
            rng: d.rng,
        });
    }
}

/// What one founder knows: the baseline always; adults also each of the
/// settlement's other technologies with chance `skill_share`, if they know
/// its prerequisites, together with the settlement's variants of it.
fn starting_skills(
    settlement: &Knowledge,
    tree: &TechTree,
    adult: bool,
    config: &FoundingConfig,
    rng: &mut PersonRng,
) -> Knowledge {
    let mut skills = Knowledge::new(tree);
    for &id in tree.order() {
        if !settlement.knows(id) || !tree.prerequisites_met(&skills, id) {
            continue;
        }
        let baseline = tree.tech(id).category == Category::Baseline;
        if baseline || (adult && rng.chance_per_mille(config.skill_share)) {
            let variants: Vec<_> = tree
                .variants(id)
                .filter(|&v| settlement.knows_variant(v))
                .collect();
            if variants.is_empty() {
                skills.learn(tree, id, None);
            }
            for v in variants {
                skills.learn(tree, id, Some(v));
            }
        }
    }
    skills
}
