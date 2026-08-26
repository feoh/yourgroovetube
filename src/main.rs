use std::path::PathBuf;
use std::process::Command as ProcessCommand;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use crossterm::event;
use yourgroovetube::app::{Action, App, PlaylistDialog};
use yourgroovetube::artwork::ArtworkState;
use yourgroovetube::config::{AppConfig, config_path};
use yourgroovetube::download::{SaveError, SaveProgress, VideoSaver, YoutubeSaver};
use yourgroovetube::models::{PlaybackMode, SavedPlaylist};
use yourgroovetube::playback::{MpvEngine, PlaybackEngine, PlaybackError};
use yourgroovetube::provider::{CatalogPage, SearchQuery, VideoCatalog};
use yourgroovetube::ui;
use yourgroovetube::youtube::{YoutubeCatalog, parse_playlist_id};

struct SaveJob {
    task: tokio::task::JoinHandle<Result<PathBuf, SaveError>>,
    progress: tokio::sync::mpsc::UnboundedReceiver<SaveProgress>,
    latest_progress: SaveProgress,
    started_at: Instant,
}

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

fn acquire_youtube_api_key(config: &mut AppConfig) -> Result<(String, bool)> {
    let existing_api_key = config.youtube_api_key();
    acquire_youtube_api_key_with(config, existing_api_key, || {
        rpassword::prompt_password("YouTube API key (input hidden; blank cancels): ")
            .context("could not read the YouTube API key from the terminal")
    })
}

fn acquire_youtube_api_key_with<F>(
    config: &mut AppConfig,
    existing_api_key: Option<String>,
    prompt: F,
) -> Result<(String, bool)>
where
    F: FnOnce() -> Result<String>,
{
    if let Some(api_key) = existing_api_key {
        return Ok((api_key, false));
    }

    let path = config_path().context("could not determine where to save configuration")?;
    eprintln!("yourgroovetube requires a YouTube Data API v3 key.");
    eprintln!(
        "Enable or create one at:\n  https://console.cloud.google.com/marketplace/product/google/youtube.googleapis.com"
    );
    eprintln!(
        "No key was found in YOURGROOVETUBE_YOUTUBE_API_KEY or {}.",
        path.display()
    );
    eprintln!("The key will be saved after the application validates it.");
    let api_key = prompt()?.trim().to_owned();
    if api_key.is_empty() {
        bail!("a YouTube API key is required; application startup cancelled");
    }
    config.youtube.api_key = Some(api_key.clone());
    Ok((api_key, true))
}

async fn validate_youtube_catalog<C, F>(catalog: &C, after_validation: F) -> Result<CatalogPage>
where
    C: VideoCatalog,
    F: FnOnce() -> Result<()>,
{
    let page = catalog
        .default_feed(None)
        .await
        .context("could not connect to the YouTube Data API; check the API key and network")?;
    after_validation()?;
    Ok(page)
}

async fn run_app(no_images: bool) -> Result<()> {
    let mut config = AppConfig::load().context("could not load yourgroovetube configuration")?;
    let (api_key, prompted) = acquire_youtube_api_key(&mut config)?;
    let catalog = YoutubeCatalog::new(
        api_key,
        config.youtube.region_code.clone(),
        config.youtube.results_per_page,
    )
    .context("could not configure the YouTube catalog")?;
    let page = validate_youtube_catalog(&catalog, || {
        if prompted {
            let path = config
                .save()
                .context("the API key was valid but could not be saved")?;
            eprintln!("Saved YouTube API configuration to {}.", path.display());
        }
        Ok(())
    })
    .await?;
    let mut app = App::with_saved_playlists(config.playlists.clone());
    app.replace_catalog_page(page, None);

    let mut artwork = if no_images {
        ArtworkState::halfblocks()
    } else {
        ArtworkState::detect()
    }
    .context("could not initialize terminal thumbnails")?;
    let cookies_from_browser = config.cookies_from_browser();
    let saver = YoutubeSaver::new(
        config.plex.library_dir.clone(),
        cookies_from_browser.clone(),
    );
    let mut player = MpvEngine::new(cookies_from_browser);
    let mut terminal = ratatui::init();
    let result = async {
        draw_ui(&mut terminal, &app, &mut artwork)?;
        update_artwork(&mut app, &mut artwork).await;
        run_event_loop(
            &mut terminal,
            &mut app,
            &catalog,
            &mut player,
            &mut artwork,
            &saver,
            &mut config,
        )
        .await
    }
    .await;
    ratatui::restore();
    result
}

