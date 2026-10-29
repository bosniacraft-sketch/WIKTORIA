//! The one place that maps keys to actions (docs/UI-LAYOUT.md, "Keyboard shortcuts").
//! These are defaults. Later the table can be filled from the install's interface/*.gui `shortcut` lines
//! and from player rebinding: `ui::App` only ever asks `Bindings::pressed`.

use eframe::egui::{Context, Key};

use crate::ui::Win;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Toggle a country window (or the ledger).
    Window(Win),
    /// Select map mode number n (0-based, in `ViewMode::ALL` order).
    MapMode(usize),
    Pause,
    SpeedUp,
    SpeedDown,
    /// ESCAPE: close the topmost window.
    CloseTop,
    LedgerPrev,
    LedgerNext,
}

#[derive(Debug, Clone)]
pub struct Bindings {
    pub table: Vec<(Key, Action)>,
}

impl Default for Bindings {
    fn default() -> Self {
        let mut table = vec![
            (Key::F1, Action::Window(Win::Production)),
            (Key::F2, Action::Window(Win::Budget)),
            (Key::F3, Action::Window(Win::Technology)),
            (Key::F4, Action::Window(Win::Politics)),
            (Key::F5, Action::Window(Win::Population)),
            (Key::F6, Action::Window(Win::Trade)),
            (Key::F7, Action::Window(Win::Diplomacy)),
            (Key::F8, Action::Window(Win::Military)),
            (Key::L, Action::Window(Win::Ledger)),
            (Key::Space, Action::Pause),
            (Key::Plus, Action::SpeedUp),
            (Key::Equals, Action::SpeedUp),
            (Key::Minus, Action::SpeedDown),
            (Key::Escape, Action::CloseTop),
            (Key::ArrowLeft, Action::LedgerPrev),
            (Key::ArrowRight, Action::LedgerNext),
        ];
        // Map modes 1-10 on the top letter row, 11-19 on the home row (L is taken by the ledger).
        let row1 = [Key::Q, Key::W, Key::E, Key::R, Key::T, Key::Y, Key::U, Key::I, Key::O, Key::P];
        let row2 = [Key::A, Key::S, Key::D, Key::F, Key::G, Key::H, Key::J, Key::K, Key::L];
        for (i, k) in row1.into_iter().chain(row2).enumerate() {
            if !table.iter().any(|(tk, _)| *tk == k) {
                table.push((k, Action::MapMode(i)));
            }
        }
        Bindings { table }
    }
}

impl Bindings {
    /// Actions whose key went down this frame (no modifiers held).
    pub fn pressed(&self, ctx: &Context) -> Vec<Action> {
        if ctx.egui_wants_keyboard_input() {
            return Vec::new();
        }
        ctx.input(|i| {
            if i.modifiers.command || i.modifiers.alt {
                return Vec::new();
            }
            self.table.iter().filter(|(k, _)| i.key_pressed(*k)).map(|(_, a)| *a).collect()
        })
    }

    /// Key label for an action, for button hints.
    pub fn hint(&self, a: Action) -> Option<String> {
        self.table.iter().find(|(_, x)| *x == a).map(|(k, _)| k.name().to_string())
    }
}
