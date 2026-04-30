use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, TableState, Wrap},
};

use crate::app::{
    App, AppMode, EditClickTarget, EditForm, FormField, StatusKind, TextInput, UiRect, VisibleRow,
};
use crate::cron::FIELD_HELP;

// ── Palette ───────────────────────────────────────────────────────────────────

const C_ACCENT: Color = Color::Rgb(137, 180, 250); // blue
const C_GOLD: Color = Color::Rgb(249, 226, 175); // yellow
const C_GREEN: Color = Color::Rgb(166, 227, 161); // green
const C_MUTED: Color = Color::Rgb(127, 132, 156); // overlay1
const C_ERROR: Color = Color::Rgb(243, 139, 168); // red
const C_NEXT: Color = Color::Rgb(148, 226, 213); // teal
const C_SEL_BG: Color = Color::Rgb(88, 91, 112);
const C_CMT: Color = Color::Rgb(166, 173, 200); // subtext1
const C_DIM: Color = Color::Rgb(69, 71, 90); // surface1
const C_HOV: Color = Color::Rgb(250, 179, 135); // peach
const C_BAR_LOW: Color = Color::Rgb(94, 129, 172); // soft blue variant
const C_BAR_MED: Color = Color::Rgb(137, 220, 235); // sky

// ── Tab helpers ───────────────────────────────────────────────────────────────

/// Expand tab characters to 8-column tabstops (POSIX / crontab convention).
fn expand_tabs(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut col = 0usize;
    for c in s.chars() {
        if c == '\t' {
            let spaces = 8 - (col % 8);
            for _ in 0..spaces {
                out.push(' ');
            }
            col += spaces;
        } else {
            out.push(c);
            col += 1;
        }
    }
    out
}

/// Return the visual column of `byte_pos` in `s`, expanding tabs at 8-wide stops.
fn visual_col(s: &str, byte_pos: usize) -> usize {
    let mut col = 0usize;
    for c in s[..byte_pos].chars() {
        if c == '\t' {
            col = (col / 8 + 1) * 8;
        } else {
            col += 1;
        }
    }
    col
}

// ── Root ──────────────────────────────────────────────────────────────────────

pub fn render(f: &mut Frame, app: &mut App) {
    app.clear_mouse_regions();
    let root = f.area();
    let chunks = Layout::vertical([
        Constraint::Min(0),    // table
        Constraint::Length(6), // aggregate timeline — always visible
        Constraint::Length(4), // footer
    ])
    .split(root);

    render_table(f, app, chunks[0]);
    // Store table geometry for mouse hit-testing (3=border+header+blank)
    app.table_top_row = chunks[0].y + 1 + 2; // border + "Schedule|..." header + blank margin
    render_aggregate_timeline(f, app, chunks[1]);
    render_footer(f, app, chunks[2]);

    match &app.mode {
        AppMode::EditEntry => render_edit_modal(f, app, root),
        AppMode::EditComment => render_comment_modal(f, app, root),
        AppMode::Info => render_info_panel(f, app, root),
        AppMode::ConfirmDelete => render_confirm(
            f,
            app,
            "Delete Row",
            "Delete this row?",
            "[y] Yes    [n] Cancel",
            C_ERROR,
            root,
        ),
        AppMode::ConfirmQuit => render_confirm(
            f,
            app,
            "Unsaved Changes",
            "You have unsaved changes.",
            "[s] Save & quit    [y] Discard & quit    [n] Cancel",
            C_GOLD,
            root,
        ),
        AppMode::Help => render_help(f, app, root),
        AppMode::Normal => {}
    }
}

// ── Main table ────────────────────────────────────────────────────────────────

