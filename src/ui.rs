use ratatui::{
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, TableState, Wrap},
    Frame,
};

use crate::app::{App, AppMode, EditForm, FormField, StatusKind, TextInput};
use crate::cron::{SPECIALS, FIELD_HELP};

// ── Palette ───────────────────────────────────────────────────────────────────

const C_ACCENT: Color = Color::Cyan;
const C_GOLD:   Color = Color::Yellow;
const C_GREEN:  Color = Color::Green;
const C_MUTED:  Color = Color::DarkGray;
const C_ERROR:  Color = Color::Red;
const C_NEXT:   Color = Color::Rgb(100, 210, 180);
const C_SEL_BG: Color = Color::Rgb(30, 50, 65);
const C_DIM:    Color = Color::Rgb(60, 60, 80);

// ── Root ──────────────────────────────────────────────────────────────────────

pub fn render(f: &mut Frame, app: &App) {
    let root   = f.area();
    let chunks = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(0),
        Constraint::Length(4),
    ]).split(root);

    render_header(f, chunks[0], app);
    render_table(f, app, chunks[1]);
    render_footer(f, app, chunks[2]);

    match &app.mode {
        AppMode::EditEntry     => render_edit_modal(f, app, root),
        AppMode::Info          => render_info_panel(f, app, root),
        AppMode::ConfirmDelete => render_confirm(f, "Delete Entry",
            "Delete this entry?", "[y] Yes    [n] Cancel", C_ERROR, root),
        AppMode::ConfirmQuit   => render_confirm(f, "Unsaved Changes",
            "You have unsaved changes.",
            "[s] Save & quit    [y] Discard & quit    [n] Cancel", C_GOLD, root),
        AppMode::Help          => render_help(f, root),
        AppMode::Normal        => {}
    }
}

// ── Header ────────────────────────────────────────────────────────────────────

fn render_header(f: &mut Frame, area: Rect, app: &App) {
    let dirty    = if app.dirty { Span::styled("  ✦ unsaved", Style::default().fg(C_GOLD)) } else { Span::raw("") };
    let clock_lbl = if app.use_24h { "24h" } else { "12h" };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("  cronv", Style::default().fg(C_ACCENT).add_modifier(Modifier::BOLD)),
            Span::styled(format!("  ·  {} job{}  ·  ", app.entry_count(),
                if app.entry_count() == 1 { "" } else { "s" }), Style::default().fg(C_MUTED)),
            Span::styled(app.source_label(), Style::default().fg(Color::Gray)),
            Span::styled(format!("  [{}]", clock_lbl), Style::default().fg(C_MUTED)),
            dirty,
        ])).block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(C_ACCENT))),
        area,
    );
}

// ── Main table ────────────────────────────────────────────────────────────────

