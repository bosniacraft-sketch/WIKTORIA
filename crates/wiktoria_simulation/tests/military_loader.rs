//! The military loader on a made-up save.

use std::collections::HashSet;

use wiktoria_data::date::Date;
use wiktoria_data::save::parse_save;
use wiktoria_simulation::military::battle::CombatKind;
use wiktoria_simulation::military::units::{LeaderKind, UnitDef};
use wiktoria_simulation::military::{MilitaryRules, MilitaryState};

const SAVE: &str = r#"
date="1850.1.1"
player="AAA"
1=
{
	name="PROV_ONE"
	owner="AAA"
	controller="BBB"
	garrison=40.500
	fort=
	{
 2.000 2.000 	}
	last_controller_change="1849.7.11"
	soldiers=
	{
		id=10
		size=9000.000
		alpha=one
	}
	farmers=
	{
		id=11
		size=20000.000
		alpha=one
	}
	military_construction=
	{
		id=
		{
			id=500
			type=1
		}
		start_date="1849.7.22"
		date="1850.6.2"
		location=1
		country="AAA"
		name="1st Test Frigate"
		type="frigate"
		unit=
		{
			id=501
			type=1
		}
		regiment=
		{
			id=502
			type=1
		}
		count=1
		rally_point=-1
	}
}
2=
{
	name="PROV_TWO"
	owner="BBB"
	controller="BBB"
	garrison=100.000
	naval_base=
	{
 1.000 1.000 	}
}
3=
{
	name="PROV_SEA"
	garrison=100.000
}
REB=
{
	leader=
	{
		name="Rebel Man"
		date="1849.8.25"
		type=land
		personality="skilled"
		background="yesman"
		country="REB"
		picture="x"
		prestige=0.031
		id=
		{
			id=900
			type=37
		}
	}
}
AAA=
{
	mobilize=yes
	war_exhaustion=12.500
	leadership=7.000
	auto_assign_leaders=yes
	auto_create_leaders=no
	leader=
	{
		name="Sea Dog"
		date="1840.1.1"
		type=sea
		personality="skilled"
		background="yesman"
		country="AAA"
		picture="y"
		prestige=0.5
		id=
		{
			id=901
			type=37
		}
	}
	army=
	{
		id=
		{
			id=7000
			type=1
		}
		name="1st Army"
		leader=
		{
			id=900
			type=37
		}
		previous=2
		movement_progress=5.920
		path=
		{
			2 3 		}
		location=1
		dig_in=1.000
		dig_in_last_date="1849.12.29"
		supplies=0.299
		regiment=
		{
			id=
			{
				id=70
				type=1
			}
			name="1st Foot"
			pop=
			{
				id=10
				type=1
			}
			organisation=24.0
			strength=3.000
			experience=5.000
			count=2
			type=infantry
		}
		regiment=
		{
			id=
			{
				id=71
				type=1
			}
			name="Farmer Levy"
			pop=
			{
				id=11
				type=1
			}
			organisation=10.0
			strength=1.500
			experience=0.000
			type=irregular
		}
		base=1
	}
	navy=
	{
		id=
		{
			id=8000
			type=1
		}
		name="1st Fleet"
		leader=
		{
			id=901
			type=37
		}
		location=3
		supplies=0.350
		ship=
		{
			id=
			{
				id=80
				type=1
			}
			name="HMS Test"
			organisation=28.0
			strength=100.000
			experience=5.000
			type=manowar
		}
		ship=
		{
			id=
			{
				id=81
				type=1
			}
			name="HMS Other"
			organisation=28.0
			strength=60.000
			type=frigate
		}
		army=
		{
			id=
			{
				id=7001
				type=1
			}
			name="2nd Army"
			location=3
			supplies=0.299
			regiment=
			{
				id=
				{
					id=72
					type=1
				}
				name="Marines"
				pop=
				{
					id=10
					type=1
				}
				organisation=20.0
				strength=3.000
				type=infantry
			}
			target=2
			base=1
		}
		at_sea=1
		no_supply_days=4
	}
}
BBB=
{
	war_exhaustion=0.000
}
great_nations=
{
2 }
active_war=
{
	name=" Test War"
	history=
	{
		1849.1.5=
		{
			add_attacker="BBB"
		}
		1849.1.5=
		{
			add_defender="AAA"
		}
		1849.3.9=
		{
			battle=
			{
				name="Hill"
				location=1
				result=yes
				attacker=
				{
					country="BBB"
					leader="Gen One"
					infantry=3000
					losses=300
				}
				defender=
				{
					country="AAA"
					leader="Gen Two"
					irregular=2000
					losses=1200
				}
			}
		}
	}
	attacker="BBB"
	defender="AAA"
	original_attacker="BBB"
	original_defender="AAA"
	original_wargoal=
	{
		state_province_id=1
		casus_belli="acquire_state"
		actor="BBB"
		receiver="AAA"
		score=0.000
		change=0.000
		date="-1.1.1"
		is_fulfilled=no
	}
	action="1849.3.9"
	war_goal=
	{
		state_province_id=1
		casus_belli="acquire_state"
		actor="BBB"
		receiver="AAA"
		score=0.100
		change=0.100
		date="1849.1.5"
		is_fulfilled=yes
	}
}
previous_war=
{
	name="Old War"
	history=
	{
		1830.4.19=
		{
			add_attacker="AAA"
		}
		1830.4.19=
		{
			add_defender="BBB"
		}
		1832.9.3=
		{
			rem_attacker="AAA"
		}
		1832.9.3=
		{
			rem_defender="BBB"
		}
	}
	original_attacker="---"
	original_defender="---"
	original_wargoal=
	{
	}
	action="1.1.1"
}
combat=
{
	siege_combat=
	{
		location=1
		day=3
		duration=7
		attacker=
		{
			dice=0
			unit=
			{
				id=7000
				type=1
			}
			losses=0.000
			accumulated_losses=
			{
0.000 0.000 			}
front={}
back={}
		}
		defender=
		{
			dice=0
			losses=0.000
		}
		total=12
	}
	land_combat=
	{
		location=2
		day=2
		duration=12
		attacker=
		{
			dice=4
			unit=
			{
				id=7000
				type=1
			}
			losses=5898.000
			artillery=9000.000
			infantry=21000.000
front={11=
			{
				id=70
				type=1
			}
12=
			{
				id=71
				type=1
			}
}
back={15=
			{
				id=72
				type=1
			}
}
			reserves=
			{
				id=74
				type=1
			}
			reserves=
			{
				id=73
				type=1
			}
			retreat=
			{
				id=7296
				type=1
			}
		}
		defender=
		{
			dice=1
			losses=27797.000
			irregular=33000.000
front={}
back={}
		}
	}
}
"#;