fn render_table(f: &mut Frame, app: &mut App, area: Rect) {
    let visible = app.visible_rows();
    let u24 = app.use_24h;

    if visible.is_empty() {
        f.render_widget(
            Paragraph::new(vec![
                Line::from(""),
                Line::from(Span::styled(
                    "No cron jobs yet.",
                    Style::default().fg(C_MUTED),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "Press  n  to add your first job.",
                    Style::default().fg(Color::Gray),
                )),
            ])
            .alignment(Alignment::Center)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(C_MUTED)),
            ),
            area,
        );
        return;
    }

    let header = Row::new(vec![
        Cell::from(" ").style(Style::default().fg(C_GOLD).add_modifier(Modifier::BOLD)),
        Cell::from("Schedule").style(Style::default().fg(C_GOLD).add_modifier(Modifier::BOLD)),
        Cell::from("Description").style(Style::default().fg(C_GOLD).add_modifier(Modifier::BOLD)),
        Cell::from("Next Run").style(Style::default().fg(C_GOLD).add_modifier(Modifier::BOLD)),
        Cell::from("Command").style(Style::default().fg(C_GOLD).add_modifier(Modifier::BOLD)),
    ])
    .bottom_margin(1);

    let sel = app.selected;
    let mut comment_overlays: Vec<(usize, String, bool)> = Vec::new();
    let rows: Vec<Row> = visible
        .iter()
        .enumerate()
        .map(|(idx, row)| {
            let is_sel = idx == sel;
            let bg = if is_sel { C_SEL_BG } else { Color::Reset };

            match row {
                VisibleRow::Comment(li) => {
                    let line = if let crate::cron::CrontabLine::Comment(s) = &app.lines[*li] {
                        if s.is_empty() {
                            "#".to_string()
                        } else {
                            s.clone()
                        }
                    } else {
                        String::new()
                    };
                    comment_overlays.push((idx, line, is_sel));
                    Row::new(vec![
                        Cell::from("").style(Style::default().bg(bg)),
                        Cell::from("").style(Style::default().bg(bg)),
                        Cell::from("").style(Style::default().bg(bg)),
                        Cell::from("").style(Style::default().bg(bg)),
                        Cell::from("").style(Style::default().fg(C_CMT).bg(bg).add_modifier(
                            if is_sel {
                                Modifier::BOLD
                            } else {
                                Modifier::empty()
                            },
                        )),
                    ])
                }
                VisibleRow::Entry(li) => {
                    let e = if let crate::cron::CrontabLine::Entry(e) = &app.lines[*li] {
                        e
                    } else {
                        return Row::new(vec![Cell::from("")]);
                    };
                    let off = !e.enabled;
                    let dot = if e.enabled {
                        Span::styled("●", Style::default().fg(C_GREEN).bg(bg))
                    } else {
                        Span::styled("○", Style::default().fg(Color::White).bg(bg))
                    };
                    let next_s = if e.enabled {
                        e.schedule.next_run(u24).unwrap_or_else(|| "—".into())
                    } else {
                        "disabled".into()
                    };
                    let (sf, df, nf, tf) = if off {
                        (C_MUTED, C_MUTED, C_MUTED, C_MUTED)
                    } else {
                        (C_ACCENT, C_GOLD, C_NEXT, Color::White)
                    };
                    Row::new(vec![
                        Cell::from(Line::from(dot)),
                        Cell::from(e.schedule.display()).style(Style::default().fg(sf).bg(bg)),
                        Cell::from(e.schedule.describe(u24)).style(Style::default().fg(df).bg(bg)),
                        Cell::from(next_s).style(Style::default().fg(nf).bg(bg)),
                        Cell::from(e.command.as_str()).style(Style::default().fg(tf).bg(bg)),
                    ])
                }
            }
        })
        .collect();

    let widths = [
        Constraint::Length(3),
        Constraint::Length(18),
        Constraint::Length(32),
        Constraint::Length(22),
        Constraint::Min(10),
    ];
    let source = truncate_middle(&app.source_label(), area.width.saturating_sub(44) as usize);
    let left_title = format!(
        " {}  ·  {} job{} ",
        source,
        app.entry_count(),
        if app.entry_count() == 1 { "" } else { "s" }
    );
    let right_title = format!(
        " {}  ·  {} ",
        if app.use_24h { "24h" } else { "12h" },
        if app.dirty { "unsaved" } else { "ready" }
    );
    let mut state = TableState::default();
    state.select(Some(sel));
    f.render_stateful_widget(
        Table::new(rows, widths)
            .header(header)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(C_MUTED))
                    .title(left_title)
                    .title_style(Style::default().fg(C_ACCENT).add_modifier(Modifier::BOLD)),
            )
            .highlight_symbol("")
            .row_highlight_style(Style::default().bg(C_SEL_BG).add_modifier(Modifier::BOLD)),
        area,
        &mut state,
    );

    // Keep comment overlays in sync with table scrolling on small screens.
    let scroll = state.offset();
    let visible_body_rows = area.height.saturating_sub(4) as usize;

    if area.width > 2 {
        let title_area = Rect::new(area.x + 1, area.y, area.width.saturating_sub(2), 1);
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                right_title,
                Style::default().fg(C_NEXT),
            )))
            .alignment(Alignment::Right),
            title_area,
        );
    }

    // Paint comment rows over the table so they read as one full-width line.
    let inner_w = area.width.saturating_sub(2);
    for (idx, line, is_sel) in comment_overlays {
        if visible_body_rows == 0 || idx < scroll || idx >= scroll + visible_body_rows {
            continue;
        }
        let y = area.y + 1 + 2 + (idx - scroll) as u16; // top border + header + margin + viewport row
        if y >= area.y + area.height.saturating_sub(1) {
            continue;
        }
        let row_area = Rect::new(area.x + 1, y, inner_w, 1);
        f.render_widget(Clear, row_area);
        let bg = if is_sel { C_SEL_BG } else { Color::Reset };
        // FIX: expand tabs so \t characters display correctly in the table view
        f.render_widget(
            Paragraph::new(Line::from(expand_tabs(&line))).style(
                Style::default().fg(C_CMT).bg(bg).add_modifier(if is_sel {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
            ),
            row_area,
        );
    }
}

// ── Aggregate timeline (all enabled jobs) ─────────────────────────────────────

fn render_aggregate_timeline(f: &mut Frame, app: &App, area: Rect) {
    let schedules = app.all_schedules();

    // Sum firings per hour across all enabled entries
    let mut totals = [0u32; 24];
    for s in &schedules {
        let fph = s.firings_per_hour();
        for h in 0..24 {
            totals[h] += fph[h] as u32;
        }
    }

    // Firings for the currently hovered entry (Entry rows only; None for comments)
    let hovered: Option<[u8; 24]> = {
        let rows = app.visible_rows();
        rows.get(app.selected).and_then(|r| {
            if let VisibleRow::Entry(li) = r
                && let crate::cron::CrontabLine::Entry(e) = &app.lines[*li]
                && e.enabled
            {
                return Some(e.schedule.firings_per_hour());
            }
            None
        })
    };

    let max = totals.iter().copied().max().unwrap_or(1).max(1);

    // Header: hour labels — highlight hovered hours in gold
    let hdr: Vec<Span> = (0..24u8)
        .map(|h| {
            let is_hov = hovered.map(|fph| fph[h as usize] > 0).unwrap_or(false);
            let fg = if is_hov { C_HOV } else { C_MUTED };
            Span::styled(format!("{:>2} ", h), Style::default().fg(fg))
        })
        .collect();

    // Bar row: base color from density, overridden to C_HOV when hovered entry fires
    let bars: Vec<Span> = totals
        .iter()
        .enumerate()
        .map(|(h, &n)| {
            let is_hov = hovered.map(|fph| fph[h] > 0).unwrap_or(false);
            let (ch, base_color) = if n == 0 {
                ("░░ ", C_DIM)
            } else {
                let frac = n as f32 / max as f32;
                match (frac * 4.0) as u8 {
                    0 => ("▂▂ ", C_BAR_LOW),
                    1 => ("▄▄ ", C_BAR_LOW),
                    2 => ("▆▆ ", C_BAR_MED),
                    _ => ("██ ", C_NEXT),
                }
            };
            let color = if is_hov { C_HOV } else { base_color };
            Span::styled(ch, Style::default().fg(color))
        })
        .collect();

    // AM/PM markers — highlight if any hovered hour in that half
    let markers: Vec<Span> = (0..24u8)
        .map(|h| {
            let lbl = match h {
                0 => "AM",
                12 => "PM",
                _ => "  ",
            };
            let is_hov = hovered.map(|fph| fph[h as usize] > 0).unwrap_or(false);
            let fg = if is_hov && lbl != "  " {
                C_HOV
            } else {
                C_MUTED
            };
            Span::styled(format!("{:<3}", lbl), Style::default().fg(fg))
        })
        .collect();

    // Legend
    let total_jobs = schedules.len();
    let mut legend_spans = vec![
        Span::styled("░ ", Style::default().fg(C_DIM)),
        Span::raw("idle  "),
        Span::styled("▂ ", Style::default().fg(C_BAR_LOW)),
        Span::raw("low  "),
        Span::styled("▆ ", Style::default().fg(C_BAR_MED)),
        Span::raw("med  "),
        Span::styled("█ ", Style::default().fg(C_NEXT)),
        Span::raw("high  "),
        Span::styled(
            format!(
                "   {} active job{}",
                total_jobs,
                if total_jobs == 1 { "" } else { "s" }
            ),
            Style::default().fg(C_MUTED),
        ),
    ];
    if hovered.is_some() {
        legend_spans.push(Span::styled("   ██ ", Style::default().fg(C_HOV)));
        legend_spans.push(Span::styled("selected", Style::default().fg(C_HOV)));
    }
    let legend = Line::from(legend_spans);

    f.render_widget(
        Paragraph::new(vec![
            Line::from(hdr),
            Line::from(bars),
            Line::from(markers),
            Line::from(""),
            legend,
        ])
        .alignment(Alignment::Center)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(C_MUTED))
                .title(" All Jobs — 24h Firing Pattern ")
                .title_style(Style::default().fg(C_MUTED)),
        ),
        area,
    );
}

