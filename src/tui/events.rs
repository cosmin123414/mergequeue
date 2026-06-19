//! Crossterm event loop integration.
//!
//! Two concerns:
//!
//! 1. A pure key-mapping function ([`map_key`]) that translates a
//!    `KeyEvent` into a [`TuiAction`]. Pure → unit-testable without
//!    spawning crossterm.
//! 2. A polling wrapper ([`poll_action`]) that drives crossterm with
//!    a deadline and routes through the mapper.

use std::time::Duration;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

/// All key-driven actions the TUI can produce. Anything outside this
/// enum is ignored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TuiAction {
    /// Terminal tab/window gained focus.
    FocusGained,
    /// Terminal tab/window lost focus.
    FocusLost,
    /// `q`: soft shutdown.
    QuitSoft,
    /// `Q` or `Ctrl-C`: hard shutdown.
    QuitHard,
    /// `↓` or `j`.
    SelectDown,
    /// `↑` or `k`.
    SelectUp,
    /// `d`: delete the selected entry (only valid if Queued; the
    /// app-state layer enforces this).
    DeleteSelected,
    /// `t`: attach to the selected NeedsHelp agent session.
    AttachSelected,
    /// Space: expand/collapse the selected queue entry.
    ToggleExpanded,
    /// `?`: toggle the help overlay.
    ToggleHelp,
    /// A keystroke the TUI doesn't bind to anything.
    Ignore,
}

/// Translate a key event to a `TuiAction`. Pure function.
pub fn map_key(ev: KeyEvent) -> TuiAction {
    // Ctrl-C always means hard quit, regardless of which key the
    // user pressed.
    if ev.modifiers.contains(KeyModifiers::CONTROL) && ev.code == KeyCode::Char('c') {
        return TuiAction::QuitHard;
    }
    match ev.code {
        KeyCode::Char('q') => TuiAction::QuitSoft,
        KeyCode::Char('Q') => TuiAction::QuitHard,
        KeyCode::Char('j') | KeyCode::Down => TuiAction::SelectDown,
        KeyCode::Char('k') | KeyCode::Up => TuiAction::SelectUp,
        KeyCode::Char('d') => TuiAction::DeleteSelected,
        KeyCode::Char('t') => TuiAction::AttachSelected,
        KeyCode::Char(' ') => TuiAction::ToggleExpanded,
        KeyCode::Char('?') => TuiAction::ToggleHelp,
        _ => TuiAction::Ignore,
    }
}

/// Block on stdin for up to `timeout` waiting for one key event,
/// then map it. Returns `None` if no key was pressed within the
/// budget (the render loop should treat this as "tick the sprite and
/// redraw").
///
/// This is the production wrapper. Tests don't go through it; they
/// invoke `map_key` directly.
pub fn poll_action(timeout: Duration) -> std::io::Result<Option<TuiAction>> {
    if crossterm::event::poll(timeout)? {
        match crossterm::event::read()? {
            Event::Key(k) => Ok(Some(map_key(k))),
            Event::FocusGained => Ok(Some(TuiAction::FocusGained)),
            Event::FocusLost => Ok(Some(TuiAction::FocusLost)),
            // Resize and other events trigger a redraw via the outer
            // loop's next tick; we treat them all as Ignore here so
            // the loop continues.
            _ => Ok(Some(TuiAction::Ignore)),
        }
    } else {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }
    fn kc(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, mods)
    }

    #[test]
    fn q_is_soft_quit() {
        assert_eq!(map_key(k(KeyCode::Char('q'))), TuiAction::QuitSoft);
    }

    #[test]
    fn shift_q_is_hard_quit() {
        // Crossterm reports shifted letters as their uppercase form.
        assert_eq!(map_key(k(KeyCode::Char('Q'))), TuiAction::QuitHard);
    }

    #[test]
    fn ctrl_c_is_hard_quit() {
        let ev = kc(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(map_key(ev), TuiAction::QuitHard);
    }

    #[test]
    fn arrow_keys_move_selection() {
        assert_eq!(map_key(k(KeyCode::Down)), TuiAction::SelectDown);
        assert_eq!(map_key(k(KeyCode::Up)), TuiAction::SelectUp);
    }

    #[test]
    fn vim_keys_move_selection() {
        assert_eq!(map_key(k(KeyCode::Char('j'))), TuiAction::SelectDown);
        assert_eq!(map_key(k(KeyCode::Char('k'))), TuiAction::SelectUp);
    }

    #[test]
    fn delete_and_help() {
        assert_eq!(map_key(k(KeyCode::Char('d'))), TuiAction::DeleteSelected);
        assert_eq!(map_key(k(KeyCode::Char('?'))), TuiAction::ToggleHelp);
    }

    #[test]
    fn t_attaches_selected() {
        assert_eq!(map_key(k(KeyCode::Char('t'))), TuiAction::AttachSelected);
    }

    #[test]
    fn space_toggles_expanded() {
        assert_eq!(map_key(k(KeyCode::Char(' '))), TuiAction::ToggleExpanded);
    }

    #[test]
    fn enter_and_r_are_unbound() {
        assert_eq!(map_key(k(KeyCode::Enter)), TuiAction::Ignore);
        assert_eq!(map_key(k(KeyCode::Char('r'))), TuiAction::Ignore);
    }

    #[test]
    fn unknown_keys_ignored() {
        assert_eq!(map_key(k(KeyCode::Char('x'))), TuiAction::Ignore);
        assert_eq!(map_key(k(KeyCode::F(5))), TuiAction::Ignore);
    }
}
