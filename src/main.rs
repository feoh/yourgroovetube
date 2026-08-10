use std::process::Command as ProcessCommand;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use crossterm::event::{self, Event};
use yourgroovetube::app::{Action, App};
use yourgroovetube::config::{AppConfig, config_path};
use yourgroovetube::provider::{SearchQuery, VideoCatalog};
use yourgroovetube::ui;
use yourgroovetube::youtube::YoutubeCatalog;

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
    let catalog = config
        .youtube_api_key()
        .map(|api_key| {
            YoutubeCatalog::new(
                api_key,
                config.youtube.region_code.clone(),
                config.youtube.results_per_page,
            )
        })
        .transpose()
        .context("could not configure the YouTube catalog")?;
    let mut app = App::new(catalog.is_some());
    if let Some(catalog) = catalog.as_ref() {
        app.status = "Loading popular videos…".to_owned();
        match catalog.default_feed(None).await {
            Ok(page) => app.replace_catalog_page(page, None),
            Err(error) => app.status = format!("Could not load popular videos: {error}"),
        }
    }

    let mut terminal = ratatui::init();
    let result = run_event_loop(&mut terminal, &mut app, catalog.as_ref()).await;
    ratatui::restore();
    result
}

async fn run_event_loop(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut App,
    catalog: Option<&YoutubeCatalog>,
) -> Result<()> {
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
                let Some(catalog) = catalog else {
                    app.status = "Configure a YouTube Data API key before searching".to_owned();
                    terminal.draw(|frame| ui::draw(frame, app))?;
                    continue;
                };
                app.status = format!("Searching for {query}…");
                terminal.draw(|frame| ui::draw(frame, app))?;
                match catalog.search(SearchQuery::new(query.clone())).await {
                    Ok(page) => app.replace_catalog_page(page, Some(query)),
                    Err(error) => app.status = format!("Search failed: {error}"),
                }
            }
            Action::NextPage => {
                let Some(catalog) = catalog else {
                    app.status = "Configure a YouTube Data API key before loading more".to_owned();
                    terminal.draw(|frame| ui::draw(frame, app))?;
                    continue;
                };
                let Some(page_token) = app.next_page_token.clone() else {
                    continue;
                };
                app.status = "Loading more videos…".to_owned();
                terminal.draw(|frame| ui::draw(frame, app))?;
                let result = if let Some(query) = app.active_search.clone() {
                    let mut search = SearchQuery::new(query);
                    search.page_token = Some(page_token);
                    catalog.search(search).await
                } else {
                    catalog.default_feed(Some(page_token)).await
                };
                match result {
                    Ok(page) => app.append_catalog_page(page),
                    Err(error) => app.status = format!("Could not load more videos: {error}"),
                }
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
        "  info YouTube region/page size: {}/{}",
        config.youtube.region_code, config.youtube.results_per_page
    );
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
