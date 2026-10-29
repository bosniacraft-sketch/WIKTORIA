//! Running script effects: the commands inside an event option or a decision.
//!
//! An effect block is a list of entries. An entry with a block value either changes who the
//! entries inside it apply to (a scope: `any_country`, `any_owned`, `any_pop`, a country tag, a
//! province number, `owner`, `controller`) or is an effect that takes several parameters
//! (`add_province_modifier`). An entry with a plain value is an effect on whatever is in scope.
//!
//! Measured in the real game (owner's event 10100001, 2026-10-04):
//! * `any_country` reaches every country that owns a province, except the one in scope.
//! * `any_owned` reaches every province the country in scope owns.
//! * `any_pop` in a country scope reaches every pop in the provinces it owns.
//! * `add_province_modifier = { name duration }` puts a timed modifier on the province; it
//!   expires `duration` days after the current date.
//! * `prestige = x` adds x and stops at FLD_PRESTIGE_MIN.
//! * `consciousness = x` / `militancy = x` on a pop add x within 0..FLD_POP_CON_MAX /
//!   0..FLD_POP_MIL_MAX.
//! * `treasury = x` adds x to the country's money, not below FLD_TREASURY_MIN.
//! * `capital_scope` reaches the capital province of the country in scope; `any_pop` in a
//!   province reaches every pop there (owner's event 10100600).
//! * `reduce_pop = x` multiplies the pop's size by x; a pop cannot exceed FLD_POP_SIZE_MAX.
//!
//! Taken from the meaning of the script words and the wiki's lists, NOT yet measured in the game
//! (lists what is open):
//! * flags, removing modifiers, `literacy`, `money` (pop savings; in a country scope the same as
//!   `treasury`), `plurality`, `badboy`, `war_exhaustion`, `leadership`, `life_rating`,
//!   `government`, the reform effects, `civilized`, `capital`, `trade_goods`, cores,
//!   `secede_province`, `inherit`, `annex_to`, `add_accepted_culture`, `primary_culture`.
//! * States: a state is the provinces of one region held by one owner. `any_state`,
//!   `random_state`, `state_scope`; inside a state `any_pop`, `any_owned` and the pop groups work
//!   on its provinces. A region name reaches every province of the region.
//! * Pop groups: a pop type name (`farmers = { }`) or `poor_strata` / `middle_strata` /
//!   `rich_strata` reaches the matching pops of the country, state or province in scope.
//! * `relation = { who value }` adds to the relation of both countries within
//!   +-FLD_RELATION_LIMIT.
//! * `ideology` / `dominant_issue = { value factor }` on a pop and `upper_house = { ideology
//!   value }` add the amount to that share and rescale all shares to the same total, so a share
//!   of zero raised by N ends at N / (1 + N) (the wiki's upper house formula).
//! * `scaled_militancy` / `scaled_consciousness = { ideology|issue factor }` add factor times the
//!   pop's share (0..1) of that ideology or issue.
//! * `random_*` scopes, `random` and `random_list` use the port's own seeded choice
//!   (FLD_SCRIPT_SEED); the original's dice are not reproduced (D10).
//! * `country_event` / `province_event = { id days }` are queued in `State::pending_events`.
//!
//! Conditions, from the wiki's list of conditions (not measured): an ideology or party issue
//! name with a number (`liberal = 10`) asks whether at least that many percent of the people in
//! scope support it; `is_primary_culture`, `is_accepted_culture` and `culture` on a province
//! look at its largest culture; a number condition on a country, state or province
//! (`literacy`, `militancy`, `consciousness`) is the average of its pops.
//!
//! Conditions taken from the description of the original's script engine (its own words for
//! what each one tests; the comment at each says what the port had to assume):
//! * `NOT` with several entries is "none of them"; a region name is "any province of it";
//!   `any_pop` asked of a country is always false (it works on provinces and states only);
//!   `all_core` is false for a country without cores and true for a province without cores.
//! * A number is "at least this much"; `rank = n` is "rank n or better"; `badboy = x` is a
//!   share of BADBOY_LIMIT.
//! * A condition asked where the thing it needs is missing is false (also its `= no` form where
//!   the description says so), not unknown.
//! * The port has no crises, truces, war-goal justifications or lost wars: `involved_in_crisis`,
//!   `truce_with`, `constructing_cb_progress` are false, `crisis_exist` and
//!   `has_recently_lost_war` answer as in a world without them.
//!
//! Effects taken from the same description (details at each): `change_tag` (the country's record
//! moves to the new tag, cores stay: checked on the original's saves), `change_tag_no_core_switch`
//! (moves only the player), `inherit` / `annex_to`, `release` / `release_vassal`,
//! `secede_province` (only from THIS's land or to THIS), cores by province number in a country
//! scope, `assimilate` (the whole province), `move_pop`, `add_tax_relative_income`, `election`,
//! `add_accepted_culture = union`, `add_country_modifier` outside a country scope (the owner's).
//! `random` takes a whole-number chance, `random_list` favours its first entry by one case, and
//! `any_owned` asks its `limit` a second time just before acting on each province.
//!
//! Anything this module does not know is never guessed: it is counted in `Report::unsupported`
//! and skipped, together with everything inside it. A scope whose `limit` holds a condition the
//! module does not know is skipped as a whole.

use std::collections::{BTreeMap, BTreeSet};

use wiktoria_data::date::Date;
use wiktoria_data::engine_rules::EngineRules;
use wiktoria_data::save::SaveGame;
use wiktoria_data::script::{Block, Value};
use wiktoria_data::world::World;

use crate::calendar::{Calendar, GameDate};

/// A modifier put on a country or province by an event or decision. `expires` None = permanent.
#[derive(Debug, Clone, PartialEq)]
pub struct TimedModifier {
    pub name: String,
    pub expires: Option<Date>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct PopState {
    pub pop_type: String,
    pub id: u64,
    pub size: f64,
    pub culture: String,
    pub religion: String,
    pub money: f64,
    pub literacy: f64,
    pub militancy: f64,
    pub consciousness: f64,
    /// Ideology shares in percent, by the ideology's number (position in `State::ideologies` + 1).
    pub ideology: BTreeMap<u32, f64>,
    /// Issue shares in percent, by the option's number (position in `State::issues` + 1).
    pub issues: BTreeMap<u32, f64>,
    /// How far each tier of needs is met (0..1). While the economy is switched off these are
    /// taken from the original's saves.
    pub life_needs: f64,
    pub everyday_needs: f64,
    pub luxury_needs: f64,
    /// Artisans: the production type they work at (`artisan_fabric`, ...).
    pub production: Option<String>,
    /// Artisans: how much of their capacity they aim to use (0.15..1), and the share of a full
    /// day's inputs they hold for the next day's work.
    pub throttle: f64,
    pub stock: f64,
    /// Artisans: the share of a full day of each input good they hold, in the order of the
    /// production type's inputs; `stock` is the smallest of them. Empty until the first market
    /// day, which starts it at `stock` for every good.
    pub held: Vec<f64>,
    /// People waiting to promote / to demote: added to on the pop's day every month, moved once
    /// there are more than one (`pops::move_due_pops`).
    pub promotion_due: f64,
    pub demotion_due: f64,
}

/// One factory: a state building of a country.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FactoryState {
    pub owner: String,
    /// Index into `State::regions` of the state it stands in.
    pub region: Option<u16>,
    /// Its production type (`paper_mill`, ...).
    pub kind: String,
    pub level: f64,
    /// Its cash as the save stores it: money units times FLD_POP_MONEY_EFFECT_SCALE, like a pop's.
    pub money: f64,
    /// Yesterday's sales, input costs and pay to workers and owners (for reports).
    pub income: f64,
    pub spending: f64,
    pub pay: f64,
    /// People it employs at most today; `None` = as many as the state offers. A factory that
    /// loses money lets people go day by day and takes everyone back once it earns again.
    pub staff: Option<f64>,
    /// Days until the building work is done: a new factory (level 0) opens at level 1, a
    /// standing one gains a level. `state_built`: the government pays for that work.
    pub building_days: Option<f64>,
    pub state_built: bool,
    /// Out of cash: it stands empty.
    pub closed: bool,
    /// Days in a row on which it bought no inputs (11 at level 1 = shut), days in a row with
    /// sales below input costs, and whether the state subsidises it.
    pub no_input_days: u32,
    pub loss_days: u32,
    pub subsidised: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ProvinceState {
    pub owner: Option<String>,
    pub controller: Option<String>,
    /// The day the controller last changed (the military state's `last_controller_change`).
    pub controller_since: Option<Date>,
    pub cores: Vec<String>,
    /// 0 = a full state, 1 = protectorate, 2 = colony (as stored in a save).
    pub colonial: u8,
    pub modifiers: Vec<TimedModifier>,
    pub pops: Vec<PopState>,
    /// Index into `State::regions`.
    pub region: Option<u16>,
    pub continent: Option<String>,
    pub life_rating: f64,
    pub trade_goods: Option<String>,
    pub flags: BTreeSet<String>,
    /// The state this province belongs to is a slave state.
    pub slave_state: bool,
    /// Borders a sea province.
    pub coastal: bool,
    /// The state this province belongs to has at least one factory.
    pub has_factory: bool,
    /// Province buildings (fort, naval base, railroad) by name, with their level.
    pub buildings: BTreeMap<String, f64>,
    /// Position of the province's crime in the crime list, if it has one.
    pub crime: Option<usize>,
    /// People its farm or mine employs at most; `None` = everyone it has room for. And the days
    /// of fast hiring left after its owner got control back.
    pub rgo_staff: Option<f64>,
    pub rgo_fast_days: u32,
    /// Name set by `change_province_name` (display only, nothing reads it).
    pub name_override: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct CountryState {
    pub prestige: f64,
    pub money: f64,
    pub modifiers: Vec<TimedModifier>,
    pub capital: Option<u16>,
    pub flags: BTreeSet<String>,
    pub primary_culture: Option<String>,
    pub accepted_cultures: Vec<String>,
    pub civilized: bool,
    pub plurality: f64,
    pub badboy: f64,
    pub war_exhaustion: f64,
    pub leadership: f64,
    /// Research points saved up while no technology is being researched.
    pub research_points: f64,
    /// National focus by state: region number (position in `State::regions`) -> focus name.
    pub national_focus: BTreeMap<u16, String>,
    /// The day the last reform was passed.
    pub last_reform: Option<Date>,
    /// The day the last election ended.
    pub last_election: Option<Date>,
    /// The technology being researched and the points put into it so far.
    pub research: Option<(String, f64)>,
    /// Research points a day, as worked out at the start of the month.
    pub research_rate: f64,
    pub government: Option<String>,
    /// Relation with other countries, by tag.
    pub relations: BTreeMap<String, f64>,
    /// Chosen option of each reform or party issue group, by group name.
    pub reforms: BTreeMap<String, String>,
    /// Upper house shares (0..1) by ideology name.
    pub upper_house: BTreeMap<String, f64>,
    /// Researched technologies, by name.
    pub technologies: BTreeSet<String>,
    /// Active inventions, by number (position in `State::inventions` + 1).
    pub inventions: BTreeSet<u32>,
    /// Number of the ruling party (position in `State::party_ideologies` + 1).
    pub ruling_party: Option<u32>,
    pub national_value: Option<String>,
    pub tech_school: Option<String>,
    pub religion: Option<String>,
    /// Played by a person.
    pub human: bool,
    /// Countries in this country's sphere.
    pub sphere: BTreeSet<String>,
    /// Countries this one has military access through (the relation record's `military_access`).
    pub access: BTreeSet<String>,
    /// Other countries that have money invested here.
    pub foreign_investors: BTreeSet<String>,
    /// Tax collected yesterday (the market's estimate of what the state can spend today).
    pub tax_income: f64,
    /// Goods the army and navy need every day, by good (the save's land and naval supply cost).
    /// Kept as read: the port has no armies yet.
    pub state_purchases: BTreeMap<String, f64>,
    /// Budget sliders (0..1) by name: education_spending, crime_fighting, social_spending,
    /// military_spending.
    pub spending: BTreeMap<String, f64>,
    /// The ruling party's limits on the sliders: [min tax, max tax, max military spending, min
    /// tariff, max tariff] (set by the month's politics; None until then).
    pub party_limits: Option<[f64; 5]>,
    /// Influence points this country has on other countries, by tag (as the save has them; the
    /// port does not move them).
    pub influence: BTreeMap<String, f64>,
    /// Land regiments by the type of the pop behind them, ships by unit type, and the number of
    /// leaders, as the save has them (`rank::attach_forces`; the port does not move them).
    pub regiments: BTreeMap<String, f64>,
    pub ships: BTreeMap<String, f64>,
    pub leaders: f64,
    /// Mobilised: its mobilised regiments do not count in the military score.
    pub mobilized: bool,
    /// The last day it ranked inside the great powers' band (`rank::rank_day`).
    pub last_greatness: Option<Date>,
    /// Money other countries have invested here, by investor.
    pub foreign_investment: BTreeMap<String, f64>,
    /// Yesterday's military score and total score (`rank::rank_day`).
    pub military_score: f64,
    pub total_score: f64,
    /// The AI strategy block as the save has it (CAIStrategy): the value of
    /// provinces it wants (`conquer_prov`, by province) and its threat, rival and befriend lists (by
    /// tag). The port does not rebuild them (their builders are not read): kept as read.
    pub ai_lists: AiLists,
}

/// The save's AI strategy lists of one country.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AiLists {
    pub conquer_prov: BTreeMap<u16, f64>,
    pub threat: BTreeMap<String, f64>,
    pub rival: BTreeMap<String, f64>,
    pub befriend: BTreeMap<String, f64>,
}

impl AiLists {
    fn from_block(b: &Block) -> AiLists {
        let list = |k: &str| -> Vec<(String, f64)> {
            b.get_all(k)
                .filter_map(Value::as_block)
                .filter_map(|e| {
                    let id = e.get("id")?;
                    let id = id.as_str().map(str::to_string).or_else(|| id.as_f64().map(|x| (x as i64).to_string()))?;
                    Some((id, e.get("value")?.as_f64()?))
                })
                .collect()
        };
        AiLists {
            conquer_prov: list("conquer_prov").into_iter().filter_map(|(k, v)| Some((k.parse().ok()?, v))).collect(),
            threat: list("threat").into_iter().collect(),
            rival: list("rival").into_iter().collect(),
            befriend: list("befriend").into_iter().collect(),
        }
    }
}

/// An event an effect asked for, to fire on `fires`.
#[derive(Debug, Clone, PartialEq)]
pub struct PendingEvent {
    pub id: u64,
    pub target: Scope,
    /// What the new event will see as FROM: the scope that asked for it.
    pub from: Scope,
    pub fires: Date,
}

/// The part of the game state that effects change, with the definitions they need.
#[derive(Debug, Clone, PartialEq)]
pub struct State {
    pub date: Date,
    pub provinces: BTreeMap<u16, ProvinceState>,
    pub countries: BTreeMap<String, CountryState>,
    pub global_flags: BTreeSet<String>,
    pub great_powers: Vec<String>,
    /// Running wars: attackers and defenders.
    pub wars: Vec<(BTreeSet<String>, BTreeSet<String>)>,
    /// (overlord, vassal) pairs and allied pairs.
    pub vassals: Vec<(String, String)>,
    pub alliances: Vec<(String, String)>,
    pub pending_events: Vec<PendingEvent>,
    pub factories: Vec<FactoryState>,
    /// (owner, state) -> what its capitalists have put aside to build factories.
    pub investment: BTreeMap<(String, Option<u16>), f64>,
    /// Price of every good today.
    pub prices: BTreeMap<String, f64>,
    /// Goods that are on sale already (the save's `discovered_goods`).
    pub goods_on_sale: BTreeSet<String>,
    /// Per good, from yesterday's market: the share of paid demand that could be served, and
    /// the share of supply that found a buyer (both 0..1; a good not listed counts as 1).
    pub market_fill: BTreeMap<String, f64>,
    pub market_sold: BTreeMap<String, f64>,
    /// Country -> good -> [share of its paid demand it could buy yesterday, share of what it
    /// made that was sold, share of what it bought that came from the world market].
    pub home_market: BTreeMap<String, BTreeMap<String, [f64; 3]>>,
    /// Countries that own land in yesterday's overall rank order, best first (`economy::rank_order`).
    pub rank_order: Vec<String>,
    /// (owner, state) -> the one building project of its capitalists (`industry::Project`).
    pub projects: BTreeMap<(String, Option<u16>), crate::industry::Project>,
    /// What THIS and FROM mean for the event or decision being run.
    pub this: Option<Scope>,
    pub from: Option<Scope>,
    /// State of the port's own random choices.
    pub seed: u64,
    // ---- definitions (from the install; empty until `attach_world`)
    /// Pop types that are slaves.
    pub slave_types: BTreeSet<String>,
    /// Pop type -> strata (`poor`, `middle`, `rich`).
    pub strata: BTreeMap<String, String>,
    /// Region names; a province's `region` indexes this list.
    pub regions: Vec<String>,
    /// Ideology names in file order.
    pub ideologies: Vec<String>,
    /// Ideology -> the date it becomes available (those without a date always are).
    pub ideology_dates: BTreeMap<String, Date>,
    /// Ideologies an effect enabled early.
    pub enabled_ideologies: BTreeSet<String>,
    /// (option, group) of every issue option in file order.
    pub issues: Vec<(String, String)>,
    /// Province building kind (`infrastructure`, `fort`, `naval_base`) -> its name in a save and
    /// its highest level.
    pub building_kinds: BTreeMap<String, (String, f64)>,
    /// Good -> the pop type that works its farm or mine (`grain` -> farmers, `coal` -> labourers).
    pub rgo_workers: BTreeMap<String, String>,
    /// The category of each entry of `issues` (`political_reforms`, `social_reforms`, ...).
    pub issue_categories: Vec<String>,
    /// Culture -> culture group.
    pub culture_groups: BTreeMap<String, String>,
    /// Culture groups that name a union country.
    pub union_groups: BTreeSet<String>,
    /// Technology names.
    pub technologies: BTreeSet<String>,
    /// Invention names in the order a save numbers them.
    pub inventions: Vec<String>,
    /// Ideology of every party in the order a save numbers them.
    pub party_ideologies: Vec<String>,
    /// By party number - 1: its economic policy lets pops build factories (`can_build_factory`).
    pub party_pops_build: Vec<bool>,
    /// What administrative efficiency needs from the install (`pops::attach_admin_rules`).
    pub admin_rules: crate::pops::AdminRules,
    /// Land neighbours of each province.
    pub neighbours: BTreeMap<u16, BTreeSet<u16>>,
    /// Provinces of each region (a lookup; empty until `attach_world`).
    pub region_members: BTreeMap<u16, Vec<u16>>,
    /// (overlord, substate) pairs.
    pub substates: Vec<(String, String)>,
    /// Culture group -> the tag of its union country.
    pub union_tags: BTreeMap<String, String>,
    /// BADBOY_LIMIT and COLONIAL_RANK of the install's defines (0 until `attach_world`).
    pub badboy_limit: f64,
    pub colonial_rank: f64,
    /// MAX_INFLUENCE of the install; 0 = not installed, influence is then not capped.
    pub max_influence: f64,
    /// Names set by `change_region_name`, by region number (display only).
    pub region_names: BTreeMap<u16, String>,
    /// Factory type -> (the good it makes, the people one level employs).
    pub factory_types: BTreeMap<String, (String, f64)>,
    /// Pop types that work in factories.
    pub factory_worker_types: BTreeSet<String>,
    /// Province -> number of the stretch of land it lies on (provinces one can walk between
    /// without crossing water share a number).
    pub land_mass: BTreeMap<u16, u32>,
    /// Government -> months between two elections, for the governments that hold them
    /// (`politics::PoliticsRules`; filled by the caller).
    pub election_months: BTreeMap<String, i32>,
    /// Banks, stockpiles, loans and pops' savings (`economy::MoneyState`; `market_day` keeps it).
    pub money: crate::economy::MoneyState,
    /// The rules file's economy numbers (`EconomyNumbers::from_engine_rules`; filled by the caller).
    pub economy_numbers: crate::economy::EconomyNumbers,
    /// Production factor of mobilised countries (logistics::mobilisation_production_factor), by tag.
    pub mobilisation: BTreeMap<String, f64>,
}

/// Names of the flags in a save's `flags = { name = yes ... }` block.
fn flags(block: &Block) -> BTreeSet<String> {
    block.get("flags").and_then(Value::as_block).map(|f| f.entries().map(|(k, _, _)| k.to_string()).collect()).unwrap_or_default()
}

fn timed(block: &Block) -> Vec<TimedModifier> {
    block
        .get_all("modifier")
        .filter_map(|m| {
            let m = m.as_block()?;
            Some(TimedModifier { name: m.get("modifier")?.as_str()?.to_string(), expires: m.get("date").and_then(Value::as_str).and_then(Date::parse) })
        })
        .collect()
}

impl State {
    /// An empty state on `date`.
    pub fn new(date: Date) -> State {
        State {
            date,
            provinces: BTreeMap::new(),
            countries: BTreeMap::new(),
            global_flags: BTreeSet::new(),
            great_powers: Vec::new(),
            wars: Vec::new(),
            vassals: Vec::new(),
            alliances: Vec::new(),
            pending_events: Vec::new(),
            factories: Vec::new(),
            prices: BTreeMap::new(),
            goods_on_sale: BTreeSet::new(),
            market_fill: BTreeMap::new(),
            market_sold: BTreeMap::new(),
            home_market: BTreeMap::new(),
            rank_order: Vec::new(),
            projects: BTreeMap::new(),
            this: None,
            from: None,
            seed: 0,
            slave_types: BTreeSet::new(),
            strata: BTreeMap::new(),
            regions: Vec::new(),
            ideologies: Vec::new(),
            ideology_dates: BTreeMap::new(),
            investment: BTreeMap::new(),
            party_pops_build: Vec::new(),
            admin_rules: crate::pops::AdminRules::default(),
            enabled_ideologies: BTreeSet::new(),
            issues: Vec::new(),
            issue_categories: Vec::new(),
            rgo_workers: BTreeMap::new(),
            building_kinds: BTreeMap::new(),
            culture_groups: BTreeMap::new(),
            union_groups: BTreeSet::new(),
            technologies: BTreeSet::new(),
            inventions: Vec::new(),
            party_ideologies: Vec::new(),
            neighbours: BTreeMap::new(),
            region_members: BTreeMap::new(),
            substates: Vec::new(),
            union_tags: BTreeMap::new(),
            badboy_limit: 0.0,
            colonial_rank: 0.0,
            max_influence: 0.0,
            region_names: BTreeMap::new(),
            factory_types: BTreeMap::new(),
            factory_worker_types: BTreeSet::new(),
            land_mass: BTreeMap::new(),
            election_months: BTreeMap::new(),
            money: Default::default(),
            economy_numbers: Default::default(),
            mobilisation: BTreeMap::new(),
        }
    }

