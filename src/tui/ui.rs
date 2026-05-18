//! ratatui drawing.
//!
//! Layout (deliberately minimal — no panel borders, no titles, just
//! the smith and small text at the edges):
//!
//! ```text
//!   MergeSmith
//!
//!
//!
//!                  [stippled blacksmith]
//!
//!
//!
//!   ▶ feat/auth → main  · rebasing   queued: 3   needs help: 0
//!   ↑↓ select   d delete   ? help   q quit
//! ```
//!
//! The sprite is the centerpiece and fills most of the available
//! area. ratatui draws nothing in the sprite rect — the Kitty place
//! command writes pixels there directly, overlaying the (empty) cells.
//!
//! Sizing rule: the sprite occupies all rows between the title row
//! (top, 2 rows) and the footer (bottom, 3 rows), with a column
//! margin on either side. We then constrain the on-screen footprint
//! via Kitty's `c=` / `r=` keys so the terminal scales the image to
//! match.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::core::queue::{QueueEntry, QueueStatus};
use crate::tui::app::AppState;
use crate::tui::sprite::state::SpriteState;

/// Number of rows reserved at the top for the brand line.
pub const HEADER_ROWS: u16 = 2;
/// Number of rows reserved at the bottom for the queue summary +
/// keybind hints.
pub const FOOTER_ROWS: u16 = 3;
/// Horizontal margin around the sprite (in cells).
pub const SPRITE_X_MARGIN: u16 = 4;

/// Per-frame layout. The Kitty place command needs the sprite cell
/// origin and dimensions; the rest of the rects are for ratatui's
/// own widgets.
#[derive(Debug, Clone, Copy)]
pub struct DrawLayout {
    pub root: Rect,
    pub header: Rect,
    pub sprite: Rect,
    pub footer_status: Rect,
    pub footer_hints: Rect,
}

pub fn compute_layout(area: Rect) -> DrawLayout {
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(HEADER_ROWS),
            Constraint::Min(5),
            Constraint::Length(FOOTER_ROWS),
        ])
        .split(area);

    let header = outer[0];
    let center = outer[1];
    let footer = outer[2];

    // Sprite: center horizontally with a margin on each side.
    let margin = SPRITE_X_MARGIN.min(center.width / 8);
    let sprite_x = center.x + margin;
    let sprite_w = center.width.saturating_sub(margin * 2);
    let sprite = Rect::new(sprite_x, center.y, sprite_w, center.height);

    // Footer: two rows for queue status / active line, one row for hints.
    let footer_split = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Length(1)])
        .split(footer);

    DrawLayout {
        root: area,
        header,
        sprite,
        footer_status: footer_split[0],
        footer_hints: footer_split[1],
    }
}

pub fn draw(frame: &mut Frame, app: &mut AppState) -> DrawLayout {
    let layout = compute_layout(frame.area());

    draw_header(frame, layout.header, app);
    // The sprite rect is left empty for the Kitty layer to paint.
    // We don't even fill it with blanks — terminal background covers
    // it, and Kitty pixels land on top.
    draw_footer_status(frame, layout.footer_status, app);
    draw_footer_hints(frame, layout.footer_hints, app);

    if app.help_visible {
        draw_help_overlay(frame, layout.root);
    }

    layout
}

fn draw_header(frame: &mut Frame, area: Rect, _app: &AppState) {
    // Tight to the top-left, no borders. We pad one column for breathing room.
    let padded = Rect::new(
        area.x + 2,
        area.y,
        area.width.saturating_sub(2),
        area.height,
    );
    let title = Line::from(vec![
        Span::styled(
            "MergeSmith",
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "  ·  local merge queue",
            Style::default().fg(Color::DarkGray),
        ),
    ]);
    frame.render_widget(Paragraph::new(title), padded);
}