// ── Footer ────────────────────────────────────────────────────────────────────

fn render_footer(f: &mut Frame, app: &App, area: Rect) {
    let status = if let Some((msg, kind)) = &app.status {
        let (icon, c) = match kind {
            StatusKind::Success => ("✓", C_GREEN),
            StatusKind::Error => ("✖", C_ERROR),
            StatusKind::Info => ("i", C_ACCENT),
        };
        Line::from(vec![
            Span::styled(
                format!(" {} ", icon),
                Style::default().fg(c).add_modifier(Modifier::BOLD),
            ),
            Span::styled(msg.as_str(), Style::default().fg(c)),
        ])
    } else {
        Line::from(Span::styled(
            " Ready. Select a row and press Enter to edit.",
            Style::default().fg(C_MUTED),
        ))
    };

    let hints = if area.width >= 118 {
        " [N] New  [E] Edit  [R] Raw  [D] Delete  [T] Toggle  [S] Save  |  ↑/↓ Move  Shift+↑/↓ Reorder  [I] Info  [C] Clock  [?] Help  [Q] Quit"
    } else if area.width >= 92 {
        " [N] New  [E] Edit  [R] Raw  [D] Delete  [T] Toggle  [S] Save  |  [I] Info  [C] Clock  [?] Help  [Q] Quit"
    } else {
        " [N] [E] [R] [D] [T] [S]  [I] [C] [?] [Q]"
    };

    if area.height == 0 {
        return;
    }

    let rows = Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).split(area);
    f.render_widget(Paragraph::new(status).alignment(Alignment::Left), rows[0]);
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            hints,
            Style::default().fg(Color::Blue),
        )))
        .alignment(Alignment::Center),
        rows[1],
    );
}

