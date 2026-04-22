use anyhow::{Context, Result};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::cron::{parse_crontab, serialize_crontab, CronEntry, CronSchedule, CrontabLine};

// ── Modes ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum AppMode {
    Normal,
    EditEntry,
    EditComment,
    ConfirmDelete,
    ConfirmQuit,
    Info,
    Help,
}

// ── Text input ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct TextInput {
    pub value:  String,
    pub cursor: usize,
}

impl TextInput {
    pub fn new(v: impl Into<String>) -> Self {
        let value  = v.into();
        let cursor = value.len();
        Self { value, cursor }
    }
    pub fn insert(&mut self, c: char) {
        self.value.insert(self.cursor, c);
        self.cursor += c.len_utf8();
    }
    pub fn backspace(&mut self) {
        if self.cursor > 0 {
            let prev = self.value[..self.cursor]
                .char_indices().last().map(|(i, _)| i).unwrap_or(0);
            self.value.remove(prev);
            self.cursor = prev;
        }
    }
    pub fn delete_char(&mut self) {
        if self.cursor < self.value.len() { self.value.remove(self.cursor); }
    }
    pub fn move_left(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            while self.cursor > 0 && !self.value.is_char_boundary(self.cursor) { self.cursor -= 1; }
        }
    }
    pub fn move_right(&mut self) {
        if self.cursor < self.value.len() {
            self.cursor += 1;
            while self.cursor < self.value.len() && !self.value.is_char_boundary(self.cursor) {
                self.cursor += 1;
            }
        }
    }
    pub fn home(&mut self) { self.cursor = 0; }
    pub fn end(&mut self)  { self.cursor = self.value.len(); }
}

// ── Edit form (for cron entries) ──────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum FormField { Minute, Hour, Day, Month, Weekday, Special, Command }

#[derive(Debug, Clone)]
pub struct EditForm {
    pub is_special:    bool,
    pub special:       TextInput,
    pub minute:        TextInput,
    pub hour:          TextInput,
    pub day:           TextInput,
    pub month:         TextInput,
    pub weekday:       TextInput,
    pub command:       TextInput,
    pub focused:       FormField,
    pub editing_index: Option<usize>, // index into App::lines
    pub insert_after:  Option<usize>, // line index to insert after (new entries only)
}

impl EditForm {
    pub fn from_entry(e: &CronEntry, idx: Option<usize>) -> Self {
        let (is_special, special, min, hr, dom, mon, dow) = match &e.schedule {
            CronSchedule::Special(s) =>
                (true, s.clone(), "0".into(), "9".into(), "*".into(), "*".into(), "*".into()),
            CronSchedule::Standard { minute, hour, day, month, weekday } =>
                (false, "@daily".into(),
                 minute.clone(), hour.clone(), day.clone(), month.clone(), weekday.clone()),
        };
        EditForm {
            is_special, editing_index: idx, insert_after: None,
            focused: if is_special { FormField::Special } else { FormField::Minute },
            special: TextInput::new(special), minute: TextInput::new(min),
            hour: TextInput::new(hr),  day: TextInput::new(dom),
            month: TextInput::new(mon), weekday: TextInput::new(dow),
            command: TextInput::new(&e.command),
        }
    }
    pub fn active_input_mut(&mut self) -> &mut TextInput {
        match self.focused {
            FormField::Special  => &mut self.special,
            FormField::Minute   => &mut self.minute,
            FormField::Hour     => &mut self.hour,
            FormField::Day      => &mut self.day,
            FormField::Month    => &mut self.month,
            FormField::Weekday  => &mut self.weekday,
            FormField::Command  => &mut self.command,
        }
    }
    pub fn active_input(&self) -> &TextInput {
        match self.focused {
            FormField::Special  => &self.special,
            FormField::Minute   => &self.minute,
            FormField::Hour     => &self.hour,
            FormField::Day      => &self.day,
            FormField::Month    => &self.month,
            FormField::Weekday  => &self.weekday,
            FormField::Command  => &self.command,
        }
    }
    pub fn next_field(&mut self) {
        self.focused = if self.is_special {
            match self.focused { FormField::Special => FormField::Command, _ => FormField::Special }
        } else {
            match self.focused {
                FormField::Minute  => FormField::Hour,
                FormField::Hour    => FormField::Day,
                FormField::Day     => FormField::Month,
                FormField::Month   => FormField::Weekday,
                FormField::Weekday => FormField::Command,
                FormField::Command => FormField::Minute,
                FormField::Special => FormField::Command,
            }
        };
    }
    pub fn prev_field(&mut self) {
        self.focused = if self.is_special {
            match self.focused { FormField::Command => FormField::Special, _ => FormField::Command }
        } else {
            match self.focused {
                FormField::Minute  => FormField::Command,
                FormField::Hour    => FormField::Minute,
                FormField::Day     => FormField::Hour,
                FormField::Month   => FormField::Day,
                FormField::Weekday => FormField::Month,
                FormField::Command => FormField::Weekday,
                FormField::Special => FormField::Command,
            }
        };
    }
    pub fn preview(&self, use_24h: bool) -> String {
        self.to_schedule().describe(use_24h)
    }
    fn to_schedule(&self) -> CronSchedule {
        if self.is_special {
            CronSchedule::Special(self.special.value.trim().into())
        } else {
            CronSchedule::Standard {
                minute: self.minute.value.trim().into(), hour: self.hour.value.trim().into(),
                day: self.day.value.trim().into(), month: self.month.value.trim().into(),
                weekday: self.weekday.value.trim().into(),
            }
        }
    }
    pub fn to_entry(&self, enabled: bool) -> CronEntry {
        CronEntry { enabled, schedule: self.to_schedule(), command: self.command.value.trim().into() }
    }
    pub fn is_valid(&self) -> bool { !self.command.value.trim().is_empty() }
}

