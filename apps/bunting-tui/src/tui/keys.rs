// Modified from Longbridge Terminal at commit 05c9bbf7fd1c4ab5c34d5316fedf6e1ed5f1fcc3.
// Copyright 2026 Longbridge. Licensed under Apache-2.0.
// Rust guideline compliant 2026-02-21

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Action {
    None,
    Quit,
    Escape,
    Submit,
    Backspace,
    Character(char),
    TabMarket,
    TabOrders,
    TabFix,
    ToggleHelp,
    ToggleLog,
    BeginCommand,
    BeginBuy,
    BeginSell,
    BeginCancel,
    BeginReplace,
    Refresh,
    SelectPrevious,
    SelectNext,
}

pub fn resolve(event: KeyEvent, editing: bool) -> Action {
    if event.kind != KeyEventKind::Press {
        return Action::None;
    }
    if event.modifiers.contains(KeyModifiers::CONTROL) && event.code == KeyCode::Char('c') {
        return Action::Quit;
    }
    match event.code {
        KeyCode::Esc => Action::Escape,
        KeyCode::Enter => Action::Submit,
        KeyCode::Backspace if editing => Action::Backspace,
        KeyCode::Char(character) if editing => Action::Character(character),
        KeyCode::Char('q') => Action::Quit,
        KeyCode::Char('?') | KeyCode::F(1) => Action::ToggleHelp,
        KeyCode::Char('`') => Action::ToggleLog,
        KeyCode::Char('/') => Action::BeginCommand,
        KeyCode::Char('b') => Action::BeginBuy,
        KeyCode::Char('s') => Action::BeginSell,
        KeyCode::Char('c') => Action::BeginCancel,
        KeyCode::Char('m') => Action::BeginReplace,
        KeyCode::Char('r' | 'R') => Action::Refresh,
        KeyCode::Char('1') => Action::TabMarket,
        KeyCode::Char('2') => Action::TabOrders,
        KeyCode::Char('3') => Action::TabFix,
        KeyCode::Up => Action::SelectPrevious,
        KeyCode::Down => Action::SelectNext,
        _ => Action::None,
    }
}