async fn run_event_loop(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut App,
    catalog: &YoutubeCatalog,
    player: &mut MpvEngine,
    artwork: &mut ArtworkState,
    saver: &YoutubeSaver,
    config: &mut AppConfig,
) -> Result<()> {
    let mut save_job: Option<SaveJob> = None;
    draw_ui(terminal, app, artwork)?;

    while !app.should_quit {
        if !event::poll(Duration::from_millis(100))? {
            let previous_artwork_url = desired_artwork_url(app);
            sync_playback(app, player)?;
            advance_finished_queue(app, player);
            if desired_artwork_url(app) != previous_artwork_url {
                update_artwork(app, artwork).await;
            }
            poll_save_job(app, &mut save_job).await;
            draw_ui(terminal, app, artwork)?;
            continue;
        }
        let previous_artwork_url = desired_artwork_url(app);
        let Some(action) = ui::dispatch_input_event(terminal, app, event::read()?, Some(artwork))?
        else {
            continue;
        };

        match action {
            Action::None => {}
            Action::Quit => app.should_quit = true,
            Action::Search(query) => {
                app.status = format!("Searching for {query}…");
                draw_ui(terminal, app, artwork)?;
                match catalog.search(SearchQuery::new(query.clone())).await {
                    Ok(page) => app.replace_catalog_page(page, Some(query)),
                    Err(error) => app.status = format!("Search failed: {error}"),
                }
            }
            Action::LoadPlaylist { value, label } => match parse_playlist_id(&value) {
                Ok(playlist_id) => {
                    app.status = "Loading playlist…".to_owned();
                    draw_ui(terminal, app, artwork)?;
                    match catalog.playlist(&playlist_id, None).await {
                        Ok(page) => app.replace_playlist_page(page, playlist_id, label),
                        Err(error) => app.status = format!("Could not load playlist: {error}"),
                    }
                }
                Err(error) => app.status = error.to_string(),
            },
            Action::SavePlaylist { name, value } => {
                app.status = match persist_saved_playlist(config, app, name.clone(), value.clone())
                {
                    Ok(message) => message,
                    Err(error) => {
                        app.playlist_pending_name = name;
                        app.playlist_query = value;
                        app.playlist_dialog = PlaylistDialog::AddValue;
                        format!("Could not save playlist: {error}")
                    }
                };
            }
            Action::DeleteSavedPlaylist(index) => {
                app.status = match delete_saved_playlist(config, app, index) {
                    Ok(message) => message,
                    Err(error) => format!("Could not delete playlist: {error}"),
                };
            }
            Action::NextPage => {
                let Some(page_token) = app.next_page_token.clone() else {
                    continue;
                };
                app.status = "Loading more videos…".to_owned();
                draw_ui(terminal, app, artwork)?;
                let result = if let Some(playlist_id) = app.active_playlist.clone() {
                    catalog.playlist(&playlist_id, Some(page_token)).await
                } else if let Some(query) = app.active_search.clone() {
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
                play_video(app, player, video, true);
            }
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
            Action::QueueNext => play_queue_relative(app, player, 1),
            Action::QueuePrevious => play_queue_relative(app, player, -1),
            Action::SaveToPlex if save_job.is_some() => {
                app.status = "A Plex save is already running".to_owned();
            }
            Action::SaveToPlex => match app.playback.current.clone() {
                Some(video) => {
                    let saver = saver.clone();
                    let (progress_sender, progress) = tokio::sync::mpsc::unbounded_channel();
                    let task = tokio::spawn(async move {
                        saver.save_with_progress(&video, progress_sender).await
                    });
                    save_job = Some(SaveJob {
                        task,
                        progress,
                        latest_progress: SaveProgress::Preparing,
                        started_at: Instant::now(),
                    });
                    app.status = "Saving current video for Plex…".to_owned();
                }
                None => app.status = "Nothing is playing".to_owned(),
            },
        }

        sync_playback(app, player)?;
        if desired_artwork_url(app) != previous_artwork_url {
            update_artwork(app, artwork).await;
        }
        poll_save_job(app, &mut save_job).await;
        draw_ui(terminal, app, artwork)?;
    }

    if let Some(job) = save_job {
        job.task.abort();
    }
    let _ = player.stop();
    Ok(())
}

fn persist_saved_playlist(
    config: &mut AppConfig,
    app: &mut App,
    name: String,
    value: String,
) -> Result<String> {
    let name = name.trim().to_owned();
    if name.is_empty() {
        bail!("playlist name must not be empty");
    }
    let playlist_id = parse_playlist_id(&value)?;
    let playlist = SavedPlaylist {
        name: name.clone(),
        playlist_id,
    };
    let mut updated = config.clone();
    let replaced = updated
        .playlists
        .iter()
        .position(|saved| saved.name.eq_ignore_ascii_case(&name));
    if let Some(index) = replaced {
        updated.playlists[index] = playlist;
    } else {
        updated.playlists.push(playlist);
    }
    updated
        .save()
        .context("could not write the configuration file")?;
    *config = updated;
    app.set_saved_playlists(config.playlists.clone());
    if let Some(index) = config
        .playlists
        .iter()
        .position(|playlist| playlist.name.eq_ignore_ascii_case(&name))
    {
        app.saved_playlist_selected = index;
    }
    Ok(if replaced.is_some() {
        format!("Updated saved playlist: {name}")
    } else {
        format!("Saved playlist: {name}")
    })
}

fn delete_saved_playlist(config: &mut AppConfig, app: &mut App, index: usize) -> Result<String> {
    let mut updated = config.clone();
    let name = updated
        .playlists
        .get(index)
        .map(|playlist| playlist.name.clone())
        .context("saved playlist no longer exists")?;
    updated.playlists.remove(index);
    updated
        .save()
        .context("could not write the configuration file")?;
    *config = updated;
    app.set_saved_playlists(config.playlists.clone());
    Ok(format!("Deleted saved playlist: {name}"))
}

async fn poll_save_job(app: &mut App, save_job: &mut Option<SaveJob>) {
    let Some(job) = save_job.as_mut() else {
        return;
    };
    while let Ok(progress) = job.progress.try_recv() {
        job.latest_progress = progress;
    }
    if !job.task.is_finished() {
        app.status = running_save_status(job);
        return;
    }
    let Some(job) = save_job.take() else {
        return;
    };
    app.status = match job.task.await {
        Ok(Ok(path)) => format!("✓ Saved for Plex: {}", path.display()),
        Ok(Err(error)) => format!("✗ Could not save for Plex: {error}"),
        Err(error) => format!("✗ Plex save task failed: {error}"),
    };
}

fn running_save_status(job: &SaveJob) -> String {
    const SPINNER: [&str; 8] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧"];
    let elapsed = job.started_at.elapsed();
    let spinner = SPINNER[(elapsed.as_millis() / 100) as usize % SPINNER.len()];
    let phase = match &job.latest_progress {
        SaveProgress::Preparing => "preparing".to_owned(),
        SaveProgress::Downloading(percent) => format!("downloading {percent}"),
        SaveProgress::Finalizing => "finalizing".to_owned(),
    };
    format!(
        "{spinner} Saving for Plex · {phase} · {}:{:02}",
        elapsed.as_secs() / 60,
        elapsed.as_secs() % 60
    )
}

fn play_video(
    app: &mut App,
    player: &mut MpvEngine,
    video: yourgroovetube::models::Video,
    prepare_queue: bool,
) -> bool {
    match player.play(&video, app.playback.mode) {
        Ok(()) => {
            if prepare_queue {
                app.prepare_queue(&video);
            }
            app.start_playback(video);
            app.status = "Playing through mpv".to_owned();
            true
        }
        Err(error) => {
            app.status = format!("Playback failed: {error}");
            false
        }
    }
}

fn play_queue_relative(app: &mut App, player: &mut MpvEngine, delta: isize) {
    match app.queue_relative(delta) {
        Some(video) => {
            play_video(app, player, video, false);
        }
        None => app.status = "No playlist video in that direction".to_owned(),
    }
}

fn advance_finished_queue(app: &mut App, player: &mut MpvEngine) {
    if !app.playback.eof_reached || app.queue_index.is_none() {
        return;
    }
    match app.finish_queue_item() {
        Some(video) => {
            if !play_video(app, player, video, false) {
                app.queue_index = None;
            }
        }
        None => app.status = "Playlist queue finished".to_owned(),
    }
}

fn draw_ui(
    terminal: &mut ratatui::DefaultTerminal,
    app: &App,
    artwork: &mut ArtworkState,
) -> Result<()> {
    terminal.draw(|frame| ui::draw(frame, app, Some(artwork)))?;
    Ok(())
}

fn desired_artwork_url(app: &App) -> Option<String> {
    if app.playback.mode == PlaybackMode::Audio {
        app.playback
            .current
            .as_ref()
            .or_else(|| app.selected_video())
    } else {
        app.selected_video()
    }
    .and_then(|video| video.thumbnail_url.clone())
}

async fn update_artwork(app: &mut App, artwork: &mut ArtworkState) {
    match desired_artwork_url(app) {
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
        None => {
            println!("  miss YouTube Data API key: run the app interactively to configure it");
            println!(
                "  info Or set YOURGROOVETUBE_YOUTUBE_API_KEY or edit {}",
                config_path()?.display()
            );
        }
    }
    println!(
        "  info YouTube region/page size: {}/{}",
        config.youtube.region_code, config.youtube.results_per_page
    );
    match config.cookies_from_browser() {
        Some(browser) => println!("  info yt-dlp cookies: from browser {browser}"),
        None => println!("  info yt-dlp cookies: none (anonymous extraction)"),
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

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use yourgroovetube::provider::{CatalogError, CatalogFuture};

    use super::*;

    struct FakeCatalog {
        fail_default_feed: bool,
    }

    impl VideoCatalog for FakeCatalog {
        fn default_feed(&self, _page_token: Option<String>) -> CatalogFuture<'_> {
            Box::pin(async move {
                if self.fail_default_feed {
                    Err(CatalogError::Request("validation failed".to_owned()))
                } else {
                    Ok(CatalogPage::default())
                }
            })
        }

        fn search(&self, _query: SearchQuery) -> CatalogFuture<'_> {
            Box::pin(async { Ok(CatalogPage::default()) })
        }

        fn playlist<'a>(
            &'a self,
            _playlist_id: &'a str,
            _page_token: Option<String>,
        ) -> CatalogFuture<'a> {
            Box::pin(async { Ok(CatalogPage::default()) })
        }
    }

    #[test]
    fn configured_api_key_skips_the_prompt() {
        let mut config = AppConfig::default();
        let result = acquire_youtube_api_key_with(
            &mut config,
            Some("configured-key".to_owned()),
            || -> Result<String> { panic!("configured key must not prompt") },
        );
        let Ok((api_key, prompted)) = result else {
            panic!("configured key should be accepted");
        };

        assert_eq!(api_key, "configured-key");
        assert!(!prompted);
    }

    #[test]
    fn prompted_api_key_is_trimmed_and_staged_for_saving() {
        let mut config = AppConfig::default();
        let result =
            acquire_youtube_api_key_with(&mut config, None, || Ok("  prompted-key  ".to_owned()));
        let Ok((api_key, prompted)) = result else {
            panic!("prompted key should be accepted");
        };

        assert_eq!(api_key, "prompted-key");
        assert!(prompted);
        assert_eq!(config.youtube.api_key.as_deref(), Some("prompted-key"));
    }

    #[test]
    fn blank_prompt_cancels_startup() {
        let mut config = AppConfig::default();
        let result = acquire_youtube_api_key_with(&mut config, None, || Ok("   ".to_owned()));

        assert!(result.is_err());
        assert!(config.youtube.api_key.is_none());
    }

    #[tokio::test]
    async fn failed_catalog_validation_does_not_save_configuration() {
        let catalog = FakeCatalog {
            fail_default_feed: true,
        };
        let saved = Cell::new(false);

        let result = validate_youtube_catalog(&catalog, || {
            saved.set(true);
            Ok(())
        })
        .await;

        assert!(result.is_err());
        assert!(!saved.get());
    }

    #[tokio::test]
    async fn successful_catalog_validation_saves_before_startup_continues() {
        let catalog = FakeCatalog {
            fail_default_feed: false,
        };
        let saved = Cell::new(false);

        let result = validate_youtube_catalog(&catalog, || {
            saved.set(true);
            Ok(())
        })
        .await;

        assert!(result.is_ok());
        assert!(saved.get());
    }
}