// ── Status ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum StatusKind { Info, Success, Error }

// ── Source ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum CrontabSource { System, File(PathBuf) }

// ── Comment target: editing an existing line vs inserting a new one ───────────

#[derive(Debug, Clone)]
pub enum CommentTarget {
    Edit(usize),         // replace lines[idx] in-place
    InsertAfter(usize),  // insert new comment line after lines[idx]
    Append,              // append at end
}

// ── Visible row — what the cursor moves over ──────────────────────────────────

/// Every non-blank, non-variable line gets a cursor position.
#[derive(Debug, Clone)]
pub enum VisibleRow {
    Entry(usize),   // index into lines[], holds Entry
    Comment(usize), // index into lines[], holds Comment
}

#[derive(Debug, Clone, Copy)]
pub struct UiRect {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

#[derive(Debug, Clone)]
pub enum EditClickTarget {
    Field(FormField),
    ToggleSpecial,
    ToggleStandard,
}

impl UiRect {
    pub fn contains(&self, row: u16, col: u16) -> bool {
        row >= self.y
            && row < self.y.saturating_add(self.height)
            && col >= self.x
            && col < self.x.saturating_add(self.width)
    }
}

// ── App ───────────────────────────────────────────────────────────────────────

pub struct App {
    pub lines:         Vec<CrontabLine>,
    pub mode:          AppMode,
    pub selected:      usize, // index into visible_rows()
    pub form:          Option<EditForm>,
    pub comment_input: Option<(TextInput, CommentTarget)>,
    pub status:        Option<(String, StatusKind)>,
    pub dirty:         bool,
    pub source:        CrontabSource,
    pub use_24h:       bool,
    pub raw_edit_requested: bool,
    // pixel geometry updated by ui each frame for mouse hit-testing
    pub table_top_row:    u16,  // absolute y of first data row
    pub modal_bounds: Option<UiRect>,
    pub comment_input_bounds: Option<UiRect>,
    pub edit_click_targets: Vec<(EditClickTarget, UiRect)>,
    pub last_click_row: Option<usize>,
    pub last_click_at: Option<Instant>,
}

impl App {
    pub fn new(source: CrontabSource) -> Result<Self> {
        let content = load_content(&source)?;
        let lines   = parse_crontab(&content);
        Ok(App { lines, mode: AppMode::Normal, selected: 0, form: None,
                 comment_input: None, status: None, dirty: false, source, use_24h: true, raw_edit_requested: false, table_top_row: 5,
                 modal_bounds: None, comment_input_bounds: None, edit_click_targets: Vec::new(),
                 last_click_row: None, last_click_at: None })
    }

    pub fn clear_mouse_regions(&mut self) {
        self.modal_bounds = None;
        self.comment_input_bounds = None;
        self.edit_click_targets.clear();
    }

