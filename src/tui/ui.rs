//! ratatui drawing.
//!
//! Layout (per `docs/08-tui-and-sprite.md`):
//!
//! ```text
//! ┌─ MergeSmith ────────────────────────────────────────────────┐
//! │ [sprite]   │ Queue                                          │
//! │            │ status  branch          target    repo         │
//! │ status     │ …                                              │
//! ├────────────┴────────────────────────────────────────────────┤
//! │ details                                                     │
//! ├─────────────────────────────────────────────────────────────┤
//! │ [↑↓ select]  [d delete-queued]  [? help]  [q quit]  [Q hard]│
//! └─────────────────────────────────────────────────────────────┘
//! ```
//!
//! The "sprite" panel is a reserved rectangle; we paint a dim
//! background there in ratatui, then the [`crate::tui::sprite`] layer
//! writes a Kitty place-command to stdout *after* the ratatui frame
//! flushes, overwriting those cells with the actual sprite pixels.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table};
use ratatui::Frame;

use crate::core::queue::QueueStatus;
use crate::tui::app::AppState;

/// Cells reserved for the sprite. 64×64 sprite ÷ (cell ≈ 8×16 px) ≈ 8
/// cols × 4 rows; we add some padding and round to 12×6 for a nicer
/// frame in the UI.
pub const SPRITE_COLS: u16 = 12;
pub const SPRITE_ROWS: u16 = 6;

/// The Kitty place command needs the sprite's cell-origin in
/// (col, row). [`compute_layout`] returns this alongside the
/// other rectangles so the render path is deterministic.
#[derive(Debug, Clone, Copy)]
pub struct DrawLayout {
    pub root: Rect,
    pub sprite: Rect,
    pub status_under_sprite: Rect,
    pub queue: Rect,
    pub details: Rect,
    pub hints: Rect,
}

pub fn compute_layout(area: Rect) -> DrawLayout {
    // Three vertical sections: top (sprite + queue) | details | hints.
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(SPRITE_ROWS + 2), // sprite+status block
            Constraint::Min(5),                  // details fills the rest
            Constraint::Length(1),               // hints
        ])
        .split(area);

    // Top section splits horizontally: sprite | queue.
    let top = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(SPRITE_COLS + 4), // sprite + a status line under it
            Constraint::Min(20),                 // queue fills the rest
        ])
        .split(outer[0]);

    // Within the sprite column: the sprite cells on top, status line below.
    let sprite_col = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(SPRITE_ROWS), Constraint::Min(1)])
        .split(top[0]);

    DrawLayout {
        root: area,
        sprite: sprite_col[0],
        status_under_sprite: sprite_col[1],
        queue: top[1],
        details: outer[1],
        hints: outer[2],
    }
}

pub fn draw(frame: &mut Frame, app: &mut AppState) -> DrawLayout {
    let layout = compute_layout(frame.area());

    draw_sprite_panel(frame, layout.sprite, app);
    draw_status_under_sprite(frame, layout.status_under_sprite, app);
    draw_queue(frame, layout.queue, app);
    draw_details(frame, layout.details, app);
    draw_hints(frame, layout.hints, app);

    if app.help_visible {
        draw_help_overlay(frame, layout.root);
    }

    layout
}

fn draw_sprite_panel(frame: &mut Frame, area: Rect, _app: &AppState) {
    // A dim-block background that the Kitty place command will paint
    // over. Looks intentional even before the sprite arrives.
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(Span::styled(
            " smith ",
            Style::default().fg(Color::Gray).add_modifier(Modifier::DIM),
        ));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Fill the inner area with shaded blocks so the panel doesn't look
    // empty on terminals that strip the Kitty escape (the next render
    // pass will overwrite this).
    let filler = Paragraph::new(vec![
        Line::from(Span::styled(
            "  ░░░░░░░░  ",
            Style::default().fg(Color::DarkGray),
        ));
        usize::from(inner.height)
    ]);
    frame.render_widget(filler, inner);
}