    /// The state as an original save describes it (for the layout).
    pub fn from_save(save: &SaveGame) -> State {
        let root = &save.doc.root;
        let mut state = State::new(save.date);
        for (id, p) in &save.provinces {
            let block = root.get(&id.to_string()).and_then(Value::as_block);
            let pops = p
                .pops
                .iter()
                .map(|q| PopState {
                    pop_type: q.pop_type.clone(),
                    id: q.id,
                    size: q.size,
                    culture: q.culture.clone(),
                    religion: q.religion.clone(),
                    money: q.money,
                    literacy: q.literacy,
                    militancy: q.militancy,
                    consciousness: q.consciousness,
                    ideology: q.ideology.clone(),
                    issues: q.issues.clone(),
                    life_needs: q.life_needs,
                    everyday_needs: q.everyday_needs,
                    luxury_needs: q.luxury_needs,
                    production: None,
                    throttle: 1.0,
                    stock: 1.0,
                    held: Vec::new(),
                    promotion_due: 0.0,
                    demotion_due: 0.0,
                })
                .collect();
            let num = |k: &str| block.and_then(|b| b.get(k)).and_then(Value::as_f64).unwrap_or(0.0);
            state.provinces.insert(
                *id,
                ProvinceState {
                    owner: p.owner.clone(),
                    controller: p.controller.clone(),
                    cores: p.cores.clone(),
                    colonial: num("colonial") as u8,
                    modifiers: block.map(timed).unwrap_or_default(),
                    pops,
                    life_rating: num("life_rating"),
                    trade_goods: block.and_then(|b| b.get("rgo")).and_then(Value::as_block).and_then(|r| r.get("goods_type")).and_then(Value::as_str).map(str::to_string),
                    flags: block.map(flags).unwrap_or_default(),
                    ..Default::default()
                },
            );
        }
        for tag in &save.countries {
            let Some(c) = root.get(tag).and_then(Value::as_block) else { continue };
            let num = |k: &str| c.get(k).and_then(Value::as_f64).unwrap_or(0.0);
            let text = |k: &str| c.get(k).and_then(Value::as_str).map(str::to_string);
            let accepted = c.get("culture").and_then(Value::as_block).map(|b| b.bare_scalars().into_iter().map(str::to_string).collect()).unwrap_or_default();
            // A relation block with level 5 marks a member of this country's sphere.
            let sphere = c.entries().filter(|(k, v, _)| is_tag(k) && v.as_block().and_then(|b| b.get("level")).and_then(Value::as_f64) == Some(5.0)).map(|(k, _, _)| k.to_string()).collect();
            let technologies = c.get("technology").and_then(Value::as_block).map(|b| b.entries().map(|(k, _, _)| k.to_string()).collect()).unwrap_or_default();
            let inventions = c.get("active_inventions").and_then(Value::as_block).map(|b| b.bare_scalars().iter().filter_map(|n| n.parse().ok()).collect()).unwrap_or_default();
            // foreign_investment lists an amount per country number (slot 0 unused); the
            // country's own slot is not foreign.
            let amounts = c.get("foreign_investment").and_then(Value::as_block).map(|b| b.bare_scalars()).unwrap_or_default();
            let foreign_investors = amounts
                .iter()
                .enumerate()
                .filter(|(_, v)| v.parse::<f64>().is_ok_and(|x| x > 0.0))
                .filter_map(|(i, _)| save.countries.get(i.checked_sub(1)?))
                .filter(|t| *t != tag)
                .cloned()
                .collect();
            let relations = c
                .entries()
                .filter(|(k, _, _)| is_tag(k))
                .filter_map(|(k, v, _)| Some((k.to_string(), v.as_block()?.get("value").and_then(Value::as_f64).unwrap_or(0.0))))
                .collect();
            let upper_house = c.get("upper_house").and_then(Value::as_block).map(|b| b.entries().filter_map(|(k, v, _)| Some((k.to_string(), v.as_f64()?))).collect()).unwrap_or_default();
            for s in c.get_all("state").filter_map(Value::as_block) {
                let slave = s.get("is_slave").and_then(Value::as_str) == Some("yes");
                let factory = s.get("state_buildings").is_some();
                if !slave && !factory {
                    continue;
                }
                for id in s.get("provinces").and_then(Value::as_block).map(|b| b.bare_scalars()).unwrap_or_default() {
                    if let Some(p) = id.parse::<u16>().ok().and_then(|id| state.provinces.get_mut(&id)) {
                        p.slave_state |= slave;
                        p.has_factory |= factory;
                    }
                }
            }
            let mut spending: BTreeMap<String, f64> = ["education_spending", "crime_fighting", "social_spending", "military_spending"]
                .iter()
                .filter_map(|k| Some((k.to_string(), c.get(k)?.as_block()?.get("settings")?.as_f64()?)))
                .collect();
            // Tax sliders sit next to them under their own names.
            for k in ["rich_tax", "middle_tax", "poor_tax"] {
                if let Some(x) = c.get(k).and_then(Value::as_block).and_then(|b| b.get("current")).and_then(Value::as_f64) {
                    spending.insert(k.to_string(), x);
                }
            }
            if let Some(x) = c.get("tariffs").and_then(Value::as_f64) {
                spending.insert("tariffs".to_string(), x);
            }
            // What army and navy need a day: the save's land and naval supply cost, by good.
            let mut state_purchases: BTreeMap<String, f64> = BTreeMap::new();
            for k in ["land_supply_cost", "naval_supply_cost"] {
                for (good, v, _) in c.get(k).and_then(Value::as_block).map(|b| b.entries().collect::<Vec<_>>()).unwrap_or_default() {
                    if let Some(x) = v.as_f64() {
                        *state_purchases.entry(good.to_string()).or_default() += x;
                    }
                }
            }
            state.countries.insert(
                tag.clone(),
                CountryState {
                    state_purchases,
                    tax_income: 0.0,
                    prestige: num("prestige"),
                    money: num("money"),
                    modifiers: timed(c),
                    capital: c.get("capital").and_then(Value::as_f64).map(|x| x as u16),
                    flags: flags(c),
                    primary_culture: text("primary_culture"),
                    accepted_cultures: accepted,
                    civilized: c.get("civilized").and_then(Value::as_str) == Some("yes"),
                    plurality: num("plurality"),
                    badboy: num("badboy"),
                    war_exhaustion: num("war_exhaustion"),
                    leadership: num("leadership"),
                    research_points: num("research_points"),
                    research: c.get("research").and_then(Value::as_block).and_then(|r| Some((r.get("technology")?.as_str()?.to_string(), r.get("cost").and_then(Value::as_f64).unwrap_or(0.0)))),
                    national_focus: c.get("national_focus").and_then(Value::as_block).map(|b| b.entries().filter_map(|(k, v, _)| Some((k.trim_matches('"').parse().ok()?, v.as_str()?.to_string()))).collect()).unwrap_or_default(),
                    last_reform: c.get("last_reform").and_then(Value::as_str).and_then(Date::parse),
                    last_election: c.get("last_election").and_then(Value::as_str).and_then(Date::parse),
                    research_rate: c.get("research").and_then(Value::as_block).and_then(|r| r.get("last_spending")).and_then(Value::as_f64).unwrap_or(0.0),
                    government: text("government"),
                    relations,
                    reforms: BTreeMap::new(),
                    upper_house,
                    technologies,
                    inventions,
                    ruling_party: c.get("ruling_party").and_then(Value::as_f64).map(|x| x as u32),
                    national_value: text("nationalvalue"),
                    tech_school: text("schools"),
                    religion: text("religion"),
                    human: c.get("human").and_then(Value::as_str) == Some("yes"),
                    sphere,
                    access: c.entries().filter(|(k, v, _)| is_tag(k) && v.as_block().and_then(|b| b.get("military_access")).and_then(Value::as_str) == Some("yes")).map(|(k, _, _)| k.to_string()).collect(),
                    ai_lists: c.get("ai").and_then(Value::as_block).map(AiLists::from_block).unwrap_or_default(),
                    foreign_investors,
                    spending,
                    influence: c.entries().filter(|(k, _, _)| is_tag(k)).filter_map(|(k, v, _)| Some((k.to_string(), v.as_block()?.get("influence_value")?.as_f64()?))).collect(),
                    ..Default::default()
                },
            );
        }
        state.global_flags = flags(root);
        let tags = |b: &Block, k: &str| -> BTreeSet<String> { b.get_all(k).filter_map(Value::as_str).map(str::to_string).collect() };
        state.wars = root.get_all("active_war").filter_map(Value::as_block).map(|w| (tags(w, "attacker"), tags(w, "defender"))).collect();
        if let Some(d) = root.get("diplomacy").and_then(Value::as_block) {
            let pairs = |k: &str| -> Vec<(String, String)> {
                d.get_all(k).filter_map(Value::as_block).filter_map(|b| Some((b.get("first")?.as_str()?.to_string(), b.get("second")?.as_str()?.to_string()))).collect()
            };
            state.vassals = pairs("vassal");
            state.alliances = pairs("alliance");
            state.substates = pairs("substate");
        }
        // great_nations numbers the country blocks from 1.
        let numbers = root.get("great_nations").and_then(Value::as_block).map(|b| b.bare_scalars()).unwrap_or_default();
        if let Some(pool) = root.get("worldmarket").and_then(Value::as_block).and_then(|w| w.get("price_pool")).and_then(Value::as_block) {
            state.prices = pool.entries().filter_map(|(k, v, _)| Some((k.to_string(), v.as_f64()?))).collect();
        }
        if let Some(pool) = root.get("worldmarket").and_then(Value::as_block).and_then(|w| w.get("discovered_goods")).and_then(Value::as_block) {
            state.goods_on_sale = pool.entries().filter(|(_, v, _)| v.as_f64().is_some_and(|x| x > 0.0)).map(|(k, _, _)| k.to_string()).collect();
        }
        state.great_powers = numbers.iter().filter_map(|n| save.countries.get(n.parse::<usize>().ok()?.checked_sub(1)?).cloned()).collect();
        state
    }