    pub fn set_modal_bounds(&mut self, rect: UiRect) {
        self.modal_bounds = Some(rect);
    }

    pub fn set_comment_input_bounds(&mut self, rect: UiRect) {
        self.comment_input_bounds = Some(rect);
    }

    pub fn set_edit_click_targets(&mut self, targets: Vec<(EditClickTarget, UiRect)>) {
        self.edit_click_targets = targets;
    }

    pub fn visible_index_for_mouse_row(&self, row: u16) -> Option<usize> {
        if row < self.table_top_row {
            return None;
        }
        let rel = (row - self.table_top_row) as usize;
        if rel < self.visible_count() {
            Some(rel)
        } else {
            None
        }
    }

    // ── Visible rows ──────────────────────────────────────────────────────────

    pub fn visible_rows(&self) -> Vec<VisibleRow> {
        self.lines.iter().enumerate().filter_map(|(i, l)| match l {
            CrontabLine::Entry(_)   => Some(VisibleRow::Entry(i)),
            CrontabLine::Comment(_) => Some(VisibleRow::Comment(i)),
            _                       => None,
        }).collect()
    }

    pub fn visible_count(&self) -> usize { self.visible_rows().len() }

    pub fn entry_count(&self) -> usize {
        self.lines.iter().filter(|l| matches!(l, CrontabLine::Entry(_))).count()
    }

    /// Line index of currently selected row, if any.
    pub fn selected_line_idx(&self) -> Option<usize> {
        self.visible_rows().get(self.selected).map(|r| match r {
            VisibleRow::Entry(i) | VisibleRow::Comment(i) => *i,
        })
    }

    #[allow(dead_code)]
    pub fn selected_is_entry(&self) -> bool {
        matches!(self.visible_rows().get(self.selected), Some(VisibleRow::Entry(_)))
    }

    #[allow(dead_code)]
    pub fn selected_is_comment(&self) -> bool {
        matches!(self.visible_rows().get(self.selected), Some(VisibleRow::Comment(_)))
    }

    fn clamp_selected(&mut self) {
        let n = self.visible_count();
        if n == 0 { self.selected = 0; } else if self.selected >= n { self.selected = n - 1; }
    }

    /// Returns all enabled entry schedules for the aggregate timeline.
    pub fn all_schedules(&self) -> Vec<&CronSchedule> {
        self.lines.iter().filter_map(|l| {
            if let CrontabLine::Entry(e) = l { if e.enabled { Some(&e.schedule) } else { None } } else { None }
        }).collect()
    }

    pub fn source_label(&self) -> String {
        match &self.source {
            CrontabSource::System    => "system crontab".into(),
            CrontabSource::File(p)   => p.display().to_string(),
        }
    }

    // ── Navigation ────────────────────────────────────────────────────────────

    pub fn move_up(&mut self) {
        if self.selected > 0 { self.selected -= 1; }
    }
    pub fn move_down(&mut self) {
        if self.selected + 1 < self.visible_count() { self.selected += 1; }
    }

    /// Swap the selected visible row with the one above it in self.lines.
    pub fn move_row_up(&mut self) {
        let rows = self.visible_rows();
        if self.selected == 0 { return; }
        let a = match rows[self.selected - 1] { VisibleRow::Entry(i) | VisibleRow::Comment(i) => i };
        let b = match rows[self.selected]     { VisibleRow::Entry(i) | VisibleRow::Comment(i) => i };
        self.lines.swap(a, b);
        self.selected -= 1;
        self.dirty = true;
    }

    /// Swap the selected visible row with the one below it in self.lines.
    pub fn move_row_down(&mut self) {
        let rows = self.visible_rows();
        if self.selected + 1 >= rows.len() { return; }
        let a = match rows[self.selected]     { VisibleRow::Entry(i) | VisibleRow::Comment(i) => i };
        let b = match rows[self.selected + 1] { VisibleRow::Entry(i) | VisibleRow::Comment(i) => i };
        self.lines.swap(a, b);
        self.selected += 1;
        self.dirty = true;
    }

    // ── Actions ───────────────────────────────────────────────────────────────

