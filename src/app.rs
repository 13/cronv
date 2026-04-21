use anyhow::{Context, Result};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use crate::cron::{parse_crontab, serialize_crontab, CronEntry, CronSchedule, CrontabLine};

// ── Modes ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum AppMode {
    Normal,
    EditEntry,
    ConfirmDelete,
    ConfirmQuit,
    Help,
    Info,
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

// ── Edit form ─────────────────────────────────────────────────────────────────

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
    pub editing_index: Option<usize>,
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
        let focused = if is_special { FormField::Special } else { FormField::Minute };
        EditForm {
            is_special, focused, editing_index: idx,
            special: TextInput::new(special),
            minute:  TextInput::new(min),
            hour:    TextInput::new(hr),
            day:     TextInput::new(dom),
            month:   TextInput::new(mon),
            weekday: TextInput::new(dow),
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
                minute:  self.minute.value.trim().into(),
                hour:    self.hour.value.trim().into(),
                day:     self.day.value.trim().into(),
                month:   self.month.value.trim().into(),
                weekday: self.weekday.value.trim().into(),
            }
        }
    }

    pub fn to_entry(&self, enabled: bool) -> CronEntry {
        CronEntry { enabled, schedule: self.to_schedule(), command: self.command.value.trim().into() }
    }

    pub fn is_valid(&self) -> bool {
        !self.command.value.trim().is_empty()
    }
}

// ── Status ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum StatusKind { Info, Success, Error }

// ── Source: where we read/write the crontab ───────────────────────────────────

#[derive(Debug, Clone)]
pub enum CrontabSource {
    /// Use the system `crontab` command (default)
    System,
    /// Read/write a specific file directly
    File(PathBuf),
}

// ── App ───────────────────────────────────────────────────────────────────────

pub struct App {
    pub lines:    Vec<CrontabLine>,
    pub mode:     AppMode,
    pub selected: usize,
    pub form:     Option<EditForm>,
    pub status:   Option<(String, StatusKind)>,
    pub dirty:    bool,
    pub source:   CrontabSource,
    pub use_24h:  bool,
}

impl App {
    pub fn new(source: CrontabSource) -> Result<Self> {
        let content = load_content(&source)?;
        let lines   = parse_crontab(&content);
        Ok(App { lines, mode: AppMode::Normal, selected: 0, form: None, status: None,
                 dirty: false, source, use_24h: true })
    }

    // ── Accessors ─────────────────────────────────────────────────────────────

    pub fn entries(&self) -> Vec<(usize, &CronEntry)> {
        self.lines.iter().enumerate()
            .filter_map(|(i, l)| if let CrontabLine::Entry(e) = l { Some((i, e)) } else { None })
            .collect()
    }

    pub fn entry_count(&self) -> usize {
        self.lines.iter().filter(|l| matches!(l, CrontabLine::Entry(_))).count()
    }

    fn selected_line_idx(&self) -> Option<usize> {
        self.entries().get(self.selected).map(|(i, _)| *i)
    }

    fn clamp_selected(&mut self) {
        let n = self.entry_count();
        if n == 0 { self.selected = 0; } else if self.selected >= n { self.selected = n - 1; }
    }

    pub fn source_label(&self) -> String {
        match &self.source {
            CrontabSource::System    => "system crontab".into(),
            CrontabSource::File(p)  => p.display().to_string(),
        }
    }

    // ── Navigation ────────────────────────────────────────────────────────────

    pub fn move_up(&mut self) {
        if self.selected > 0 { self.selected -= 1; }
    }

    pub fn move_down(&mut self) {
        if self.selected + 1 < self.entry_count() { self.selected += 1; }
    }

    // ── Actions ───────────────────────────────────────────────────────────────

    pub fn start_add(&mut self) {
        self.form = Some(EditForm::from_entry(&CronEntry::default_new(), None));
        self.mode = AppMode::EditEntry;
    }

    pub fn start_edit(&mut self) {
        let info: Option<(usize, CronEntry)> = self.entries()
            .get(self.selected)
            .map(|(i, e)| (*i, (*e).clone()));
        if let Some((li, entry)) = info {
            self.form = Some(EditForm::from_entry(&entry, Some(li)));
            self.mode = AppMode::EditEntry;
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
            self.lines.push(CrontabLine::Entry(new_entry));
            self.selected = self.entry_count().saturating_sub(1);
            self.set_status("Entry added.".into(), StatusKind::Success);
        }
        self.dirty = true;
        self.mode  = AppMode::Normal;
    }

    pub fn cancel_edit(&mut self) {
        self.form = None;
        self.mode = AppMode::Normal;
    }

