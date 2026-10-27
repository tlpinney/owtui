//! owtui: browse and edit OpenWhispr history, dictionary and snippets.

mod app;
mod editor;
mod insights;
mod player;
mod privacy;
mod store;
#[cfg(test)]
mod tests;
mod ui;

use anyhow::Result;
use app::App;
use crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, Event};
use ratatui::DefaultTerminal;
use std::path::Path;
use std::time::Duration;
use store::Store;

const USAGE: &str = "\
owtui: browse and edit OpenWhispr history, dictionary and snippets

Usage:
  owtui                  open the TUI
  owtui --export FILE    write heard→final training pairs as JSONL and exit
  owtui --version        print the version";

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
        [flag] if flag == "-V" || flag == "--version" => {
            println!("owtui {}", env!("CARGO_PKG_VERSION"));
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
    crossterm::execute!(std::io::stdout(), EnableMouseCapture)?;
    let result = run(&mut terminal, &mut app);
    let _ = crossterm::execute!(std::io::stdout(), DisableMouseCapture);
    ratatui::restore();
    result
}

fn run(terminal: &mut DefaultTerminal, app: &mut App) -> Result<()> {
    let mut pixel_mouse = false;
    while !app.quit {
        app.player.poll();
        app.watch();
        terminal.draw(|frame| ui::draw(frame, app))?;
        // Wake now and then so status messages expire.
        if event::poll(Duration::from_millis(500))? {
            match event::read()? {
                Event::Key(key) => app.handle_key(key),
                Event::Mouse(mouse) => {
                    let mouse = match crossterm::terminal::window_size() {
                        Ok(size) => {
                            let size = app::ScreenSize {
                                columns: size.columns,
                                rows: size.rows,
                                width_px: size.width,
                                height_px: size.height,
                            };
                            app::mouse_to_cells(mouse, size, &mut pixel_mouse)
                        }
                        Err(_) => mouse,
                    };
                    app.handle_mouse(mouse)
                }
                _ => {}
            }
        }
    }
    Ok(())
}
