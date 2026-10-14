//! The technology file loads, the validator catches broken files, and
//! cities start with knowledge that suits where they stand.

mod common;

use serde_json::{Value, json};
use societies::biome::Biome;
use societies::map::{Landform, Soil};
use societies::rng::SimRng;
use societies::tech::{Knowledge, Site, TechError, TechTree};

fn builtin_value() -> Value {
    serde_json::from_str(societies::tech::BUILTIN_JSON).unwrap()
}

fn tech_mut<'a>(file: &'a mut Value, id: &str) -> &'a mut Value {
    file["technologies"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|t| t["id"] == id)
        .unwrap_or_else(|| panic!("no technology {id}"))
}

/// Validates `file` and returns the problems it reports.
fn problems(file: &Value) -> Vec<String> {
    match TechTree::from_json(&file.to_string()) {
        Ok(_) => Vec::new(),
        Err(TechError::Invalid(p)) => p,
        Err(e) => vec![e.to_string()],
    }
}

fn assert_rejected(file: &Value, needle: &str) {
    let p = problems(file);
    assert!(
        p.iter().any(|s| s.contains(needle)),
        "expected a problem containing {needle:?}, got {p:?}"
    );
}

fn site(biome: Biome) -> Site {
    Site {
        biome,
        landform: Landform::Plains,
        soil: Soil::Loam,
        river: false,
        lake: false,
        coast: false,
    }
}

#[test]
fn builtin_file_is_valid() {
    let tree = TechTree::builtin();
    assert_eq!(tree.start_year(), -4500);
    assert!(tree.len() >= 80, "{} technologies", tree.len());
    assert!(tree.variant_count() >= 15);
    // The order puts every prerequisite first.
    let mut seen = vec![false; tree.len()];
    for &id in tree.order() {
        for p in tree.requires(id).iter().chain(tree.requires_any(id)) {
            assert!(
                seen[usize::from(p.0)],
                "{} before its prerequisite",
                tree.tech(id).id
            );
        }
        seen[usize::from(id.0)] = true;
    }
    assert!(seen.iter().all(|&s| s));
}

#[test]
fn builtin_file_round_trips() {
    let tree = TechTree::builtin();
    let text = serde_json::to_string_pretty(tree.file()).unwrap();
    let again = TechTree::from_json(&text).unwrap();
    assert_eq!(tree.file(), again.file());
}

#[test]
fn variants_resolve_to_their_technology() {
    let tree = TechTree::builtin();
    let cereal = tree.id("cereal_cultivation").unwrap();
    let rice = tree.variant_id("rice").unwrap();
    assert_eq!(tree.variant_of(rice), cereal);
    assert!(tree.variants(cereal).any(|v| v == rice));
    assert_eq!(tree.variant(rice).id, "rice");
}

#[test]
fn malformed_json_and_unknown_fields_are_rejected() {
    assert!(matches!(TechTree::from_json("{"), Err(TechError::Parse(_))));
    let mut file = builtin_value();
    tech_mut(&mut file, "pottery")["colour"] = json!("red");
    assert!(matches!(
        TechTree::from_json(&file.to_string()),
        Err(TechError::Parse(_))
    ));
    let mut file = builtin_value();
    tech_mut(&mut file, "pottery")["start"]["where"] = json!({"biomes": ["Jungle"]});
    assert!(matches!(
        TechTree::from_json(&file.to_string()),
        Err(TechError::Parse(_))
    ));
}

#[test]
fn duplicate_ids_are_rejected() {
    let mut file = builtin_value();
    tech_mut(&mut file, "pottery")["id"] = json!("weaving");
    assert_rejected(&file, "weaving: duplicate id");
    // Variants share the namespace.
    let mut file = builtin_value();
    tech_mut(&mut file, "cattle")["variants"][0]["id"] = json!("rice");
    assert_rejected(&file, "rice: duplicate id");
}

#[test]
fn unknown_prerequisites_are_rejected() {
    let mut file = builtin_value();
    tech_mut(&mut file, "writing")["requires"] = json!(["telepathy"]);
    assert_rejected(
        &file,
        "writing: requires names unknown technology telepathy",
    );
    let mut file = builtin_value();
    tech_mut(&mut file, "writing")["requires"] = json!(["rice"]);
    assert_rejected(&file, "names the variant rice");
}

#[test]
fn cycles_are_rejected() {
    let mut file = builtin_value();
    tech_mut(&mut file, "fire")["requires"] = json!(["writing"]);
    assert_rejected(&file, "prerequisite cycle");
}

#[test]
fn unknown_effect_kinds_and_resources_are_rejected() {
    let mut file = builtin_value();
    tech_mut(&mut file, "pottery")["effects"][0]["kind"] = json!("magic");
    assert_rejected(&file, "pottery: unknown effect kind magic");
    let mut file = builtin_value();
    tech_mut(&mut file, "pottery")["needs"] = json!({"resources_any": ["unobtainium"]});
    assert_rejected(&file, "pottery: needs unknown resource unobtainium");
}

