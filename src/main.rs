mod app;
mod cron;
mod ui;

use anyhow::{Context, Result};
use app::{App, CrontabSource, StatusKind};
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind, MouseButton,
        MouseEventKind,
    },
    execute,
    terminal::{EnterAlternateScreen, enable_raw_mode},
};
use ratatui::DefaultTerminal;
use std::{io, io::Write, path::PathBuf, process::Command};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const NAME: &str = env!("CARGO_PKG_NAME");

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
  {NAME} --file ~/jobs.cron     Edit a crontab file directly
  {NAME} -f ~/jobs.cron         Same, short form
"
    );
}

fn parse_args() -> Result<CrontabSource, String> {
    let mut args = std::env::args().skip(1);
    let arg = match args.next() {
        None => return Ok(CrontabSource::System),
        Some(a) => a,
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
        a if a.len() > 2 && a.starts_with("-f") => Ok(CrontabSource::File(PathBuf::from(&a[2..]))),
        _ => Err(format!(
            "Unknown option: {}\nRun '{} --help' for usage.",
            arg, NAME
        )),
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
    // ratatui::try_init installs a panic hook that restores raw mode and the
    // alternate screen; chain one that also disables mouse capture so a panic
    // can't leave the terminal spewing mouse escape codes.
    let mut term = ratatui::try_init()?;
    execute!(io::stdout(), EnableMouseCapture)?;
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = execute!(io::stdout(), DisableMouseCapture);
        prev_hook(info);
    }));

    let result = event_loop(&mut term, app);

    let _ = execute!(io::stdout(), DisableMouseCapture);
    ratatui::restore();
    result
}

fn event_loop(term: &mut DefaultTerminal, app: &mut App) -> Result<()> {
    loop {
        term.draw(|f| ui::render(f, app))?;
        match event::read()? {
            // Ignore release/repeat events (Windows and kitty-protocol
            // terminals report them; acting on both fires keys twice).
            Event::Key(key) if key.kind == KeyEventKind::Press && app.handle_key(key)? => {
                break;
            }
            Event::Mouse(mouse) => {
                if app.is_modal_open() {
                    if let MouseEventKind::Down(MouseButton::Left) = mouse.kind {
                        app.handle_modal_click(mouse.row, mouse.column);
                    }
                } else {
                    match mouse.kind {
                        // Left click — select the row under the cursor
                        MouseEventKind::Down(MouseButton::Left) => {
                            app.handle_mouse_click(mouse.row, mouse.column);
                        }
                        // Scroll wheel — navigate list
                        MouseEventKind::ScrollUp => {
                            app.move_up();
                        }
                        MouseEventKind::ScrollDown => {
                            app.move_down();
                        }
                        _ => {}
                    }
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
    term: &mut DefaultTerminal,
    app: &mut App,
    content: &str,
) -> Result<()> {
    // Crontabs can hold secrets — tempfile gives a 0600, race-free file that
    // is removed on drop.
    let mut tmp = tempfile::Builder::new()
        .prefix("cronv-")
        .suffix(".cron")
        .tempfile()
        .context("Failed to create temp file")?;
    tmp.write_all(content.as_bytes())?;
    tmp.flush()?;

    suspend_tui()?;
    let edit_result = run_editor_on_file(tmp.path());
    resume_tui(term)?;

    match edit_result {
        Ok(()) => match std::fs::read_to_string(tmp.path()) {
            Ok(edited) => app.apply_raw_content(&edited),
            Err(e) => app.notify_status(
                format!("Failed to read edited content: {}", e),
                StatusKind::Error,
            ),
        },
        Err(e) => app.notify_status(format!("Raw editor failed: {}", e), StatusKind::Error),
    }

    Ok(())
}

fn suspend_tui() -> Result<()> {
    execute!(io::stdout(), DisableMouseCapture)?;
    ratatui::restore();
    Ok(())
}

fn resume_tui(term: &mut DefaultTerminal) -> Result<()> {
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
    term.clear()?;
    Ok(())
}

fn run_editor_on_file(path: &std::path::Path) -> Result<()> {
    let editor = std::env::var("VISUAL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            std::env::var("EDITOR")
                .ok()
                .filter(|s| !s.trim().is_empty())
        })
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