    pub fn delete_selected(&mut self) {
        let idx = self.selected_line_idx();
        self.mode = AppMode::Normal;
        if let Some(i) = idx {
            self.lines.remove(i);
            self.clamp_selected();
            self.dirty = true;
            self.set_status("Entry deleted.".into(), StatusKind::Success);
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

    pub fn save(&mut self) -> Result<()> {
        let content = serialize_crontab(&self.lines);
        save_content(&self.source, &content)?;
        self.dirty = false;
        self.set_status("Crontab saved!".into(), StatusKind::Success);
        Ok(())
    }

    fn set_status(&mut self, msg: String, kind: StatusKind) {
        self.status = Some((msg, kind));
    }

    // ── Key handling ──────────────────────────────────────────────────────────

    pub fn handle_key(&mut self, key: KeyEvent) -> Result<bool> {
        match self.mode.clone() {
            AppMode::Normal        => self.key_normal(key),
            AppMode::EditEntry     => self.key_edit(key),
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
            KeyCode::Up   | KeyCode::Char('k') => self.move_up(),
            KeyCode::Down | KeyCode::Char('j') => self.move_down(),
            KeyCode::Char('n') | KeyCode::Char('a') => self.start_add(),
            KeyCode::Enter | KeyCode::Char('e') => {
                if self.entry_count() > 0 { self.start_edit(); }
            }
            KeyCode::Char('d') | KeyCode::Delete => {
                if self.entry_count() > 0 { self.mode = AppMode::ConfirmDelete; }
            }
            KeyCode::Char('t') => self.toggle_selected(),
            KeyCode::Char('s') => { self.save()?; }
            KeyCode::Char('c') => {
                self.use_24h = !self.use_24h;
                let fmt = if self.use_24h { "24-hour" } else { "12-hour" };
                self.set_status(format!("Switched to {} clock.", fmt), StatusKind::Info);
            }
            KeyCode::Char('i') => { if self.entry_count() > 0 { self.mode = AppMode::Info; } }
            KeyCode::Char('?') | KeyCode::Char('h') => self.mode = AppMode::Help,
            _ => {}
        }
        Ok(false)
    }

    fn key_edit(&mut self, key: KeyEvent) -> Result<bool> {
        let ctrl  = key.modifiers.contains(KeyModifiers::CONTROL);
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        match key.code {
            KeyCode::Esc => { self.cancel_edit(); return Ok(false); }
            KeyCode::Char('s') if ctrl => { self.confirm_edit(); return Ok(false); }
            KeyCode::Enter => {
                let on_cmd = self.form.as_ref()
                    .map(|f| f.focused == FormField::Command).unwrap_or(false);
                if on_cmd { self.confirm_edit(); }
                else if let Some(f) = &mut self.form { f.next_field(); }
                return Ok(false);
            }
            KeyCode::Tab => {
                if let Some(f) = &mut self.form {
                    if shift { f.prev_field(); } else { f.next_field(); }
                }
            }
            KeyCode::BackTab => { if let Some(f) = &mut self.form { f.prev_field(); } }
            KeyCode::Left    => { if let Some(f) = &mut self.form { f.active_input_mut().move_left(); } }
            KeyCode::Right   => { if let Some(f) = &mut self.form { f.active_input_mut().move_right(); } }
            KeyCode::Home    => { if let Some(f) = &mut self.form { f.active_input_mut().home(); } }
            KeyCode::End     => { if let Some(f) = &mut self.form { f.active_input_mut().end(); } }
            KeyCode::Backspace => { if let Some(f) = &mut self.form { f.active_input_mut().backspace(); } }
            KeyCode::Delete    => { if let Some(f) = &mut self.form { f.active_input_mut().delete_char(); } }
            KeyCode::Char(c) if !ctrl => {
                if let Some(f) = &mut self.form { f.active_input_mut().insert(c); }
            }
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

// ── I/O ───────────────────────────────────────────────────────────────────────

fn load_content(source: &CrontabSource) -> Result<String> {
    match source {
        CrontabSource::File(p) => {
            if p.exists() {
                std::fs::read_to_string(p)
                    .with_context(|| format!("Failed to read {}", p.display()))
            } else {
                Ok(String::new()) // new file — start empty
            }
        }
        CrontabSource::System => {
            let out = Command::new("crontab").arg("-l").output()
                .context("Failed to run crontab -l")?;
            if out.status.success() {
                return Ok(String::from_utf8_lossy(&out.stdout).into());
            }
            let stderr = String::from_utf8_lossy(&out.stderr);
            if stderr.contains("no crontab") { Ok(String::new()) }
            else { Err(anyhow::anyhow!("crontab -l: {}", stderr.trim())) }
        }
    }
}

fn save_content(source: &CrontabSource, content: &str) -> Result<()> {
    match source {
        CrontabSource::File(p) => {
            std::fs::write(p, content)
                .with_context(|| format!("Failed to write {}", p.display()))
        }
        CrontabSource::System => {
            let mut child = Command::new("crontab").arg("-")
                .stdin(Stdio::piped()).spawn()
                .context("Failed to launch crontab -")?;
            if let Some(mut stdin) = child.stdin.take() {
                stdin.write_all(content.as_bytes()).context("Failed to write to crontab")?;
            }
            let status = child.wait().context("crontab - did not finish")?;
            if !status.success() { return Err(anyhow::anyhow!("crontab - returned an error")); }
            Ok(())
        }
    }
}