    /// Adds what only the install knows: regions, neighbours, pop strata, ideology and issue
    /// names, culture groups; and reads each country's chosen reforms from the save.
    pub fn attach_world(&mut self, world: &World, save: &SaveGame) {
        self.regions = world.regions.iter().map(|r| r.name.clone()).collect();
        for (id, p) in &mut self.provinces {
            let Some(w) = world.province(*id) else { continue };
            p.region = w.region;
            if let Some(r) = w.region {
                self.region_members.entry(r).or_default().push(*id);
            }
            p.continent = w.continent.clone();
            if let Some(block) = save.doc.root.get(&id.to_string()).and_then(Value::as_block) {
                for b in &world.defs.buildings {
                    let level = block.get(&b.name).and_then(Value::as_block).and_then(|l| l.bare_scalars().first().and_then(|s| s.parse::<f64>().ok()));
                    if let Some(level) = level.filter(|l| *l > 0.0) {
                        p.buildings.insert(b.name.clone(), level);
                    }
                }
                p.crime = block.get("crime").and_then(Value::as_f64).map(|c| c as usize);
                // Artisans: what each pop is producing.
                for q in block.get_all("artisans").filter_map(Value::as_block) {
                    let (Some(pop_id), Some(kind)) = (q.get("id").and_then(Value::as_f64), q.get("production_type").and_then(Value::as_str)) else { continue };
                    if let Some(pop) = p.pops.iter_mut().find(|x| x.id == pop_id as u64 && x.pop_type == "artisans") {
                        pop.production = Some(kind.to_string());
                        pop.throttle = q.get("throttle").and_then(Value::as_f64).unwrap_or(1.0);
                        pop.stock = q.get("percent_afforded").and_then(Value::as_f64).unwrap_or(1.0);
                    }
                }
            }
            if let Some(near) = world.adjacency.get(*id as usize) {
                p.coastal = near.iter().any(|n| world.province(*n).is_some_and(|q| q.is_sea));
                let land: BTreeSet<u16> = near.iter().copied().filter(|n| world.province(*n).is_some_and(|q| !q.is_sea)).collect();
                self.neighbours.insert(*id, land);
            }
        }
        // A strait crossing makes two provinces neighbours (measured: England and Morocco).
        for x in world.extra_adjacencies.iter().filter(|x| x.kind == "sea") {
            for (a, b) in [(x.from, x.to), (x.to, x.from)] {
                if self.provinces.contains_key(&a) && self.provinces.contains_key(&b) {
                    self.neighbours.entry(a).or_default().insert(b);
                }
            }
        }
        self.slave_types = world.defs.pop_types.iter().filter(|t| t.is_slave).map(|t| t.name.clone()).collect();
        self.strata = world.defs.pop_types.iter().map(|t| (t.name.clone(), t.strata.clone())).collect();
        self.ideologies = world.defs.ideologies.clone();
        self.ideology_dates = world.defs.ideologies.iter().zip(&world.defs.ideology_dates).filter_map(|(n, d)| Some((n.clone(), (*d)?))).collect();
        self.issues = world.defs.issues.iter().flat_map(|g| g.options.iter().map(move |o| (o.clone(), g.name.clone()))).collect();
        self.building_kinds = world.defs.buildings.iter().filter(|b| b.kind != "factory" && b.max_level > 0).map(|b| (b.kind.clone(), (b.name.clone(), f64::from(b.max_level)))).collect();
        self.rgo_workers = world.defs.rgo_types.iter().filter_map(|r| Some((r.good.clone(), r.employees.iter().map(|j| &j.pop_type).find(|t| !world.defs.pop_types.iter().any(|p| p.is_slave && p.name == **t))?.clone()))).collect();
        self.issue_categories = world.defs.issues.iter().flat_map(|g| g.options.iter().map(move |_| g.category.clone())).collect();
        self.technologies = world.defs.technologies.iter().map(|t| t.name.clone()).collect();
        // Factories: the state buildings of every country.
        self.factories.clear();
        for tag in &save.countries {
            let Some(c) = save.doc.root.get(tag).and_then(Value::as_block) else { continue };
            for s in c.get_all("state").filter_map(Value::as_block) {
                let first = s.get("provinces").and_then(Value::as_block).and_then(|b| b.bare_scalars().first().and_then(|x| x.parse::<u16>().ok()));
                let region = first.and_then(|id| world.province(id)).and_then(|p| p.region);
                for b in s.get_all("state_buildings").filter_map(Value::as_block) {
                    if let Some(kind) = b.get("building").and_then(Value::as_str) {
                        self.factories.push(FactoryState { owner: tag.clone(), region, kind: kind.to_string(), level: b.get("level").and_then(Value::as_f64).unwrap_or(1.0), money: b.get("money").and_then(Value::as_f64).unwrap_or(0.0), staff: b.get("employment").and_then(Value::as_block).and_then(|e| e.get("employees")).and_then(Value::as_block).map(|e| e.items.iter().filter_map(|i| i.value.as_block()?.get("count")?.as_f64()).sum()), ..Default::default() });
                    }
                }
            }
        }
        self.inventions = world.defs.inventions.clone();
        // Parties are numbered through all countries in definition order.
        self.party_ideologies = world.countries.iter().flat_map(|c| c.country.parties.iter().map(|p| p.ideology.clone().unwrap_or_default())).collect();
        let cultures = &world.defs.cultures;
        self.culture_groups = cultures.cultures.iter().filter_map(|c| Some((c.name.clone(), cultures.groups.get(c.group as usize)?.name.clone()))).collect();
        self.union_groups = cultures.groups.iter().filter(|g| g.union.is_some()).map(|g| g.name.clone()).collect();
        self.union_tags = cultures.groups.iter().filter_map(|g| Some((g.name.clone(), g.union.clone()?))).collect();
        self.badboy_limit = world.defines.get("country", "BADBOY_LIMIT").unwrap_or(0.0);
        self.colonial_rank = world.defines.get("country", "COLONIAL_RANK").unwrap_or(0.0);
        self.max_influence = world.defines.get("diplomacy", "MAX_INFLUENCE").unwrap_or(0.0);
        crate::rank::attach_forces(self, save);
        self.factory_types = world.defs.factory_types.iter().map(|t| (t.name.clone(), (t.output_good.clone(), t.workforce))).collect();
        self.factory_worker_types = world.defs.factory_types.iter().flat_map(|t| t.employees.iter()).map(|j| j.pop_type.clone()).collect();
        // Stretches of land: provinces joined by land borders (a strait crossing does not join
        // two of them; the port's reading of the original's land-connection number).
        self.land_mass.clear();
        let land = |id: u16| self.provinces.contains_key(&id) && world.province(id).is_some_and(|q| !q.is_sea);
        let ids: Vec<u16> = self.provinces.keys().copied().filter(|id| land(*id)).collect();
        for (n, start) in ids.into_iter().enumerate() {
            if self.land_mass.contains_key(&start) {
                continue;
            }
            let mut todo = vec![start];
            while let Some(id) = todo.pop() {
                if self.land_mass.insert(id, n as u32).is_none() {
                    todo.extend(world.adjacency.get(id as usize).into_iter().flatten().copied().filter(|x| land(*x) && !self.land_mass.contains_key(x)));
                }
            }
        }
        for (tag, c) in &mut self.countries {
            let Some(block) = save.doc.root.get(tag).and_then(Value::as_block) else { continue };
            for g in &world.defs.issues {
                if let Some(option) = block.get(&g.name).and_then(Value::as_str) {
                    c.reforms.insert(g.name.clone(), option.to_string());
                }
            }
        }
    }

    /// Countries that own at least one province.
    pub fn existing(&self) -> BTreeSet<String> {
        self.provinces.values().filter_map(|p| p.owner.clone()).collect()
    }

    pub(crate) fn owned(&self, tag: &str) -> Vec<u16> {
        self.provinces.iter().filter(|(_, p)| p.owner.as_deref() == Some(tag)).map(|(id, _)| *id).collect()
    }

    fn in_region(&self, region: u16) -> Vec<u16> {
        // The list made by `attach_world`, when there is one (a province never changes region).
        match self.region_members.get(&region) {
            Some(ids) => ids.clone(),
            None => self.provinces.iter().filter(|(_, p)| p.region == Some(region)).map(|(id, _)| *id).collect(),
        }
    }

    /// The provinces a scope covers. None for a pop.
    pub(crate) fn provinces_of(&self, scope: &Scope) -> Option<Vec<u16>> {
        match scope {
            Scope::Country(tag) => Some(self.owned(tag)),
            Scope::State(region, tag) => Some(self.in_region(*region).into_iter().filter(|id| self.provinces[id].owner.as_deref() == Some(tag.as_str())).collect()),
            Scope::Province(id) => Some(vec![*id]),
            Scope::Pop(..) => None,
        }
    }

    /// The pops a scope covers, optionally only those `keep` accepts.
    pub(crate) fn pops_of(&self, scope: &Scope, keep: impl Fn(&PopState) -> bool) -> Option<Vec<Scope>> {
        let ids = self.provinces_of(scope)?;
        Some(ids.into_iter().flat_map(|id| self.provinces[&id].pops.iter().enumerate().filter(|(_, p)| keep(p)).map(move |(i, _)| Scope::Pop(id, i)).collect::<Vec<_>>()).collect())
    }

    /// The states of a country: one per region it holds land in.
    pub(crate) fn states_of(&self, tag: &str) -> Vec<Scope> {
        let regions: BTreeSet<u16> = self.provinces.values().filter(|p| p.owner.as_deref() == Some(tag)).filter_map(|p| p.region).collect();
        regions.into_iter().map(|r| Scope::State(r, tag.to_string())).collect()
    }

    /// Owners of land next to the country's land, except itself.
    pub(crate) fn neighbour_countries(&self, tag: &str) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        for id in self.owned(tag) {
            for n in self.neighbours.get(&id).into_iter().flatten() {
                if let Some(o) = self.provinces.get(n).and_then(|p| p.owner.as_deref()).filter(|o| *o != tag) {
                    out.insert(o.to_string());
                }
            }
        }
        out
    }

    pub(crate) fn enemies(&self, tag: &str) -> BTreeSet<&str> {
        let mut out = BTreeSet::new();
        for (a, d) in &self.wars {
            if a.contains(tag) {
                out.extend(d.iter().map(String::as_str));
            }
            if d.contains(tag) {
                out.extend(a.iter().map(String::as_str));
            }
        }
        out
    }

    /// The great power whose sphere holds `tag`.
    fn sphere_leader(&self, tag: &str) -> Option<&str> {
        self.countries.iter().find(|(_, c)| c.sphere.contains(tag)).map(|(t, _)| t.as_str())
    }

    /// The union country of a culture's group (`union = TAG` in the cultures file).
    fn union_of(&self, culture: Option<&str>) -> Option<&str> {
        self.union_tags.get(self.culture_groups.get(culture?)?).map(String::as_str)
    }

    /// The country `tag` is a vassal or a substate of.
    fn overlord(&self, tag: &str) -> Option<&str> {
        self.vassals.iter().chain(&self.substates).find(|(_, v)| v == tag).map(|(o, _)| o.as_str())
    }

    /// A country's place in yesterday's ranking, 1 = best. None while no ranking was made.
    fn rank_of(&self, tag: &str) -> Option<usize> {
        self.rank_order.iter().position(|t| t == tag).map(|i| i + 1)
    }

    /// The provinces of the state a province belongs to: same region, same owner.
    pub(crate) fn state_of(&self, id: u16) -> Vec<u16> {
        match self.provinces.get(&id) {
            Some(p) if p.region.is_some() && p.owner.is_some() => self.in_region(p.region.unwrap_or(0)).into_iter().filter(|x| self.provinces[x].owner == p.owner).collect(),
            _ => vec![id],
        }
    }

    /// A country's industrial score, counted as the ranking counts it (`economy::industrial_score`).
    pub fn industrial_score(&self, tag: &str) -> f64 {
        crate::economy::industrial_score(&self.economy_numbers, &self.industrial_states(tag).into_values().collect::<Vec<_>>())
    }

    /// What the industrial score counts in each state of a country that has a factory: the sum
    /// of the factory levels, the places (level x workforce) and the people of the pop types
    /// that work in factories.
    pub fn industrial_states(&self, tag: &str) -> BTreeMap<Option<u16>, (f64, f64, f64)> {
        let mut states: BTreeMap<Option<u16>, (f64, f64, f64)> = BTreeMap::new();
        for f in self.factories.iter().filter(|f| f.owner == tag && f.level >= 1.0) {
            if let Some((_, workforce)) = self.factory_types.get(&f.kind) {
                let e = states.entry(f.region).or_default();
                (e.0, e.1) = (e.0 + f.level, e.1 + f.level * workforce);
            }
        }
        for p in self.provinces.values().filter(|p| p.owner.as_deref() == Some(tag)) {
            if let Some(e) = states.get_mut(&p.region) {
                e.2 += p.pops.iter().filter(|q| self.factory_worker_types.contains(&q.pop_type)).map(|q| q.size).sum::<f64>();
            }
        }
        states
    }

    /// The share of a state's factory workers that its open factories employ (0..1).
    fn factory_rate(&self, owner: Option<&str>, region: Option<u16>) -> f64 {
        let (Some(owner), Some(region)) = (owner, region) else { return 0.0 };
        let jobs: f64 = self
            .factories
            .iter()
            .filter(|f| f.owner == owner && f.region == Some(region) && f.level >= 1.0 && !f.closed)
            .map(|f| {
                let places = f.level * self.factory_types.get(&f.kind).map_or(0.0, |t| t.1);
                f.staff.map_or(places, |s| s.min(places))
            })
            .sum();
        let people: f64 = self.in_region(region).iter().filter_map(|id| self.provinces.get(id)).filter(|p| p.owner.as_deref() == Some(owner)).flat_map(|p| &p.pops).filter(|q| self.factory_worker_types.contains(&q.pop_type)).map(|q| q.size).sum();
        if people > 0.0 { (jobs / people).min(1.0) } else { 0.0 }
    }

    /// (People with work, people) among the pops of the given provinces, optionally of one pop
    /// type only, as far as the port knows employment:
    /// * the pop types that work in factories share the staffed places of their state's open
    ///   factories;
    /// * the pop type that works the province's farm or mine (and slaves) share the people it
    ///   employs (`ProvinceState::rgo_staff`; not set = all of them);
    /// * everybody else cannot be out of work.
    fn employment(&self, ids: &[u16], kind: Option<&str>) -> (f64, f64) {
        let mut rates: BTreeMap<(Option<&str>, Option<u16>), f64> = BTreeMap::new();
        let (mut employed, mut people) = (0.0, 0.0);
        for p in ids.iter().filter_map(|id| self.provinces.get(id)) {
            let worker = p.trade_goods.as_ref().and_then(|g| self.rgo_workers.get(g));
            let on_farm = |q: &PopState| self.slave_types.contains(&q.pop_type) || worker == Some(&q.pop_type);
            let farm_people: f64 = p.pops.iter().filter(|q| on_farm(q)).map(|q| q.size).sum();
            let farm_rate = match p.rgo_staff {
                Some(staff) if farm_people > 0.0 => (staff / farm_people).clamp(0.0, 1.0),
                _ => 1.0,
            };
            let factory_rate = *rates.entry((p.owner.as_deref(), p.region)).or_insert_with(|| self.factory_rate(p.owner.as_deref(), p.region));
            for q in p.pops.iter().filter(|q| kind.is_none_or(|k| q.pop_type == k)) {
                people += q.size;
                employed += q.size * if self.factory_worker_types.contains(&q.pop_type) { factory_rate } else if on_farm(q) { farm_rate } else { 1.0 };
            }
        }
        (employed, people)
    }

    /// The share (0..1) of those people without work; 0 when there are no such people.
    pub fn unemployment(&self, ids: &[u16], kind: Option<&str>) -> f64 {
        let (employed, people) = self.employment(ids, kind);
        if people > 0.0 { 1.0 - employed / people } else { 0.0 }
    }