fn render_table(f: &mut Frame, app: &App, area: Rect) {
    let entries = app.entries();
    let u24 = app.use_24h;

    if entries.is_empty() {
        f.render_widget(
            Paragraph::new(vec![
                Line::from(""),
                Line::from(Span::styled("No cron jobs yet.", Style::default().fg(C_MUTED))),
                Line::from(""),
                Line::from(Span::styled("Press  n  to add your first job.", Style::default().fg(Color::Gray))),
            ])
            .alignment(Alignment::Center)
            .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(C_MUTED))),
            area,
        );
        return;
    }

    let header = Row::new(vec![
        Cell::from(" ").style(Style::default().fg(C_ACCENT).add_modifier(Modifier::BOLD)),
        Cell::from("Schedule").style(Style::default().fg(C_ACCENT).add_modifier(Modifier::BOLD)),
        Cell::from("Description").style(Style::default().fg(C_ACCENT).add_modifier(Modifier::BOLD)),
        Cell::from("Next Run").style(Style::default().fg(C_ACCENT).add_modifier(Modifier::BOLD)),
        Cell::from("Command").style(Style::default().fg(C_ACCENT).add_modifier(Modifier::BOLD)),
    ]).bottom_margin(1);

    let sel  = app.selected;
    let rows: Vec<Row> = entries.iter().enumerate().map(|(idx, (_, e))| {
        let is_sel = idx == sel;
        let bg  = if is_sel { C_SEL_BG } else { Color::Reset };
        let off = !e.enabled;
        let dot = if e.enabled { Span::styled("●", Style::default().fg(C_GREEN).bg(bg)) }
                  else         { Span::styled("○", Style::default().fg(C_MUTED).bg(bg)) };
        let next_str = if e.enabled { e.schedule.next_run(u24).unwrap_or_else(|| "—".into()) }
                       else { "disabled".into() };
        let (sf, df, nf, tf) = if off { (C_MUTED,C_MUTED,C_MUTED,C_MUTED) }
                                else  { (C_ACCENT,C_GOLD,C_NEXT,Color::White) };
        Row::new(vec![
            Cell::from(Line::from(dot)),
            Cell::from(e.schedule.display()).style(Style::default().fg(sf).bg(bg)),
            Cell::from(e.schedule.describe(u24)).style(Style::default().fg(df).bg(bg)),
            Cell::from(next_str).style(Style::default().fg(nf).bg(bg)),
            Cell::from(e.command.as_str()).style(Style::default().fg(tf).bg(bg)),
        ])
    }).collect();

    let widths = [
        Constraint::Length(3), Constraint::Length(18), Constraint::Length(34),
        Constraint::Length(22), Constraint::Min(10),
    ];
    let mut state = TableState::default();
    state.select(Some(sel));
    f.render_stateful_widget(
        Table::new(rows, widths)
            .header(header)
            .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(C_MUTED)))
            .highlight_symbol("▶ ")
            .highlight_style(Style::default().add_modifier(Modifier::BOLD)),
        area, &mut state,
    );
}

// ── Footer ────────────────────────────────────────────────────────────────────

fn render_footer(f: &mut Frame, app: &App, area: Rect) {
    let status = if let Some((msg, kind)) = &app.status {
        let color = match kind {
            StatusKind::Success => C_GREEN, StatusKind::Error => C_ERROR, StatusKind::Info => C_ACCENT,
        };
        Line::from(Span::styled(msg.as_str(), Style::default().fg(color)))
    } else { Line::from("") };
    f.render_widget(
        Paragraph::new(vec![
            status,
            Line::from(Span::styled(
                " n  New    e  Edit    i  Info    d  Delete    t  Toggle    s  Save    c  Clock    ?  Help    q  Quit",
                Style::default().fg(C_MUTED),
            )),
        ])
        .alignment(Alignment::Center)
        .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(C_MUTED))),
        area,
    );
}

// ── Info panel ────────────────────────────────────────────────────────────────

