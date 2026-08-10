use std::process::Command as ProcessCommand;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use crossterm::event::{self, Event};
use yourgroovetube::app::{Action, App};
use yourgroovetube::artwork::ArtworkState;
use yourgroovetube::config::{AppConfig, config_path};
use yourgroovetube::models::PlaybackMode;
use yourgroovetube::playback::{MpvEngine, PlaybackEngine, PlaybackError};
use yourgroovetube::provider::{SearchQuery, VideoCatalog};
use yourgroovetube::ui;
use yourgroovetube::youtube::YoutubeCatalog;

#[derive(Debug, Parser)]
#[command(author, version, about)]
struct Cli {
    /// Force portable Unicode half-block thumbnails.
    #[arg(long, global = true)]
    no_images: bool,
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
    let Cli { command, no_images } = Cli::parse();
    match command {
        Some(Command::Doctor) => run_doctor(),
        Some(Command::Config {
            command: ConfigCommand::Path,
        }) => {
            println!("{}", config_path()?.display());
            Ok(())
        }
        None => run_app(no_images).await,
    }
}

async fn run_app(no_images: bool) -> Result<()> {
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

    let mut artwork = if no_images {
        ArtworkState::halfblocks()
    } else {
        ArtworkState::detect()
    }
    .context("could not initialize terminal thumbnails")?;
    update_artwork(&mut app, &mut artwork).await;
    let mut player = MpvEngine::new();
    let mut terminal = ratatui::init();
    let result = run_event_loop(
        &mut terminal,
        &mut app,
        catalog.as_ref(),
        &mut player,
        &mut artwork,
    )
    .await;
    ratatui::restore();
    result
}

async fn run_event_loop(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut App,
    catalog: Option<&YoutubeCatalog>,
    player: &mut MpvEngine,
    artwork: &mut ArtworkState,
) -> Result<()> {
    draw_ui(terminal, app, artwork)?;

    while !app.should_quit {
        if !event::poll(Duration::from_millis(100))? {
            sync_playback(app, player)?;
            draw_ui(terminal, app, artwork)?;
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
                    draw_ui(terminal, app, artwork)?;
                    continue;
                };
                app.status = format!("Searching for {query}…");
                draw_ui(terminal, app, artwork)?;
                match catalog.search(SearchQuery::new(query.clone())).await {
                    Ok(page) => app.replace_catalog_page(page, Some(query)),
                    Err(error) => app.status = format!("Search failed: {error}"),
                }
            }
            Action::NextPage => {
                let Some(catalog) = catalog else {
                    app.status = "Configure a YouTube Data API key before loading more".to_owned();
                    draw_ui(terminal, app, artwork)?;
                    continue;
                };
                let Some(page_token) = app.next_page_token.clone() else {
                    continue;
                };
                app.status = "Loading more videos…".to_owned();
                draw_ui(terminal, app, artwork)?;
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
            Action::Play(video) => match player.play(&video, app.playback.mode) {
                Ok(()) => {
                    app.start_playback(video);
                    app.status = "Playing through mpv".to_owned();
                }
                Err(error) => app.status = format!("Playback failed: {error}"),
            },
            Action::SetMode(mode) => match player.set_mode(mode) {
                Ok(()) => app.status = format!("Playback mode: {}", mode.label()),
                Err(error) => app.status = format!("Could not change playback mode: {error}"),
            },
            Action::TogglePause if app.playback.current.is_none() => {
                app.status = "Nothing is playing".to_owned();
            }
            Action::TogglePause => {
                let paused = !app.playback.paused;
                match player.set_paused(paused) {
                    Ok(()) => {
                        app.playback.paused = paused;
                        app.status = if paused {
                            "Playback paused".to_owned()
                        } else {
                            "Playback resumed".to_owned()
                        };
                    }
                    Err(error) => app.status = format!("Could not pause playback: {error}"),
                }
            }
            Action::SaveToPlex => {
                app.status = "Plex save workflow is not implemented yet".to_owned();
            }
        }

        update_artwork(app, artwork).await;
        sync_playback(app, player)?;
        draw_ui(terminal, app, artwork)?;
    }

    let _ = player.stop();
    Ok(())
}

fn draw_ui(
    terminal: &mut ratatui::DefaultTerminal,
    app: &App,
    artwork: &mut ArtworkState,
) -> Result<()> {
    terminal.draw(|frame| ui::draw(frame, app, Some(artwork)))?;
    Ok(())
}

async fn update_artwork(app: &mut App, artwork: &mut ArtworkState) {
    let thumbnail = if app.playback.mode == PlaybackMode::Audio {
        app.playback
            .current
            .as_ref()
            .or_else(|| app.selected_video())
    } else {
        app.selected_video()
    }
    .and_then(|video| video.thumbnail_url.clone());

    match thumbnail {
        Some(url) => {
            if let Err(error) = artwork.load_url(&url).await {
                artwork.show_placeholder();
                app.status = format!("Could not load thumbnail: {error}");
            }
        }
        None => artwork.show_placeholder(),
    }
}

fn sync_playback(app: &mut App, player: &MpvEngine) -> Result<(), PlaybackError> {
    let snapshot = player.snapshot()?;
    if snapshot.last_error != app.playback.last_error
        && let Some(error) = snapshot.last_error.as_ref()
    {
        app.status = format!("mpv: {error}");
    }
    app.playback = snapshot;
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