    /// The country `old` becomes `new` (`change_tag`): its whole record moves to the new tag,
    /// the unused record of the new tag takes the old tag's place, and everything that named
    /// one of the two names the other. Cores are left as they are (checked on the original's
    /// saves: after Prussia became the North German Federation in 1846 all 47 Prussian cores
    /// were still there, and the old tag had an empty record).
    /// Not followed: the parties stay those of the old tag (`ruling_party` is a party number).
    fn rename_country(&mut self, old: &str, new: &str) {
        let swap = |t: &mut String| {
            if t == old {
                *t = new.to_string();
            } else if t == new {
                *t = old.to_string();
            }
        };
        let swapped = |t: &str| if t == old { new.to_string() } else if t == new { old.to_string() } else { t.to_string() };
        let swap_scope = |s: &mut Scope| {
            if let Scope::Country(t) | Scope::State(_, t) = s {
                swap(t);
            }
        };
        let (a, b) = (self.countries.remove(old), self.countries.remove(new));
        self.countries.insert(new.to_string(), a.unwrap_or_default());
        self.countries.insert(old.to_string(), b.unwrap_or_default());
        for p in self.provinces.values_mut() {
            p.owner.iter_mut().chain(p.controller.iter_mut()).for_each(swap);
        }
        for c in self.countries.values_mut() {
            c.sphere = c.sphere.iter().map(|t| swapped(t)).collect();
            c.foreign_investors = c.foreign_investors.iter().map(|t| swapped(t)).collect();
            c.relations = std::mem::take(&mut c.relations).into_iter().map(|(t, v)| (swapped(&t), v)).collect();
            c.influence = std::mem::take(&mut c.influence).into_iter().map(|(t, v)| (swapped(&t), v)).collect();
        }
        self.factories.iter_mut().for_each(|f| swap(&mut f.owner));
        self.investment = std::mem::take(&mut self.investment).into_iter().map(|((t, r), v)| ((swapped(&t), r), v)).collect();
        self.projects = std::mem::take(&mut self.projects).into_iter().map(|((t, r), v)| ((swapped(&t), r), v)).collect();
        self.home_market = std::mem::take(&mut self.home_market).into_iter().map(|(t, v)| (swapped(&t), v)).collect();
        self.great_powers.iter_mut().chain(self.rank_order.iter_mut()).for_each(swap);
        for (x, y) in self.vassals.iter_mut().chain(self.substates.iter_mut()).chain(self.alliances.iter_mut()) {
            swap(x);
            swap(y);
        }
        for (x, y) in &mut self.wars {
            *x = x.iter().map(|t| swapped(t)).collect();
            *y = y.iter().map(|t| swapped(t)).collect();
        }
        for e in &mut self.pending_events {
            swap_scope(&mut e.target);
            swap_scope(&mut e.from);
        }
        self.this.iter_mut().chain(self.from.iter_mut()).for_each(swap_scope);
    }

    /// Hands a province to another country: owner and controller change. A receiver without a
    /// capital on its own land takes this province as its capital. When the giver has nothing
    /// left of the state, the state's factories go to the receiver and the giver's building
    /// plans there are dropped. Not followed: the giver's capital, its armies, the pops' money.
    fn give_province(&mut self, id: u16, to: &str) {
        let Some(p) = self.provinces.get_mut(&id) else { return };
        let (from, region) = (p.owner.replace(to.to_string()), p.region);
        p.controller = Some(to.to_string());
        if from.as_deref() == Some(to) {
            return;
        }
        let homeless = self.countries.get(to).and_then(|c| c.capital).and_then(|cap| self.provinces.get(&cap)).is_none_or(|q| q.owner.as_deref() != Some(to));
        if homeless {
            self.countries.entry(to.to_string()).or_default().capital = Some(id);
        }
        let Some(region) = region else { return };
        let members = self.in_region(region);
        if let Some(from) = from.filter(|f| !members.iter().any(|x| self.provinces[x].owner.as_deref() == Some(f.as_str()))) {
            self.factories.iter_mut().filter(|f| f.owner == from && f.region == Some(region)).for_each(|f| f.owner = to.to_string());
            let key = (from, Some(region));
            self.investment.remove(&key);
            self.projects.remove(&key);
        }
        for x in members {
            let has = self.provinces[&x].owner.as_ref().is_some_and(|o| self.factories.iter().any(|f| f.owner == *o && f.region == Some(region)));
            if let Some(q) = self.provinces.get_mut(&x) {
                q.has_factory = has;
            }
        }
    }

    /// The port's own dice: a number from 0 up to, not including, 1.
    pub fn roll_fraction(&mut self) -> f64 {
        self.roll(1 << 30) as f64 / (1u64 << 30) as f64
    }

    /// The port's own dice: a number below `n` (n > 0).
    fn roll(&mut self, n: usize) -> usize {
        self.seed = self.seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.seed;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        ((z ^ (z >> 31)) % n as u64) as usize
    }

    /// The country a name stands for: a tag, or THIS / FROM when they are known.
    fn tag_named(&self, name: &str) -> Option<String> {
        match name {
            "THIS" => self.this.as_ref().and_then(|s| country_of(self, s)).map(str::to_string),
            "FROM" => self.from.as_ref().and_then(|s| country_of(self, s)).map(str::to_string),
            _ if is_tag(name) && self.countries.contains_key(name) => Some(name.to_string()),
            _ => None,
        }
    }

    fn pop_mut(&mut self, scope: &Scope) -> Option<&mut PopState> {
        match scope {
            Scope::Pop(id, i) => self.provinces.get_mut(id)?.pops.get_mut(*i),
            _ => None,
        }
    }
}

/// What the entries of a block apply to.
#[derive(Debug, Clone, PartialEq)]
pub enum Scope {
    Country(String),
    /// Region (index into `State::regions`) and owner.
    State(u16, String),
    Province(u16),
    /// Province and position of the pop in it.
    Pop(u16, usize),
}

/// Limits the engine applies to values that effects change.
#[derive(Debug, Clone, PartialEq)]
pub struct EffectRules {
    pub prestige_min: f64,
    pub treasury_min: f64,
    pub consciousness_max: f64,
    pub militancy_max: f64,
    pub literacy_max: f64,
    pub pop_size_max: f64,
    pub plurality_max: f64,
    pub war_exhaustion_max: f64,
    pub badboy_min: f64,
    pub relation_limit: f64,
    /// `money = x` on a pop adds x times this to the pop's stored money (measured: 1000).
    pub pop_money_scale: f64,
    /// Start of the port's own random choices.
    pub seed: u64,
}

impl EffectRules {
    pub fn from_engine_rules(m: &EngineRules) -> Result<EffectRules, String> {
        Ok(EffectRules {
            prestige_min: crate::num_field(m, "FLD_PRESTIGE_MIN")?,
            treasury_min: crate::num_field(m, "FLD_TREASURY_MIN")?,
            consciousness_max: crate::num_field(m, "FLD_POP_CON_MAX")?,
            militancy_max: crate::num_field(m, "FLD_POP_MIL_MAX")?,
            literacy_max: crate::num_field(m, "FLD_POP_LITERACY_MAX")?,
            pop_size_max: crate::num_field(m, "FLD_POP_SIZE_MAX")?,
            plurality_max: crate::num_field(m, "FLD_PLURALITY_MAX")?,
            war_exhaustion_max: crate::num_field(m, "FLD_WAR_EXHAUSTION_MAX")?,
            badboy_min: crate::num_field(m, "FLD_BADBOY_MIN")?,
            relation_limit: crate::num_field(m, "FLD_RELATION_LIMIT")?,
            pop_money_scale: crate::num_field(m, "FLD_POP_MONEY_EFFECT_SCALE")?,
            seed: crate::num_field(m, "FLD_SCRIPT_SEED")? as u64,
        })
    }
}

/// What a run did: how often each effect was applied, and what was skipped because the engine
/// does not know it (the key, with the scope it appeared in).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Report {
    pub applied: BTreeMap<String, usize>,
    pub unsupported: BTreeMap<String, usize>,
    /// Times each named modifier was added to a province or country.
    pub modifiers_added: BTreeMap<String, usize>,
}

impl Report {
    fn did(&mut self, what: &str) {
        *self.applied.entry(what.to_string()).or_default() += 1;
    }
    fn skip(&mut self, key: &str, scope: &Scope) {
        let kind = match scope {
            Scope::Country(_) => "country",
            Scope::State(..) => "state",
            Scope::Province(_) => "province",
            Scope::Pop(..) => "pop",
        };
        *self.unsupported.entry(format!("{key} (in {kind} scope)")).or_default() += 1;
    }
}

pub(crate) fn is_tag(key: &str) -> bool {
    key.len() == 3 && key.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()) && key.chars().next().is_some_and(|c| c.is_ascii_uppercase())
}

/// Entries that are not effects: an option's label and AI weight, a scope's `limit` (handled
/// where the scope picks its targets) and the `chance` of a `random` block.
const OPTION_LABELS: [&str; 4] = ["name", "ai_chance", "limit", "chance"];

/// Applies every entry of `block` to `scope`.
pub fn run_effects(state: &mut State, cal: &Calendar, rules: &EffectRules, block: &Block, scope: &Scope, report: &mut Report) {
    // `change_tag` turns the country in scope into another one: the entries written after it
    // in the same block apply to the new tag.
    let mut renamed: Option<Scope> = None;
    for item in &block.items {
        let Some(key) = item.key.as_deref() else { continue };
        if OPTION_LABELS.contains(&key) {
            continue;
        }
        if let ("change_tag", Some(name)) = (key, item.value.as_str()) {
            let new = match renamed.as_ref().unwrap_or(scope) {
                Scope::Country(old) => change_tag(state, old, name),
                _ => None,
            };
            match new {
                Some(new) => {
                    report.did(key);
                    renamed = Some(Scope::Country(new));
                }
                None => report.skip(key, renamed.as_ref().unwrap_or(scope)),
            }
            continue;
        }
        let scope = renamed.as_ref().unwrap_or(scope);
        match &item.value {
            Value::Block(inner) => run_block_entry(state, cal, rules, key, inner, scope, report),
            other => match (other.as_f64(), other.as_str()) {
                (Some(x), _) => run_number_effect(state, rules, key, x, scope, report),
                (None, Some(name)) => run_name_effect(state, key, name, scope, report),
                (None, None) => report.skip(key, scope),
            },
        }
    }
}

/// `change_tag = TAG` or `= culture` (the union country of the primary culture's group; nothing
/// when the group has none). Returns the new tag. Nothing happens when the new tag already owns
/// land (the port's reading: two countries cannot share a tag).
fn change_tag(state: &mut State, old: &str, name: &str) -> Option<String> {
    let new = match name {
        "culture" => state.union_of(state.countries.get(old)?.primary_culture.as_deref())?.to_string(),
        "THIS" | "FROM" => state.tag_named(name)?,
        _ if is_tag(name) => name.to_string(),
        _ => return None,
    };
    if new == old || state.provinces.values().any(|p| p.owner.as_deref() == Some(new.as_str())) {
        return None;
    }
    state.rename_country(old, &new);
    Some(new)
}

/// What a technology's or an invention's block unlocks: (buildings, unit). These two words are
/// not effects but settings of the block, and the original keeps ONE slot for each: of several
/// `activate_unit` lines only the last exists. Of several `activate_building` lines the
/// technology itself also remembers only the last, but every named building records "this
/// technology unlocks me", so all of them are returned (which of the two records the original
/// asks when it unlocks is not known; `industry` reads all lines as well).
pub fn activations(block: &Block) -> (Vec<String>, Option<String>) {
    let names = |k: &str| -> Vec<String> { block.get_all(k).filter_map(Value::as_str).map(str::to_string).collect() };
    (names("activate_building"), names("activate_unit").pop())
}

/// The date `days` days after today.
/// Days from year 0 on the game's calendar (365 days, no leap years).
fn day_count(d: Date) -> i64 {
    const BEFORE: [i64; 12] = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
    i64::from(d.year) * 365 + BEFORE[(d.month.clamp(1, 12) - 1) as usize] + i64::from(d.day)
}

fn days_from_now(state: &State, cal: &Calendar, days: f64) -> Option<Date> {
    cal.to_game(state.date).map(|g| cal.to_date(GameDate(g.0 + days as i64 * 24)))
}

/// Adds `amount` (as a fraction of the whole) to one share and rescales all shares so they sum
/// to `total` again.
fn shift_share<K: Ord>(shares: &mut BTreeMap<K, f64>, key: K, amount: f64, total: f64) {
    let v = shares.entry(key).or_insert(0.0);
    *v = (*v + amount * total).max(0.0);
    let sum: f64 = shares.values().sum();
    if sum > 0.0 {
        shares.values_mut().for_each(|v| *v *= total / sum);
    }
}

/// Effects that take a block of parameters. None = `key` is not such an effect;
/// Some(false) = it is, but could not be applied here.
fn run_block_effect(state: &mut State, cal: &Calendar, rules: &EffectRules, key: &str, inner: &Block, scope: &Scope) -> Option<bool> {
    let text = |k: &str| inner.get(k).and_then(Value::as_str);
    let number = |k: &str| inner.get(k).and_then(Value::as_f64);
    let done = match key {
        "add_province_modifier" | "add_country_modifier" => (|| {
            let days = number("duration").unwrap_or(0.0);
            let expires = if days > 0.0 { days_from_now(state, cal, days) } else { None };
            let modifier = TimedModifier { name: text("name")?.to_string(), expires };
            match (key, scope) {
                ("add_province_modifier", Scope::Province(id)) => state.provinces.get_mut(id)?.modifiers.push(modifier),
                ("add_country_modifier", Scope::Country(tag)) => state.countries.get_mut(tag)?.modifiers.push(modifier),
                // In a province, state or pop scope the modifier goes to the owner (the port's
                // reading: the description gives this fall-back for `treasury`, not for this word).
                ("add_country_modifier", _) => {
                    let owner = country_of(state, scope)?.to_string();
                    state.countries.get_mut(&owner)?.modifiers.push(modifier)
                }
                _ => return None,
            }
            Some(())
        })(),
        "relation" => (|| {
            let Scope::Country(me) = scope else { return None };
            let (who, x) = (state.tag_named(text("who")?)?, number("value")?);
            if who == *me {
                return None;
            }
            for (a, b) in [(me, &who), (&who, me)] {
                let r = state.countries.get_mut(a)?.relations.entry(b.clone()).or_insert(0.0);
                *r = (*r + x).clamp(-rules.relation_limit, rules.relation_limit);
            }
            Some(())
        })(),
        // Adds `value` to the influence the scope country holds in `who`: not below 0, and not
        // above MAX_INFLUENCE when the install's value is known (the original's routine for the
        // effect was not read; the condition reads the same figure).
        "diplomatic_influence" => (|| {
            let Scope::Country(me) = scope else { return None };
            let (who, x) = (state.tag_named(text("who")?)?, number("value")?);
            let cap = if state.max_influence > 0.0 { state.max_influence } else { f64::INFINITY };
            let c = state.countries.get_mut(me)?;
            let v = c.influence.entry(who).or_insert(0.0);
            *v = (*v + x).clamp(0.0, cap);
            Some(())
        })(),
        "upper_house" => (|| {
            let Scope::Country(tag) = scope else { return None };
            let (name, x) = (text("ideology")?.to_string(), number("value")?);
            shift_share(&mut state.countries.get_mut(tag)?.upper_house, name, x, 1.0);
            Some(())
        })(),
        "ideology" | "dominant_issue" => (|| {
            let (name, x) = (text("value")?, number("factor")?);
            let n = if key == "ideology" { state.ideologies.iter().position(|i| i == name)? } else { state.issues.iter().position(|(o, _)| o == name)? } as u32 + 1;
            let pop = state.pop_mut(scope)?;
            shift_share(if key == "ideology" { &mut pop.ideology } else { &mut pop.issues }, n, x, 100.0);
            Some(())
        })(),
        "scaled_militancy" | "scaled_consciousness" => (|| {
            let factor = number("factor")?;
            let by_ideology = text("ideology").map(|name| state.ideologies.iter().position(|i| i == name));
            let by_issue = text("issue").map(|name| state.issues.iter().position(|(o, _)| o == name));
            let (con_max, mil_max) = (rules.consciousness_max, rules.militancy_max);
            // Rule: "no real pop -> 0". Inferred: outside a pop scope (the base events write it
            // straight in an option) it is applied and changes nothing.
            let Some(pop) = state.pop_mut(scope) else { return Some(()) };
            let share = match (by_ideology, by_issue) {
                // Inferred: a name that is no ideology or issue of the install (the base events
                // use `issue = jingoism`, which no file defines) gives the pop no support: adds 0.
                (Some(i), _) => i.and_then(|i| pop.ideology.get(&(i as u32 + 1)).copied()).unwrap_or(0.0),
                (None, Some(i)) => i.and_then(|i| pop.issues.get(&(i as u32 + 1)).copied()).unwrap_or(0.0),
                (None, None) => return None,
            } / 100.0;
            if key == "scaled_militancy" {
                pop.militancy = (pop.militancy + factor * share).clamp(0.0, mil_max);
            } else {
                pop.consciousness = (pop.consciousness + factor * share).clamp(0.0, con_max);
            }
            Some(())
        })(),
        "country_event" | "province_event" => (|| {
            let target = match (key, scope) {
                ("country_event", _) => Scope::Country(country_of(state, scope)?.to_string()),
                ("province_event", Scope::Province(_)) => scope.clone(),
                _ => return None,
            };
            let fires = days_from_now(state, cal, number("days").unwrap_or(0.0))?;
            state.pending_events.push(PendingEvent { id: number("id")? as u64, target, from: state.this.clone().unwrap_or_else(|| scope.clone()), fires });
            Some(())
        })(),
        _ => return None,
    };
    Some(done.is_some())
}

