// Adapted from makeev/alphai-tui src/keymap.rs at
// f814697c6159d76b2dfb503ba5201b8c3fb702ad under the MIT license.
// Modified for Bunting's command-entry and FIX-log workflow.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Action {
    Quit,
    Submit,
    Clear,
    Backspace,
    ToggleHelp,
    Character(char),
    None,
}

#[must_use]
pub fn resolve(event: KeyEvent) -> Action {
    if event.modifiers.contains(KeyModifiers::CONTROL) && event.code == KeyCode::Char('c') {
        return Action::Quit;
    }
    match event.code {
        KeyCode::Esc => Action::Quit,
        KeyCode::Enter => Action::Submit,
        KeyCode::Backspace => Action::Backspace,
        KeyCode::Delete => Action::Clear,
        KeyCode::F(1) => Action::ToggleHelp,
        KeyCode::Char(character)
            if event
                .modifiers
                .intersection(KeyModifiers::CONTROL | KeyModifiers::ALT)
                .is_empty() =>
        {
            Action::Character(character)
        }
        _ => Action::None,
    }
}
