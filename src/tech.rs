//! Technologies: the configurable tree in `data/technologies.json` and what
//! each city knows.
//!
//! The JSON file is data only. This module loads it, checks it for mistakes
//! (unknown ids, cycles, typos in effect kinds or biome names, starting
//! technologies whose prerequisites do not start), and gives each city its
//! starting knowledge when it is founded. What the effects do to the world
//! is decided by later systems.
//!
//! Knowledge belongs to cities. A technology may have regional variants
//! (wheat and barley, millet, rice, ...): a city knows the technology if it
//! knows any of its variants, and a variant can only be known where its
//! environment suits it.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::Path;

use bevy_ecs::prelude::{Component, Resource};

use crate::biome::{Biome, Biomes};
use crate::map::{Landform, Map, Soil, Water};
use crate::rng::SimRng;

/// The file format tag and version this build understands.
pub const FORMAT: &str = "societies-technologies";
pub const VERSION: u32 = 1;

/// The technology file that ships with the simulation.
pub const BUILTIN_JSON: &str = include_str!("../data/technologies.json");

// ---------------------------------------------------------------------------
// File schema
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TechFile {
    pub format: String,
    pub version: u32,
    /// Calendar year the simulation starts at (negative = BCE).
    pub start_year: i32,
    #[serde(default)]
    pub notes: Vec<String>,
    /// Every effect kind a technology may use, with what it means.
    pub effect_kinds: BTreeMap<String, String>,
    /// Every resource a technology may need, with what it is.
    pub resources: BTreeMap<String, String>,
    pub technologies: Vec<Technology>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Category {
    Baseline,
    Agriculture,
    Water,
    Crafts,
    Metallurgy,
    Transport,
    Trade,
    Knowledge,
    Society,
    Warfare,
}

/// Access to water from a city's tile: on or next to a river, next to a lake,
/// or next to the sea.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum WaterAccess {
    River,
    Lake,
    Coast,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Technology {
    pub id: String,
    pub name: String,
    pub category: Category,
    /// Earliest known real-world date, years BCE. For calibration only.
    pub earliest_bce: i32,
    /// All of these must be known.
    pub requires: Vec<String>,
    /// At least one of these must be known (ignored when empty).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires_any: Vec<String>,
    #[serde(default, skip_serializing_if = "Needs::is_empty")]
    pub needs: Needs,
    /// Relative chance of independent invention (0 = never, baseline 100).
    pub invention: u32,
    /// How readily it spreads between cities in contact, 0–1000.
    pub diffusion: u32,
    /// Who knows it at the start year. Technologies with variants put this
    /// on their variants instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<Start>,
    pub effects: Vec<Effect>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variants: Vec<Variant>,
    pub description: String,
}

/// What a city's surroundings must offer. Every non-empty list must be
/// satisfied by at least one entry.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Needs {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub biomes_any: Vec<Biome>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub water_any: Vec<WaterAccess>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub landforms_any: Vec<Landform>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resources_any: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resources_all: Vec<String>,
}

impl Needs {
    pub fn is_empty(&self) -> bool {
        self.biomes_any.is_empty()
            && self.water_any.is_empty()
            && self.landforms_any.is_empty()
            && self.resources_any.is_empty()
            && self.resources_all.is_empty()
    }