fn run_block_entry(state: &mut State, cal: &Calendar, rules: &EffectRules, key: &str, inner: &Block, scope: &Scope, report: &mut Report) {
    // `country_event = 123` style entries never reach here; a pop type scope may share its name
    // with nothing else, so effects are tried first.
    if let Some(applied) = run_block_effect(state, cal, rules, key, inner, scope) {
        if let (true, Some(name)) = (applied && key.ends_with("_modifier"), inner.get("name").and_then(Value::as_str)) {
            *report.modifiers_added.entry(name.to_string()).or_default() += 1;
        }
        return if applied { report.did(key) } else { report.skip(key, scope) };
    }
    // The port's own dice (FLD_SCRIPT_SEED).
    if key == "random" {
        let Some(chance) = inner.get("chance").and_then(Value::as_f64) else { return report.skip(key, scope) };
        report.did(key);
        // The chance is a whole number of percent: what is written after the point is dropped
        // (`chance = 0.5` never happens).
        if (state.roll(100) as f64) < chance.trunc() {
            run_effects(state, cal, rules, inner, scope, report);
        }
        return;
    }
    if key == "random_list" {
        let choices: Vec<(u64, &Block)> = inner.items.iter().filter_map(|i| Some((i.key.as_deref()?.parse().ok()?, i.value.as_block()?))).collect();
        let total: u64 = choices.iter().map(|c| c.0).sum();
        if total == 0 {
            return report.skip(key, scope);
        }
        report.did(key);
        // A number below the total; each entry takes its weight off it and the first one to
        // bring it to zero or below is chosen. The first entry so wins one case more than its
        // weight and the last one case fewer, as in the original.
        let mut at = state.roll(total as usize) as i64;
        for (weight, block) in choices {
            at -= weight as i64;
            if at <= 0 {
                return run_effects(state, cal, rules, block, scope, report);
            }
        }
        return;
    }
    // Scopes. A `limit` inside one keeps only the targets that meet its conditions.
    let limit = inner.get("limit").and_then(Value::as_block);
    let me = match scope {
        Scope::Country(tag) => Some(tag.as_str()),
        _ => None,
    };
    let others = |list: Vec<String>| -> Vec<Scope> { list.into_iter().filter(|t| Some(t.as_str()) != me).map(Scope::Country).collect() };
    let provinces = |ids: Vec<u16>| -> Vec<Scope> { ids.into_iter().map(Scope::Province).collect() };
    let strata = key.strip_suffix("_strata");
    let targets: Option<Vec<Scope>> = match (key, scope) {
        ("any_country" | "random_country", _) => Some(others(state.existing().into_iter().collect())),
        ("any_greater_power", _) => Some(others(state.great_powers.clone())),
        ("any_neighbor_country", Scope::Country(tag)) => Some(others(state.neighbour_countries(tag).into_iter().collect())),
        ("capital_scope", Scope::Country(tag)) => state.countries.get(tag).and_then(|c| c.capital).filter(|id| state.provinces.contains_key(id)).map(|id| vec![Scope::Province(id)]),
        ("any_owned" | "random_owned" | "random_province", Scope::Country(_) | Scope::State(..)) => state.provinces_of(scope).map(provinces),
        ("all_core", Scope::Country(tag)) => Some(provinces(state.provinces.iter().filter(|(_, p)| p.cores.contains(tag)).map(|(id, _)| *id).collect())),
        ("any_state" | "random_state", Scope::Country(tag)) => Some(state.states_of(tag)),
        ("state_scope", Scope::Province(id) | Scope::Pop(id, _)) => state.provinces.get(id).and_then(|p| Some(vec![Scope::State(p.region?, p.owner.clone()?)])),
        ("any_pop" | "random_pop", _) => state.pops_of(scope, |_| true),
        ("owner" | "country", Scope::Province(_) | Scope::Pop(..) | Scope::State(..)) => country_of(state, scope).map(|t| vec![Scope::Country(t.to_string())]),
        // A scope word whose target does not exist does nothing.
        ("sphere_owner", Scope::Country(tag)) => Some(state.sphere_leader(tag).map(|t| Scope::Country(t.to_string())).into_iter().collect()),
        ("overlord", Scope::Country(tag)) => Some(state.overlord(tag).map(|t| Scope::Country(t.to_string())).into_iter().collect()),
        ("cultural_union", Scope::Country(tag)) => Some(state.union_of(state.countries.get(tag).and_then(|c| c.primary_culture.as_deref())).map(|t| Scope::Country(t.to_string())).into_iter().collect()),
        ("cultural_union", Scope::Pop(id, i)) => Some(state.union_of(state.provinces.get(id).and_then(|p| p.pops.get(*i)).map(|q| q.culture.as_str())).map(|t| Scope::Country(t.to_string())).into_iter().collect()),
        ("controller", Scope::Province(id)) => state.provinces.get(id).and_then(|p| p.controller.clone()).map(|t| vec![Scope::Country(t)]),
        ("location", Scope::Pop(id, _)) => Some(vec![Scope::Province(*id)]),
        ("THIS", _) => state.this.clone().map(|s| vec![s]),
        ("FROM", _) => state.from.clone().map(|s| vec![s]),
        _ if state.strata.contains_key(key) => state.pops_of(scope, |p| p.pop_type == key),
        _ if strata.is_some() && !state.strata.is_empty() => state.pops_of(scope, |p| state.strata.get(&p.pop_type).map(String::as_str) == strata),
        _ if is_tag(key) && state.countries.contains_key(key) => Some(vec![Scope::Country(key.to_string())]),
        _ => match state.regions.iter().position(|r| r == key) {
            Some(region) => Some(provinces(state.in_region(region as u16))),
            None => key.parse::<u16>().ok().filter(|id| state.provinces.contains_key(id)).map(|id| vec![Scope::Province(id)]),
        },
    };
    let Some(mut targets) = targets else { return report.skip(key, scope) };
    if let Some(limit) = limit {
        let mut kept = Vec::with_capacity(targets.len());
        for t in targets {
            match conditions_hold(state, limit, &t) {
                Ok(true) => kept.push(t),
                Ok(false) => {}
                // A condition the engine does not know: skip the whole scope rather than guess.
                Err(unknown) => return report.skip(&format!("{key} with limit on {unknown}"), scope),
            }
        }
        targets = kept;
    }
    // A random scope reaches one of the targets that passed the limit.
    if key.starts_with("random_") && targets.len() > 1 {
        let pick = state.roll(targets.len());
        targets = vec![targets.swap_remove(pick)];
    }
    report.did(key);
    for t in &targets {
        // `any_owned` asks the limit a second time just before it acts on a province, so an
        // effect that makes later provinces fail the limit stops them.
        if key == "any_owned" && limit.is_some_and(|limit| conditions_hold(state, limit, t) != Ok(true)) {
            continue;
        }
        run_effects(state, cal, rules, inner, t, report);
    }
}

// ---------------------------------------------------------------- conditions

/// The country a scope belongs to: itself, the owner of the state or province, or the owner of
/// the province a pop lives in.
pub(crate) fn country_of<'a>(state: &'a State, scope: &'a Scope) -> Option<&'a str> {
    match scope {
        Scope::Country(tag) | Scope::State(_, tag) => Some(tag),
        Scope::Province(id) | Scope::Pop(id, _) => state.provinces.get(id).and_then(|p| p.owner.as_deref()),
    }
}

pub(crate) fn yes(value: &Value) -> Option<bool> {
    match value.as_str()? {
        "yes" => Some(true),
        "no" => Some(false),
        _ => None,
    }
}

/// Whether every condition of `block` holds for `scope` (the entries of a block are joined by
/// "and"; `OR`, `AND` and `NOT` blocks combine their own entries). Err names the first
/// condition the engine does not know.
///
/// A numeric condition such as `militancy = 5` means "at least 5", as in the game's scripts.
pub fn conditions_hold(state: &State, block: &Block, scope: &Scope) -> Result<bool, String> {
    for item in &block.items {
        let Some(key) = item.key.as_deref() else { continue };
        // The weight of a `modifier = { factor = x <conditions> }` block is not a condition.
        if key == "factor" {
            continue;
        }
        if !condition(state, key, &item.value, scope)? {
            return Ok(false);
        }
    }
    Ok(true)
}

/// The culture with the most people in a province.
fn majority_culture(p: &ProvinceState) -> Option<&str> {
    let mut sizes: BTreeMap<&str, f64> = BTreeMap::new();
    for pop in &p.pops {
        *sizes.entry(pop.culture.as_str()).or_default() += pop.size;
    }
    sizes.into_iter().max_by(|a, b| a.1.total_cmp(&b.1)).map(|(c, _)| c)
}

/// The controller of a province that rebels hold.
const REBELS: &str = "REB";
// rules file: FLD_HAS_BUILDING_NO_INPUT_DAYS - days in a row without inputs from which a factory no longer counts for `has_building` (a fixed number of the original, read in its routine)
const HAS_BUILDING_NO_INPUT_DAYS: u32 = 11;

/// The culture, religion, ideology or issue with the most people behind it in these provinces.
fn majority<'a>(state: &'a State, ids: &[u16], what: &str) -> Option<&'a str> {
    let mut sizes: BTreeMap<&str, f64> = BTreeMap::new();
    for q in ids.iter().filter_map(|id| state.provinces.get(id)).flat_map(|p| &p.pops) {
        match what {
            "culture" => *sizes.entry(q.culture.as_str()).or_default() += q.size,
            "religion" => *sizes.entry(q.religion.as_str()).or_default() += q.size,
            _ => {
                for (n, share) in if what == "ideology" { &q.ideology } else { &q.issues } {
                    let name = n.checked_sub(1).and_then(|i| if what == "ideology" { state.ideologies.get(i as usize).map(String::as_str) } else { state.issues.get(i as usize).map(|x| x.0.as_str()) });
                    if let Some(name) = name {
                        *sizes.entry(name).or_default() += q.size * share;
                    }
                }
            }
        }
    }
    sizes.into_iter().max_by(|a, b| a.1.total_cmp(&b.1)).map(|(name, _)| name)
}

/// Size-weighted average of a pop value over the pops a scope covers.
pub(crate) fn average(state: &State, scope: &Scope, value: impl Fn(&PopState) -> f64) -> Option<f64> {
    let (mut sum, mut people) = (0.0, 0.0);
    for id in state.provinces_of(scope)? {
        for p in &state.provinces[&id].pops {
            sum += value(p) * p.size;
            people += p.size;
        }
    }
    Some(if people > 0.0 { sum / people } else { 0.0 })
}

