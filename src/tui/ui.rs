//! ratatui drawing.
//!
//! Layout (deliberately minimal: borderless text/status controls on the
//! left, Penrose animation on the right):
//!
//! ```text
//!   MERGEQUEUE                        [penrose tiling animation]
//!   WORKING
//!   feat/auth -> main
//!   rebasing
//!
//!   queued 3
//!   working 1
//!   help 0
//!   done 2
//!
//!   ↑↓ select   d delete   ? help   q quit
//! ```
//!
//! The centerpiece animation is sampled into a Braille wireframe and
//! drawn into the sprite rect with ratatui.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::core::ids::QueueEntryId;
use crate::core::queue::{QueueEntry, QueueEntryDetailStatus, QueueStatus};
use crate::tui::app::AppState;
use crate::tui::sprite::glyph;
use crate::tui::sprite::state::SpriteState;
use crate::tui::sprite::DotBitmap;

/// Preferred width for the borderless text/status column.
pub const LEFT_PANEL_WIDTH: u16 = 68;
/// Gap between the left column and animation stage.
pub const CENTER_GAP: u16 = 2;
/// Minimum terminal width for the left/right layout.
pub const WIDE_LAYOUT_MIN_WIDTH: u16 = 78;
/// Minimum terminal height for the left/right layout.
pub const WIDE_LAYOUT_MIN_HEIGHT: u16 = 14;
/// Rows reserved for the text panel in compact stacked layout.
pub const COMPACT_PANEL_ROWS: u16 = 12;
const APP_BG: Color = Color::Rgb(0x2e, 0x34, 0x40);

/// Per-frame layout. The sprite rect holds the Braille animation; the
/// rest of the rects are for ratatui's own widgets.
#[derive(Debug, Clone, Copy)]
pub struct DrawLayout {
    pub root: Rect,
    pub left_panel: Rect,
    pub sprite: Rect,
    pub compact: bool,
}

pub fn compute_layout(area: Rect) -> DrawLayout {
    if area.width >= WIDE_LAYOUT_MIN_WIDTH && area.height >= WIDE_LAYOUT_MIN_HEIGHT {
        let left_w = LEFT_PANEL_WIDTH.min(area.width.saturating_sub(CENTER_GAP + 20));
        let split = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(left_w),
                Constraint::Length(CENTER_GAP),
                Constraint::Min(10),
            ])
            .split(area);

        return DrawLayout {
            root: area,
            left_panel: split[0],
            sprite: split[2],
            compact: false,
        };
    }

    let top_h = COMPACT_PANEL_ROWS.min(area.height.saturating_sub(1));
    let split = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(top_h), Constraint::Min(1)])
        .split(area);

    DrawLayout {
        root: area,
        left_panel: split[0],
        sprite: split[1],
        compact: true,
    }
}

pub fn draw(frame: &mut Frame, app: &mut AppState) -> DrawLayout {
    let layout = compute_layout(frame.area());

    frame.render_widget(
        Block::default().style(Style::default().bg(APP_BG)),
        layout.root,
    );

    draw_left_panel(frame, layout.left_panel, app, layout.compact);
    // Draw the sampled tiling into the sprite rect with ratatui.
    draw_glyph_animation(frame, layout.sprite, app);
    if app.help_visible {
        draw_help_overlay(frame, layout.root);
    }

    layout
}