fn rules() -> MilitaryRules {
    MilitaryRules { rebel_tag: "REB".into(), full_garrison: 100.0, men_per_strength: 1000.0, units: Default::default() }
}

fn load() -> MilitaryState {
    let pop_types: HashSet<String> = ["soldiers", "farmers"].iter().map(|s| s.to_string()).collect();
    let cultures: HashSet<String> = ["alpha".to_string()].into();
    let save = parse_save(SAVE.as_bytes(), &pop_types, &cultures).unwrap();
    MilitaryState::from_save(&save, &rules())
}

#[test]
fn armies_regiments_and_the_pop_types_behind_them() {
    let m = load();
    assert_eq!(m.armies.len(), 2);
    let a = m.armies.iter().find(|a| a.id == 7000).unwrap();
    assert_eq!((a.owner.as_str(), a.location, a.previous, a.leader, a.base), ("AAA", 1, Some(2), Some(900), Some(1)));
    assert_eq!(a.path, vec![2, 3]);
    assert_eq!(a.carried_by, None);
    assert!((a.movement_progress - 5.92).abs() < 1e-9 && (a.supplies - 0.299).abs() < 1e-9);
    assert_eq!(a.dig_in_date, Some(Date::new(1849, 12, 29)));
    assert_eq!(a.regiments.len(), 2);
    assert_eq!(a.regiments[0].kind, "infantry");
    assert_eq!(a.regiments[0].pop_type.as_deref(), Some("soldiers"));
    assert_eq!(a.regiments[1].pop_type.as_deref(), Some("farmers"));
    assert_eq!(a.regiments[0].name_count, 2);
    assert!((a.men(1000.0) - 4500.0).abs() < 1e-9);
    assert_eq!(m.regiment_count("AAA"), 3);
    assert_eq!(m.regiment_count("BBB"), 0);
}