pub(crate) fn condition(state: &State, key: &str, value: &Value, scope: &Scope) -> Result<bool, String> {
    let unknown = || Err(key.to_string());
    // Most keys are lower case already: no copy for them.
    let owned;
    let lower: &str = if key.bytes().any(|b| b.is_ascii_uppercase()) {
        owned = key.to_ascii_lowercase();
        &owned
    } else {
        key
    };
    if let Value::Block(inner) = value {
        return match lower {
            "and" => conditions_hold(state, inner, scope),
            "not" => Ok(!any_holds(state, inner, scope)?),
            "or" => any_holds(state, inner, scope),
            "owner" | "country" => match (scope, country_of(state, scope)) {
                (Scope::Province(_) | Scope::Pop(..) | Scope::State(..), Some(tag)) => conditions_hold(state, inner, &Scope::Country(tag.to_string())),
                _ => Ok(false),
            },
            "capital_scope" => match country_of(state, scope).and_then(|t| state.countries.get(t)).and_then(|c| c.capital) {
                Some(id) => conditions_hold(state, inner, &Scope::Province(id)),
                None => Ok(false),
            },
            "location" => match scope {
                Scope::Pop(id, _) => conditions_hold(state, inner, &Scope::Province(*id)),
                _ => unknown(),
            },
            "state_scope" => match scope {
                Scope::Province(id) | Scope::Pop(id, _) => match state.provinces.get(id).and_then(|p| Some(Scope::State(p.region?, p.owner.clone()?))) {
                    Some(s) => conditions_hold(state, inner, &s),
                    None => Ok(false),
                },
                _ => unknown(),
            },
            "relation" => {
                let who = inner.get("who").and_then(Value::as_str).and_then(|w| state.tag_named(w));
                match (who, inner.get("value").and_then(Value::as_f64), country_of(state, scope).and_then(|t| state.countries.get(t))) {
                    (Some(who), Some(x), Some(c)) => Ok(c.relations.get(&who).copied().unwrap_or(0.0) >= x),
                    _ => unknown(),
                }
            }
            // Stand-in while the economy is off: factory jobs exist where the state has a
            // factory; every other kind of work is always available.
            "work_available" => match (inner.get("worker").and_then(Value::as_str), scope) {
                (Some("craftsmen" | "clerks"), Scope::Province(id) | Scope::Pop(id, _)) => Ok(state.provinces.get(id).is_some_and(|p| p.has_factory)),
                (Some(_), _) => Ok(true),
                (None, _) => unknown(),
            },
            "upper_house" => match (inner.get("ideology").and_then(Value::as_str), inner.get("value").and_then(Value::as_f64), country_of(state, scope).and_then(|t| state.countries.get(t))) {
                (Some(name), Some(x), Some(c)) => Ok(c.upper_house.get(name).copied().unwrap_or(0.0) >= x),
                _ => unknown(),
            },
            "sphere_owner" => match country_of(state, scope).and_then(|t| state.sphere_leader(t)) {
                Some(leader) => conditions_hold(state, inner, &Scope::Country(leader.to_string())),
                None => Ok(false),
            },
            // The union country of the primary culture's group; the overlord of a vassal or
            // substate. Without one the scope is not valid: false.
            "cultural_union" | "overlord" => {
                let target = match (scope, lower) {
                    (Scope::Country(t), "cultural_union") => state.union_of(state.countries.get(t).and_then(|c| c.primary_culture.as_deref())),
                    // A pop: the union country of its own culture's group.
                    (Scope::Pop(id, i), "cultural_union") => state.union_of(state.provinces.get(id).and_then(|p| p.pops.get(*i)).map(|q| q.culture.as_str())),
                    (Scope::Country(t), _) => state.overlord(t),
                    _ => None,
                };
                match target {
                    Some(t) => conditions_hold(state, inner, &Scope::Country(t.to_string())),
                    None => Ok(false),
                }
            }
            // The share of one pop type without work is at least `value`: in the province, else
            // the state, else the country; for a pop, in its state. Nobody of the type = 0.
            "unemployment_by_type" => {
                let ids = match scope {
                    Scope::Pop(id, _) => Some(state.state_of(*id)),
                    _ => state.provinces_of(scope),
                };
                match (inner.get("type").and_then(Value::as_str), inner.get("value").and_then(Value::as_f64), ids) {
                    (Some(kind), Some(x), Some(ids)) => Ok(state.unemployment(&ids, Some(kind)) >= x),
                    _ => unknown(),
                }
            }
            // Influence points on `who` are at least `value` (a country's own figure; asked
            // anywhere else it is "0 at least value"). The port keeps the save's figures.
            "diplomatic_influence" => {
                let who = inner.get("who").and_then(Value::as_str).and_then(|w| state.tag_named(w));
                match (who, inner.get("value").and_then(Value::as_f64)) {
                    (Some(who), Some(x)) => Ok(match scope {
                        Scope::Country(t) => state.countries.get(t).and_then(|c| c.influence.get(&who)).copied().unwrap_or(0.0),
                        _ => 0.0,
                    } >= x),
                    _ => unknown(),
                }
            }
            // Asked of a country, `any_pop` is false: it looks at a province or a state only.
            "any_pop" if matches!(scope, Scope::Country(_)) => Ok(false),
            "any_neighbor_country" | "any_greater_power" | "any_owned_province" | "any_state" | "any_pop" | "any_core" | "all_core" | "any_neighbor_province" => {
                let tag = country_of(state, scope);
                let countries = |list: Vec<String>| -> Vec<Scope> { list.into_iter().map(Scope::Country).collect() };
                let provinces = |ids: Vec<u16>| -> Vec<Scope> { ids.into_iter().map(Scope::Province).collect() };
                let targets = match (lower, scope) {
                    ("any_neighbor_country", Scope::Country(t)) => Some(countries(state.neighbour_countries(t).into_iter().collect())),
                    ("any_greater_power", _) => Some(countries(state.great_powers.clone())),
                    ("any_owned_province", Scope::Country(_) | Scope::State(..)) => state.provinces_of(scope).map(provinces),
                    ("any_state", Scope::Country(t)) => Some(state.states_of(t)),
                    ("any_pop", _) => state.pops_of(scope, |_| true),
                    ("any_core" | "all_core", Scope::Country(_)) => tag.map(|t| provinces(state.provinces.iter().filter(|(_, p)| p.cores.iter().any(|c| c == t)).map(|(id, _)| *id).collect())),
                    // In a province: the countries with a core on it.
                    ("any_core" | "all_core", Scope::Province(id)) => state.provinces.get(id).map(|p| countries(p.cores.clone())),
                    ("any_neighbor_province", Scope::Province(id)) => state.neighbours.get(id).map(|n| provinces(n.iter().copied().collect())),
                    _ => None,
                };
                let Some(targets) = targets else { return unknown() };
                let (mut any, mut all) = (false, true);
                for t in &targets {
                    let holds = conditions_hold(state, inner, t)?;
                    any |= holds;
                    all &= holds;
                }
                // A country without cores fails `all_core`; a province without cores passes it.
                Ok(if lower == "all_core" { all && (!targets.is_empty() || matches!(scope, Scope::Province(_))) } else { any })
            }
            "this" | "from" => match if lower == "this" { &state.this } else { &state.from } {
                Some(s) => conditions_hold(state, inner, s),
                None => unknown(),
            },
            _ if is_tag(key) => conditions_hold(state, inner, &Scope::Country(key.to_string())),
            _ => match (state.regions.iter().position(|r| r == key), key.parse::<u16>()) {
                // A region name: true when ANY province of the region passes.
                (Some(region), _) => {
                    for id in state.in_region(region as u16) {
                        if conditions_hold(state, inner, &Scope::Province(id))? {
                            return Ok(true);
                        }
                    }
                    Ok(false)
                }
                (None, Ok(id)) => conditions_hold(state, inner, &Scope::Province(id)),
                (None, Err(_)) => unknown(),
            },
        };
    }
    // THIS / FROM as a value stand for a country, when the caller said which.
    let named: Option<std::borrow::Cow<str>> = match value.as_str() {
        Some(t @ ("THIS" | "FROM")) => match state.tag_named(t) {
            Some(tag) => Some(tag.into()),
            None => return unknown(),
        },
        other => other.map(Into::into),
    };
    let text = named.as_deref().unwrap_or("");
    let number = value.as_f64();
    let tag = country_of(state, scope);
    let country = tag.and_then(|t| state.countries.get(t));
    let province = match scope {
        Scope::Province(id) | Scope::Pop(id, _) => state.provinces.get(id),
        _ => None,
    };
    let pop = match scope {
        Scope::Pop(id, i) => state.provinces.get(id).and_then(|p| p.pops.get(*i)),
        _ => None,
    };
    // For a state: whether any of its provinces passes.
    let in_state = |test: &dyn Fn(&ProvinceState) -> bool| match scope {
        Scope::State(..) | Scope::Country(_) => state.provinces_of(scope).map(|ids| ids.iter().any(|id| test(&state.provinces[id]))),
        _ => province.map(test),
    };
    let group_of = |culture: Option<&str>| culture.and_then(|c| state.culture_groups.get(c)).map(String::as_str);
    // The country a condition "of a country" looks at: the scope's own, or a pop's. A province
    // or a state has none; such a condition then sees nothing (zero, or false).
    let own = match scope {
        Scope::Country(t) => Some(t.as_str()),
        Scope::Pop(..) => tag,
        _ => None,
    };
    // The amount to reach: the written number, or with THIS / FROM the same figure of that country.
    let other = matches!(value.as_str(), Some("THIS" | "FROM")).then_some(text);
    let amount = |of: &dyn Fn(&str) -> f64| number.or_else(|| other.map(of));
    // The provinces a condition about "the people here" covers; a pop stands for its province.
    let here = || match scope {
        Scope::Pop(id, _) => Some(vec![*id]),
        _ => state.provinces_of(scope),
    };
    let known = match lower {
        // A province or a state has no country of its own: false there.
        "tag" => Some(matches!(scope, Scope::Country(_) | Scope::Pop(..)) && tag == Some(text)),
        // The option in force in that reform's group is the one just before the named one
        // (political and social reforms only; anything else, or no country, is false).
        "is_next_reform" => Some(country.is_some_and(|c| {
            state.issues.iter().position(|(o, _)| o == text).filter(|i| *i > 0).is_some_and(|i| {
                let (before, this) = (&state.issues[i - 1], &state.issues[i]);
                before.1 == this.1 && state.issue_categories.get(i).is_some_and(|k| k == "political_reforms" || k == "social_reforms") && c.reforms.get(&this.1) == Some(&before.0)
            })
        })),
        // A pop's support for the ideology of the ruling party, as a share (0..1), is at least
        // the number. No pop or no ruling party: false.
        "agree_with_ruling_party" => number.map(|x| {
            pop.zip(country).and_then(|(q, c)| {
                let name = state.party_ideologies.get(c.ruling_party?.checked_sub(1)? as usize)?;
                let n = state.ideologies.iter().position(|i| i == name)? as u32 + 1;
                Some(q.ideology.get(&n).copied().unwrap_or(0.0) / 100.0 >= x)
            }) == Some(true)
        }),
        // Never true of a country. A province: its farm or mine makes the good. A state: one
        // of its factories makes it, or the farm or mine of one of its provinces.
        "produces" => Some(match scope {
            Scope::Country(_) => false,
            Scope::State(r, t) => state.factories.iter().any(|f| f.owner == *t && f.region == Some(*r) && state.factory_types.get(&f.kind).is_some_and(|x| x.0 == text)) || in_state(&|p| p.trade_goods.as_deref() == Some(text)) == Some(true),
            _ => province.is_some_and(|p| p.trade_goods.as_deref() == Some(text)),
        }),
        // A state: it has a factory of that type that is built, open and not out of inputs for
        // 11 days. A province: the same of its state; for a fort, naval base or railroad its
        // own level. Never true of a country. `factory` = any factory type (the port's reading).
        "has_building" => {
            let any_type = text == "factory";
            let running = |owner: &str, region: u16| state.factories.iter().any(|f| f.owner == owner && f.region == Some(region) && (any_type || f.kind == text) && f.level > 0.0 && f.no_input_days < HAS_BUILDING_NO_INPUT_DAYS && !f.closed);
            let factory = any_type || state.factory_types.contains_key(text) || state.factories.iter().any(|f| f.kind == text);
            Some(match scope {
                Scope::Country(_) => false,
                Scope::State(r, t) => factory && running(t, *r),
                _ => province.is_some_and(|p| if factory { p.owner.as_deref().zip(p.region).is_some_and(|(o, r)| running(o, r)) } else { p.buildings.get(text).is_some_and(|l| *l > 0.0) }),
            })
        }
        // The share of the people without work is at least the number: of the country, state or
        // province; for a pop, of the people of its own type in its province (the port cannot
        // tell one pop's workers from another's).
        "unemployment" => number.and_then(|x| {
            Some(match scope {
                Scope::Pop(id, _) => state.unemployment(&[*id], pop.map(|q| q.pop_type.as_str())),
                _ => state.unemployment(&state.provinces_of(scope)?, None),
            } >= x)
        }),
        // Infamy as a share of BADBOY_LIMIT.
        "badboy" if state.badboy_limit > 0.0 => number.map(|x| own.and_then(|t| state.countries.get(t)).map_or(0.0, |c| c.badboy / state.badboy_limit) >= x),
        // Provinces of the country held by rebels (controller `REB`).
        "num_of_revolts" => number.map(|x| own.map_or(0, |t| state.owned(t).iter().filter(|id| state.provinces[*id].controller.as_deref() == Some(REBELS)).count()) as f64 >= x),
        "revolt_percentage" => number.map(|x| {
            own.map(|t| state.owned(t)).filter(|ids| !ids.is_empty()).is_some_and(|ids| ids.iter().filter(|id| state.provinces[*id].controller.as_deref() == Some(REBELS)).count() as f64 / ids.len() as f64 >= x)
        }),
        // The share of the country's core provinces that somebody else owns.
        "lost_national" => number.map(|x| {
            let (mut cores, mut lost) = (0.0, 0.0);
            for p in state.provinces.values().filter(|p| own.is_some_and(|t| p.cores.iter().any(|c| c == t))) {
                cores += 1.0;
                lost += f64::from(p.owner.as_deref() != own);
            }
            (if cores > 0.0 { lost / cores } else { 0.0 }) >= x
        }),
        // Rank n or better. Not of a country: false. Unknown until a ranking was made.
        "rank" => number.and_then(|x| match own {
            Some(t) => state.rank_of(t).map(|r| r as f64 <= x),
            None => Some(false),
        }),
        // Civilised, not a great power, and ranked no worse than COLONIAL_RANK.
        "is_secondary_power" if state.colonial_rank > 0.0 => yes(value).and_then(|want| {
            let (t, c) = (tag?, country?);
            if !c.civilized || state.great_powers.iter().any(|g| g == t) {
                return Some(!want);
            }
            // Inferred: with no ranking made yet the rank test counts as failed.
            Some(state.rank_of(t).is_some_and(|r| r as f64 <= state.colonial_rank) == want)
        }),
        "industrial_score" => amount(&|t| state.industrial_score(t)).map(|x| matches!(scope, Scope::Country(_)) && tag.is_some_and(|t| state.industrial_score(t) >= x)),
        // Yesterday's military score (`rank::rank_day`), against a number or another country's.
        "military_score" => amount(&|t| state.countries.get(t).map_or(0.0, |c| c.military_score)).map(|x| country.is_some_and(|c| c.military_score >= x)),
        "total_score" => amount(&|t| state.countries.get(t).map_or(0.0, |c| c.total_score)).map(|x| country.is_some_and(|c| c.total_score >= x)),
        // The port has no crisis: a country is in none (also asked with `no`), and none exists.
        "involved_in_crisis" => yes(value).map(|_| false),
        "crisis_exist" => yes(value).map(|want| !want),
        // No truces and no war-goal justifications in the port.
        "truce_with" if is_tag(text) => Some(false),
        "constructing_cb_progress" => number.map(|_| false),
        // The port knows of no lost war; outside a country scope the answer is false either way.
        "has_recently_lost_war" => yes(value).map(|want| matches!(scope, Scope::Country(_)) && !want),
        // The same thing as `crime_fighting`: the administration slider.
        "administration_spending" => number.and_then(|x| country.and_then(|c| c.spending.get("crime_fighting")).map(|s| *s >= x)),
        // More than one culture lives here (province, state or country; a pop: its province).
        "has_national_minority" => yes(value).and_then(|want| here().map(|ids| (ids.iter().flat_map(|id| &state.provinces[id].pops).map(|q| q.culture.as_str()).collect::<BTreeSet<_>>().len() > 1) == want)),
        // People live here whose culture the owner neither has nor accepts.
        "minorities" => yes(value).and_then(|want| {
            let (c, ids) = (country?, here()?);
            Some(ids.iter().flat_map(|id| &state.provinces[id].pops).any(|q| q.size > 0.0 && c.primary_culture.as_deref() != Some(q.culture.as_str()) && !c.accepted_cultures.contains(&q.culture)) == want)
        }),
        // Another continent than the owner's capital and no way there by land. Only a province
        // (or a pop's province) can be overseas.
        "is_overseas" => yes(value).map(|want| match scope {
            Scope::Province(id) | Scope::Pop(id, _) => province.zip(country).and_then(|(p, c)| {
                let capital = c.capital?;
                let joined = state.land_mass.get(id).zip(state.land_mass.get(&capital)).is_some_and(|(a, b)| a == b);
                Some((p.continent != state.provinces.get(&capital)?.continent && !joined) == want)
            }) == Some(true),
            _ => false,
        }),
        "controlled_by_rebels" => yes(value).map(|want| (province.is_some_and(|p| p.controller.as_deref() == Some(REBELS))) == want),
        // The written yes / no is not looked at.
        "has_empty_adjacent_province" => Some(match scope {
            Scope::Province(id) | Scope::Pop(id, _) => state.neighbours.get(id).is_some_and(|n| n.iter().any(|x| state.provinces.get(x).is_some_and(|q| q.owner.is_none()))),
            _ => false,
        }),
        "empty" => yes(value).map(|want| match scope {
            Scope::Province(_) => province.is_some_and(|p| p.owner.is_none() == want),
            _ => false,
        }),
        // The largest culture, religion, ideology or issue among the people of the country,
        // state or province. A pop alone: false, except that it has a largest ideology of its own.
        "pop_majority_culture" | "pop_majority_religion" | "pop_majority_ideology" | "pop_majority_issue" => {
            let what = &lower["pop_majority_".len()..];
            Some(match (scope, pop) {
                (Scope::Pop(..), Some(q)) => what == "ideology" && q.ideology.iter().max_by(|a, b| a.1.total_cmp(b.1)).and_then(|(n, _)| state.ideologies.get(n.checked_sub(1)? as usize)).is_some_and(|i| i == text),
                (Scope::Pop(..), None) => false,
                _ => state.provinces_of(scope).is_some_and(|ids| majority(state, &ids, what) == Some(text)),
            })
        }
        // The country is the union country of its own culture group (`yes` / `no`), or of the
        // culture group of the named country (`this_union` = of THIS). Not in the description:
        // the port's reading of the word.
        "is_cultural_union" => {
            let mine = state.union_of(country.and_then(|c| c.primary_culture.as_deref()));
            let of = |t: Option<String>| state.union_of(t.and_then(|t| state.countries.get(&t)).and_then(|c| c.primary_culture.as_deref()));
            match (yes(value), text) {
                (Some(want), _) => tag.map(|t| (mine == Some(t)) == want),
                (None, "this_union") => tag.map(|t| of(state.tag_named("THIS")) == Some(t)),
                (None, t) if is_tag(t) => tag.map(|me| of(Some(t.to_string())) == Some(me)),
                _ => None,
            }
        }
        // The union country of the country's culture group is the named one (`this_union` =
        // the union country of THIS's group).
        "this_culture_union" => {
            let mine = state.union_of(own.and_then(|t| state.countries.get(t)).and_then(|c| c.primary_culture.as_deref()));
            match text {
                "this_union" => Some(mine.is_some() && mine == state.union_of(state.tag_named("THIS").and_then(|t| state.countries.get(&t)).and_then(|c| c.primary_culture.as_deref()))),
                t if is_tag(t) => Some(mine == Some(t)),
                _ => None,
            }
        }
        "substate_of" if is_tag(text) => Some(own.is_some_and(|t| state.substates.iter().any(|(o, v)| o == text && v == t))),
        "exists" => Some(if is_tag(text) { state.existing().contains(text) } else { tag.is_some_and(|t| state.existing().contains(t)) == (text != "no") }),
        "civilized" => yes(value).and_then(|want| country.map(|c| c.civilized == want)),
        "is_greater_power" => yes(value).and_then(|want| tag.map(|t| state.great_powers.iter().any(|g| g == t) == want)),
        "war" => yes(value).and_then(|want| tag.map(|t| !state.enemies(t).is_empty() == want)),
        "war_with" if is_tag(text) => tag.map(|t| state.enemies(t).contains(text)),
        "is_ideology_enabled" if state.ideologies.iter().any(|i| i == text) => Some(state.enabled_ideologies.contains(text) || state.ideology_dates.get(text).is_none_or(|d| state.date >= *d)),
        "is_coastal" => yes(value).and_then(|want| province.map(|p| p.coastal == want)),
        "life_rating" => number.and_then(|x| province.map(|p| p.life_rating >= x)),
        "is_state_religion" => yes(value).and_then(|want| pop.zip(country).map(|(p, c)| (c.religion.as_deref() == Some(p.religion.as_str())) == want)),
        // Stand-in until armies exist: nobody counts as recruited, so only `= 0` holds.
        "recruited_percentage" => number.map(|x| x <= 0.0),
        // The port has no reform movements: their strength is 0.
        "political_movement_strength" | "social_movement_strength" => number.map(|x| x <= 0.0),
        // A core here of a country whose primary culture is the pop's (accepted cultures do
        // not count; read from the original's routine).
        "has_culture_core" => yes(value).and_then(|want| {
            pop.zip(province).map(|(q, p)| p.cores.iter().any(|t| state.countries.get(t).is_some_and(|c| c.primary_culture.as_deref() == Some(q.culture.as_str()))) == want)
        }),
        // A pop with at least 6.25 in cash (its money as a save stores it) whose country has a
        // factory type unlocked. Read from the original's routine; "a factory type unlocked" is
        // taken as "is civilised" (the port's stand-in). Nothing about the ruling party, free
        // places or the state.
        // rules file: FLD_CAN_BUILD_FACTORY_CASH - 6.25 = 2.5 x 2.5, a fixed number of the original.
        "can_build_factory" => yes(value).and_then(|want| pop.zip(country).map(|(q, c)| (q.money >= 6.25 && c.civilized) == want)),
        "has_factories" => yes(value).and_then(|want| in_state(&|p| p.has_factory).map(|has| has == want)),
        "education_spending" | "crime_fighting" | "social_spending" | "military_spending" => number.and_then(|x| country.and_then(|c| c.spending.get(lower)).map(|s| *s >= x)),
        "can_nationalize" => yes(value).and_then(|want| country.map(|c| !c.foreign_investors.is_empty() == want)),
        "ai" => yes(value).and_then(|want| country.map(|c| !c.human == want)),
        "invention" => state.inventions.iter().position(|i| i == text).and_then(|n| country.map(|c| c.inventions.contains(&(n as u32 + 1)))),
        "ruling_party_ideology" => country.and_then(|c| Some(state.party_ideologies.get(c.ruling_party?.checked_sub(1)? as usize)? == text)),
        "nationalvalue" => country.map(|c| c.national_value.as_deref() == Some(text)),
        "tech_school" => country.map(|c| c.tech_school.as_deref() == Some(text)),
        "part_of_sphere" => yes(value).and_then(|want| tag.map(|t| state.sphere_leader(t).is_some() == want)),
        "in_sphere" if is_tag(text) => tag.map(|t| state.countries.get(text).is_some_and(|c| c.sphere.contains(t))),
        "is_sphere_leader_of" if is_tag(text) => country.map(|c| c.sphere.contains(text)),
        "is_vassal" => yes(value).and_then(|want| tag.map(|t| state.vassals.iter().any(|(_, v)| v == t) == want)),
        "is_independant" => yes(value).and_then(|want| tag.map(|t| state.vassals.iter().all(|(_, v)| v != t) == want)),
        "military_access" if is_tag(text) => country.map(|c| c.access.contains(text)),
        "is_our_vassal" if is_tag(text) => tag.map(|t| state.vassals.iter().any(|(o, v)| o == t && v == text)),
        "vassal_of" if is_tag(text) => tag.map(|t| state.vassals.iter().chain(&state.substates).any(|(o, v)| o == text && v == t)),
        "alliance_with" if is_tag(text) => tag.map(|t| state.alliances.iter().any(|(a, b)| (a == t && b == text) || (a == text && b == t))),
        "colonial_nation" => yes(value).and_then(|want| tag.map(|t| state.owned(t).iter().any(|id| state.provinces[id].colonial != 0) == want)),
        "capital" => number.and_then(|x| country.map(|c| c.capital == Some(x as u16))),
        "is_capital" => yes(value).and_then(|want| match scope {
            Scope::Province(id) => country.map(|c| (c.capital == Some(*id)) == want),
            _ => None,
        }),
        "trade_goods" => province.map(|p| p.trade_goods.as_deref() == Some(text)),
        "controlled_by" if text == "owner" => province.map(|p| p.owner.is_some() && p.controller == p.owner),
        "has_country_flag" => country.map(|c| c.flags.contains(text)),
        // `always = yes/no` returns the stored flag.
        "always" => yes(value),
        // Inferred: the port has no immigration and no casus belli under construction, so
        // neither has happened.
        "has_recent_imigration" => number.map(|_| false),
        "constructing_cb_type" => Some(false),
        "has_global_flag" => Some(state.global_flags.contains(text)),
        "has_country_modifier" => country.map(|c| c.modifiers.iter().any(|m| m.name == text)),
        "has_province_modifier" => province.map(|p| p.modifiers.iter().any(|m| m.name == text)),
        "has_province_flag" => province.map(|p| p.flags.contains(text)),
        "government" => country.map(|c| c.government.as_deref() == Some(text)),
        "prestige" => number.and_then(|x| country.map(|c| c.prestige >= x)),
        "plurality" => number.and_then(|x| country.map(|c| c.plurality >= x)),
        "war_exhaustion" => number.and_then(|x| country.map(|c| c.war_exhaustion >= x)),
        "money" => number.and_then(|x| match scope {
            Scope::Country(_) => country.map(|c| c.money >= x),
            _ => pop.map(|p| p.money >= x),
        }),
        "num_of_cities" => amount(&|t| state.owned(t).len() as f64).map(|x| own.map_or(0.0, |t| state.owned(t).len() as f64) >= x),
        "neighbour" if is_tag(text) => tag.map(|t| state.neighbour_countries(t).contains(text)),
        "year" => number.map(|x| state.date.year as f64 >= x),
        "month" => number.map(|x| state.date.month as f64 >= x),
        "province_id" => number.and_then(|x| match scope {
            Scope::Province(id) | Scope::Pop(id, _) => Some(*id as f64 == x),
            _ => None,
        }),
        "continent" => match scope {
            Scope::Country(_) => country.and_then(|c| c.capital).and_then(|id| state.provinces.get(&id)).map(|p| p.continent.as_deref() == Some(text)),
            _ => province.map(|p| p.continent.as_deref() == Some(text)),
        },
        "is_colonial" => yes(value).and_then(|want| in_state(&|p| (p.colonial != 0) == want)),
        "is_core" if is_tag(text) => province.map(|p| p.cores.iter().any(|c| c == text)),
        "owned_by" if is_tag(text) => province.map(|p| p.owner.as_deref() == Some(text)),
        "controlled_by" if is_tag(text) => province.map(|p| p.controller.as_deref() == Some(text)),
        // Days since the controller changed, on the game's 365-day calendar; no change on record:
        // held since before the game began.
        "province_control_days" => number.and_then(|x| province.map(|p| p.controller_since.is_none_or(|d| day_count(state.date) - day_count(d) >= x as i64))),
        "owns" => number.map(|x| state.provinces.get(&(x as u16)).is_some_and(|p| p.owner.as_deref() == tag && p.owner.is_some())),
        "has_pop_culture" => match scope {
            Scope::Pop(..) => pop.map(|p| p.culture == text),
            _ => in_state(&|p| p.pops.iter().any(|q| q.culture == text)),
        },
        // `culture = this` / `religion = this` on a pop: the same as its country's.
        "culture" if text.eq_ignore_ascii_case("this") => pop.zip(country).map(|(p, c)| c.primary_culture.as_deref() == Some(p.culture.as_str())),
        "religion" if text.eq_ignore_ascii_case("this") => pop.zip(country).map(|(p, c)| c.religion.as_deref() == Some(p.religion.as_str())),
        "strata" => pop.map(|p| state.strata.get(&p.pop_type).map(String::as_str) == Some(text)),
        // Share (0..1) of a pop, or of the people in scope, asking for political / social reform:
        // the pops' issue shares for options of those reforms (the port's reading).
        "political_reform_want" | "social_reform_want" => {
            let category = if lower.starts_with("political") { "political_reforms" } else { "social_reforms" };
            let want = |p: &PopState| p.issues.iter().filter(|(n, _)| n.checked_sub(1).and_then(|i| state.issue_categories.get(i as usize)).is_some_and(|c| c == category)).map(|(_, v)| v).sum::<f64>() / 100.0;
            number.and_then(|x| pop.map(want).or_else(|| average(state, scope, want)).map(|a| a >= x))
        }
        // Average militancy or needs of one strata among the people in scope (for a pop: of its
        // province).
        "poor_strata_militancy" | "middle_strata_militancy" | "rich_strata_militancy" | "poor_strata_life_needs" | "middle_strata_life_needs" | "rich_strata_life_needs" | "poor_strata_everyday_needs" | "middle_strata_everyday_needs" | "rich_strata_everyday_needs" | "poor_strata_luxury_needs" | "middle_strata_luxury_needs" | "rich_strata_luxury_needs" => {
            let (strata, what) = lower.split_once("_strata_").unwrap_or(("", ""));
            let ids = match scope {
                Scope::Pop(id, _) => Some(vec![*id]),
                _ => state.provinces_of(scope),
            };
            ids.zip(number).map(|(ids, x)| {
                let (mut sum, mut people) = (0.0, 0.0);
                for q in ids.iter().filter_map(|id| state.provinces.get(id)).flat_map(|p| &p.pops).filter(|q| state.strata.get(&q.pop_type).map_or("poor", String::as_str) == strata) {
                    people += q.size;
                    sum += q.size * match what { "militancy" => q.militancy, "life_needs" => q.life_needs, "everyday_needs" => q.everyday_needs, _ => q.luxury_needs };
                }
                people > 0.0 && sum / people >= x
            })
        }
        // The port does not track revanchism: it is 0.
        "revanchism" => number.map(|x| x <= 0.0),
        "life_needs" => number.and_then(|x| pop.map(|p| p.life_needs >= x)),
        "everyday_needs" => number.and_then(|x| pop.map(|p| p.everyday_needs >= x)),
        "luxury_needs" => number.and_then(|x| pop.map(|p| p.luxury_needs >= x)),
        "culture" => match scope {
            Scope::Pop(..) => pop.map(|p| p.culture == text),
            _ => province.map(|p| majority_culture(p) == Some(text)),
        },
        // The people of the province, state or country; for a pop, of its state.
        "total_pops" => number.and_then(|x| {
            let ids = match scope {
                Scope::Pop(id, _) => Some(state.state_of(*id)),
                _ => state.provinces_of(scope),
            };
            ids.map(|ids| ids.iter().flat_map(|id| &state.provinces[id].pops).map(|p| p.size).sum::<f64>() >= x)
        }),
        // On a pop its own culture's group, otherwise the country's primary culture's.
        "culture_has_union_tag" => yes(value).and_then(|want| {
            let culture = match scope {
                Scope::Pop(..) => pop.map(|p| p.culture.as_str()),
                _ => country.and_then(|c| c.primary_culture.as_deref()),
            };
            group_of(culture).map(|g| state.union_groups.contains(g) == want)
        }),
        "has_pop_religion" => match scope {
            Scope::Pop(..) => pop.map(|p| p.religion == text),
            _ => in_state(&|p| p.pops.iter().any(|q| q.religion == text)),
        },
        "religion" => match scope {
            Scope::Pop(..) => pop.map(|p| p.religion == text),
            _ => country.map(|c| c.religion.as_deref() == Some(text)),
        },
        "has_pop_type" | "pop_type" => match scope {
            Scope::Pop(..) => pop.map(|p| p.pop_type == text),
            _ => in_state(&|p| p.pops.iter().any(|q| q.pop_type == text)),
        },
        "type" => pop.map(|p| p.pop_type == text),
        "is_slave" => yes(value).and_then(|want| match scope {
            Scope::Pop(..) => pop.map(|p| state.slave_types.contains(&p.pop_type) == want),
            _ => in_state(&|p| p.slave_state).map(|slave| slave == want),
        }),
        // On a pop its own culture; on a province the culture with the most people.
        // `= THIS`: the country's primary (or an accepted) culture is THIS pop's culture, or THIS
        // country's primary culture (THIS a pop: the rebel types' spawn chances).
        "is_primary_culture" | "is_accepted_culture" if value.as_str() == Some("THIS") => (|| {
            let wanted = match &state.this {
                Some(Scope::Pop(id, i)) => state.provinces.get(id).and_then(|p| p.pops.get(*i)).map(|q| q.culture.clone()),
                Some(s) => country_of(state, s).and_then(|t| state.countries.get(t)).and_then(|c| c.primary_culture.clone()),
                None => None,
            }?;
            let c = country?;
            Some(if lower == "is_primary_culture" { c.primary_culture.as_deref() == Some(wanted.as_str()) } else { c.accepted_cultures.contains(&wanted) })
        })(),
        "is_primary_culture" | "is_accepted_culture" => yes(value).and_then(|want| {
            let culture = match scope {
                Scope::Pop(..) => pop.map(|p| p.culture.as_str()),
                _ => province.and_then(majority_culture),
            }?;
            let c = country?;
            Some(if lower == "is_primary_culture" { c.primary_culture.as_deref() == Some(culture) } else { c.accepted_cultures.iter().any(|a| a == culture) } == want)
        }),
        "primary_culture" => country.map(|c| c.primary_culture.as_deref() == Some(text)),
        "is_culture_group" if !state.culture_groups.is_empty() => {
            let mine = match scope {
                Scope::Pop(..) => group_of(pop.map(|p| p.culture.as_str())),
                _ => group_of(country.and_then(|c| c.primary_culture.as_deref())),
            };
            // The value is a group name, or a country whose primary culture's group is meant.
            let wanted = if is_tag(text) { group_of(state.countries.get(text).and_then(|c| c.primary_culture.as_deref())) } else { Some(text) };
            Some(mine.is_some() && mine == wanted)
        }
        // On a pop its own value; on a country, state or province the average of its pops.
        "consciousness" | "average_consciousness" => number.and_then(|x| pop.map(|p| p.consciousness).or_else(|| average(state, scope, |p| p.consciousness)).map(|a| a >= x)),
        "militancy" | "average_militancy" => number.and_then(|x| pop.map(|p| p.militancy).or_else(|| average(state, scope, |p| p.militancy)).map(|a| a >= x)),
        "literacy" => number.and_then(|x| pop.map(|p| p.literacy).or_else(|| average(state, scope, |p| p.literacy)).map(|a| a >= x)),
        // A technology: `electricity = 1` asks whether it has been researched.
        tech if state.technologies.contains(tech) => number.and_then(|x| country.map(|c| c.technologies.contains(tech) == (x >= 1.0))),
        // A pop type with a number: at least that share of the people in scope (for a pop: of
        // its province) are of that type. `clergymen = 0.02`.
        kind if number.is_some() && state.strata.contains_key(kind) => {
            let ids = match scope {
                Scope::Pop(id, _) => Some(vec![*id]),
                _ => state.provinces_of(scope),
            };
            ids.zip(number).map(|(ids, x)| {
                let (mut of_kind, mut all) = (0.0, 0.0);
                for q in ids.iter().filter_map(|id| state.provinces.get(id)).flat_map(|p| &p.pops) {
                    all += q.size;
                    if q.pop_type == kind {
                        of_kind += q.size;
                    }
                }
                all > 0.0 && of_kind / all >= x
            })
        }
        // Support for an ideology or issue in percent of the people in scope: `liberal = 10`.
        name if number.is_some() && (state.ideologies.iter().any(|i| i == name) || state.issues.iter().any(|(o, _)| o == name)) => {
            let ideology = state.ideologies.iter().position(|i| i == name).map(|n| n as u32 + 1);
            let issue = state.issues.iter().position(|(o, _)| o == name).map(|n| n as u32 + 1);
            let share = |p: &PopState| match (ideology, issue) {
                (Some(n), _) => p.ideology.get(&n).copied().unwrap_or(0.0),
                (None, Some(n)) => p.issues.get(&n).copied().unwrap_or(0.0),
                (None, None) => 0.0,
            };
            pop.map(share).or_else(|| average(state, scope, share)).zip(number).map(|(a, x)| a >= x)
        }
        // A reform or party issue group: `slavery = no_slavery`.
        group if state.issues.iter().any(|(_, g)| g == group) => country.map(|c| c.reforms.get(group).map(String::as_str) == Some(text)),
        _ => None,
    };
    known.map_or_else(unknown, Ok)
}