fn draw_footer_status(frame: &mut Frame, area: Rect, app: &AppState) {
    let padded = Rect::new(
        area.x + 2,
        area.y,
        area.width.saturating_sub(4),
        area.height,
    );

    // Line 1: the currently-active entry, if any. We pick the first
    // entry in a "currently working" status, else the head of the
    // queue, else "idle".
    let active = pick_active(&app.entries);
    let line1 = match active {
        Some(e) => Line::from(vec![
            Span::styled("▶ ", Style::default().fg(Color::White)),
            Span::styled(e.source_branch.clone(), Style::default().fg(Color::White)),
            Span::styled(" → ", Style::default().fg(Color::DarkGray)),
            Span::styled(e.target_branch.clone(), Style::default().fg(Color::Gray)),
            Span::styled("   ", Style::default()),
            Span::styled(
                status_phrase(e.status),
                Style::default().fg(status_color(e.status)),
            ),
        ]),
        None => Line::from(Span::styled(
            "▷ idle — no entry in flight",
            Style::default().fg(Color::DarkGray),
        )),
    };

    // Line 2: counts.
    let (queued, working, needs_help, terminal) = counts(&app.entries);
    let smith_label = match app.sprite_state {
        SpriteState::Idle => "smith: at rest",
        SpriteState::Working => "smith: at the anvil",
        SpriteState::NeedsHelp => "smith: needs your help",
    };
    let line2 = Line::from(vec![
        Span::styled(
            smith_label,
            Style::default()
                .fg(match app.sprite_state {
                    SpriteState::Idle => Color::DarkGray,
                    SpriteState::Working => Color::White,
                    SpriteState::NeedsHelp => Color::Yellow,
                })
                .add_modifier(Modifier::DIM),
        ),
        Span::styled("    queued ", Style::default().fg(Color::DarkGray)),
        Span::styled(format!("{queued}"), Style::default().fg(Color::Gray)),
        Span::styled("    working ", Style::default().fg(Color::DarkGray)),
        Span::styled(format!("{working}"), Style::default().fg(Color::White)),
        Span::styled("    needs help ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{needs_help}"),
            Style::default().fg(if needs_help > 0 {
                Color::Yellow
            } else {
                Color::DarkGray
            }),
        ),
        Span::styled("    done ", Style::default().fg(Color::DarkGray)),
        Span::styled(format!("{terminal}"), Style::default().fg(Color::DarkGray)),
    ]);

    frame.render_widget(Paragraph::new(vec![line1, line2]), padded);
}

fn draw_footer_hints(frame: &mut Frame, area: Rect, app: &AppState) {
    let padded = Rect::new(
        area.x + 2,
        area.y,
        area.width.saturating_sub(4),
        area.height,
    );
    let hint = Line::from(vec![
        Span::styled("↑↓", Style::default().fg(Color::Gray)),
        Span::styled(" select   ", Style::default().fg(Color::DarkGray)),
        Span::styled("d", Style::default().fg(Color::Gray)),
        Span::styled(" delete-queued   ", Style::default().fg(Color::DarkGray)),
        Span::styled("?", Style::default().fg(Color::Gray)),
        Span::styled(" help   ", Style::default().fg(Color::DarkGray)),
        Span::styled("q", Style::default().fg(Color::Gray)),
        Span::styled(" quit   ", Style::default().fg(Color::DarkGray)),
        Span::styled("·  ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            app.status_line.clone(),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::ITALIC),
        ),
    ]);
    frame.render_widget(Paragraph::new(hint), padded);
}

fn draw_help_overlay(frame: &mut Frame, area: Rect) {
    let h = 11;
    let w = 56;
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    let overlay = Rect::new(x, y, w.min(area.width), h.min(area.height));

    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(
            " help ",
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(Color::Black));
    let inner = block.inner(overlay);

    let lines = vec![
        Line::from(""),
        Line::from(" ↑ / ↓ / j / k    move selection in the queue"),
        Line::from(" d                delete the selected entry (if Queued)"),
        Line::from(" ?                toggle this help overlay"),
        Line::from(" q                quit (soft shutdown)"),
        Line::from(" Q  /  Ctrl-C     force quit (hard shutdown)"),
        Line::from(""),
        Line::from(" The TUI is read-only; enqueue / cancel / retry /"),
        Line::from(" resolve are CLI subcommands. Run `mergesmith --help`."),
    ];

    frame.render_widget(block, overlay);
    frame.render_widget(Paragraph::new(lines), inner);
}

// ---------------------------------------------------------------------
// Pure helpers — no ratatui types in/out, easy to test.
// ---------------------------------------------------------------------

fn pick_active(entries: &[QueueEntry]) -> Option<&QueueEntry> {
    // Prefer NeedsHelp (it's the only state asking for the user).
    // Then in-flight statuses. Then the head of the queue. Terminal
    // statuses are ignored — they're not "active" anything.
    fn pri(s: QueueStatus) -> Option<u8> {
        match s {
            QueueStatus::NeedsHelp => Some(0),
            QueueStatus::Rebasing | QueueStatus::CIRunning | QueueStatus::Merging => Some(1),
            QueueStatus::Queued => Some(2),
            _ => None,
        }
    }
    entries
        .iter()
        .filter(|e| pri(e.status).is_some())
        .min_by_key(|e| pri(e.status).unwrap_or(u8::MAX))
}

fn counts(entries: &[QueueEntry]) -> (u32, u32, u32, u32) {
    let mut queued = 0;
    let mut working = 0;
    let mut needs_help = 0;
    let mut terminal = 0;
    for e in entries {
        match e.status {
            QueueStatus::Queued => queued += 1,
            QueueStatus::Rebasing | QueueStatus::CIRunning | QueueStatus::Merging => working += 1,
            QueueStatus::NeedsHelp => needs_help += 1,
            QueueStatus::Merged | QueueStatus::Failed | QueueStatus::Cancelled => terminal += 1,
        }
    }
    (queued, working, needs_help, terminal)
}

