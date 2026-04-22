mod app;
mod cron;
mod ui;

use anyhow::Result;
use app::{App, CrontabSource, StatusKind};
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event,
        MouseButton, MouseEventKind,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::{io, path::PathBuf, process::Command, time::{SystemTime, UNIX_EPOCH}};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const NAME:    &str = env!("CARGO_PKG_NAME");

// ── CLI ───────────────────────────────────────────────────────────────────────

fn print_help() {
    println!(
"Usage: {NAME} [OPTIONS]

A modern terminal UI for managing cron jobs.

Options:
  -f, --file <PATH>   Edit a crontab file directly instead of the system crontab
  -V, --version       Print version and exit
  -h, --help          Print this help and exit

Examples:
  {NAME}                        Edit the current user's system crontab
  {NAME} --file /etc/crontab    Edit a specific crontab file
  {NAME} -f ~/jobs.cron         Edit a custom cron file
"
    );
}

fn parse_args() -> Result<CrontabSource, String> {
    let mut args = std::env::args().skip(1);
    let arg = match args.next() {
        None      => return Ok(CrontabSource::System),
        Some(a)   => a,
    };
    match arg.as_str() {
        "-h" | "--help" => {
            print_help();
            std::process::exit(0);
        }
        "-V" | "--version" => {
            println!("{} {}", NAME, VERSION);
            std::process::exit(0);
        }
        "-f" | "--file" => match args.next() {
            Some(p) if !p.is_empty() => Ok(CrontabSource::File(PathBuf::from(p))),
            _ => Err(format!("'{}' requires a PATH argument", arg)),
        },
        a if a.starts_with("--file=") => {
            let path = &a["--file=".len()..];
            if path.is_empty() {
                Err("--file= requires a non-empty path".into())
            } else {
                Ok(CrontabSource::File(PathBuf::from(path)))
            }
        }
        a if a.len() > 2 && a.starts_with("-f") => {
            Ok(CrontabSource::File(PathBuf::from(&a[2..])))
        }
        _ => Err(format!("Unknown option: {}\nRun '{} --help' for usage.", arg, NAME)),
    }
}

// ── Entry point ───────────────────────────────────────────────────────────────

fn main() {
    let source = parse_args().unwrap_or_else(|e| {
        eprintln!("{}: {}", NAME, e);
        std::process::exit(1);
    });

    let mut app = App::new(source).unwrap_or_else(|e| {
        eprintln!("{}: {}", NAME, e);
        std::process::exit(1);
    });

    if let Err(e) = run_tui(&mut app) {
        eprintln!("{}: {}", NAME, e);
        std::process::exit(1);
    }
}

fn run_tui(app: &mut App) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend  = CrosstermBackend::new(stdout);
    let mut term = Terminal::new(backend)?;

    let result = event_loop(&mut term, app);

    disable_raw_mode()?;
    execute!(term.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;
    term.show_cursor()?;
    result
}

fn event_loop(
    term: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app:  &mut App,
) -> Result<()> {
    loop {
        term.draw(|f| ui::render(f, app))?;
        match event::read()? {
            Event::Key(key) => {
                if app.handle_key(key)? { break; }
            }
            Event::Mouse(mouse) => {
                match mouse.kind {
                    // Left click — select the row under the cursor
                    MouseEventKind::Down(MouseButton::Left) => {
                        app.handle_mouse_click(mouse.row, mouse.column);
                    }
                    // Scroll wheel — navigate list
                    MouseEventKind::ScrollUp   => { app.move_up(); }
                    MouseEventKind::ScrollDown => { app.move_down(); }
                    _ => {}
                }
            }
            _ => {}
        }

        if let Some(content) = app.take_raw_edit_request() {
            launch_external_raw_editor(term, app, &content)?;
        }
    }
    Ok(())
}