/// Sample the tiling edges into a high-res Braille wireframe filling the
/// whole `area`.
///
/// To maximize resolution we use *every* available terminal cell:
/// `dot_w = cols*2`, `dot_h = rows*4` (each Braille cell is a 2×4 dot
/// matrix). The source disk is square and the dot grid usually isn't, so
/// the sampler ([`PenroseAnimator::dot_bitmap`]) preserves the disk's
/// aspect ratio and centers it within the grid — the tiling stays round
/// while the surrounding cells are simply empty.
fn draw_glyph_animation(frame: &mut Frame, area: Rect, app: &mut AppState) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let cols = area.width;
    let rows = area.height;

    let dt = app.frame_dt;
    let state = app.sprite_state;
    // High-res: 2 dots per column, 4 per row → 8 samples per cell, across
    // the entire sprite rect.
    let bitmap = app.animator.dot_bitmap(dt, state, cols * 2, rows * 4);
    let bitmap = smooth_glyph_bitmap(&mut app.glyph_prev_bitmap, bitmap, dt);
    let lines = glyph::render_hex_braille(&bitmap, state);

    frame.render_widget(
        Paragraph::new(lines).style(Style::default().bg(APP_BG)),
        area,
    );
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn smooth_glyph_bitmap(prev: &mut Option<DotBitmap>, current: DotBitmap, dt: f32) -> DotBitmap {
    const MIN_ALPHA: u8 = 6;
    const ATTACK_TAU_SECS: f32 = 0.032;
    const DECAY_TAU_SECS: f32 = 0.075;

    let len = usize::from(current.width) * usize::from(current.height);
    if prev.as_ref().is_none_or(|p| {
        p.width != current.width || p.height != current.height || p.alpha.len() != len
    }) {
        *prev = Some(current.clone());
        return current;
    }

    let dt = dt.max(0.0);
    let attack = (1.0 - (-dt / ATTACK_TAU_SECS).exp()).clamp(0.18, 0.48);
    let decay = (-dt / DECAY_TAU_SECS).exp().clamp(0.74, 0.96);
    let previous = prev.as_ref().expect("previous bitmap checked above");
    let mut smoothed = DotBitmap {
        width: current.width,
        height: current.height,
        dots: vec![false; len],
        alpha: vec![0u8; len],
    };

    for i in 0..len {
        let previous_a = f32::from(previous.alpha[i]);
        let current_a = f32::from(current.alpha[i]);
        let a = if current_a >= previous_a {
            previous_a + (current_a - previous_a) * attack
        } else {
            current_a.max(previous_a * decay)
        }
        .round()
        .clamp(0.0, 255.0) as u8;
        if a >= MIN_ALPHA {
            smoothed.dots[i] = true;
            smoothed.alpha[i] = a;
        }
    }

    *prev = Some(smoothed.clone());
    smoothed
}

fn draw_left_panel(frame: &mut Frame, area: Rect, app: &AppState, compact: bool) {
    let padded = Rect::new(
        area.x + 2,
        area.y + 1,
        area.width.saturating_sub(4),
        area.height.saturating_sub(1),
    );
    if padded.width == 0 || padded.height == 0 {
        return;
    }

    let accent = animation_color(app.sprite_state);

    let mut lines = Vec::new();
    lines.extend(wordmark_lines(accent));
    lines.push(Line::from(""));
    if !compact {
        lines.push(Line::from(""));
        lines.push(Line::from(""));
        lines.push(Line::from(""));
    }
    lines.extend(bucket_lines(
        &app.entries,
        &app.repo_labels,
        &app.expanded_entry_ids,
        app.selected().map(|entry| entry.id),
        app.sprite_tick,
        accent,
        usize::from(padded.width.saturating_sub(4)),
        compact,
    ));

    if compact {
        lines.push(Line::from(""));
        lines.push(hint_line(app));
    } else {
        while lines.len() + 4 < usize::from(padded.height) {
            lines.push(Line::from(""));
        }
        lines.push(hint_line(app));
        lines.push(Line::from(Span::styled(
            app.status_line.clone(),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::ITALIC),
        )));
    }

    frame.render_widget(
        Paragraph::new(lines).style(Style::default().bg(APP_BG)),
        padded,
    );
}

fn wordmark_lines(accent: Color) -> Vec<Line<'static>> {
    let style = Style::default().fg(accent).add_modifier(Modifier::BOLD);
    vec![Line::from(Span::styled("M E R G E Q U E U E", style))]
}

