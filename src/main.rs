use std::process::Command as ProcessCommand;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use crossterm::event::{self, Event};
use yourgroovetube::app::{Action, App};
use yourgroovetube::config::{AppConfig, config_path};
use yourgroovetube::ui;

#[derive(Debug, Parser)]
#[command(author, version, about)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Check local configuration and external playback dependencies.
    Doctor,
    /// Inspect application configuration.
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
}

#[derive(Debug, Subcommand)]
enum ConfigCommand {
    /// Print the platform-specific configuration file path.
    Path,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Some(Command::Doctor) => run_doctor(),
        Some(Command::Config {
            command: ConfigCommand::Path,
        }) => {
            println!("{}", config_path()?.display());
            Ok(())
        }
        None => run_app().await,
    }
}

async fn run_app() -> Result<()> {
    let config = AppConfig::load().context("could not load yourgroovetube configuration")?;
    let mut app = App::new(config.youtube_api_key().is_some());

    let mut terminal = ratatui::init();
    let result = run_event_loop(&mut terminal, &mut app).await;
    ratatui::restore();
    result
}

async fn run_event_loop(terminal: &mut ratatui::DefaultTerminal, app: &mut App) -> Result<()> {
    terminal.draw(|frame| ui::draw(frame, app))?;

    while !app.should_quit {
        if !event::poll(Duration::from_millis(100))? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };

        match app.handle_key(key) {
            Action::None => {}
            Action::Quit => app.should_quit = true,
            Action::Search(query) => {
                app.status = format!("Search queued for: {query}");
            }
            Action::Play(video) => {
                app.start_playback(video);
                app.status = "Playback engine will be connected in the next milestone".to_owned();
            }
            Action::TogglePause => {
                app.status = "Pause command will be sent through mpv JSON IPC".to_owned();
            }
            Action::SaveToPlex => {
                app.status = "Plex save workflow is not implemented yet".to_owned();
            }
        }

        terminal.draw(|frame| ui::draw(frame, app))?;
    }

    Ok(())
}

fn run_doctor() -> Result<()> {
    let config = AppConfig::load().context("could not load configuration")?;
    let checks = [
        ("mpv", dependency_version("mpv")),
        ("yt-dlp", dependency_version("yt-dlp")),
    ];

    println!("yourgroovetube doctor");
    for (name, result) in checks {
        match result {
            Some(version) => println!("  ok   {name}: {version}"),
            None => println!("  miss {name}: not found on PATH"),
        }
    }
    match config.youtube_api_key() {
        Some(_) => println!("  ok   YouTube Data API key configured"),
        None => println!(
            "  miss YouTube Data API key: set YOURGROOVETUBE_YOUTUBE_API_KEY or config.toml"
        ),
    }
    println!(
        "  info Plex destination: {}",
        config.plex.library_dir.display()
    );
    Ok(())
}

fn dependency_version(program: &str) -> Option<String> {
    let output = ProcessCommand::new(program)
        .arg("--version")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()?
        .lines()
        .next()
        .map(str::to_owned)
}