fn truncate_middle(s: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max_chars {
        return s.to_string();
    }
    if max_chars <= 3 {
        return "..."[..max_chars].to_string();
    }
    let left = (max_chars - 3) / 2;
    let right = max_chars - 3 - left;
    format!(
        "{}...{}",
        chars[..left].iter().collect::<String>(),
        chars[chars.len() - right..].iter().collect::<String>()
    )
}

// ── Comment edit modal ────────────────────────────────────────────────────────

fn render_comment_modal(f: &mut Frame, app: &mut App, area: Rect) {
    let Some((input, _)) = app.comment_input.clone() else {
        return;
    };

    let w = 70_u16.min(area.width.saturating_sub(4));
    let modal = centered_rect(w, 9, area);
    render_modal_shell(f, area, modal, " Edit Comment ", C_CMT);
    app.set_modal_bounds(to_ui_rect(modal));

    let inner = inner_rect(modal);
    let rows = Layout::vertical([
        Constraint::Length(3), // text field
        Constraint::Length(1), // spacer
        Constraint::Length(1), // hint
    ])
    .split(inner);
    app.set_comment_input_bounds(to_ui_rect(rows[0]));

    // FIX: expand tabs for display so \t characters render correctly
    let display_value = expand_tabs(&input.value);

    // Text field
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            display_value,
            Style::default().fg(Color::White),
        )))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(C_ACCENT))
                .title(" Comment text (without #) ")
                .title_style(Style::default().fg(C_MUTED)),
        ),
        rows[0],
    );

    // FIX: use visual_col so the cursor lands at the correct terminal column
    // even when tabs precede it (each \t may occupy up to 8 columns).
    let cx = rows[0].x + 1 + visual_col(&input.value, input.cursor) as u16;
    let cy = rows[0].y + 1;
    if cx < rows[0].x + rows[0].width.saturating_sub(1) {
        f.set_cursor_position((cx, cy));
    }

    // Hint
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            " [Enter] Save    [Ctrl+S] Save    [Esc] Cancel    (empty = delete)",
            Style::default().fg(C_MUTED),
        )))
        .alignment(Alignment::Center),
        rows[2],
    );
}