fn bucket_lines(
    entries: &[QueueEntry],
    repo_labels: &std::collections::HashMap<crate::core::ids::RepoId, String>,
    expanded_entry_ids: &std::collections::HashSet<QueueEntryId>,
    selected_entry_id: Option<QueueEntryId>,
    tick: u64,
    accent: Color,
    label_width: usize,
    compact: bool,
) -> Vec<Line<'static>> {
    let queued: Vec<&QueueEntry> = entries
        .iter()
        .filter(|e| e.status == QueueStatus::Queued)
        .collect();
    let working: Vec<&QueueEntry> = entries
        .iter()
        .filter(|e| {
            matches!(
                e.status,
                QueueStatus::Rebasing | QueueStatus::CIRunning | QueueStatus::Merging
            )
        })
        .collect();
    let help: Vec<&QueueEntry> = entries
        .iter()
        .filter(|e| e.status == QueueStatus::NeedsHelp)
        .collect();
    let done: Vec<&QueueEntry> = entries.iter().filter(|e| e.status.is_terminal()).collect();

    let mut lines = Vec::new();
    let section_gap = if compact { 1 } else { 3 };
    append_bucket(
        &mut lines,
        "NEEDS HELP",
        help,
        repo_labels,
        expanded_entry_ids,
        selected_entry_id,
        if entries.iter().any(|e| e.status == QueueStatus::NeedsHelp) {
            Color::Yellow
        } else {
            Color::DarkGray
        },
        None,
        label_width,
    );
    push_blank_lines(&mut lines, section_gap);
    append_bucket(
        &mut lines,
        "IN PROGRESS",
        working,
        repo_labels,
        expanded_entry_ids,
        selected_entry_id,
        Color::Rgb(0xb4, 0x8e, 0xf7),
        Some((tick, accent)),
        label_width,
    );
    push_blank_lines(&mut lines, section_gap);
    append_bucket(
        &mut lines,
        "QUEUED",
        queued,
        repo_labels,
        expanded_entry_ids,
        selected_entry_id,
        Color::Rgb(0x81, 0xa1, 0xc1),
        None,
        label_width,
    );
    push_blank_lines(&mut lines, section_gap);
    append_bucket(
        &mut lines,
        "DONE",
        done,
        repo_labels,
        expanded_entry_ids,
        selected_entry_id,
        Color::Rgb(0x9d, 0xc1, 0x8f),
        None,
        label_width,
    );
    lines
}

fn push_blank_lines(lines: &mut Vec<Line<'static>>, count: usize) {
    for _ in 0..count {
        lines.push(Line::from(""));
    }
}

fn append_bucket(
    lines: &mut Vec<Line<'static>>,
    label: &'static str,
    entries: Vec<&QueueEntry>,
    repo_labels: &std::collections::HashMap<crate::core::ids::RepoId, String>,
    expanded_entry_ids: &std::collections::HashSet<QueueEntryId>,
    selected_entry_id: Option<QueueEntryId>,
    color: Color,
    queued_tick: Option<(u64, Color)>,
    label_width: usize,
) {
    lines.push(section_header_line(label, entries.len(), color));
    if entries.is_empty() {
        lines.push(Line::from(Span::styled(
            "  none",
            Style::default().fg(Color::DarkGray),
        )));
        return;
    }

    let visible_entries = visible_bucket_entries(entries, selected_entry_id);
    for entry in visible_entries {
        let expanded = expanded_entry_ids.contains(&entry.id);
        let selected = selected_entry_id == Some(entry.id);
        lines.push(entry_line(
            entry,
            repo_labels,
            queued_tick,
            label_width,
            expanded,
            selected,
        ));
        if expanded {
            lines.extend(entry_detail_lines(entry, label_width));
        }
    }
}

fn visible_bucket_entries(
    entries: Vec<&QueueEntry>,
    selected_entry_id: Option<QueueEntryId>,
) -> Vec<&QueueEntry> {
    let selected =
        selected_entry_id.and_then(|id| entries.iter().copied().find(|entry| entry.id == id));
    let mut visible: Vec<&QueueEntry> = entries.into_iter().take(4).collect();
    if let Some(selected) = selected {
        if !visible.iter().any(|entry| entry.id == selected.id) {
            if visible.len() == 4 {
                visible.pop();
            }
            visible.push(selected);
        }
    }
    visible
}