#[test]
fn an_army_on_board_is_listed_once_and_knows_its_fleet() {
    let m = load();
    let n = &m.navies[0];
    assert_eq!((n.id, n.owner.as_str(), n.location, n.leader), (8000, "AAA", 3, Some(901)));
    assert_eq!(n.ships.len(), 2);
    assert_eq!(n.carried, vec![7001]);
    assert_eq!((n.at_sea, n.no_supply_days), (1.0, 4.0));
    let carried = m.armies.iter().find(|a| a.id == 7001).unwrap();
    assert_eq!((carried.carried_by, carried.target), (Some(8000), Some(2)));
    assert_eq!(m.ship_count("AAA"), 2);
    // Carried regiments count as the country's regiments.
    assert_eq!(m.regiment_count("AAA"), 3);
}

#[test]
fn leaders_of_every_owner_including_rebels() {
    let m = load();
    assert_eq!(m.leaders.len(), 2);
    let sea = m.leaders.iter().find(|l| l.id == 901).unwrap();
    assert_eq!((sea.kind, sea.owner.as_str(), sea.created), (LeaderKind::Sea, "AAA", Some(Date::new(1840, 1, 1))));
    assert_eq!(m.leaders_of("REB").count(), 1);
    assert_eq!(m.leaders_of("AAA").count(), 1);
}

#[test]
fn country_flags() {
    let m = load();
    let a = &m.countries["AAA"];
    assert!(a.mobilized && a.great_power && a.auto_assign_leaders && !a.auto_create_leaders);
    assert!((a.war_exhaustion - 12.5).abs() < 1e-9 && (a.leadership - 7.0).abs() < 1e-9);
    assert!(!m.countries["BBB"].mobilized && !m.countries["BBB"].great_power);
}

#[test]
fn wars_with_goals_battles_and_dates() {
    let m = load();
    assert_eq!(m.wars.len(), 2);
    let w = &m.wars[0];
    assert!(w.active);
    assert_eq!(w.name, "Test War");
    assert_eq!((w.attacker_leader(), w.defender_leader()), (Some("BBB"), Some("AAA")));
    assert!(w.takes_part("AAA") && !w.takes_part("CCC"));
    assert_eq!(w.started(), Some(Date::new(1849, 1, 5)));
    assert_eq!(w.ended(), None);
    assert_eq!(w.goals.len(), 1);
    assert!(w.goals[0].fulfilled && (w.goals[0].score - 0.1).abs() < 1e-9);
    assert_eq!(w.goals[0].state_province, Some(1));
    assert_eq!(w.original_goal.as_ref().unwrap().date, None);
    let b: Vec<_> = w.battles().collect();
    assert_eq!(b.len(), 1);
    assert!(b[0].attacker_won);
    assert_eq!((b[0].attacker.forces["infantry"], b[0].defender.losses), (3000.0, 1200.0));
    assert_eq!(m.active_wars_of("AAA").count(), 1);
    assert_eq!(m.active_wars_of("CCC").count(), 0);

    let old = &m.wars[1];
    assert!(!old.active);
    assert_eq!((old.started(), old.ended()), (Some(Date::new(1830, 4, 19)), Some(Date::new(1832, 9, 3))));
    assert_eq!((old.original_attacker.clone(), old.original_goal.clone()), (None, None));
    assert_eq!(m.active_wars_of("BBB").count(), 1);
}