fn render_info_panel(f: &mut Frame, app: &App, area: Rect) {
    let Some((_, entry)) = app.entries().into_iter().nth(app.selected) else { return };

    let w = 78_u16.min(area.width.saturating_sub(4));
    let h = 36_u16.min(area.height.saturating_sub(2));
    let popup = centered_rect(w, h, area);
    f.render_widget(Clear, popup);

    let inner = inner_rect(popup);
    let u24   = app.use_24h;

    // Layout: schedule/desc | next runs | timeline
    let rows = Layout::vertical([
        Constraint::Length(3),  // schedule + description
        Constraint::Length(1),  // spacer
        Constraint::Length(14), // next runs (up to 10)
        Constraint::Length(1),  // spacer
        Constraint::Length(5),  // timeline
        Constraint::Min(0),
        Constraint::Length(1),  // hint
    ]).split(inner);

    // ── Schedule & description ────────────────────────────────────────────────
    let sched_str = entry.schedule.display();
    let desc_str  = entry.schedule.describe(u24);
    f.render_widget(
        Paragraph::new(vec![
            Line::from(vec![
                Span::styled("Schedule:  ", Style::default().fg(C_MUTED)),
                Span::styled(&sched_str, Style::default().fg(C_ACCENT).add_modifier(Modifier::BOLD)),
            ]),
            Line::from(vec![
                Span::styled("Frequency: ", Style::default().fg(C_MUTED)),
                Span::styled(&desc_str, Style::default().fg(C_GOLD)),
            ]),
            Line::from(vec![
                Span::styled("Command:   ", Style::default().fg(C_MUTED)),
                Span::styled(entry.command.as_str(), Style::default().fg(Color::White)),
            ]),
        ])
        .block(Block::default().borders(Borders::ALL)
            .border_style(Style::default().fg(C_MUTED))),
        rows[0],
    );

    // ── Next runs ─────────────────────────────────────────────────────────────
    let runs = entry.schedule.next_n_runs(10, u24);
    let run_lines: Vec<Line> = if runs.is_empty() {
        vec![Line::from(Span::styled("  No future runs (e.g. @reboot)", Style::default().fg(C_MUTED)))]
    } else {
        runs.iter().enumerate().map(|(i, (_, s))| {
            Line::from(vec![
                Span::styled(format!("  {:>2}.  ", i + 1), Style::default().fg(C_MUTED)),
                Span::styled(s.as_str(), Style::default().fg(C_NEXT)),
            ])
        }).collect()
    };

    f.render_widget(
        Paragraph::new(run_lines)
            .block(Block::default().borders(Borders::ALL)
                .border_style(Style::default().fg(C_MUTED))
                .title(" Next 10 Runs ")
                .title_style(Style::default().fg(C_MUTED))),
        rows[2],
    );

    // ── 24-hour timeline ──────────────────────────────────────────────────────
    render_timeline(f, entry.schedule.firings_per_hour(), rows[4]);

    // ── Hint ──────────────────────────────────────────────────────────────────
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "  Any key to close", Style::default().fg(C_MUTED),
        ))).alignment(Alignment::Center),
        rows[6],
    );

    // Border
    f.render_widget(
        Block::default().borders(Borders::ALL)
            .border_style(Style::default().fg(C_ACCENT))
            .title(" Job Info ")
            .title_alignment(Alignment::Center)
            .title_style(Style::default().fg(C_ACCENT).add_modifier(Modifier::BOLD)),
        popup,
    );
}

fn render_timeline(f: &mut Frame, counts: [u8; 24], area: Rect) {
    // Header row: hour numbers
    let header: Vec<Span> = (0..24u8).map(|h| {
        Span::styled(format!("{:>2} ", h), Style::default().fg(C_MUTED))
    }).collect();

    // Bar row: colored blocks by density
    let bars: Vec<Span> = counts.iter().map(|&n| {
        let (ch, color) = match n {
            0       => ("░░ ", C_DIM),
            1       => ("▒▒ ", Color::Rgb(30, 140, 130)),
            2..=5   => ("▓▓ ", Color::Rgb(60, 185, 165)),
            6..=11  => ("██ ", C_NEXT),
            _       => ("██ ", Color::White),
        };
        Span::styled(ch, Style::default().fg(color))
    }).collect();

    // AM/PM marker row
    let markers: Vec<Span> = (0..24u8).map(|h| {
        let label = match h { 0 => "AM", 12 => "PM", _ => "  " };
        Span::styled(format!("{:<3}", label), Style::default().fg(C_MUTED))
    }).collect();

    let legend = Line::from(vec![
        Span::styled("  ░░ ", Style::default().fg(C_DIM)),
        Span::raw("none  "),
        Span::styled("▒▒ ", Style::default().fg(Color::Rgb(30,140,130))),
        Span::raw("1  "),
        Span::styled("▓▓ ", Style::default().fg(Color::Rgb(60,185,165))),
        Span::raw("2–5  "),
        Span::styled("██ ", Style::default().fg(C_NEXT)),
        Span::raw("6–11  "),
        Span::styled("██ ", Style::default().fg(Color::White)),
        Span::raw("12+  firings/hour"),
    ]);

    f.render_widget(
        Paragraph::new(vec![
            Line::from(header),
            Line::from(bars),
            Line::from(markers),
            Line::from(""),
            legend,
        ])
        .block(Block::default().borders(Borders::ALL)
            .border_style(Style::default().fg(C_MUTED))
            .title(" 24-Hour Firing Pattern ")
            .title_style(Style::default().fg(C_MUTED))),
        area,
    );
}

