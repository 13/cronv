mod app;
mod cron;
mod ui;

use anyhow::Result;
use app::{App, CrontabSource};
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::{io, path::PathBuf};

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
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        if arg == "-h" || arg == "--help" {
            print_help();
            std::process::exit(0);
        } else if arg == "-V" || arg == "--version" {
            println!("{} {}", NAME, VERSION);
            std::process::exit(0);
        } else if arg == "-f" || arg == "--file" {
            i += 1;
            if i >= args.len() {
                return Err(format!("'{}' requires a PATH argument", args[i - 1]));
            }
            return Ok(CrontabSource::File(PathBuf::from(&args[i])));
        } else if let Some(path) = arg.strip_prefix("--file=") {
            if path.is_empty() {
                return Err("--file= requires a non-empty path".into());
            }
            return Ok(CrontabSource::File(PathBuf::from(path)));
        } else if arg.len() > 2 && arg.starts_with("-f") {
            return Ok(CrontabSource::File(PathBuf::from(&arg[2..])));
        } else {
            return Err(format!(
                "Unknown option: {}\nRun '{} --help' for usage.", arg, NAME
            ));
        }
        // unreachable in practice (all branches above return), but satisfies the loop
        #[allow(unreachable_code)]
        { i += 1; }
    }
    Ok(CrontabSource::System)
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
        if let Event::Key(key) = event::read()? {
            if app.handle_key(key)? {
                break;
            }
        }
    }
    Ok(())
}