// ── Edit modal (cron entry) ───────────────────────────────────────────────────

pub fn render_edit_modal(f: &mut Frame, app: &mut App, area: Rect) {
    let Some(form) = app.form.clone() else { return };
    let w = 72_u16.min(area.width.saturating_sub(4));
    let h = 24_u16.min(area.height.saturating_sub(4));
    let modal = centered_rect(w, h, area);
    let title = if form.editing_index.is_some() {
        " Edit Cron Job "
    } else {
        " New Cron Job "
    };
    render_modal_shell(f, area, modal, title, C_ACCENT);
    app.set_modal_bounds(to_ui_rect(modal));
    render_form_body(f, app, &form, app.use_24h, inner_rect(modal));
}

fn render_form_body(f: &mut Frame, app: &mut App, form: &EditForm, use_24h: bool, area: Rect) {
    let rows = Layout::vertical([
        Constraint::Length(1), // type toggle
        Constraint::Length(1), // spacer
        Constraint::Length(3), // schedule fields
        Constraint::Length(2), // field help
        Constraint::Length(3), // preview
        Constraint::Length(1), // spacer
        Constraint::Length(3), // command
        Constraint::Min(0),
        Constraint::Length(1), // hints
    ])
    .split(area);

    let (special_toggle, standard_toggle) = render_type_toggle(f, form, rows[0]);

    let sched_area = rows[2];
    let mut targets: Vec<(EditClickTarget, UiRect)> = Vec::new();
    targets.push((EditClickTarget::ToggleSpecial, special_toggle));
    targets.push((EditClickTarget::ToggleStandard, standard_toggle));
    if form.is_special {
        render_field(
            f,
            "Special (@keyword)",
            &form.special,
            form.focused == FormField::Special,
            sched_area,
        );
        targets.push((
            EditClickTarget::Field(FormField::Special),
            to_ui_rect(sched_area),
        ));
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "  @reboot @hourly @daily @weekly @monthly @yearly @annually @midnight",
                Style::default().fg(C_MUTED),
            ))),
            rows[3],
        );
    } else {
        let fcols = Layout::horizontal([
            Constraint::Ratio(1, 5),
            Constraint::Ratio(1, 5),
            Constraint::Ratio(1, 5),
            Constraint::Ratio(1, 5),
            Constraint::Ratio(1, 5),
        ])
        .split(sched_area);
        render_field(
            f,
            "Minute",
            &form.minute,
            form.focused == FormField::Minute,
            fcols[0],
        );
        render_field(
            f,
            "Hour",
            &form.hour,
            form.focused == FormField::Hour,
            fcols[1],
        );
        render_field(
            f,
            "Day",
            &form.day,
            form.focused == FormField::Day,
            fcols[2],
        );
        render_field(
            f,
            "Month",
            &form.month,
            form.focused == FormField::Month,
            fcols[3],
        );
        render_field(
            f,
            "Weekday",
            &form.weekday,
            form.focused == FormField::Weekday,
            fcols[4],
        );
        targets.push((
            EditClickTarget::Field(FormField::Minute),
            to_ui_rect(fcols[0]),
        ));
        targets.push((
            EditClickTarget::Field(FormField::Hour),
            to_ui_rect(fcols[1]),
        ));
        targets.push((EditClickTarget::Field(FormField::Day), to_ui_rect(fcols[2])));
        targets.push((
            EditClickTarget::Field(FormField::Month),
            to_ui_rect(fcols[3]),
        ));
        targets.push((
            EditClickTarget::Field(FormField::Weekday),
            to_ui_rect(fcols[4]),
        ));
        render_field_help(f, form, rows[3]);
    }

    let preview = form.preview(use_24h);
    let pcol = if preview.contains("Invalid") {
        C_ERROR
    } else {
        C_GOLD
    };
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            &preview,
            Style::default().fg(pcol),
        )))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(C_MUTED))
                .title(" Preview ")
                .title_style(Style::default().fg(C_MUTED)),
        ),
        rows[4],
    );

    render_field(
        f,
        "Command",
        &form.command,
        form.focused == FormField::Command,
        rows[6],
    );
    targets.push((
        EditClickTarget::Field(FormField::Command),
        to_ui_rect(rows[6]),
    ));
    app.set_edit_click_targets(targets);

    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            " [Tab/Shift+Tab] Fields    [F1] Toggle    [Ctrl+S] Save    [Esc] Cancel",
            Style::default().fg(C_MUTED),
        )))
        .alignment(Alignment::Center),
        rows[8],
    );
    set_cursor(f, form, area, sched_area);
}