// ── Edit modal ────────────────────────────────────────────────────────────────

pub fn render_edit_modal(f: &mut Frame, app: &App, area: Rect) {
    let Some(form) = &app.form else { return };

    let typed    = form.special.value.trim().to_lowercase();
    let is_known = SPECIALS.iter().any(|s| s.keyword == typed);
    let show_kw  = form.is_special && form.focused == FormField::Special && !is_known;

    let w = if show_kw { 88_u16 } else { 72_u16 }.min(area.width.saturating_sub(4));
    let h = 24_u16.min(area.height.saturating_sub(4));
    let modal = centered_rect(w, h, area);
    f.render_widget(Clear, modal);

    let title = if form.editing_index.is_some() { " Edit Cron Job " } else { " New Cron Job " };
    f.render_widget(
        Block::default().borders(Borders::ALL)
            .border_style(Style::default().fg(C_ACCENT))
            .title(title).title_alignment(Alignment::Center),
        modal,
    );

    let inner = inner_rect(modal);
    if show_kw {
        let cols = Layout::horizontal([Constraint::Min(0), Constraint::Length(34)]).split(inner);
        render_form_body(f, form, app.use_24h, cols[0]);
        render_special_panel(f, form, cols[1]);
    } else {
        render_form_body(f, form, app.use_24h, inner);
    }
}

fn render_form_body(f: &mut Frame, form: &EditForm, use_24h: bool, area: Rect) {
    let rows = Layout::vertical([
        Constraint::Length(1), // type toggle
        Constraint::Length(1), // spacer
        Constraint::Length(3), // schedule
        Constraint::Length(2), // field help
        Constraint::Length(3), // preview
        Constraint::Length(1), // spacer
        Constraint::Length(3), // command
        Constraint::Min(0),
        Constraint::Length(1), // hints
    ]).split(area);

    render_type_toggle(f, form, rows[0]);

    let sched_area = rows[2];
    if form.is_special {
        render_field(f, "Special (@keyword)", &form.special, form.focused == FormField::Special, sched_area);
        // For special mode, show plain hint
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "  @reboot @daily @weekly @monthly @yearly @hourly @annually @midnight",
                Style::default().fg(C_MUTED),
            ))),
            rows[3],
        );
    } else {
        let fcols = Layout::horizontal([
            Constraint::Ratio(1,5), Constraint::Ratio(1,5), Constraint::Ratio(1,5),
            Constraint::Ratio(1,5), Constraint::Ratio(1,5),
        ]).split(sched_area);
        render_field(f, "Minute",  &form.minute,  form.focused == FormField::Minute,  fcols[0]);
        render_field(f, "Hour",    &form.hour,    form.focused == FormField::Hour,    fcols[1]);
        render_field(f, "Day",     &form.day,     form.focused == FormField::Day,     fcols[2]);
        render_field(f, "Month",   &form.month,   form.focused == FormField::Month,   fcols[3]);
        render_field(f, "Weekday", &form.weekday, form.focused == FormField::Weekday, fcols[4]);

        // Field-specific help
        render_field_help(f, form, rows[3]);
    }

    let preview = form.preview(use_24h);
    let pcolor  = if preview.contains("Invalid") { C_ERROR } else { C_GOLD };
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(&preview, Style::default().fg(pcolor))))
            .block(Block::default().borders(Borders::ALL)
                .border_style(Style::default().fg(C_MUTED))
                .title(" Preview ")
                .title_style(Style::default().fg(C_MUTED))),
        rows[4],
    );

    render_field(f, "Command", &form.command, form.focused == FormField::Command, rows[6]);

    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            " Tab/↑↓  Navigate    Ctrl+S  Save    F1  Toggle type    Esc  Cancel",
            Style::default().fg(C_MUTED),
        ))).alignment(Alignment::Center),
        rows[8],
    );

    set_cursor(f, form, area, sched_area);
}