fn entry_line(
    entry: &QueueEntry,
    repo_labels: &std::collections::HashMap<crate::core::ids::RepoId, String>,
    queued_tick: Option<(u64, Color)>,
    label_width: usize,
    expanded: bool,
    selected: bool,
) -> Line<'static> {
    const SPINNER: [&str; 10] = ["⠋", "⠙", "⠚", "⠞", "⠖", "⠦", "⠴", "⠲", "⠳", "⠓"];
    let marker = queued_tick.map_or(" ", |(tick, _)| {
        SPINNER[usize::try_from(tick / 14).unwrap_or(0) % SPINNER.len()]
    });
    let marker_color = queued_tick.map_or(Color::Gray, |(_, color)| color);
    let expander = if expanded {
        "▾"
    } else if entry.details.is_some() || entry.message.is_some() {
        "▸"
    } else {
        " "
    };
    let selector = if selected { ">" } else { " " };
    let label_style = if selected {
        Style::default()
            .fg(Color::Rgb(0xb8, 0xff, 0x9d))
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White)
    };

    Line::from(vec![
        Span::styled(" ", Style::default()),
        Span::styled(
            if selected { "‹" } else { " " },
            Style::default().fg(Color::Rgb(0x3a, 0x6f, 0x38)),
        ),
        Span::styled(
            selector,
            Style::default()
                .fg(Color::Rgb(0x9d, 0xff, 0x7a))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            if selected { "›" } else { " " },
            Style::default().fg(Color::Rgb(0x3a, 0x6f, 0x38)),
        ),
        Span::styled(" ", Style::default()),
        Span::styled(expander, Style::default().fg(Color::DarkGray)),
        Span::styled(" ", Style::default()),
        Span::styled(marker, Style::default().fg(marker_color)),
        Span::styled(" ", Style::default()),
        Span::styled(entry_label(entry, repo_labels, label_width), label_style),
    ])
}

fn entry_detail_lines(entry: &QueueEntry, label_width: usize) -> Vec<Line<'static>> {
    const HEAD_PREFIX: &str = "        │ ";
    const ITEM_PREFIX: &str = "        ";
    const BODY_RAIL_PREFIX: &str = "        │   ";
    const BODY_LAST_PREFIX: &str = "            ";

    let mut lines = Vec::new();
    lines.push(Line::from(""));
    if let Some(details) = &entry.details {
        if let Some(headline) = &details.headline {
            let wrap_w = detail_wrap_width(label_width, HEAD_PREFIX.len());
            for chunk in wrap_text(headline, wrap_w) {
                lines.push(Line::from(vec![
                    Span::styled(HEAD_PREFIX, Style::default().fg(Color::DarkGray)),
                    Span::styled(chunk, Style::default().fg(Color::Rgb(0xd8, 0xde, 0xe9))),
                ]));
            }
            if !details.items.is_empty() {
                lines.push(Line::from(vec![Span::styled(
                    "        │",
                    Style::default().fg(Color::DarkGray),
                )]));
            }
        }
        let item_count = details.items.len();
        for (i, item) in details.items.iter().enumerate() {
            let is_last = i + 1 == item_count;
            lines.extend(detail_item_lines(
                item.status,
                &item.title,
                item.detail.as_deref(),
                label_width,
                is_last,
                ITEM_PREFIX,
                BODY_RAIL_PREFIX,
                BODY_LAST_PREFIX,
            ));
            if !is_last {
                lines.push(Line::from(vec![Span::styled(
                    "        │",
                    Style::default().fg(Color::DarkGray),
                )]));
            }
        }
    } else if let Some(message) = &entry.message {
        let wrap_w = detail_wrap_width(label_width, HEAD_PREFIX.len());
        for chunk in wrap_text(message, wrap_w) {
            lines.push(Line::from(vec![
                Span::styled(HEAD_PREFIX, Style::default().fg(Color::DarkGray)),
                Span::styled(chunk, Style::default().fg(Color::Rgb(0xd8, 0xde, 0xe9))),
            ]));
        }
    }
    if lines.len() == 1 {
        lines.push(Line::from(Span::styled(
            "        no details yet",
            Style::default().fg(Color::DarkGray),
        )));
    }
    lines.push(Line::from(""));
    lines
}

fn detail_item_lines(
    status: QueueEntryDetailStatus,
    title: &str,
    detail: Option<&str>,
    label_width: usize,
    is_last: bool,
    item_prefix: &'static str,
    body_rail_prefix: &'static str,
    body_last_prefix: &'static str,
) -> Vec<Line<'static>> {
    let color = detail_status_color(status);
    let branch = if is_last { "└" } else { "├" };
    let body_prefix = if is_last {
        body_last_prefix
    } else {
        body_rail_prefix
    };
    let mut lines = Vec::new();
    lines.push(Line::from(vec![
        Span::raw(item_prefix),
        Span::styled(branch, Style::default().fg(Color::DarkGray)),
        Span::raw(" "),
        Span::styled(status.marker(), Style::default().fg(color)),
        Span::raw(" "),
        Span::styled(
            title.to_string(),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
    ]));
    if let Some(detail) = detail.filter(|detail| !detail.is_empty()) {
        let wrap_w = detail_wrap_width(label_width, body_prefix.len());
        for chunk in wrap_text(detail, wrap_w) {
            lines.push(Line::from(vec![
                Span::styled(body_prefix, Style::default().fg(Color::DarkGray)),
                Span::styled(chunk, Style::default().fg(Color::Gray)),
            ]));
        }
    }
    lines
}