    pub fn start_add(&mut self) {
        // Determine where to insert: after the currently selected visible row,
        // or None (append) when the list is empty.
        let insert_after = self.visible_rows()
            .get(self.selected)
            .map(|r| match r { VisibleRow::Entry(i) | VisibleRow::Comment(i) => *i });
        let mut form = EditForm::from_entry(&CronEntry::default_new(), None);
        form.insert_after = insert_after;
        self.form = Some(form);
        self.mode = AppMode::EditEntry;
    }

    pub fn start_edit(&mut self) {
        let rows = self.visible_rows();
        if let Some(row) = rows.get(self.selected) {
            match row {
                VisibleRow::Entry(li) => {
                    let li = *li;
                    if let CrontabLine::Entry(e) = &self.lines[li] {
                        let e = e.clone();
                        self.form = Some(EditForm::from_entry(&e, Some(li)));
                        self.mode = AppMode::EditEntry;
                    }
                }
                VisibleRow::Comment(li) => {
                    let li = *li;
                    if let CrontabLine::Comment(s) = &self.lines[li] {
                        let text = s.trim_start_matches('#').trim().to_string();
                        self.comment_input = Some((TextInput::new(text), CommentTarget::Edit(li)));
                        self.mode = AppMode::EditComment;
                    }
                }
            }
        }
    }

    pub fn confirm_edit(&mut self) {
        let Some(form) = self.form.take() else { return };
        if !form.is_valid() {
            self.set_status("Command cannot be empty.".into(), StatusKind::Error);
            self.form = Some(form);
            return;
        }
        let enabled = form.editing_index
            .and_then(|i| if let CrontabLine::Entry(e) = &self.lines[i] { Some(e.enabled) } else { None })
            .unwrap_or(true);
        let new_entry = form.to_entry(enabled);
        if let Some(li) = form.editing_index {
            self.lines[li] = CrontabLine::Entry(new_entry);
            self.set_status("Entry updated.".into(), StatusKind::Success);
        } else {
            match form.insert_after {
                Some(li) => { self.lines.insert(li + 1, CrontabLine::Entry(new_entry)); }
                None     => { self.lines.push(CrontabLine::Entry(new_entry)); }
            }
            // Move cursor to the newly inserted row
            let new_vis = self.visible_count();
            // Find which visible index now holds the new entry
            let rows = self.visible_rows();
            let insert_line = form.insert_after.map(|li| li + 1)
                .unwrap_or(self.lines.len().saturating_sub(1));
            if let Some(vi) = rows.iter().position(|r| match r {
                VisibleRow::Entry(i) | VisibleRow::Comment(i) => *i == insert_line,
            }) {
                self.selected = vi;
            } else {
                self.selected = new_vis.saturating_sub(1);
            }
            self.set_status("Entry added.".into(), StatusKind::Success);
        }
        self.dirty = true;
        self.mode  = AppMode::Normal;
    }

    pub fn cancel_edit(&mut self) { self.form = None; self.mode = AppMode::Normal; }

    pub fn confirm_comment(&mut self) {
        if let Some((input, target)) = self.comment_input.take() {
            let new_text = input.value.trim().to_string();
            match target {
                CommentTarget::Edit(li) => {
                    self.lines[li] = if new_text.is_empty() {
                        CrontabLine::Blank
                    } else {
                        CrontabLine::Comment(format!("# {}", new_text))
                    };
                    self.set_status("Comment updated.".into(), StatusKind::Success);
                }
                CommentTarget::InsertAfter(li) => {
                    if !new_text.is_empty() {
                        self.lines.insert(li + 1, CrontabLine::Comment(format!("# {}", new_text)));
                        // Move cursor to the new comment
                        let insert_line = li + 1;
                        let rows = self.visible_rows();
                        if let Some(vi) = rows.iter().position(|r| match r {
                            VisibleRow::Entry(i) | VisibleRow::Comment(i) => *i == insert_line,
                        }) { self.selected = vi; }
                    }
                    self.set_status("Comment added.".into(), StatusKind::Success);
                }
                CommentTarget::Append => {
                    if !new_text.is_empty() {
                        self.lines.push(CrontabLine::Comment(format!("# {}", new_text)));
                        self.selected = self.visible_count().saturating_sub(1);
                    }
                    self.set_status("Comment added.".into(), StatusKind::Success);
                }
            }
            self.dirty = true;
            self.clamp_selected();
        }
        self.mode = AppMode::Normal;
    }

    pub fn cancel_comment(&mut self) { self.comment_input = None; self.mode = AppMode::Normal; }