/// Every condition of `block` the engine does not know (for a coverage count). Unlike
/// `conditions_hold` this does not stop at the first entry that fails; inside a scope change
/// only the first unknown condition is named.
pub fn unknown_conditions(state: &State, block: &Block, scope: &Scope) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for item in &block.items {
        let Some(key) = item.key.as_deref() else { continue };
        match (&item.value, key.to_ascii_lowercase().as_str()) {
            (Value::Block(inner), "and" | "or" | "not") => out.extend(unknown_conditions(state, inner, scope)),
            _ => {
                if let Err(name) = condition(state, key, &item.value, scope) {
                    out.insert(name);
                }
            }
        }
    }
    out
}

/// True when at least one entry of the block holds.
pub(crate) fn any_holds(state: &State, block: &Block, scope: &Scope) -> Result<bool, String> {
    let mut found = false;
    for item in &block.items {
        let Some(key) = item.key.as_deref() else { continue };
        // Every entry is checked, so an unknown condition is always reported.
        found |= condition(state, key, &item.value, scope)?;
    }
    Ok(found)
}

// ---------------------------------------------------------------- plain effects

/// Effects whose value is a name: flags, removing a modifier, reforms, cores, ownership.
fn run_name_effect(state: &mut State, key: &str, name: &str, scope: &Scope, report: &mut Report) {
    let done = (|| match (key, scope) {
        ("enable_ideology", _) if state.ideologies.iter().any(|i| i == name) => {
            state.enabled_ideologies.insert(name.to_string());
            Some(())
        }
        ("set_global_flag", _) => {
            state.global_flags.insert(name.to_string());
            Some(())
        }
        ("clr_global_flag", _) => {
            state.global_flags.remove(name);
            Some(())
        }
        ("set_country_flag", Scope::Country(tag)) => state.countries.get_mut(tag).map(|c| {
            c.flags.insert(name.to_string());
        }),
        ("clr_country_flag", Scope::Country(tag)) => state.countries.get_mut(tag).map(|c| {
            c.flags.remove(name);
        }),
        ("set_province_flag", Scope::Province(id)) => state.provinces.get_mut(id).map(|p| {
            p.flags.insert(name.to_string());
        }),
        ("clr_province_flag", Scope::Province(id)) => state.provinces.get_mut(id).map(|p| {
            p.flags.remove(name);
        }),
        ("remove_country_modifier", Scope::Country(tag)) => state.countries.get_mut(tag).map(|c| c.modifiers.retain(|m| m.name != name)),
        ("remove_province_modifier", Scope::Province(id)) => state.provinces.get_mut(id).map(|p| p.modifiers.retain(|m| m.name != name)),
        // Inferred: the country's tech school is replaced by the named one.
        ("tech_school", Scope::Country(tag)) => state.countries.get_mut(tag).map(|c| c.tech_school = Some(name.to_string())),
        // Display names only (set the display name, never read).
        ("change_province_name", Scope::Province(id)) => state.provinces.get_mut(id).map(|p| p.name_override = Some(name.trim_matches('"').to_string())),
        ("change_region_name", Scope::Province(_) | Scope::State(..)) => {
            let region = match scope {
                Scope::State(r, _) => Some(*r),
                Scope::Province(id) => state.provinces.get(id).and_then(|p| p.region),
                _ => None,
            }?;
            state.region_names.insert(region, name.trim_matches('"').to_string());
            Some(())
        }
        ("government", Scope::Country(tag)) => state.countries.get_mut(tag).map(|c| c.government = Some(name.to_string())),
        // Built directly, no acceptance test; the relation change and the cost
        // of the diplomacy screen's action are left out.
        ("create_alliance", Scope::Country(tag)) => {
            let other = state.tag_named(name).filter(|o| o != tag)?;
            if !state.alliances.iter().any(|(a, b)| (a == tag && *b == other) || (*a == other && b == tag)) {
                state.alliances.push((tag.clone(), other));
            }
            Some(())
        }
        ("leave_alliance", Scope::Country(tag)) => {
            let other = state.tag_named(name)?;
            state.alliances.retain(|(a, b)| !((a == tag && *b == other) || (*a == other && b == tag)));
            Some(())
        }
        // Inferred: clears the flag in both directions.
        ("end_military_access", Scope::Country(tag)) => {
            let other = state.tag_named(name).filter(|o| o != tag)?;
            for (x, y) in [(tag.clone(), other.clone()), (other, tag.clone())] {
                if let Some(c) = state.countries.get_mut(&x) {
                    c.access.remove(&y);
                }
            }
            Some(())
        }
        ("nationalize", Scope::Country(tag)) if name == "yes" => state.countries.get_mut(tag).map(|c| c.foreign_investors.clear()),
        ("civilized", Scope::Country(tag)) => {
            let want = name == "yes";
            state.countries.get_mut(tag).map(|c| c.civilized = want)
        }
        ("political_reform" | "social_reform" | "economic_reform" | "military_reform", Scope::Country(tag)) => {
            let group = state.issues.iter().find(|(o, _)| o == name)?.1.clone();
            state.countries.get_mut(tag).map(|c| {
                c.reforms.insert(group, name.to_string());
            })
        }
        // `union`: every other culture of the primary culture's group (not in the description;
        // the port's reading of the word, as when a union country is formed).
        ("add_accepted_culture", Scope::Country(tag)) => {
            let names: Vec<String> = match name {
                "union" => {
                    let primary = state.countries.get(tag)?.primary_culture.clone()?;
                    let group = state.culture_groups.get(&primary)?;
                    state.culture_groups.iter().filter(|(c, g)| *g == group && **c != primary).map(|(c, _)| c.clone()).collect()
                }
                _ => vec![name.to_string()],
            };
            state.countries.get_mut(tag).map(|c| {
                for n in names {
                    if !c.accepted_cultures.contains(&n) {
                        c.accepted_cultures.push(n);
                    }
                }
            })
        }
        // Only the player's country: the player goes on as the other tag; the country itself is
        // not turned into it.
        ("change_tag_no_core_switch", Scope::Country(tag)) => {
            let other = state.tag_named(name).or_else(|| is_tag(name).then(|| name.to_string()))?;
            if state.countries.get(tag)?.human && other != *tag {
                state.countries.get_mut(tag)?.human = false;
                state.countries.entry(other).or_default().human = true;
            }
            Some(())
        }
        // Starts an election: the campaign begins today. The port's elections end a fixed time
        // after the last one (`politics`), so the date of the last one is moved back by the
        // government's time between elections. Nothing where the government holds none.
        ("election", Scope::Country(tag)) if name == "yes" => {
            let today = state.date;
            let months = state.countries.get(tag)?.government.as_ref().and_then(|g| state.election_months.get(g)).copied();
            if let Some(months) = months {
                let at = today.year * 12 + i32::from(today.month) - 1 - months;
                state.countries.get_mut(tag)?.last_election = Some(Date::new(at.div_euclid(12), at.rem_euclid(12) as u8 + 1, today.day.min(28)));
            }
            Some(())
        }
        // Every pop of the PROVINCE takes the primary culture of its owner (not only the pop in scope).
        ("assimilate", Scope::Province(id) | Scope::Pop(id, _)) if name == "yes" => {
            let culture = country_of(state, scope).and_then(|t| state.countries.get(t)).and_then(|c| c.primary_culture.clone())?;
            state.provinces.get_mut(id).map(|p| p.pops.iter_mut().for_each(|q| q.culture = culture.clone()))
        }
        // The named country takes the provinces it has cores on from the country in scope
        // (`release_vassal` also makes it a vassal). Naming the country in scope itself with
        // `release_vassal` frees it from its overlord (the port's reading; that routine of the
        // original was not read). Left out: the new country's government, parties and army,
        // rebels as the receiver, and what happens to the giver's capital.
        ("release" | "release_vassal", Scope::Country(tag)) => {
            let other = state.tag_named(name).or_else(|| (is_tag(name) && name != REBELS).then(|| name.to_string()))?;
            if other == *tag {
                if key == "release_vassal" {
                    state.vassals.retain(|(_, v)| *v != other);
                    state.substates.retain(|(_, v)| *v != other);
                }
                return Some(());
            }
            let ids: Vec<u16> = state.provinces.iter().filter(|(_, p)| p.owner.as_deref() == Some(tag.as_str()) && p.cores.contains(&other)).map(|(id, _)| *id).collect();
            for id in &ids {
                state.give_province(*id, &other);
            }
            if key == "release_vassal" && !ids.is_empty() && state.overlord(&other).is_none() {
                state.vassals.push((tag.clone(), other));
            }
            Some(())
        }
        ("primary_culture", Scope::Country(tag)) if !is_tag(name) => state.countries.get_mut(tag).map(|c| c.primary_culture = Some(name.to_string())),
        // A whole pop becomes another type at once (in the original's files only one decision
        // does this: farmers to soldiers; mods and cheat events use it more).
        ("pop_type", Scope::Pop(..)) if state.strata.contains_key(name) => state.pop_mut(scope).map(|p| p.pop_type = name.to_string()),
        // The product changes and, the same day, the people who worked the old farm or mine become
        // the pop type that works the new one (measured: London grain -> coal turned its 179,250
        // farmers into labourers at once, Edinburgh coal -> grain the other way).
        ("trade_goods", Scope::Province(id)) => {
            let worker = state.rgo_workers.get(name).cloned();
            let all: BTreeSet<String> = state.rgo_workers.values().cloned().collect();
            state.provinces.get_mut(id).map(|p| {
                p.trade_goods = Some(name.to_string());
                let Some(worker) = worker else { return };
                for i in 0..p.pops.len() {
                    if p.pops[i].pop_type == worker || !all.contains(&p.pops[i].pop_type) {
                        continue;
                    }
                    let (culture, religion, size) = (p.pops[i].culture.clone(), p.pops[i].religion.clone(), p.pops[i].size);
                    match p.pops.iter().position(|q| q.pop_type == worker && q.culture == culture && q.religion == religion) {
                        Some(j) => {
                            p.pops[j].size += size;
                            p.pops[i].size = 0.0;
                        }
                        None => p.pops[i].pop_type = worker.clone(),
                    }
                }
            })
        }
        ("add_core", Scope::Province(id)) => {
            let tag = state.tag_named(name)?;
            state.provinces.get_mut(id).map(|p| {
                if !p.cores.contains(&tag) {
                    p.cores.push(tag);
                }
            })
        }
        ("remove_core", Scope::Province(id)) => {
            let tag = state.tag_named(name)?;
            state.provinces.get_mut(id).map(|p| p.cores.retain(|c| *c != tag))
        }
        // The giver is THIS (a province scope has no country of its own). The province changes
        // hands only when the giver owns it or is itself the receiver; otherwise nothing. A
        // receiver without land comes into being with it. Left out: rebels as the receiver, the
        // pops' money that the original sets to zero, the giver's armies.
        ("secede_province", Scope::Province(id)) => {
            let tag = state.tag_named(name).or_else(|| (is_tag(name) && name != REBELS).then(|| name.to_string()))?;
            let giver = state.this.as_ref().and_then(|s| country_of(state, s)).map(str::to_string);
            let owner = state.provinces.get(id)?.owner.clone();
            if giver.is_none() || owner == giver || giver.as_deref() == Some(tag.as_str()) {
                state.give_province(*id, &tag);
            }
            Some(())
        }
        // `inherit = X`: the country in scope takes X's land; `annex_to = X`: X takes its land.
        ("inherit" | "annex_to", Scope::Country(tag)) => {
            let other = state.tag_named(name)?;
            let (loser, winner) = if key == "inherit" { (other, tag.clone()) } else { (tag.clone(), other) };
            if loser == winner {
                return None;
            }
            for id in state.owned(&loser) {
                state.give_province(id, &winner);
            }
            // A country without land has no overlord, subjects, allies or sphere (the port's reading).
            state.vassals.retain(|(a, b)| *a != loser && *b != loser);
            state.substates.retain(|(a, b)| *a != loser && *b != loser);
            state.alliances.retain(|(a, b)| *a != loser && *b != loser);
            for c in state.countries.values_mut() {
                c.sphere.remove(&loser);
            }
            if let Some(c) = state.countries.get_mut(&loser) {
                c.sphere.clear();
            }
            Some(())
        }
        _ => None,
    })();
    match done {
        Some(()) => report.did(key),
        None => report.skip(key, scope),
    }
}