fn detail_wrap_width(label_width: usize, prefix_width: usize) -> usize {
    label_width.saturating_sub(prefix_width).min(46).max(18)
}

fn detail_status_color(status: QueueEntryDetailStatus) -> Color {
    match status {
        QueueEntryDetailStatus::Pending => Color::DarkGray,
        QueueEntryDetailStatus::Running => Color::Rgb(0xb4, 0x8e, 0xf7),
        QueueEntryDetailStatus::Success => Color::Rgb(0x9d, 0xc1, 0x8f),
        QueueEntryDetailStatus::Blocked => Color::Yellow,
        QueueEntryDetailStatus::Info => Color::Gray,
    }
}

fn wrap_text(text: &str, width: usize) -> Vec<String> {
    let width = width.max(12);
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        let sep = usize::from(!current.is_empty());
        if !current.is_empty() && current.chars().count() + sep + word.chars().count() > width {
            lines.push(current);
            current = String::new();
        }
        if word.chars().count() > width {
            if !current.is_empty() {
                lines.push(current);
            }
            let mut chunk = String::new();
            for ch in word.chars() {
                if chunk.chars().count() == width {
                    lines.push(chunk);
                    chunk = String::new();
                }
                chunk.push(ch);
            }
            current = chunk;
            continue;
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
    }
    if !current.is_empty() || lines.is_empty() {
        lines.push(current);
    }
    lines
}

fn entry_label(
    entry: &QueueEntry,
    repo_labels: &std::collections::HashMap<crate::core::ids::RepoId, String>,
    max: usize,
) -> String {
    let owner = repo_labels
        .get(&entry.repo_id)
        .map(String::as_str)
        .or_else(|| {
            entry
                .source_worktree
                .file_name()
                .and_then(|name| name.to_str())
        })
        .unwrap_or("worktree");
    truncate(&format!("{owner}:{}", entry.source_branch), max)
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

fn section_header_line(label: &'static str, count: usize, color: Color) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            label,
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!(" {count}"), Style::default().fg(Color::Gray)),
    ])
}

fn animation_color(state: SpriteState) -> Color {
    match state {
        SpriteState::Idle => Color::Rgb(0x8f, 0xa8, 0x86),
        SpriteState::Working | SpriteState::NeedsHelp => Color::Rgb(0x9d, 0xc1, 0x8f),
    }
}

fn hint_line(app: &AppState) -> Line<'_> {
    let selected = app
        .selected()
        .map(|entry| format!("selected {} {}", entry.id.short(), entry.source_branch))
        .unwrap_or_else(|| "no selection".to_string());
    Line::from(vec![
        Span::styled("↑↓", Style::default().fg(Color::Gray)),
        Span::styled(" select   ", Style::default().fg(Color::DarkGray)),
        Span::styled("Space", Style::default().fg(Color::Gray)),
        Span::styled(" expand   ", Style::default().fg(Color::DarkGray)),
        Span::styled("d", Style::default().fg(Color::Gray)),
        Span::styled(" delete   ", Style::default().fg(Color::DarkGray)),
        Span::styled("t", Style::default().fg(Color::Gray)),
        Span::styled(" agent   ", Style::default().fg(Color::DarkGray)),
        Span::styled("?", Style::default().fg(Color::Gray)),
        Span::styled(" help   ", Style::default().fg(Color::DarkGray)),
        Span::styled("q", Style::default().fg(Color::Gray)),
        Span::styled(" quit   ", Style::default().fg(Color::DarkGray)),
        Span::styled("·  ", Style::default().fg(Color::DarkGray)),
        Span::styled(selected, Style::default().fg(Color::Yellow)),
        Span::styled("  ·  ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            app.status_line.clone(),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::ITALIC),
        ),
    ])
}