fn status_phrase(s: QueueStatus) -> &'static str {
    match s {
        QueueStatus::Queued => "queued",
        QueueStatus::Rebasing => "rebasing",
        QueueStatus::CIRunning => "running CI",
        QueueStatus::Merging => "merging",
        QueueStatus::NeedsHelp => "needs help",
        QueueStatus::Merged => "merged",
        QueueStatus::Failed => "failed",
        QueueStatus::Cancelled => "cancelled",
    }
}

fn status_color(s: QueueStatus) -> Color {
    match s {
        QueueStatus::Queued => Color::Gray,
        QueueStatus::Rebasing | QueueStatus::Merging => Color::White,
        QueueStatus::CIRunning => Color::LightBlue,
        QueueStatus::NeedsHelp => Color::Yellow,
        QueueStatus::Merged => Color::Green,
        QueueStatus::Failed => Color::Red,
        QueueStatus::Cancelled => Color::DarkGray,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ids::{QueueEntryId, RepoId};
    use crate::core::queue::QueueEntry;
    use time::OffsetDateTime;

    fn entry(status: QueueStatus, branch: &str) -> QueueEntry {
        QueueEntry {
            id: QueueEntryId::new(),
            repo_id: RepoId::new(),
            source_worktree: "/tmp/x".into(),
            source_branch: branch.into(),
            target_branch: "main".into(),
            status,
            last_outcome: None,
            enqueued_at: OffsetDateTime::from_unix_timestamp(1).unwrap(),
            started_at: None,
            finished_at: None,
            failure_reason: None,
            ci_log_dir: None,
            merge_log_path: None,
            conflict_session_id: None,
            message: None,
            claimed_by_pid: None,
            claimed_at: None,
        }
    }

    #[test]
    fn layout_reserves_header_and_footer() {
        let area = Rect::new(0, 0, 100, 30);
        let l = compute_layout(area);
        assert_eq!(l.header.height, HEADER_ROWS);
        assert_eq!(l.footer_status.height + l.footer_hints.height, FOOTER_ROWS);
        // Sprite fills everything in between, minus side margins.
        assert_eq!(l.sprite.height, area.height - HEADER_ROWS - FOOTER_ROWS);
        assert!(l.sprite.x > area.x, "sprite should have a left margin");
        assert!(
            l.sprite.x + l.sprite.width <= area.x + area.width,
            "sprite must fit horizontally"
        );
    }

    #[test]
    fn layout_survives_small_screens() {
        let area = Rect::new(0, 0, 20, 8);
        let l = compute_layout(area);
        assert!(l.sprite.width > 0);
        assert!(l.sprite.height >= 1);
    }

    #[test]
    fn pick_active_prefers_needs_help() {
        let entries = vec![
            entry(QueueStatus::Queued, "a"),
            entry(QueueStatus::Rebasing, "b"),
            entry(QueueStatus::NeedsHelp, "c"),
        ];
        assert_eq!(pick_active(&entries).unwrap().source_branch, "c");
    }

    #[test]
    fn pick_active_prefers_in_flight_over_queued() {
        let entries = vec![
            entry(QueueStatus::Queued, "a"),
            entry(QueueStatus::CIRunning, "b"),
        ];
        assert_eq!(pick_active(&entries).unwrap().source_branch, "b");
    }

    #[test]
    fn pick_active_falls_back_to_queued() {
        let entries = vec![
            entry(QueueStatus::Merged, "a"),
            entry(QueueStatus::Queued, "b"),
        ];
        assert_eq!(pick_active(&entries).unwrap().source_branch, "b");
    }

    #[test]
    fn pick_active_none_on_only_terminal() {
        let entries = vec![
            entry(QueueStatus::Merged, "a"),
            entry(QueueStatus::Failed, "b"),
            entry(QueueStatus::Cancelled, "c"),
        ];
        assert!(pick_active(&entries).is_none());
    }

    #[test]
    fn counts_partition_correctly() {
        let entries = vec![
            entry(QueueStatus::Queued, "a"),
            entry(QueueStatus::Queued, "b"),
            entry(QueueStatus::Rebasing, "c"),
            entry(QueueStatus::CIRunning, "d"),
            entry(QueueStatus::NeedsHelp, "e"),
            entry(QueueStatus::Merged, "f"),
            entry(QueueStatus::Failed, "g"),
            entry(QueueStatus::Cancelled, "h"),
        ];
        let (q, w, n, t) = counts(&entries);
        assert_eq!(q, 2);
        assert_eq!(w, 2);
        assert_eq!(n, 1);
        assert_eq!(t, 3);
    }
}