fn render_field_help(f: &mut Frame, form: &EditForm, area: Rect) {
    let (fname, range, examples) = match form.focused {
        FormField::Minute => FIELD_HELP[0],
        FormField::Hour => FIELD_HELP[1],
        FormField::Day => FIELD_HELP[2],
        FormField::Month => FIELD_HELP[3],
        FormField::Weekday => FIELD_HELP[4],
        _ => return,
    };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(format!("  {} ", fname), Style::default().fg(C_ACCENT)),
            Span::styled(format!("[{}]", range), Style::default().fg(C_GOLD)),
            Span::styled("  e.g. ", Style::default().fg(C_MUTED)),
            Span::styled(examples, Style::default().fg(Color::Gray)),
        ])),
        area,
    );
}

fn render_type_toggle(f: &mut Frame, form: &EditForm, area: Rect) -> (UiRect, UiRect) {
    let (sp, st) = if form.is_special {
        (
            Style::default().fg(C_ACCENT).add_modifier(Modifier::BOLD),
            Style::default().fg(C_MUTED),
        )
    } else {
        (
            Style::default().fg(C_MUTED),
            Style::default().fg(C_ACCENT).add_modifier(Modifier::BOLD),
        )
    };

    let prefix = "[F1] ";
    let special = "@Special";
    let sep = "   ·   ";
    let standard = "Standard 5-field";

    let sp_x = area.x + prefix.len() as u16;
    let st_x = sp_x + special.len() as u16 + sep.len() as u16;
    let sp_rect = Rect::new(sp_x, area.y, special.len() as u16, 1);
    let st_rect = Rect::new(st_x, area.y, standard.len() as u16, 1);

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(prefix, Style::default().fg(C_MUTED)),
            Span::styled(special, sp),
            Span::raw(sep),
            Span::styled(standard, st),
        ])),
        area,
    );
    (to_ui_rect(sp_rect), to_ui_rect(st_rect))
}

fn render_field(f: &mut Frame, label: &str, input: &TextInput, focused: bool, area: Rect) {
    let border = if focused {
        Style::default().fg(C_ACCENT)
    } else {
        Style::default().fg(C_MUTED)
    };
    let fg = if focused { Color::White } else { Color::Gray };
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            &input.value,
            Style::default().fg(fg),
        )))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(border)
                .title(format!(" {} ", label))
                .title_style(if focused {
                    Style::default().fg(C_ACCENT)
                } else {
                    Style::default().fg(C_MUTED)
                }),
        ),
        area,
    );
}

fn set_cursor(f: &mut Frame, form: &EditForm, area: Rect, sched_area: Rect) {
    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(3),
        Constraint::Length(2),
        Constraint::Length(3),
        Constraint::Length(1),
        Constraint::Length(3),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .split(area);

    let field_area = match form.focused {
        FormField::Command => rows[6],
        FormField::Special => sched_area,
        _ => {
            let fc = Layout::horizontal([
                Constraint::Ratio(1, 5),
                Constraint::Ratio(1, 5),
                Constraint::Ratio(1, 5),
                Constraint::Ratio(1, 5),
                Constraint::Ratio(1, 5),
            ])
            .split(sched_area);
            match form.focused {
                FormField::Minute => fc[0],
                FormField::Hour => fc[1],
                FormField::Day => fc[2],
                FormField::Month => fc[3],
                FormField::Weekday => fc[4],
                _ => return,
            }
        }
    };
    let input = form.active_input();
    let cx = field_area.x + 1 + input.cursor as u16;
    let cy = field_area.y + 1;
    if cx < field_area.x + field_area.width.saturating_sub(1) {
        f.set_cursor_position((cx, cy));
    }
}

// ── Info panel (per-job) ──────────────────────────────────────────────────────

