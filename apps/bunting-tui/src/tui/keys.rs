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
    NextField,
    PreviousField,
    Left,
    Right,
    Character(char),
    TabMarket,
    TabOrders,
    TabAccount,
    TabSimulation,
    TabCollaboration,
    TabAdministration,
    TabSession,
    ToggleHelp,
    ToggleLog,
    BeginCommand,
    BeginBuy,
    BeginSell,
    BeginQuantity,
    BeginCancel,
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
        KeyCode::Tab if editing => Action::NextField,
        KeyCode::BackTab if editing => Action::PreviousField,
        KeyCode::Left if editing => Action::Left,
        KeyCode::Right if editing => Action::Right,
        KeyCode::Char(character) if editing => Action::Character(character),
        KeyCode::Char('q') => Action::Quit,
        KeyCode::Char('?') | KeyCode::F(1) => Action::ToggleHelp,
        KeyCode::Char('`') => Action::ToggleLog,
        KeyCode::Char('/') => Action::BeginCommand,
        KeyCode::Char('b') => Action::BeginBuy,
        KeyCode::Char('s') => Action::BeginSell,
        KeyCode::Char('x') => Action::BeginQuantity,
        KeyCode::Char('c') => Action::BeginCancel,
        KeyCode::Char('r' | 'R') => Action::Refresh,
        KeyCode::Char('1') => Action::TabMarket,
        KeyCode::Char('2') => Action::TabOrders,
        KeyCode::Char('3') => Action::TabAccount,
        KeyCode::Char('4') => Action::TabSimulation,
        KeyCode::Char('5') => Action::TabCollaboration,
        KeyCode::Char('6') => Action::TabAdministration,
        KeyCode::Char('7') => Action::TabSession,
        KeyCode::Up => Action::SelectPrevious,
        KeyCode::Down => Action::SelectNext,
        _ => Action::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_shortcuts_cover_every_workspace_without_stealing_editor_keys() {
        let key = |character| KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE);
        assert_eq!(resolve(key('1'), false), Action::TabMarket);
        assert_eq!(resolve(key('7'), false), Action::TabSession);
        assert_eq!(resolve(key('7'), true), Action::Character('7'));
    }

    #[test]
    fn release_and_repeat_events_are_ignored() {
        assert_eq!(
            resolve(
                KeyEvent::new_with_kind(
                    KeyCode::Char('q'),
                    KeyModifiers::NONE,
                    KeyEventKind::Release,
                ),
                false,
            ),
            Action::None
        );
    }
}