fn draw_status_under_sprite(frame: &mut Frame, area: Rect, app: &AppState) {
    let elapsed = app.started_at.elapsed();
    let mins = elapsed.as_secs() / 60;
    let secs = elapsed.as_secs() % 60;
    let label = match app.sprite_state {
        crate::tui::sprite::state::SpriteState::Idle => "idle",
        crate::tui::sprite::state::SpriteState::Working => "working",
        crate::tui::sprite::state::SpriteState::NeedsHelp => "needs help",
    };
    let line = Line::from(vec![
        Span::styled(label, Style::default().fg(Color::Cyan)),
        Span::raw(format!("  {mins:02}:{secs:02} elapsed")),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

fn draw_queue(frame: &mut Frame, area: Rect, app: &mut AppState) {
    let header = Row::new(vec!["status", "branch", "→ target", "repo"])
        .style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
        .height(1);

    let rows: Vec<Row> = app
        .entries
        .iter()
        .map(|e| {
            let status_cell = Cell::from(status_label(e.status))
                .style(Style::default().fg(status_color(e.status)));
            Row::new(vec![
                status_cell,
                Cell::from(e.source_branch.clone()),
                Cell::from(format!("→ {}", e.target_branch)),
                Cell::from(short_repo(e)),
            ])
        })
        .collect();

    let widths = [
        Constraint::Length(12),
        Constraint::Min(16),
        Constraint::Length(20),
        Constraint::Length(12),
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .block(Block::default().borders(Borders::ALL).title(Span::styled(
            format!(" queue ({}) ", app.entries.len()),
            Style::default().fg(Color::White),
        )))
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");

    frame.render_stateful_widget(table, area, &mut app.table);
}

fn draw_details(frame: &mut Frame, area: Rect, app: &AppState) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(" details ", Style::default().fg(Color::White)));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let text = if let Some(e) = app.selected() {
        let mut lines = vec![
            Line::from(vec![
                Span::styled("id      ", Style::default().fg(Color::DarkGray)),
                Span::raw(e.id.short()),
            ]),
            Line::from(vec![
                Span::styled("status  ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    status_label(e.status),
                    Style::default().fg(status_color(e.status)),
                ),
            ]),
            Line::from(vec![
                Span::styled("branch  ", Style::default().fg(Color::DarkGray)),
                Span::raw(format!("{} → {}", e.source_branch, e.target_branch)),
            ]),
            Line::from(vec![
                Span::styled("worktree", Style::default().fg(Color::DarkGray)),
                Span::raw(format!(" {}", e.source_worktree.display())),
            ]),
        ];
        if let Some(reason) = e.failure_reason {
            lines.push(Line::from(vec![
                Span::styled("reason  ", Style::default().fg(Color::DarkGray)),
                Span::styled(reason.to_string(), Style::default().fg(Color::Red)),
            ]));
        }
        if let Some(msg) = &e.message {
            lines.push(Line::from(vec![
                Span::styled("message ", Style::default().fg(Color::DarkGray)),
                Span::raw(msg.clone()),
            ]));
        }
        Paragraph::new(lines)
    } else {
        Paragraph::new(Span::styled(
            "(no entry selected)",
            Style::default().fg(Color::DarkGray),
        ))
    };

    frame.render_widget(text, inner);
}

fn draw_hints(frame: &mut Frame, area: Rect, app: &AppState) {
    let hint = Line::from(vec![
        hint_key("↑↓"),
        Span::raw(" select   "),
        hint_key("d"),
        Span::raw(" delete-queued   "),
        hint_key("?"),
        Span::raw(" help   "),
        hint_key("q"),
        Span::raw(" quit   "),
        hint_key("Q"),
        Span::raw(" force   "),
        Span::styled(
            format!("· {}", app.status_line),
            Style::default().fg(Color::DarkGray),
        ),
    ]);
    frame.render_widget(Paragraph::new(hint), area);
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
                .fg(Color::Yellow)
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

// --- helpers ---

fn hint_key(s: &str) -> Span<'static> {
    Span::styled(
        format!("[{s}]"),
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )
}

fn status_label(s: QueueStatus) -> &'static str {
    match s {
        QueueStatus::Queued => "Queued",
        QueueStatus::Rebasing => "Rebasing",
        QueueStatus::CIRunning => "CI",
        QueueStatus::Merging => "Merging",
        QueueStatus::NeedsHelp => "NeedsHelp",
        QueueStatus::Merged => "Merged",
        QueueStatus::Failed => "Failed",
        QueueStatus::Cancelled => "Cancelled",
    }
}

fn status_color(s: QueueStatus) -> Color {
    match s {
        QueueStatus::Queued => Color::Gray,
        QueueStatus::Rebasing | QueueStatus::Merging => Color::Cyan,
        QueueStatus::CIRunning => Color::LightBlue,
        QueueStatus::NeedsHelp => Color::Yellow,
        QueueStatus::Merged => Color::Green,
        QueueStatus::Failed => Color::Red,
        QueueStatus::Cancelled => Color::DarkGray,
    }
}

fn short_repo(e: &crate::core::queue::QueueEntry) -> String {
    // No repo name in scope here; use the repo id's short form.
    e.repo_id.short()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_reserves_sprite_in_top_left() {
        let area = Rect::new(0, 0, 100, 30);
        let l = compute_layout(area);
        assert_eq!(l.sprite.x, 0);
        assert_eq!(l.sprite.y, 0);
        assert_eq!(l.sprite.width, SPRITE_COLS + 4);
        // Sprite-row height (the inner part is SPRITE_ROWS).
        assert_eq!(l.sprite.height, SPRITE_ROWS);
        // Queue starts to the right of the sprite column.
        assert_eq!(l.queue.x, SPRITE_COLS + 4);
        // Hints are exactly one row at the bottom.
        assert_eq!(l.hints.height, 1);
        assert_eq!(l.hints.y + l.hints.height, area.y + area.height);
    }

    #[test]
    fn layout_survives_small_screens() {
        // 30x10 is well below the comfortable size but should not panic.
        let area = Rect::new(0, 0, 30, 10);
        let l = compute_layout(area);
        assert!(l.queue.width > 0);
        assert!(l.details.height >= 1);
    }
}