fn render_info_panel(f: &mut Frame, app: &mut App, area: Rect) {
    // Find the selected entry (skip if cursor is on a comment)
    let entry = app
        .visible_rows()
        .into_iter()
        .nth(app.selected)
        .and_then(|r| {
            if let VisibleRow::Entry(li) = r {
                if let crate::cron::CrontabLine::Entry(e) = &app.lines[li] {
                    Some(e.clone())
                } else {
                    None
                }
            } else {
                None
            }
        });
    let Some(entry) = entry else { return };

    let w = 72_u16.min(area.width.saturating_sub(4));
    let h = 30_u16.min(area.height.saturating_sub(2));
    let popup = centered_rect(w, h, area);
    render_modal_shell(f, area, popup, " Job Info ", C_ACCENT);
    app.set_modal_bounds(to_ui_rect(popup));

    let inner = inner_rect(popup);
    let u24 = app.use_24h;

    let rows = Layout::vertical([
        Constraint::Length(3),  // schedule + command
        Constraint::Length(1),  // spacer
        Constraint::Length(14), // next 10 runs
        Constraint::Length(1),  // spacer
        Constraint::Length(5),  // per-job timeline
        Constraint::Min(0),
        Constraint::Length(1), // hint
    ])
    .split(inner);

    // Schedule / description / command
    f.render_widget(
        Paragraph::new(vec![
            Line::from(vec![
                Span::styled("Schedule:  ", Style::default().fg(C_MUTED)),
                Span::styled(
                    entry.schedule.display(),
                    Style::default().fg(C_ACCENT).add_modifier(Modifier::BOLD),
                ),
                Span::styled("   ", Style::default()),
                Span::styled(entry.schedule.describe(u24), Style::default().fg(C_GOLD)),
            ]),
            Line::from(vec![
                Span::styled("Command:   ", Style::default().fg(C_MUTED)),
                Span::styled(entry.command.as_str(), Style::default().fg(Color::White)),
            ]),
        ])
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(C_MUTED)),
        ),
        rows[0],
    );

    // Next 10 runs
    let runs = entry.schedule.next_n_runs(10, u24);
    let run_lines: Vec<Line> = if runs.is_empty() {
        vec![Line::from(Span::styled(
            "  No future runs calculable (e.g. @reboot)",
            Style::default().fg(C_MUTED),
        ))]
    } else {
        runs.iter()
            .enumerate()
            .map(|(i, (_, s))| {
                Line::from(vec![
                    Span::styled(format!("  {:>2}.  ", i + 1), Style::default().fg(C_MUTED)),
                    Span::styled(s.as_str(), Style::default().fg(C_NEXT)),
                ])
            })
            .collect()
    };
    f.render_widget(
        Paragraph::new(run_lines).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(C_MUTED))
                .title(" Next 10 Runs ")
                .title_style(Style::default().fg(C_MUTED)),
        ),
        rows[2],
    );

    // Per-job timeline
    render_single_timeline(f, entry.schedule.firings_per_hour(), rows[4]);

    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "  [Any key] Close",
            Style::default().fg(C_MUTED),
        )))
        .alignment(Alignment::Center),
        rows[6],
    );
}

fn render_single_timeline(f: &mut Frame, counts: [u8; 24], area: Rect) {
    let hdr: Vec<Span> = (0..24u8)
        .map(|h| Span::styled(format!("{:>2} ", h), Style::default().fg(C_MUTED)))
        .collect();
    let bars: Vec<Span> = counts
        .iter()
        .map(|&n| {
            let (ch, col) = match n {
                0 => ("░░ ", C_DIM),
                1 => ("▒▒ ", C_BAR_LOW),
                2..=5 => ("▓▓ ", C_BAR_MED),
                _ => ("██ ", C_NEXT),
            };
            Span::styled(ch, Style::default().fg(col))
        })
        .collect();
    let markers: Vec<Span> = (0..24u8)
        .map(|h| {
            Span::styled(
                format!(
                    "{:<3}",
                    match h {
                        0 => "AM",
                        12 => "PM",
                        _ => "  ",
                    }
                ),
                Style::default().fg(C_MUTED),
            )
        })
        .collect();
    f.render_widget(
        Paragraph::new(vec![Line::from(hdr), Line::from(bars), Line::from(markers)]).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(C_MUTED))
                .title(" Firing Pattern ")
                .title_style(Style::default().fg(C_MUTED)),
        ),
        area,
    );
}

