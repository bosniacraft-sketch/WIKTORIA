//! wiktoria_data: reads Victoria 2 data from the player's own install.
//!
//! Game data is read at runtime from a game data folder (the bundled starter world or a Victoria 2
//! install). Engine constants come from `data/engine_rules.json`. No external dependencies.

pub mod bmp;
pub mod compare;
pub mod config;
pub mod countries;
pub mod country_history;
pub mod date;
pub mod defines;
pub mod dds;
pub mod defs;
pub mod geography;
pub mod goods;
pub mod history;
pub mod localisation;
pub mod map;
pub mod engine_rules;
pub mod modifiers;
pub mod popdiff;
pub mod production;
pub mod pops;
pub mod save;
pub mod script;
pub mod setup;
pub mod install_fs;
pub mod world;