/// Show allowed value ranges + examples for the currently focused field.
fn render_field_help(f: &mut Frame, form: &EditForm, area: Rect) {
    let (field_name, range, examples) = match form.focused {
        FormField::Minute  => FIELD_HELP[0],
        FormField::Hour    => FIELD_HELP[1],
        FormField::Day     => FIELD_HELP[2],
        FormField::Month   => FIELD_HELP[3],
        FormField::Weekday => FIELD_HELP[4],
        _                  => return,
    };
    f.render_widget(
        Paragraph::new(vec![
            Line::from(vec![
                Span::styled(format!("  {} ", field_name), Style::default().fg(C_ACCENT)),
                Span::styled(format!("[{}]", range), Style::default().fg(C_GOLD)),
                Span::styled("  e.g. ", Style::default().fg(C_MUTED)),
                Span::styled(examples, Style::default().fg(Color::Gray)),
            ]),
        ]),
        area,
    );
}

// ── @Special reference panel ──────────────────────────────────────────────────

fn render_special_panel(f: &mut Frame, form: &EditForm, area: Rect) {
    let current = form.special.value.trim().to_lowercase();
    let mut lines: Vec<Line> = vec![Line::from("")];
    for s in SPECIALS {
        let matched = current == s.keyword;
        let kw_style = if matched {
            Style::default().fg(C_ACCENT).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(C_GOLD)
        };
        lines.push(Line::from(vec![
            Span::styled(if matched { "▶ " } else { "  " }, Style::default().fg(C_ACCENT)),
            Span::styled(s.keyword, kw_style),
        ]));
        lines.push(Line::from(vec![Span::raw("    "), Span::styled(s.desc, Style::default().fg(Color::Gray))]));
        lines.push(Line::from(""));
    }
    f.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false })
            .block(Block::default().borders(Borders::ALL)
                .border_style(Style::default().fg(C_MUTED))
                .title(" @keywords ")
                .title_style(Style::default().fg(C_MUTED))),
        area,
    );
}

// ── Field helpers ─────────────────────────────────────────────────────────────

fn render_type_toggle(f: &mut Frame, form: &EditForm, area: Rect) {
    let (sp, st) = if form.is_special {
        (Style::default().fg(C_ACCENT).add_modifier(Modifier::BOLD), Style::default().fg(C_MUTED))
    } else {
        (Style::default().fg(C_MUTED), Style::default().fg(C_ACCENT).add_modifier(Modifier::BOLD))
    };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("F1 ", Style::default().fg(C_MUTED)),
            Span::styled("@Special", sp), Span::raw("   ·   "),
            Span::styled("Standard 5-field", st),
        ])),
        area,
    );
}

fn render_field(f: &mut Frame, label: &str, input: &TextInput, focused: bool, area: Rect) {
    let border = if focused { Style::default().fg(C_ACCENT) } else { Style::default().fg(C_MUTED) };
    let fg     = if focused { Color::White } else { Color::Gray };
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(&input.value, Style::default().fg(fg))))
            .block(Block::default().borders(Borders::ALL).border_style(border)
                .title(format!(" {} ", label))
                .title_style(if focused { Style::default().fg(C_ACCENT) } else { Style::default().fg(C_MUTED) })),
        area,
    );
}