fn draw_help_overlay(frame: &mut Frame, area: Rect) {
    let h = 12;
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
        Line::from(" Space            expand/collapse selected entry"),
        Line::from(" t                open selected NeedsHelp agent"),
        Line::from(" d                delete the selected entry (if Queued)"),
        Line::from(" ?                toggle this help overlay"),
        Line::from(" q                quit (soft shutdown)"),
        Line::from(" Q  /  Ctrl-C     force quit (hard shutdown)"),
        Line::from(""),
        Line::from(" NeedsHelp opens the tmux agent session directly."),
        Line::from(" Use `mergequeue retry <id>` after conflicts are fixed."),
    ];

    frame.render_widget(block, overlay);
    frame.render_widget(Paragraph::new(lines), inner);
}

// ---------------------------------------------------------------------
// Pure helpers — no ratatui types in/out, easy to test.
// ---------------------------------------------------------------------

#[cfg(test)]
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

#[cfg(test)]
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ids::{QueueEntryId, RepoId};
    use crate::core::queue::{QueueEntry, QueueEntryDetails};
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
            details: None,
            claimed_by_pid: None,
            claimed_at: None,
        }
    }

    fn line_text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn wide_layout_places_animation_to_the_right() {
        let area = Rect::new(0, 0, 120, 30);
        let l = compute_layout(area);
        assert!(!l.compact);
        assert_eq!(l.left_panel.x, area.x);
        assert_eq!(l.left_panel.width, LEFT_PANEL_WIDTH);
        assert_eq!(l.left_panel.height, area.height);
        assert_eq!(l.sprite.y, area.y);
        assert_eq!(l.sprite.height, area.height);
        assert!(
            l.sprite.x >= l.left_panel.x + l.left_panel.width + CENTER_GAP,
            "sprite should sit to the right of the text panel"
        );
        assert!(
            l.sprite.x + l.sprite.width <= area.x + area.width,
            "sprite must fit horizontally"
        );
    }

    #[test]
    fn compact_layout_stacks_text_above_animation() {
        let area = Rect::new(0, 0, 20, 8);
        let l = compute_layout(area);
        assert!(l.compact);
        assert_eq!(l.left_panel.x, area.x);
        assert_eq!(l.sprite.x, area.x);
        assert!(l.sprite.y >= l.left_panel.y + l.left_panel.height);
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

    #[test]
    fn selected_bucket_entry_stays_visible_past_first_four() {
        let entries = vec![
            entry(QueueStatus::Queued, "a"),
            entry(QueueStatus::Queued, "b"),
            entry(QueueStatus::Queued, "c"),
            entry(QueueStatus::Queued, "d"),
            entry(QueueStatus::Queued, "e"),
        ];
        let selected = entries[4].id;
        let refs: Vec<&QueueEntry> = entries.iter().collect();

        let visible = visible_bucket_entries(refs, Some(selected));

        assert_eq!(visible.len(), 4);
        assert!(visible.iter().any(|entry| entry.id == selected));
        assert!(!visible.iter().any(|entry| entry.source_branch == "d"));
    }

    #[test]
    fn detail_lines_wrap_long_headline_and_details() {
        let mut entry = entry(QueueStatus::Queued, "feature");
        let mut details = QueueEntryDetails::new(
            "queued for atlas-web after the current NeedsHelp item resolves",
        );
        details.push(
            QueueEntryDetailStatus::Info,
            "change",
            Some("narrows refresh token retry backoff in browser tabs".to_string()),
        );
        entry.details = Some(details);

        let lines = entry_detail_lines(&entry, 34);
        let rendered = lines.iter().map(line_text).collect::<Vec<_>>();

        assert!(rendered.iter().all(|line| !line.contains('…')));
        assert!(rendered.iter().any(|line| line.contains("browser")));
        assert!(rendered.iter().all(|line| line.chars().count() <= 34));
    }

    #[test]
    fn wrap_text_keeps_words_when_possible() {
        assert_eq!(
            wrap_text("repo worker is busy with older queue item", 18),
            vec![
                "repo worker is".to_string(),
                "busy with older".to_string(),
                "queue item".to_string()
            ]
        );
    }
}