    pub fn delete_selected(&mut self) {
        let idx = self.selected_line_idx();
        self.mode = AppMode::Normal;
        if let Some(i) = idx {
            self.lines.remove(i);
            self.clamp_selected();
            self.dirty = true;
            self.set_status("Deleted.".into(), StatusKind::Success);
        }
    }

    pub fn toggle_selected(&mut self) {
        if let Some(i) = self.selected_line_idx() {
            if let CrontabLine::Entry(e) = &mut self.lines[i] {
                e.enabled = !e.enabled;
                self.dirty = true;
                let s = if e.enabled { "enabled" } else { "disabled" };
                self.set_status(format!("Entry {}.", s), StatusKind::Info);
            }
        }
    }

    pub fn start_raw(&mut self) {
        self.raw_edit_requested = true;
    }

    pub fn take_raw_edit_request(&mut self) -> Option<String> {
        if !self.raw_edit_requested {
            return None;
        }
        self.raw_edit_requested = false;
        Some(crate::cron::serialize_crontab(&self.lines))
    }

    pub fn apply_raw_content(&mut self, content: &str) {
        self.lines = crate::cron::parse_crontab(content);
        self.dirty = true;
        self.clamp_selected();
        self.set_status("Raw edit applied.".into(), StatusKind::Success);
    }

    pub fn notify_status(&mut self, msg: impl Into<String>, kind: StatusKind) {
        self.set_status(msg.into(), kind);
    }

    pub fn save(&mut self) -> Result<()> {
        let content = serialize_crontab(&self.lines);
        save_content(&self.source, &content)?;
        self.dirty = false;
        self.set_status("Crontab saved!".into(), StatusKind::Success);
        Ok(())
    }

    /// Called from the event loop when a left-click arrives.
    /// `row` and `col` are absolute terminal coordinates.
    pub fn handle_mouse_click(&mut self, row: u16, _col: u16) {
        if let Some(rel) = self.visible_index_for_mouse_row(row) {
            self.selected = rel;
            self.status = None;

            let now = Instant::now();
            let is_double = self.last_click_row == Some(rel)
                && self
                    .last_click_at
                    .map(|t| now.duration_since(t) <= Duration::from_millis(350))
                    .unwrap_or(false);

            self.last_click_row = Some(rel);
            self.last_click_at = Some(now);

            if is_double {
                self.start_edit();
                self.last_click_row = None;
                self.last_click_at = None;
            }
        }
    }

    fn set_status(&mut self, msg: String, kind: StatusKind) {
        self.status = Some((msg, kind));
    }

    pub fn is_modal_open(&self) -> bool {
        !matches!(self.mode, AppMode::Normal)
    }