#[test]
fn start_must_be_consistent() {
    // A starting technology whose prerequisite does not start.
    let mut file = builtin_value();
    tech_mut(&mut file, "chariot")["start"] = json!({"share": 100});
    tech_mut(&mut file, "chariot")["earliest_bce"] = json!(5000);
    assert_rejected(&file, "chariot: starts known but its prerequisite");
    // A starting technology invented after the start year.
    let mut file = builtin_value();
    tech_mut(&mut file, "pottery")["earliest_bce"] = json!(3000);
    assert_rejected(&file, "pottery: known at the start year but first appears");
    // Shares are per mille.
    let mut file = builtin_value();
    tech_mut(&mut file, "pottery")["start"]["share"] = json!(1500);
    assert_rejected(&file, "pottery: start share 1500 is above 1000");
}

#[test]
fn starting_knowledge_follows_the_environment() {
    let tree = TechTree::builtin();
    let fire = tree.id("fire").unwrap();
    let cereal = tree.id("cereal_cultivation").unwrap();
    let rice = tree.variant_id("rice").unwrap();
    let wheat = tree.variant_id("wheat_barley").unwrap();
    let yak = tree.variant_id("yak").unwrap();

    let mut grassland = Vec::new();
    let mut wetland = Vec::new();
    for seed in 0..200 {
        let mut rng = SimRng::from_seed(seed);
        grassland.push(tree.starting_knowledge(&site(Biome::Grassland), &mut rng));
        let mut wet = site(Biome::Wetland);
        wet.river = true;
        wetland.push(tree.starting_knowledge(&wet, &mut rng));
    }
    let all = |k: &[Knowledge], f: &dyn Fn(&Knowledge) -> bool| k.iter().all(f);
    let count = |k: &[Knowledge], f: &dyn Fn(&Knowledge) -> bool| k.iter().filter(|x| f(x)).count();

    // Baseline technologies are known everywhere.
    assert!(all(&grassland, &|k| k.knows(fire)));
    assert!(all(&wetland, &|k| k.knows(fire)));
    // Variants only where they suit; knowing one means knowing the technology.
    assert_eq!(count(&grassland, &|k| k.knows_variant(rice)), 0);
    assert_eq!(count(&wetland, &|k| k.knows_variant(wheat)), 0);
    assert_eq!(count(&grassland, &|k| k.knows_variant(yak)), 0);
    let wheat_share = count(&grassland, &|k| k.knows_variant(wheat));
    assert!(
        (140..=200).contains(&wheat_share),
        "wheat in {wheat_share}/200"
    );
    assert!(count(&wetland, &|k| k.knows_variant(rice)) > 40);
    for k in grassland.iter().chain(&wetland) {
        for v in k.variants() {
            assert!(k.knows(tree.variant_of(v)));
        }
        // Everything known has its prerequisites known.
        for t in k.techs() {
            assert!(
                tree.prerequisites_met(k, t),
                "{} without prerequisites",
                tree.tech(t).id
            );
        }
    }
    assert!(count(&grassland, &|k| k.knows(cereal)) > 140);
}

#[test]
fn starting_knowledge_is_deterministic() {
    let tree = TechTree::builtin();
    let s = site(Biome::MediterraneanShrubland);
    let a = tree.starting_knowledge(&s, &mut SimRng::from_seed(9));
    let b = tree.starting_knowledge(&s, &mut SimRng::from_seed(9));
    assert_eq!(a, b);
}

#[test]
fn cities_start_knowing_technologies() {
    let mut sim = common::sim(common::Fixture::Medium);
    let tree = TechTree::builtin();
    let fire = tree.id("fire").unwrap();
    let cities = sim.cities();
    assert!(!cities.is_empty());
    for c in &cities {
        assert!(c.knowledge.knows(fire), "{} does not know fire", c.name);
        assert!(
            c.knowledge.count() > 9,
            "{} knows only the baseline",
            c.name
        );
        let site = Site::of(sim.map(), sim.biomes(), sim.map().index(c.position));
        for v in c.knowledge.variants() {
            let variant = tree.variant(v);
            assert!(
                variant.needs.met_by(&site),
                "{} grows {} at {site:?}",
                c.name,
                variant.id
            );
        }
    }
    // Cities in different places know different things.
    assert!(cities.windows(2).any(|w| w[0].knowledge != w[1].knowledge));
}

#[test]
fn a_custom_tree_changes_knowledge_but_not_the_world() {
    let mut file = builtin_value();
    tech_mut(&mut file, "herbal_medicine")["start"]["share"] = json!(0);
    let tree = TechTree::from_json(&file.to_string()).unwrap();
    let herbs = tree.id("herbal_medicine").unwrap();

    let mut stock = common::sim(common::Fixture::Small);
    let mut custom = societies::Simulation::from_world_with_technologies(
        common::world(common::Fixture::Small),
        common::Fixture::Small.config(),
        tree,
    );
    assert_eq!(stock.world_digest(), custom.world_digest());
    let (a, b) = (stock.cities(), custom.cities());
    assert_eq!(a.len(), b.len());
    for (x, y) in a.iter().zip(&b) {
        assert_eq!((x.position, &x.name), (y.position, &y.name));
        assert!(!y.knowledge.knows(herbs));
    }
    assert!(a.iter().any(|c| c.knowledge.knows(herbs)));
    assert_ne!(stock.state_digest(), custom.state_digest());
}