fn set_cursor(f: &mut Frame, form: &EditForm, area: Rect, sched_area: Rect) {
    let rows = Layout::vertical([
        Constraint::Length(1), Constraint::Length(1), Constraint::Length(3),
        Constraint::Length(2), Constraint::Length(3), Constraint::Length(1),
        Constraint::Length(3), Constraint::Min(0), Constraint::Length(1),
    ]).split(area);

    let field_area = match form.focused {
        FormField::Command => rows[6],
        FormField::Special => sched_area,
        _ => {
            let fcols = Layout::horizontal([
                Constraint::Ratio(1,5), Constraint::Ratio(1,5), Constraint::Ratio(1,5),
                Constraint::Ratio(1,5), Constraint::Ratio(1,5),
            ]).split(sched_area);
            match form.focused {
                FormField::Minute  => fcols[0], FormField::Hour    => fcols[1],
                FormField::Day     => fcols[2], FormField::Month   => fcols[3],
                FormField::Weekday => fcols[4], _ => return,
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

// ── Confirm dialog ────────────────────────────────────────────────────────────

fn render_confirm(f: &mut Frame, title: &str, msg: &str, actions: &str, color: Color, area: Rect) {
    let w = 60_u16.min(area.width.saturating_sub(4));
    let dialog = centered_rect(w, 7, area);
    f.render_widget(Clear, dialog);
    f.render_widget(
        Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(msg, Style::default().fg(Color::White))),
            Line::from(""),
            Line::from(Span::styled(actions, Style::default().fg(C_GOLD))),
            Line::from(""),
        ])
        .alignment(Alignment::Center)
        .block(Block::default().borders(Borders::ALL)
            .border_style(Style::default().fg(color))
            .title(format!(" {} ", title)).title_alignment(Alignment::Center)
            .title_style(Style::default().fg(color).add_modifier(Modifier::BOLD))),
        dialog,
    );
}

// ── Help overlay ──────────────────────────────────────────────────────────────

fn render_help(f: &mut Frame, area: Rect) {
    let w = 60_u16.min(area.width.saturating_sub(4));
    let h = 32_u16.min(area.height.saturating_sub(4));
    let popup = centered_rect(w, h, area);
    f.render_widget(Clear, popup);

    fn sec(s: &'static str) -> Line<'static> {
        Line::from(Span::styled(s, Style::default().fg(C_ACCENT).add_modifier(Modifier::BOLD)))
    }
    fn kv(k: &'static str, d: &'static str) -> Line<'static> {
        Line::from(vec![
            Span::styled(format!("  {:16}", k), Style::default().fg(C_GOLD)),
            Span::raw(d),
        ])
    }

    let lines: Vec<Line> = vec![
        Line::from(""),
        sec("Navigation"),    Line::from(""),
        kv("↑ / k",          "Move up"),
        kv("↓ / j",          "Move down"),
        Line::from(""),
        sec("List actions"),  Line::from(""),
        kv("n / a",          "Add new cron job"),
        kv("e / Enter",      "Edit selected entry"),
        kv("i",              "Show job info, next runs & timeline"),
        kv("d",              "Delete selected entry"),
        kv("t",              "Toggle enable / disable"),
        kv("s",              "Save crontab"),
        kv("c",              "Toggle 12h / 24h clock"),
        kv("q / Esc",        "Quit  (prompts if unsaved)"),
        Line::from(""),
        sec("Inside editor"), Line::from(""),
        kv("Tab",            "Next field"),
        kv("Shift+Tab",      "Previous field"),
        kv("F1",             "Toggle @Special / 5-field"),
        kv("Ctrl+S",         "Save entry"),
        kv("Enter",          "Advance / save on Command"),
        kv("Esc",            "Cancel edit"),
        Line::from(""),
        Line::from(Span::styled("  Any key to close this panel.", Style::default().fg(C_MUTED))),
    ];

    f.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false })
            .block(Block::default().borders(Borders::ALL)
                .border_style(Style::default().fg(C_ACCENT))
                .title(" Help ").title_alignment(Alignment::Center)
                .title_style(Style::default().fg(C_ACCENT).add_modifier(Modifier::BOLD))),
        popup,
    );
}

// ── Geometry ──────────────────────────────────────────────────────────────────

fn centered_rect(w: u16, h: u16, area: Rect) -> Rect {
    Rect::new(
        area.x + (area.width.saturating_sub(w)) / 2,
        area.y + (area.height.saturating_sub(h)) / 2,
        w.min(area.width), h.min(area.height),
    )
}

fn inner_rect(r: Rect) -> Rect {
    Rect::new(r.x+1, r.y+1, r.width.saturating_sub(2), r.height.saturating_sub(2))
}
