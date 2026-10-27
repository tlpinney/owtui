//! owtui: browse and edit OpenWhispr history, dictionary and snippets.

mod app;
mod editor;
mod insights;
mod store;
#[cfg(test)]
mod tests;
mod ui;

use anyhow::Result;
use app::App;
use crossterm::event::{self, Event};
use ratatui::DefaultTerminal;
use std::path::Path;
use std::time::Duration;
use store::Store;

const USAGE: &str = "\
owtui: browse and edit OpenWhispr history, dictionary and snippets

Usage:
  owtui                  open the TUI
  owtui --export FILE    write heard→final training pairs as JSONL and exit";

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let store = Store::default_paths();
    match args.as_slice() {
        [] => {}
        [flag, file] if flag == "--export" => {
            let n = store.export_pairs(Path::new(file))?;
            println!("{n} pairs written to {file}");
            return Ok(());
        }
        [flag] if flag == "-h" || flag == "--help" => {
            println!("{USAGE}");
            return Ok(());
        }
        _ => {
            eprintln!("{USAGE}");
            std::process::exit(2);
        }
    }
    if !store.db_path.is_file() && !store.bridge.file.is_file() {
        anyhow::bail!("No OpenWhispr data found at {}", store.db_path.display());
    }
    let mut app = App::new(store, std::env::current_dir()?);
    let mut terminal = ratatui::init();
    let result = run(&mut terminal, &mut app);
    ratatui::restore();
    result
}

fn run(terminal: &mut DefaultTerminal, app: &mut App) -> Result<()> {
    while !app.quit {
        terminal.draw(|frame| ui::draw(frame, app))?;
        // Wake now and then so status messages expire.
        if event::poll(Duration::from_millis(500))? {
            if let Event::Key(key) = event::read()? {
                app.handle_key(key);
            }
        }
    }
    Ok(())
}