// ── Confirm dialog ────────────────────────────────────────────────────────────

fn render_confirm(
    f: &mut Frame,
    app: &mut App,
    title: &str,
    msg: &str,
    actions: &str,
    color: Color,
    area: Rect,
) {
    let w = 60_u16.min(area.width.saturating_sub(4));
    let dialog = centered_rect(w, 8, area);
    render_modal_shell(f, area, dialog, &format!(" {} ", title), color);
    app.set_modal_bounds(to_ui_rect(dialog));
    f.render_widget(
        Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(msg, Style::default().fg(Color::White))),
            Line::from(""),
            Line::from(Span::styled(actions, Style::default().fg(C_GOLD))),
            Line::from(""),
        ])
        .alignment(Alignment::Center),
        inner_rect(dialog),
    );
}

// ── Help overlay ──────────────────────────────────────────────────────────────

fn render_help(f: &mut Frame, app: &mut App, area: Rect) {
    let w = 60_u16.min(area.width.saturating_sub(4));
    let h = 32_u16.min(area.height.saturating_sub(4));
    let popup = centered_rect(w, h, area);
    render_modal_shell(f, area, popup, " Help ", C_ACCENT);
    app.set_modal_bounds(to_ui_rect(popup));

    fn sec(s: &'static str) -> Line<'static> {
        Line::from(Span::styled(
            s,
            Style::default().fg(C_ACCENT).add_modifier(Modifier::BOLD),
        ))
    }
    fn kv(k: &'static str, d: &'static str) -> Line<'static> {
        Line::from(vec![
            Span::styled(format!("  {:18}", k), Style::default().fg(C_GOLD)),
            Span::raw(d),
        ])
    }

    let lines: Vec<Line> = vec![
        Line::from(""),
        sec("Navigation"),
        Line::from(""),
        kv("↑ / k", "Move cursor up"),
        kv("↓ / j", "Move cursor down"),
        kv("Shift+↑", "Move row up"),
        kv("Shift+↓", "Move row down"),
        Line::from(""),
        sec("List actions"),
        Line::from(""),
        kv("n / a", "Add new cron job"),
        kv("e / Enter", "Edit entry  or  edit comment text"),
        kv("i", "Job info: next 10 runs + timeline"),
        kv("d", "Delete selected row"),
        kv("t", "Toggle enable / disable"),
        kv("r", "Open raw crontab in $VISUAL/$EDITOR"),
        kv("s", "Save crontab"),
        kv("c", "Toggle 12h / 24h clock"),
        kv("q / Esc", "Quit  (prompts if unsaved)"),
        Line::from(""),
        sec("Inside editor"),
        Line::from(""),
        kv("Tab / Shift+Tab", "Next / previous field"),
        kv("F1", "Toggle @Special / 5-field"),
        kv("Ctrl+S", "Save entry"),
        kv("Enter", "Advance / save on Command"),
        kv("Esc", "Cancel"),
        Line::from(""),
        Line::from(Span::styled(
            "  Any key to close.",
            Style::default().fg(C_MUTED),
        )),
    ];

    f.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(Block::default()),
        inner_rect(popup),
    );
}

fn render_modal_shell(f: &mut Frame, root: Rect, modal: Rect, title: &str, color: Color) {
    f.render_widget(
        Block::default().style(Style::default().bg(Color::Rgb(10, 12, 18))),
        root,
    );
    f.render_widget(Clear, modal);
    f.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(color))
            .title(title)
            .title_alignment(Alignment::Center)
            .title_style(Style::default().fg(color).add_modifier(Modifier::BOLD)),
        modal,
    );
}

// ── Geometry ──────────────────────────────────────────────────────────────────

fn centered_rect(w: u16, h: u16, area: Rect) -> Rect {
    Rect::new(
        area.x + (area.width.saturating_sub(w)) / 2,
        area.y + (area.height.saturating_sub(h)) / 2,
        w.min(area.width),
        h.min(area.height),
    )
}

fn inner_rect(r: Rect) -> Rect {
    Rect::new(
        r.x + 1,
        r.y + 1,
        r.width.saturating_sub(2),
        r.height.saturating_sub(2),
    )
}

fn to_ui_rect(r: Rect) -> UiRect {
    UiRect {
        x: r.x,
        y: r.y,
        width: r.width,
        height: r.height,
    }
}