fn launch_external_raw_editor(
    term: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    content: &str,
) -> Result<()> {
    let tmp = std::env::temp_dir().join(format!(
        "cronv-{}-{}.cron",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos()
    ));
    std::fs::write(&tmp, content)?;

    suspend_tui(term)?;
    let edit_result = run_editor_on_file(&tmp);
    resume_tui(term)?;

    match edit_result {
        Ok(()) => match std::fs::read_to_string(&tmp) {
            Ok(edited) => app.apply_raw_content(&edited),
            Err(e) => app.notify_status(format!("Failed to read edited content: {}", e), StatusKind::Error),
        },
        Err(e) => app.notify_status(format!("Raw editor failed: {}", e), StatusKind::Error),
    }

    let _ = std::fs::remove_file(&tmp);
    Ok(())
}

fn suspend_tui(term: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
    disable_raw_mode()?;
    execute!(term.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;
    term.show_cursor()?;
    Ok(())
}

fn resume_tui(term: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
    enable_raw_mode()?;
    execute!(term.backend_mut(), EnterAlternateScreen, EnableMouseCapture)?;
    term.clear()?;
    Ok(())
}

fn run_editor_on_file(path: &std::path::Path) -> Result<()> {
    let editor = std::env::var("VISUAL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| std::env::var("EDITOR").ok().filter(|s| !s.trim().is_empty()))
        .unwrap_or_else(|| "vi".to_string());
    let quoted_path = shell_quote(path.to_string_lossy().as_ref());
    let command = format!("{} {}", editor, quoted_path);
    let status = Command::new("sh").arg("-c").arg(command).status()?;
    if !status.success() {
        return Err(anyhow::anyhow!("editor exited with status {}", status));
    }
    Ok(())
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use crate::cron::CronSchedule;
    use chrono::NaiveDateTime;

    fn next_after(sched: &CronSchedule, from_str: &str) -> String {
        let from = NaiveDateTime::parse_from_str(from_str, "%Y-%m-%d %H:%M").unwrap();
        match sched {
            CronSchedule::Standard { minute, hour, day, month, weekday } =>
                crate::cron::next_standard(minute, hour, day, month, weekday, from)
                    .map(|t| t.format("%Y-%m-%d %H:%M").to_string())
                    .unwrap_or_else(|| "None".into()),
            _ => "special".into()
        }
    }

    fn sched(s: &str) -> CronSchedule {
        let mut p = s.split_whitespace();
        CronSchedule::Standard {
            minute:  p.next().unwrap().into(), hour:    p.next().unwrap().into(),
            day:     p.next().unwrap().into(), month:   p.next().unwrap().into(),
            weekday: p.next().unwrap().into(),
        }
    }

    #[test]
    fn next_runs() {
        assert_eq!(next_after(&sched("*/15 * * * *"), "2025-04-22 09:08"), "2025-04-22 09:15");
        assert_eq!(next_after(&sched("*/15 * * * *"), "2025-04-22 09:15"), "2025-04-22 09:30");
        assert_eq!(next_after(&sched("0 * * * *"),    "2025-04-22 09:00"), "2025-04-22 10:00");
        assert_eq!(next_after(&sched("0 2 * * *"),    "2025-04-22 09:00"), "2025-04-23 02:00");
        assert_eq!(next_after(&sched("30 2 * * 5"),   "2025-04-22 09:00"), "2025-04-25 02:30");
        assert_eq!(next_after(&sched("30 3 1 * *"),   "2025-04-22 09:00"), "2025-05-01 03:30");
        assert_eq!(next_after(&sched("0 4,5 * * *"),  "2025-04-22 03:30"), "2025-04-22 04:00");
        assert_eq!(next_after(&sched("0 4,5 * * *"),  "2025-04-22 04:30"), "2025-04-22 05:00");
        assert_eq!(next_after(&sched("0 4,5 * * *"),  "2025-04-22 05:30"), "2025-04-23 04:00");
        assert_eq!(next_after(&sched("0 4 * * 0,3"),  "2025-04-22 09:00"), "2025-04-23 04:00");
        assert_eq!(next_after(&sched("*/5 9,12 1 2-4 *"), "2025-01-31 00:00"), "2025-02-01 09:00");
        assert_eq!(next_after(&sched("*/5 9,12 1 2-4 *"), "2025-02-01 09:03"), "2025-02-01 09:05");
        assert_eq!(next_after(&sched("*/5 9,12 1 2-4 *"), "2025-02-01 09:55"), "2025-02-01 12:00");
    }
}