#[test]
fn occupation_garrison_and_buildings() {
    let m = load();
    let p = &m.provinces[&1];
    assert!(p.is_occupied());
    assert_eq!((p.owner.as_deref(), p.controller.as_deref()), (Some("AAA"), Some("BBB")));
    assert!((p.garrison - 40.5).abs() < 1e-9 && (p.fort - 2.0).abs() < 1e-9);
    assert_eq!(p.last_controller_change, Some(Date::new(1849, 7, 11)));
    assert!(!m.provinces[&2].is_occupied());
    assert!((m.provinces[&2].naval_base - 1.0).abs() < 1e-9);
    // A sea province has no owner and is never occupied.
    assert!(!m.provinces[&3].is_occupied());
    assert_eq!(m.occupied().map(|(id, _)| id).collect::<Vec<_>>(), vec![1]);
}

#[test]
fn battles_and_sieges_in_progress() {
    let m = load();
    assert_eq!(m.combats.len(), 2);
    let s = m.sieges().next().unwrap();
    assert_eq!((s.location, s.day, s.duration, s.total), (1, 3.0, 7.0, 12.0));
    assert_eq!(s.attacker.units, vec![7000]);
    assert_eq!(s.attacker.accumulated_losses.len(), 2);
    let l = m.combats.iter().find(|c| c.kind == CombatKind::Land).unwrap();
    assert_eq!((l.attacker.dice, l.defender.dice), (4.0, 1.0));
    assert_eq!((l.attacker.forces["infantry"], l.attacker.forces["artillery"], l.defender.forces["irregular"]), (21000.0, 9000.0, 33000.0));
    assert_eq!(l.attacker.front.get(&11), Some(&70));
    assert_eq!(l.attacker.back.get(&15), Some(&72));
    assert_eq!(l.attacker.reserve, vec![74, 73]);
    assert_eq!(l.attacker.retreat, Some(7296));
    assert!(l.defender.front.is_empty());
}

#[test]
fn units_under_construction() {
    let m = load();
    assert_eq!(m.constructions.len(), 1);
    let c = &m.constructions[0];
    assert_eq!((c.id, c.owner.as_str(), c.location, c.kind.as_str(), c.unit, c.rally_point), (500, "AAA", 1, "frigate", Some(501), -1));
    assert_eq!((c.start, c.end), (Some(Date::new(1849, 7, 22)), Some(Date::new(1850, 6, 2))));
}

#[test]
fn unit_types_come_from_the_install_format() {
    let doc = wiktoria_data::script::parse(
        "foot = { type = land unit_type = infantry active = no max_strength = 3 default_organisation = 30 maximum_speed = 4.00 attack = 4 defence = 4 discipline = 1.0 build_time = 90 build_cost = { goods_a = 10 goods_b = 5 } supply_cost = { goods_a = 0.5 } }\n\
         liner = { type = naval unit_type = big_ship capital = yes max_strength = 100 hull = 4 gun_power = 8 limit_per_port = -1 can_build_overseas = no }",
    )
    .unwrap();
    let units: Vec<UnitDef> = doc.root.entries().filter_map(|(n, v, _)| Some(UnitDef::from_block(n, v.as_block()?))).collect();
    let foot = &units[0];
    assert!(foot.land && !foot.active && !foot.capital);
    assert_eq!((foot.kind.as_str(), foot.max_strength, foot.organisation, foot.speed, foot.attack, foot.defence, foot.build_time), ("infantry", 3.0, 30.0, 4.0, 4.0, 4.0, 90.0));
    assert_eq!(foot.build_cost["goods_a"], 10.0);
    assert_eq!(foot.supply_cost["goods_a"], 0.5);
    let liner = &units[1];
    assert!(!liner.land && liner.capital && liner.active);
    // A negative limit means none.
    assert_eq!((liner.limit_per_port, liner.can_build_overseas, liner.hull, liner.gun_power), (None, Some(false), 4.0, 8.0));
}

#[test]
fn rules_come_from_the_engine_rules() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let engine_rules = wiktoria_data::config::load_engine_rules(&root).unwrap();
    let r = MilitaryRules::from_engine_rules(&engine_rules).unwrap();
    assert_eq!((r.rebel_tag.as_str(), r.full_garrison, r.men_per_strength), ("REB", 100.0, 1000.0));
}