    pub fn handle_modal_click(&mut self, row: u16, col: u16) {
        let inside_modal = self.modal_bounds.map(|r| r.contains(row, col)).unwrap_or(false);
        if !inside_modal {
            match self.mode {
                AppMode::EditEntry => self.cancel_edit(),
                AppMode::EditComment => self.cancel_comment(),
                AppMode::ConfirmDelete | AppMode::ConfirmQuit | AppMode::Info | AppMode::Help => {
                    self.mode = AppMode::Normal;
                }
                AppMode::Normal => {}
            }
            return;
        }

        match self.mode {
            AppMode::EditComment => {
                if let Some(field) = self.comment_input_bounds {
                    if field.contains(row, col) {
                        if let Some((input, _)) = &mut self.comment_input {
                            set_input_cursor_from_click(input, field, col);
                        }
                    }
                }
            }
            AppMode::EditEntry => {
                let mut picked: Option<(EditClickTarget, UiRect)> = None;
                for (field, rect) in &self.edit_click_targets {
                    if rect.contains(row, col) {
                        picked = Some((field.clone(), *rect));
                        break;
                    }
                }
                if let Some((target, rect)) = picked {
                    if let Some(form) = &mut self.form {
                        match target {
                            EditClickTarget::Field(field) => {
                                form.focused = field;
                                set_input_cursor_from_click(form.active_input_mut(), rect, col);
                            }
                            EditClickTarget::ToggleSpecial => {
                                form.is_special = true;
                                form.focused = FormField::Special;
                            }
                            EditClickTarget::ToggleStandard => {
                                form.is_special = false;
                                form.focused = FormField::Minute;
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    // ── Key handling ──────────────────────────────────────────────────────────

    pub fn handle_key(&mut self, key: KeyEvent) -> Result<bool> {
        match self.mode.clone() {
            AppMode::Normal        => self.key_normal(key),
            AppMode::EditEntry     => self.key_edit(key),
            AppMode::EditComment   => self.key_comment(key),
            AppMode::ConfirmDelete => self.key_confirm_delete(key),
            AppMode::ConfirmQuit   => self.key_confirm_quit(key),
            AppMode::Help          => { self.mode = AppMode::Normal; Ok(false) }
            AppMode::Info          => { self.mode = AppMode::Normal; Ok(false) }
        }
    }

    fn key_normal(&mut self, key: KeyEvent) -> Result<bool> {
        self.status = None;
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => {
                if self.dirty { self.mode = AppMode::ConfirmQuit; } else { return Ok(true); }
            }
            KeyCode::Up   | KeyCode::Char('k') if !key.modifiers.contains(KeyModifiers::SHIFT) => self.move_up(),
            KeyCode::Down | KeyCode::Char('j') if !key.modifiers.contains(KeyModifiers::SHIFT) => self.move_down(),
            KeyCode::Up   if  key.modifiers.contains(KeyModifiers::SHIFT) => self.move_row_up(),
            KeyCode::Down if  key.modifiers.contains(KeyModifiers::SHIFT) => self.move_row_down(),
            KeyCode::Char('n') | KeyCode::Char('a') => self.start_add(),
            KeyCode::Enter | KeyCode::Char('e') => {
                if self.visible_count() > 0 { self.start_edit(); }
            }
            KeyCode::Char('d') | KeyCode::Delete => {
                if self.visible_count() > 0 { self.mode = AppMode::ConfirmDelete; }
            }
            KeyCode::Char('t') => self.toggle_selected(),
            KeyCode::Char('s') => { self.save()?; }
            KeyCode::Char('i') => { if self.selected_is_entry() { self.mode = AppMode::Info; } }
            KeyCode::Char('c') => {
                self.use_24h = !self.use_24h;
                let fmt = if self.use_24h { "24-hour" } else { "12-hour" };
                self.set_status(format!("Switched to {} clock.", fmt), StatusKind::Info);
            }
            KeyCode::Char('r') => self.start_raw(),
            KeyCode::Char('?') | KeyCode::Char('h') => self.mode = AppMode::Help,
            _ => {}
        }
        Ok(false)
    }

    fn key_edit(&mut self, key: KeyEvent) -> Result<bool> {
        let ctrl  = key.modifiers.contains(KeyModifiers::CONTROL);
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        match key.code {
            KeyCode::Esc                    => { self.cancel_edit(); }
            KeyCode::Char('s') if ctrl      => { self.confirm_edit(); }
            KeyCode::Enter => {
                let on_cmd = self.form.as_ref().map(|f| f.focused == FormField::Command).unwrap_or(false);
                if on_cmd { self.confirm_edit(); }
                else if let Some(f) = &mut self.form { f.next_field(); }
            }
            KeyCode::Tab    => { if let Some(f) = &mut self.form { if shift { f.prev_field(); } else { f.next_field(); } } }
            KeyCode::BackTab => { if let Some(f) = &mut self.form { f.prev_field(); } }
            KeyCode::Left   => { if let Some(f) = &mut self.form { f.active_input_mut().move_left(); } }
            KeyCode::Right  => { if let Some(f) = &mut self.form { f.active_input_mut().move_right(); } }
            KeyCode::Home   => { if let Some(f) = &mut self.form { f.active_input_mut().home(); } }
            KeyCode::End    => { if let Some(f) = &mut self.form { f.active_input_mut().end(); } }
            KeyCode::Backspace => { if let Some(f) = &mut self.form { f.active_input_mut().backspace(); } }
            KeyCode::Delete    => { if let Some(f) = &mut self.form { f.active_input_mut().delete_char(); } }
            KeyCode::Char('#') if !ctrl => {
                // '#' in Minute field (and field is empty) → discard entry form, open comment editor
                let on_minute = self.form.as_ref()
                    .map(|f| f.focused == FormField::Minute && f.minute.value.is_empty())
                    .unwrap_or(false);
                if on_minute {
                    let insert_after = self.form.as_ref().and_then(|f| f.insert_after);
                    self.form = None;
                    let target = match insert_after {
                        Some(li) => CommentTarget::InsertAfter(li),
                        None     => CommentTarget::Append,
                    };
                    self.comment_input = Some((TextInput::new(""), target));
                    self.mode = AppMode::EditComment;
                } else if let Some(f) = &mut self.form {
                    f.active_input_mut().insert('#');
                }
            }
            KeyCode::Char(c) if !ctrl => { if let Some(f) = &mut self.form { f.active_input_mut().insert(c); } }
            KeyCode::F(1) => {
                if let Some(f) = &mut self.form {
                    f.is_special = !f.is_special;
                    f.focused = if f.is_special { FormField::Special } else { FormField::Minute };
                }
            }
            _ => {}
        }
        Ok(false)
    }

    fn key_comment(&mut self, key: KeyEvent) -> Result<bool> {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc                 => { self.cancel_comment(); }
            KeyCode::Enter               => { self.confirm_comment(); }
            KeyCode::Char('s') if ctrl   => { self.confirm_comment(); }
            KeyCode::Left                => { if let Some((inp, _)) = &mut self.comment_input { inp.move_left(); } }
            KeyCode::Right               => { if let Some((inp, _)) = &mut self.comment_input { inp.move_right(); } }
            KeyCode::Home                => { if let Some((inp, _)) = &mut self.comment_input { inp.home(); } }
            KeyCode::End                 => { if let Some((inp, _)) = &mut self.comment_input { inp.end(); } }
            KeyCode::Backspace           => { if let Some((inp, _)) = &mut self.comment_input { inp.backspace(); } }
            KeyCode::Delete              => { if let Some((inp, _)) = &mut self.comment_input { inp.delete_char(); } }
            KeyCode::Char(c) if !ctrl    => { if let Some((inp, _)) = &mut self.comment_input { inp.insert(c); } }
            _ => {}
        }
        Ok(false)
    }

    fn key_confirm_delete(&mut self, key: KeyEvent) -> Result<bool> {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => self.delete_selected(),
            _ => self.mode = AppMode::Normal,
        }
        Ok(false)
    }

    fn key_confirm_quit(&mut self, key: KeyEvent) -> Result<bool> {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => return Ok(true),
            KeyCode::Char('s') | KeyCode::Char('S') => { self.save()?; return Ok(true); }
            _ => self.mode = AppMode::Normal,
        }
        Ok(false)
    }
}

fn set_input_cursor_from_click(input: &mut TextInput, field: UiRect, col: u16) {
    let left = field.x.saturating_add(1);
    let right = field.x.saturating_add(field.width.saturating_sub(1));
    let click = col.clamp(left, right);
    let max = input.value.len();
    input.cursor = (click.saturating_sub(left)) as usize;
    if input.cursor > max {
        input.cursor = max;
    }
}

// ── I/O ───────────────────────────────────────────────────────────────────────

fn load_content(source: &CrontabSource) -> Result<String> {
    match source {
        CrontabSource::File(p) => {
            if p.exists() { std::fs::read_to_string(p).with_context(|| format!("Failed to read {}", p.display())) }
            else { Ok(String::new()) }
        }
        CrontabSource::System => {
            let out = Command::new("crontab").arg("-l").output().context("Failed to run crontab -l")?;
            if out.status.success() { return Ok(String::from_utf8_lossy(&out.stdout).into()); }
            let err = String::from_utf8_lossy(&out.stderr);
            if err.contains("no crontab") { Ok(String::new()) }
            else { Err(anyhow::anyhow!("crontab -l: {}", err.trim())) }
        }
    }
}

fn save_content(source: &CrontabSource, content: &str) -> Result<()> {
    match source {
        CrontabSource::File(p) => std::fs::write(p, content)
            .with_context(|| format!("Failed to write {}", p.display())),
        CrontabSource::System => {
            let mut child = Command::new("crontab").arg("-")
                .stdin(Stdio::piped()).spawn().context("Failed to launch crontab -")?;
            if let Some(mut stdin) = child.stdin.take() {
                stdin.write_all(content.as_bytes()).context("Failed to write to crontab")?;
            }
            let status = child.wait().context("crontab - did not finish")?;
            if !status.success() { return Err(anyhow::anyhow!("crontab - returned error")); }
            Ok(())
        }
    }
}