    /// Whether `site` meets these needs. Resources are not on the map yet,
    /// so resource needs are taken as met.
    pub fn met_by(&self, site: &Site) -> bool {
        (self.biomes_any.is_empty() || self.biomes_any.contains(&site.biome))
            && (self.landforms_any.is_empty() || self.landforms_any.contains(&site.landform))
            && (self.water_any.is_empty() || self.water_any.iter().any(|&w| site.has(w)))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Start {
    /// Per mille of the matching cities that know it at the start year.
    pub share: u32,
    #[serde(default, rename = "where", skip_serializing_if = "Where::is_empty")]
    pub at: Where,
}

/// A filter on places. Every non-empty list must contain the place's value
/// (for `water`: the place must have one of the listed kinds of access).
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Where {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub biomes: Vec<Biome>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub landforms: Vec<Landform>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub soils: Vec<Soil>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub water: Vec<WaterAccess>,
}

impl Where {
    pub fn is_empty(&self) -> bool {
        self.biomes.is_empty()
            && self.landforms.is_empty()
            && self.soils.is_empty()
            && self.water.is_empty()
    }

    pub fn matches(&self, site: &Site) -> bool {
        (self.biomes.is_empty() || self.biomes.contains(&site.biome))
            && (self.landforms.is_empty() || self.landforms.contains(&site.landform))
            && (self.soils.is_empty() || self.soils.contains(&site.soil))
            && (self.water.is_empty() || self.water.iter().any(|&w| site.has(w)))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Effect {
    /// One of the file's `effect_kinds`.
    pub kind: String,
    /// Per mille for multipliers (1000 = unchanged); see the kind's description.
    pub value: i64,
    /// For `unlock`: what becomes possible.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feature: Option<String>,
    #[serde(default, rename = "where", skip_serializing_if = "Where::is_empty")]
    pub at: Where,
}

/// A regional form of a technology. Its effects add to the technology's own.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Variant {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Needs::is_empty")]
    pub needs: Needs,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<Start>,
    pub effects: Vec<Effect>,
    pub description: String,
}

// ---------------------------------------------------------------------------
// The checked tree
// ---------------------------------------------------------------------------

/// Index of a technology in its tree (file order).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TechId(pub u16);

/// Index of a variant among all variants of the tree (file order).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct VariantId(pub u16);

#[derive(Debug)]
pub enum TechError {
    Io(std::io::Error),
    Parse(serde_json::Error),
    /// Every problem found, one line each.
    Invalid(Vec<String>),
}

impl fmt::Display for TechError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TechError::Io(e) => write!(f, "cannot read technology file: {e}"),
            TechError::Parse(e) => write!(f, "malformed technology file: {e}"),
            TechError::Invalid(problems) => {
                write!(f, "invalid technology file:")?;
                for p in problems {
                    write!(f, "\n  {p}")?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for TechError {}

/// A validated technology tree with ids resolved to indices.
#[derive(Resource, Debug, Clone)]
pub struct TechTree {
    file: TechFile,
    ids: BTreeMap<String, TechId>,
    variant_ids: BTreeMap<String, VariantId>,
    /// Owning technology of each variant.
    variant_tech: Vec<TechId>,
    /// First variant of each technology.
    first_variant: Vec<u16>,
    requires: Vec<Vec<TechId>>,
    requires_any: Vec<Vec<TechId>>,
    /// Prerequisites before dependents; ties in file order.
    order: Vec<TechId>,
}

impl TechTree {
    /// The tree in `data/technologies.json`, compiled into the binary.
    pub fn builtin() -> Self {
        Self::from_json(BUILTIN_JSON).expect("the built-in technology file is valid")
    }

    pub fn load(path: impl AsRef<Path>) -> Result<Self, TechError> {
        let text = std::fs::read_to_string(path).map_err(TechError::Io)?;
        Self::from_json(&text)
    }

    pub fn from_json(text: &str) -> Result<Self, TechError> {
        let file: TechFile = serde_json::from_str(text).map_err(TechError::Parse)?;
        Self::from_file(file)
    }

    /// Checks `file` and resolves its ids.
    pub fn from_file(file: TechFile) -> Result<Self, TechError> {
        let mut problems = Vec::new();
        let mut problem = |s: String| problems.push(s);

        if file.format != FORMAT {
            problem(format!("format is {:?}, expected {FORMAT:?}", file.format));
        }
        if file.version != VERSION {
            problem(format!("version is {}, expected {VERSION}", file.version));
        }
        if file.technologies.len() > usize::from(u16::MAX) {
            problem("too many technologies".into());
        }

        // Ids: technologies and variants share one namespace.
        let mut ids = BTreeMap::new();
        let mut variant_ids = BTreeMap::new();
        let mut variant_tech = Vec::new();
        let mut first_variant = Vec::new();
        let mut seen = BTreeSet::new();
        for (i, tech) in file.technologies.iter().enumerate() {
            let t = TechId(i as u16);
            first_variant.push(variant_tech.len() as u16);
            if !valid_id(&tech.id) {
                problem(format!("{:?}: ids must be lowercase snake_case", tech.id));
            }
            if !seen.insert(tech.id.clone()) {
                problem(format!("{}: duplicate id", tech.id));
            }
            ids.insert(tech.id.clone(), t);
            for v in &tech.variants {
                if !valid_id(&v.id) {
                    problem(format!("{:?}: ids must be lowercase snake_case", v.id));
                }
                if !seen.insert(v.id.clone()) {
                    problem(format!("{}: duplicate id", v.id));
                }
                variant_ids.insert(v.id.clone(), VariantId(variant_tech.len() as u16));
                variant_tech.push(t);
            }
        }

        // Prerequisites.
        let resolve = |tech: &Technology,
                       list: &[String],
                       field: &str,
                       problem: &mut dyn FnMut(String)| {
            let mut out = Vec::new();
            for r in list {
                match ids.get(r) {
                    Some(&id) if r == &tech.id => {
                        let _ = id;
                        problem(format!("{}: {field} names itself", tech.id));
                    }
                    Some(&id) if out.contains(&id) => {
                        problem(format!("{}: {field} lists {r} twice", tech.id))
                    }
                    Some(&id) => out.push(id),
                    None if variant_ids.contains_key(r) => problem(format!(
                        "{}: {field} names the variant {r}; name its technology instead",
                        tech.id
                    )),
                    None => problem(format!("{}: {field} names unknown technology {r}", tech.id)),
                }
            }
            out
        };
        let mut requires = Vec::new();
        let mut requires_any = Vec::new();
        for tech in &file.technologies {
            requires.push(resolve(tech, &tech.requires, "requires", &mut problem));
            requires_any.push(resolve(
                tech,
                &tech.requires_any,
                "requires_any",
                &mut problem,
            ));
        }

        // Kahn's algorithm, always taking the lowest ready index.
        let n = file.technologies.len();
        let mut pending: Vec<usize> = (0..n)
            .map(|i| requires[i].len() + requires_any[i].len())
            .collect();
        let mut dependents = vec![Vec::new(); n];
        for i in 0..n {
            for p in requires[i].iter().chain(&requires_any[i]) {
                dependents[usize::from(p.0)].push(i);
            }
        }
        let mut ready: BTreeSet<usize> = (0..n).filter(|&i| pending[i] == 0).collect();
        let mut order = Vec::with_capacity(n);
        while let Some(i) = ready.pop_first() {
            order.push(TechId(i as u16));
            for &d in &dependents[i] {
                pending[d] -= 1;
                if pending[d] == 0 {
                    ready.insert(d);
                }
            }
        }
        if order.len() < n {
            let stuck: Vec<&str> = (0..n)
                .filter(|&i| pending[i] > 0)
                .map(|i| file.technologies[i].id.as_str())
                .collect();
            problem(format!("prerequisite cycle among: {}", stuck.join(", ")));
        }

        // Contents.
        let start_bce = -i64::from(file.start_year);
        for tech in &file.technologies {
            let id = &tech.id;
            if tech.name.trim().is_empty() {
                problem(format!("{id}: empty name"));
            }
            if tech.diffusion > 1000 {
                problem(format!("{id}: diffusion {} is above 1000", tech.diffusion));
            }
            check_needs(id, &tech.needs, &file, &mut problem);
            check_effects(id, &tech.effects, &file, &mut problem);
            if let Some(start) = &tech.start {
                check_start(id, start, &mut problem);
                if !tech.variants.is_empty() {
                    problem(format!("{id}: has variants, so its start belongs on them"));
                }
            }
            for v in &tech.variants {
                if v.name.trim().is_empty() {
                    problem(format!("{}: empty name", v.id));
                }
                check_needs(&v.id, &v.needs, &file, &mut problem);
                check_effects(&v.id, &v.effects, &file, &mut problem);
                if let Some(start) = &v.start {
                    check_start(&v.id, start, &mut problem);
                }
            }
            if starts(tech) && i64::from(tech.earliest_bce) < start_bce {
                problem(format!(
                    "{id}: known at the start year but first appears in {} BCE",
                    tech.earliest_bce
                ));
            }
        }

        // A technology known at the start needs prerequisites known at the
        // start, or nobody could ever hold it.
        for (i, tech) in file.technologies.iter().enumerate() {
            if !starts(tech) {
                continue;
            }
            for p in &requires[i] {
                let req = &file.technologies[usize::from(p.0)];
                if !starts(req) {
                    problem(format!(
                        "{}: starts known but its prerequisite {} does not",
                        tech.id, req.id
                    ));
                }
            }
            if !requires_any[i].is_empty()
                && !requires_any[i]
                    .iter()
                    .any(|p| starts(&file.technologies[usize::from(p.0)]))
            {
                problem(format!(
                    "{}: starts known but none of its requires_any do",
                    tech.id
                ));
            }
        }

        if problems.is_empty() {
            Ok(Self {
                file,
                ids,
                variant_ids,
                variant_tech,
                first_variant,
                requires,
                requires_any,
                order,
            })
        } else {
            Err(TechError::Invalid(problems))
        }
    }

    pub fn file(&self) -> &TechFile {
        &self.file
    }

    pub fn start_year(&self) -> i32 {
        self.file.start_year
    }

    pub fn len(&self) -> usize {
        self.file.technologies.len()
    }

    pub fn is_empty(&self) -> bool {
        self.file.technologies.is_empty()
    }

    pub fn variant_count(&self) -> usize {
        self.variant_tech.len()
    }

    pub fn tech(&self, id: TechId) -> &Technology {
        &self.file.technologies[usize::from(id.0)]
    }

    pub fn id(&self, name: &str) -> Option<TechId> {
        self.ids.get(name).copied()
    }

    pub fn variant_id(&self, name: &str) -> Option<VariantId> {
        self.variant_ids.get(name).copied()
    }

    pub fn variant(&self, id: VariantId) -> &Variant {
        let tech = self.variant_tech[usize::from(id.0)];
        let local = id.0 - self.first_variant[usize::from(tech.0)];
        &self.tech(tech).variants[usize::from(local)]
    }

    /// The technology a variant belongs to.
    pub fn variant_of(&self, id: VariantId) -> TechId {
        self.variant_tech[usize::from(id.0)]
    }

    /// The variants of a technology.
    pub fn variants(&self, id: TechId) -> impl Iterator<Item = VariantId> + use<> {
        let first = self.first_variant[usize::from(id.0)];
        let count = self.tech(id).variants.len() as u16;
        (first..first + count).map(VariantId)
    }

    /// All technologies, prerequisites first.
    pub fn order(&self) -> &[TechId] {
        &self.order
    }

    pub fn requires(&self, id: TechId) -> &[TechId] {
        &self.requires[usize::from(id.0)]
    }

    pub fn requires_any(&self, id: TechId) -> &[TechId] {
        &self.requires_any[usize::from(id.0)]
    }

    /// Whether `knowledge` holds every prerequisite of `id`.
    pub fn prerequisites_met(&self, knowledge: &Knowledge, id: TechId) -> bool {
        let any = self.requires_any(id);
        self.requires(id).iter().all(|&p| knowledge.knows(p))
            && (any.is_empty() || any.iter().any(|&p| knowledge.knows(p)))
    }

    /// Whether a city at `site` that knows `knowledge` could adopt `id`
    /// (or, if it has variants, at least one of them).
    pub fn can_adopt(&self, knowledge: &Knowledge, id: TechId, site: &Site) -> bool {
        let tech = self.tech(id);
        self.prerequisites_met(knowledge, id)
            && tech.needs.met_by(site)
            && (tech.variants.is_empty() || tech.variants.iter().any(|v| v.needs.met_by(site)))
    }

    /// What a city founded at `site` knows at the start year. Technologies
    /// are visited prerequisites first; each one (or each variant) whose
    /// prerequisites, needs and `start.where` the city meets is known with
    /// probability `start.share` per mille.
    pub fn starting_knowledge(&self, site: &Site, rng: &mut SimRng) -> Knowledge {
        let mut knowledge = Knowledge::new(self);
        for &id in &self.order {
            let tech = self.tech(id);
            if !self.prerequisites_met(&knowledge, id) || !tech.needs.met_by(site) {
                continue;
            }
            if tech.variants.is_empty() {
                if let Some(start) = &tech.start
                    && start.at.matches(site)
                    && roll(rng, start.share)
                {
                    knowledge.learn(self, id, None);
                }
                continue;
            }
            for v in self.variants(id) {
                let variant = self.variant(v);
                if let Some(start) = &variant.start
                    && variant.needs.met_by(site)
                    && start.at.matches(site)
                    && roll(rng, start.share)
                {
                    knowledge.learn(self, id, Some(v));
                }
            }
        }
        knowledge
    }
}

fn roll(rng: &mut SimRng, share: u32) -> bool {
    match share {
        0 => false,
        1000.. => true,
        s => rng.chance_per_mille(s),
    }
}

fn starts(tech: &Technology) -> bool {
    tech.start.as_ref().is_some_and(|s| s.share > 0)
        || tech
            .variants
            .iter()
            .any(|v| v.start.as_ref().is_some_and(|s| s.share > 0))
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.starts_with(|c: char| c.is_ascii_lowercase())
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

fn check_needs(id: &str, needs: &Needs, file: &TechFile, problem: &mut dyn FnMut(String)) {
    for r in needs.resources_any.iter().chain(&needs.resources_all) {
        if !file.resources.contains_key(r) {
            problem(format!("{id}: needs unknown resource {r}"));
        }
    }
    if needs
        .biomes_any
        .iter()
        .any(|b| matches!(b, Biome::Ocean | Biome::Lake))
    {
        problem(format!(
            "{id}: needs a water biome, which no city stands on"
        ));
    }
}

fn check_effects(id: &str, effects: &[Effect], file: &TechFile, problem: &mut dyn FnMut(String)) {
    for e in effects {
        if !file.effect_kinds.contains_key(&e.kind) {
            problem(format!("{id}: unknown effect kind {}", e.kind));
        }
        if e.kind == "unlock" && e.feature.is_none() {
            problem(format!("{id}: unlock effect without a feature"));
        }
    }
}

fn check_start(id: &str, start: &Start, problem: &mut dyn FnMut(String)) {
    if start.share > 1000 {
        problem(format!("{id}: start share {} is above 1000", start.share));
    }
}

// ---------------------------------------------------------------------------
// Places and knowledge
// ---------------------------------------------------------------------------

/// The surroundings of a city, as technologies see them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Site {
    pub biome: Biome,
    pub landform: Landform,
    pub soil: Soil,
    /// On a river tile or next to one.
    pub river: bool,
    /// Next to a lake.
    pub lake: bool,
    /// Next to the sea.
    pub coast: bool,
}

impl Site {
    pub fn of(map: &Map, biomes: &Biomes, index: usize) -> Self {
        let tile = &map.tiles()[index];
        let mut site = Self {
            biome: biomes.biome(index),
            landform: tile.landform,
            soil: tile.soil,
            river: tile.water == Water::River,
            lake: tile.water == Water::Lake,
            coast: tile.water == Water::Ocean,
        };
        for n in map.neighbors(index) {
            match map.tiles()[n].water {
                Water::River => site.river = true,
                Water::Lake => site.lake = true,
                Water::Ocean => site.coast = true,
                Water::Dry => {}
            }
        }
        site
    }

    pub fn has(&self, access: WaterAccess) -> bool {
        match access {
            WaterAccess::River => self.river,
            WaterAccess::Lake => self.lake,
            WaterAccess::Coast => self.coast,
        }
    }
}

/// The technologies (and variants) one city knows.
#[derive(Component, Debug, Clone, PartialEq, Eq, Default)]
pub struct Knowledge {
    techs: BitSet,
    variants: BitSet,
}

impl Knowledge {
    pub fn new(tree: &TechTree) -> Self {
        Self {
            techs: BitSet::new(tree.len()),
            variants: BitSet::new(tree.variant_count()),
        }
    }

    pub fn knows(&self, id: TechId) -> bool {
        self.techs.contains(usize::from(id.0))
    }

    pub fn knows_variant(&self, id: VariantId) -> bool {
        self.variants.contains(usize::from(id.0))
    }

    /// Learns a technology, or one of its variants (which also counts as
    /// knowing the technology).
    pub fn learn(&mut self, tree: &TechTree, id: TechId, variant: Option<VariantId>) {
        self.techs.insert(usize::from(id.0));
        if let Some(v) = variant {
            debug_assert_eq!(tree.variant_of(v), id, "variant of another technology");
            self.variants.insert(usize::from(v.0));
        }
    }

    pub fn techs(&self) -> impl Iterator<Item = TechId> + '_ {
        self.techs.iter().map(|i| TechId(i as u16))
    }

    pub fn variants(&self) -> impl Iterator<Item = VariantId> + '_ {
        self.variants.iter().map(|i| VariantId(i as u16))
    }

    pub fn count(&self) -> usize {
        self.techs.len()
    }

    pub fn digest(&self, out: &mut dyn FnMut(u64)) {
        for &w in self.techs.words.iter().chain(&self.variants.words) {
            out(w);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct BitSet {
    words: Vec<u64>,
}

impl BitSet {
    fn new(bits: usize) -> Self {
        Self {
            words: vec![0; bits.div_ceil(64)],
        }
    }

    fn contains(&self, i: usize) -> bool {
        self.words
            .get(i / 64)
            .is_some_and(|w| w >> (i % 64) & 1 == 1)
    }

    fn insert(&mut self, i: usize) {
        if self.words.len() <= i / 64 {
            self.words.resize(i / 64 + 1, 0);
        }
        self.words[i / 64] |= 1 << (i % 64);
    }

    fn len(&self) -> usize {
        self.words.iter().map(|w| w.count_ones() as usize).sum()
    }

    fn iter(&self) -> impl Iterator<Item = usize> + '_ {
        (0..self.words.len() * 64).filter(|&i| self.contains(i))
    }
}