fn run_number_effect(state: &mut State, rules: &EffectRules, key: &str, x: f64, scope: &Scope, report: &mut Report) {
    let done = match (key, scope) {
        ("prestige", Scope::Country(tag)) => state.countries.get_mut(tag).map(|c| c.prestige = (c.prestige + x).max(rules.prestige_min)),
        // gain = x times the current prestige (the rule description adds permanent prestige from technology and a
        // modifier on gains, which the port lacks); never below the minimum.
        ("prestige_factor", Scope::Country(tag)) => state.countries.get_mut(tag).map(|c| c.prestige = (c.prestige + x * c.prestige).max(rules.prestige_min)),
        ("treasury" | "money", Scope::Country(tag)) => state.countries.get_mut(tag).map(|c| c.money = (c.money + x).max(rules.treasury_min)),
        // `treasury` in a state or province: the owner's.
        ("treasury", Scope::State(..) | Scope::Province(_)) => country_of(state, scope).map(str::to_string).and_then(|t| state.countries.get_mut(&t)).map(|c| c.money = (c.money + x).max(rules.treasury_min)),
        // x times the country's daily tax income (the port's reading of the figure the original
        // multiplies by: `CountryState::tax_income`, yesterday's taxes).
        ("add_tax_relative_income", Scope::Country(tag)) => state.countries.get_mut(tag).map(|c| c.money = (c.money + x * c.tax_income).max(rules.treasury_min)),
        // In a country scope the value is a province: the country gains / loses its core there.
        ("add_core" | "remove_core", Scope::Country(tag)) => state.provinces.get_mut(&(x as u16)).map(|p| {
            p.cores.retain(|c| c != tag);
            if key == "add_core" {
                p.cores.push(tag.clone());
            }
        }),
        // The pop moves to that province when it lives elsewhere. Its place in the old province
        // is kept, empty, so that the other pops keep their positions.
        ("move_pop", Scope::Pop(id, i)) if state.provinces.contains_key(&(x as u16)) => {
            let to = x as u16;
            if to != *id {
                let moved = state.provinces.get_mut(id).and_then(|p| p.pops.get_mut(*i)).map(|q| {
                    let moved = q.clone();
                    q.size = 0.0;
                    moved
                });
                if let (Some(moved), Some(p)) = (moved.filter(|q| q.size > 0.0), state.provinces.get_mut(&to)) {
                    p.pops.push(moved);
                }
            }
            Some(())
        }
        ("plurality", Scope::Country(tag)) => state.countries.get_mut(tag).map(|c| c.plurality = (c.plurality + x).clamp(0.0, rules.plurality_max)),
        ("war_exhaustion", Scope::Country(tag)) => state.countries.get_mut(tag).map(|c| c.war_exhaustion = (c.war_exhaustion + x).clamp(0.0, rules.war_exhaustion_max)),
        ("badboy", Scope::Country(tag)) => state.countries.get_mut(tag).map(|c| c.badboy = (c.badboy + x).max(rules.badboy_min)),
        ("leadership", Scope::Country(tag)) => state.countries.get_mut(tag).map(|c| c.leadership = (c.leadership + x).max(0.0)),
        ("capital", Scope::Country(tag)) if state.provinces.contains_key(&(x as u16)) => state.countries.get_mut(tag).map(|c| c.capital = Some(x as u16)),
        // Research points worth that many years at the country's present daily rate (the port's
        // reading of the name; used 40 times in the original's events and decisions).
        ("years_of_research", Scope::Country(tag)) => state.countries.get_mut(tag).map(|c| c.research_points = (c.research_points + x * 365.0 * c.research_rate).max(0.0)),
        // `infrastructure = 2`, `fort = 1`, `naval_base = 1`: levels added to the province's
        // building of that kind, up to the building's max_level (the owner's test: no more than 6).
        (kind, Scope::Province(id)) if state.building_kinds.contains_key(kind) => {
            let (name, max) = state.building_kinds[kind].clone();
            state.provinces.get_mut(id).map(|p| {
                let level = p.buildings.entry(name).or_insert(0.0);
                *level = (*level + x).clamp(0.0, max);
            })
        }
        ("life_rating", Scope::Province(id)) => state.provinces.get_mut(id).map(|p| p.life_rating = (p.life_rating + x).max(0.0)),
        ("consciousness", Scope::Pop(..)) => state.pop_mut(scope).map(|p| p.consciousness = (p.consciousness + x).clamp(0.0, rules.consciousness_max)),
        ("militancy", Scope::Pop(..)) => state.pop_mut(scope).map(|p| p.militancy = (p.militancy + x).clamp(0.0, rules.militancy_max)),
        ("literacy", Scope::Pop(..)) => state.pop_mut(scope).map(|p| p.literacy = (p.literacy + x).clamp(0.0, rules.literacy_max)),
        ("money", Scope::Pop(..)) => state.pop_mut(scope).map(|p| p.money = (p.money + x * rules.pop_money_scale).max(0.0)),
        ("reduce_pop", Scope::Pop(..)) if x >= 0.0 => state.pop_mut(scope).map(|p| p.size = (p.size * x).floor().min(rules.pop_size_max)),
        _ => None,
    };
    match done {
        Some(()) => report.did(key),
        None => report.skip(key, scope),
    }
}
