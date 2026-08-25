use crossterm::event::{KeyCode, KeyEvent, KeyEventKind};
use rand::seq::SliceRandom;

use crate::models::{PlaybackMode, SavedPlaylist, Video};
use crate::playback::PlaybackSnapshot;
use crate::provider::CatalogPage;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Action {
    None,
    Quit,
    Search(String),
    LoadPlaylist {
        value: String,
        label: Option<String>,
    },
    SavePlaylist {
        name: String,
        value: String,
    },
    DeleteSavedPlaylist(usize),
    NextPage,
    Play(Video),
    SetMode(PlaybackMode),
    TogglePause,
    QueueNext,
    QueuePrevious,
    SaveToPlex,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PlaylistDialog {
    #[default]
    Closed,
    Library,
    OneOff,
    AddName,
    AddValue,
}

#[derive(Debug)]
pub struct App {
    pub videos: Vec<Video>,
    pub selected: usize,
    pub search_active: bool,
    pub search_query: String,
    pub playlist_dialog: PlaylistDialog,
    pub playlist_query: String,
    pub playlist_pending_name: String,
    pub saved_playlists: Vec<SavedPlaylist>,
    pub saved_playlist_selected: usize,
    pub shuffle_enabled: bool,
    pub help_visible: bool,
    pub should_quit: bool,
    pub status: String,
    pub feed_label: String,
    pub active_search: Option<String>,
    pub active_playlist: Option<String>,
    pub next_page_token: Option<String>,
    pub queue: Vec<Video>,
    pub queue_index: Option<usize>,
    pub playback: PlaybackSnapshot,
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl App {
    pub fn new() -> Self {
        Self::with_saved_playlists(Vec::new())
    }

    pub fn with_saved_playlists(saved_playlists: Vec<SavedPlaylist>) -> Self {
        Self {
            videos: Vec::new(),
            selected: 0,
            search_active: false,
            search_query: String::new(),
            playlist_dialog: PlaylistDialog::Closed,
            playlist_query: String::new(),
            playlist_pending_name: String::new(),
            saved_playlists,
            saved_playlist_selected: 0,
            shuffle_enabled: false,
            help_visible: false,
            should_quit: false,
            status: "Press / to search YouTube".to_owned(),
            feed_label: "Popular videos".to_owned(),
            active_search: None,
            active_playlist: None,
            next_page_token: None,
            queue: Vec::new(),
            queue_index: None,
            playback: PlaybackSnapshot::default(),
        }
    }

    pub fn selected_video(&self) -> Option<&Video> {
        self.videos.get(self.selected)
    }

    pub fn set_saved_playlists(&mut self, saved_playlists: Vec<SavedPlaylist>) {
        self.saved_playlists = saved_playlists;
        self.saved_playlist_selected = self
            .saved_playlist_selected
            .min(self.saved_playlists.len().saturating_sub(1));
    }

    pub fn replace_catalog_page(&mut self, page: CatalogPage, search: Option<String>) {
        self.videos = page.videos;
        self.selected = 0;
        self.next_page_token = page.next_page_token;
        self.feed_label = search.as_ref().map_or_else(
            || "Popular videos".to_owned(),
            |query| format!("Search · {query}"),
        );
        self.active_search = search;
        self.active_playlist = None;
        self.queue.clear();
        self.queue_index = None;
        self.status = format!("Loaded {} videos", self.videos.len());
    }

    pub fn replace_playlist_page(
        &mut self,
        page: CatalogPage,
        playlist_id: String,
        label: Option<String>,
    ) {
        self.videos = page.videos;
        self.selected = 0;
        self.next_page_token = page.next_page_token;
        self.feed_label = format!(
            "Playlist · {}",
            label.unwrap_or_else(|| playlist_id.clone())
        );
        self.active_search = None;
        self.active_playlist = Some(playlist_id);
        self.queue.clear();
        self.queue_index = None;
        self.status = format!("Loaded {} playlist videos", self.videos.len());
    }

    pub fn append_catalog_page(&mut self, page: CatalogPage) {
        let CatalogPage {
            videos,
            next_page_token,
        } = page;
        let added = videos.len();
        if self.active_playlist.is_some() && !self.queue.is_empty() {
            if self.shuffle_enabled {
                let mut queued_videos = videos.clone();
                queued_videos.shuffle(&mut rand::rng());
                self.queue.extend(queued_videos);
            } else {
                self.queue.extend(videos.iter().cloned());
            }
        }
        self.videos.extend(videos);
        self.next_page_token = next_page_token;
        self.status = format!("Loaded {added} more videos ({} total)", self.videos.len());
    }

    pub fn prepare_queue(&mut self, video: &Video) {
        if self.active_playlist.is_some() {
            if self.shuffle_enabled {
                let selected_index = self.videos.iter().position(|item| item.id == video.id);
                let mut remaining = self
                    .videos
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| Some(*index) != selected_index)
                    .map(|(_, item)| item.clone())
                    .collect::<Vec<_>>();
                remaining.shuffle(&mut rand::rng());
                self.queue = std::iter::once(video.clone()).chain(remaining).collect();
                self.queue_index = Some(0);
            } else {
                self.queue = self.videos.clone();
                self.queue_index = self.queue.iter().position(|item| item.id == video.id);
            }
        } else {
            self.queue.clear();
            self.queue_index = None;
        }
    }

    pub fn queue_relative(&mut self, delta: isize) -> Option<Video> {
        let current = self.queue_index?;
        let next = current.checked_add_signed(delta)?;
        let video = self.queue.get(next)?.clone();
        self.queue_index = Some(next);
        Some(video)
    }

    pub fn finish_queue_item(&mut self) -> Option<Video> {
        let next = self.queue_relative(1);
        if next.is_none() {
            self.queue_index = None;
        }
        next
    }

    pub fn start_playback(&mut self, video: Video) {
        self.playback.duration_seconds = video.duration_seconds.unwrap_or_default() as f64;
        self.playback.current = Some(video);
        self.playback.position_seconds = 0.0;
        self.playback.paused = false;
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Action {
        // Enhanced keyboard protocols report held keys as Repeat events. They are
        // actionable input too; only Release would otherwise duplicate a key.
        if key.kind == KeyEventKind::Release {
            return Action::None;
        }
        if self.search_active {
            return self.handle_search_key(key.code);
        }
        if self.playlist_dialog != PlaylistDialog::Closed {
            return self.handle_playlist_dialog_key(key.code);
        }
        if self.help_visible {
            self.help_visible = false;
            return Action::None;
        }

        match key.code {
            KeyCode::Char('q') => Action::Quit,
            KeyCode::Char('?') => {
                self.help_visible = true;
                Action::None
            }
            KeyCode::Char('/') => {
                self.search_active = true;
                self.search_query.clear();
                Action::None
            }
            KeyCode::Char('P') => {
                self.playlist_dialog = PlaylistDialog::Library;
                self.playlist_query.clear();
                Action::None
            }
            KeyCode::Char('j') | KeyCode::Down => {
                self.move_selection(1);
                Action::None
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.move_selection(-1);
                Action::None
            }
            KeyCode::Enter | KeyCode::Char('p') => self
                .selected_video()
                .cloned()
                .map(Action::Play)
                .unwrap_or(Action::None),
            KeyCode::Char('m') => {
                self.playback.mode = self.playback.mode.toggle();
                self.status = format!("Playback mode: {}", self.playback.mode.label());
                Action::SetMode(self.playback.mode)
            }
            KeyCode::Char('n') if self.next_page_token.is_some() => Action::NextPage,
            KeyCode::Char('r') if self.active_playlist.is_some() => {
                self.shuffle_enabled = !self.shuffle_enabled;
                if let Some(video) = self.playback.current.clone()
                    && self.queue_index.is_some()
                {
                    self.prepare_queue(&video);
                }
                self.status = format!(
                    "Playlist shuffle {} for loaded videos",
                    if self.shuffle_enabled { "on" } else { "off" }
                );
                Action::None
            }
            KeyCode::Char('r') => {
                self.status = "Shuffle is available while browsing a playlist".to_owned();
                Action::None
            }
            KeyCode::Char(' ') => Action::TogglePause,
            KeyCode::Char(']') if self.queue_index.is_some() => Action::QueueNext,
            KeyCode::Char('[') if self.queue_index.is_some() => Action::QueuePrevious,
            KeyCode::Char('s') => Action::SaveToPlex,
            _ => Action::None,
        }
    }

    fn handle_playlist_dialog_key(&mut self, code: KeyCode) -> Action {
        match self.playlist_dialog {
            PlaylistDialog::Closed => Action::None,
            PlaylistDialog::Library => self.handle_playlist_library_key(code),
            PlaylistDialog::OneOff => match code {
                KeyCode::Esc => {
                    self.playlist_dialog = PlaylistDialog::Library;
                    Action::None
                }
                KeyCode::Enter => {
                    let value = self.playlist_query.trim().to_owned();
                    if value.is_empty() {
                        self.status =
                            "Enter a YouTube playlist URL or ID, or Esc to go back".to_owned();
                        Action::None
                    } else {
                        self.playlist_dialog = PlaylistDialog::Closed;
                        Action::LoadPlaylist { value, label: None }
                    }
                }
                _ => self.capture_playlist_text(code),
            },
            PlaylistDialog::AddName => match code {
                KeyCode::Esc => {
                    self.playlist_dialog = PlaylistDialog::Library;
                    Action::None
                }
                KeyCode::Enter => {
                    let name = self.playlist_query.trim().to_owned();
                    if name.is_empty() {
                        self.status =
                            "Name this playlist first — the URL comes next. Esc to cancel"
                                .to_owned();
                        Action::None
                    } else {
                        self.playlist_pending_name = name;
                        self.playlist_query.clear();
                        self.playlist_dialog = PlaylistDialog::AddValue;
                        Action::None
                    }
                }
                _ => self.capture_playlist_text(code),
            },
            PlaylistDialog::AddValue => match code {
                KeyCode::Esc => {
                    self.playlist_dialog = PlaylistDialog::Library;
                    Action::None
                }
                KeyCode::Enter => {
                    let value = self.playlist_query.trim().to_owned();
                    if value.is_empty() {
                        self.status =
                            "Enter a YouTube playlist URL or ID to save, or Esc to cancel"
                                .to_owned();
                        Action::None
                    } else {
                        self.playlist_dialog = PlaylistDialog::Library;
                        Action::SavePlaylist {
                            name: std::mem::take(&mut self.playlist_pending_name),
                            value,
                        }
                    }
                }
                _ => self.capture_playlist_text(code),
            },
        }
    }

    fn handle_playlist_library_key(&mut self, code: KeyCode) -> Action {
        match code {
            KeyCode::Esc => {
                self.playlist_dialog = PlaylistDialog::Closed;
                Action::None
            }
            KeyCode::Char('a') => {
                self.playlist_query.clear();
                self.playlist_pending_name.clear();
                self.playlist_dialog = PlaylistDialog::AddName;
                Action::None
            }
            KeyCode::Char('o') => {
                self.playlist_query.clear();
                self.playlist_dialog = PlaylistDialog::OneOff;
                Action::None
            }
            KeyCode::Char('d') if !self.saved_playlists.is_empty() => {
                Action::DeleteSavedPlaylist(self.saved_playlist_selected)
            }
            KeyCode::Char('j') | KeyCode::Down => {
                self.move_saved_playlist_selection(1);
                Action::None
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.move_saved_playlist_selection(-1);
                Action::None
            }
            KeyCode::Enter => self
                .saved_playlists
                .get(self.saved_playlist_selected)
                .cloned()
                .map(|playlist| {
                    self.playlist_dialog = PlaylistDialog::Closed;
                    Action::LoadPlaylist {
                        value: playlist.playlist_id,
                        label: Some(playlist.name),
                    }
                })
                .unwrap_or(Action::None),
            _ => Action::None,
        }
    }

    fn capture_playlist_text(&mut self, code: KeyCode) -> Action {
        match code {
            KeyCode::Backspace => {
                self.playlist_query.pop();
            }
            KeyCode::Char(character) => self.playlist_query.push(character),
            _ => {}
        }
        Action::None
    }

    fn move_saved_playlist_selection(&mut self, delta: isize) {
        if self.saved_playlists.is_empty() {
            self.saved_playlist_selected = 0;
            return;
        }
        self.saved_playlist_selected = self
            .saved_playlist_selected
            .saturating_add_signed(delta)
            .min(self.saved_playlists.len() - 1);
    }

    fn handle_search_key(&mut self, code: KeyCode) -> Action {
        match code {
            KeyCode::Esc => {
                self.search_active = false;
                Action::None
            }
            KeyCode::Enter => {
                self.search_active = false;
                let query = self.search_query.trim().to_owned();
                if query.is_empty() {
                    Action::None
                } else {
                    Action::Search(query)
                }
            }
            KeyCode::Backspace => {
                self.search_query.pop();
                Action::None
            }
            KeyCode::Char(character) => {
                self.search_query.push(character);
                Action::None
            }
            _ => Action::None,
        }
    }

    fn move_selection(&mut self, delta: isize) {
        if self.videos.is_empty() {
            self.selected = 0;
            return;
        }
        self.selected = self
            .selected
            .saturating_add_signed(delta)
            .min(self.videos.len() - 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::from(code)
    }

    fn key_with_kind(code: KeyCode, kind: KeyEventKind) -> KeyEvent {
        KeyEvent {
            kind,
            ..KeyEvent::from(code)
        }
    }

    #[test]
    fn search_captures_text_before_global_shortcuts() {
        let mut app = App::new();

        app.handle_key(key(KeyCode::Char('/')));
        assert!(app.search_active);
        app.handle_key(key(KeyCode::Char('q')));
        assert_eq!(app.search_query, "q");
        let action = app.handle_key(key(KeyCode::Enter));

        assert_eq!(action, Action::Search("q".to_owned()));
        assert!(!app.should_quit);
    }

    #[test]
    fn repeat_keys_are_input_and_release_keys_are_ignored() {
        let mut app = App::new();

        assert_eq!(
            app.handle_key(key_with_kind(KeyCode::Char('/'), KeyEventKind::Repeat)),
            Action::None
        );
        assert!(app.search_active);
        app.handle_key(key_with_kind(KeyCode::Char('a'), KeyEventKind::Repeat));
        app.handle_key(key_with_kind(KeyCode::Char('b'), KeyEventKind::Release));

        assert_eq!(app.search_query, "a");
    }

    #[test]
    fn playlist_input_captures_urls_before_global_shortcuts() {
        let mut app = App::new();

        app.handle_key(key(KeyCode::Char('P')));
        app.handle_key(key(KeyCode::Char('o')));
        for character in "PL1234567890".chars() {
            app.handle_key(key(KeyCode::Char(character)));
        }
        let action = app.handle_key(key(KeyCode::Enter));

        assert_eq!(
            action,
            Action::LoadPlaylist {
                value: "PL1234567890".to_owned(),
                label: None,
            }
        );
        assert_eq!(app.playlist_dialog, PlaylistDialog::Closed);
    }

    #[test]
    fn saved_playlist_library_opens_named_entries() {
        let mut app = App::with_saved_playlists(vec![SavedPlaylist {
            name: "Coding".to_owned(),
            playlist_id: "PL1234567890".to_owned(),
        }]);

        app.handle_key(key(KeyCode::Char('P')));
        let action = app.handle_key(key(KeyCode::Enter));

        assert_eq!(
            action,
            Action::LoadPlaylist {
                value: "PL1234567890".to_owned(),
                label: Some("Coding".to_owned()),
            }
        );
    }

    #[test]
    fn playlist_library_add_flow_collects_a_name_and_url() {
        let mut app = App::new();

        app.handle_key(key(KeyCode::Char('P')));
        app.handle_key(key(KeyCode::Char('a')));
        for character in "Coding".chars() {
            app.handle_key(key(KeyCode::Char(character)));
        }
        app.handle_key(key(KeyCode::Enter));
        for character in "PL1234567890".chars() {
            app.handle_key(key(KeyCode::Char(character)));
        }
        let action = app.handle_key(key(KeyCode::Enter));

        assert_eq!(
            action,
            Action::SavePlaylist {
                name: "Coding".to_owned(),
                value: "PL1234567890".to_owned(),
            }
        );
        assert_eq!(app.playlist_dialog, PlaylistDialog::Library);
    }

    #[test]
    fn empty_playlist_prompts_explain_themselves_instead_of_doing_nothing() {
        let mut app = App::new();

        app.handle_key(key(KeyCode::Char('P')));
        app.handle_key(key(KeyCode::Char('a')));
        let action = app.handle_key(key(KeyCode::Enter));

        assert_eq!(action, Action::None);
        assert_eq!(app.playlist_dialog, PlaylistDialog::AddName);
        assert!(app.status.contains("Name this playlist first"));

        for character in "https://www.youtube.com/playlist?list=PL1234567890".chars() {
            app.handle_key(key(KeyCode::Char(character)));
        }
        app.handle_key(key(KeyCode::Enter));
        let action = app.handle_key(key(KeyCode::Enter));

        assert_eq!(action, Action::None);
        assert_eq!(app.playlist_dialog, PlaylistDialog::AddValue);
        assert!(app.status.contains("URL or ID to save"));
    }

    #[test]
    fn playlist_queue_moves_in_order() {
        let mut app = App::new();
        app.active_playlist = Some("PL1234567890".to_owned());
        app.videos = ["first", "second", "third"]
            .into_iter()
            .map(|id| Video {
                id: id.to_owned(),
                ..Video::default()
            })
            .collect();
        let first = app.videos[0].clone();
        app.prepare_queue(&first);

        assert_eq!(
            app.queue_relative(1).map(|video| video.id),
            Some("second".to_owned())
        );
        assert_eq!(
            app.queue_relative(1).map(|video| video.id),
            Some("third".to_owned())
        );
        assert!(app.queue_relative(1).is_none());
        assert_eq!(
            app.queue_relative(-1).map(|video| video.id),
            Some("second".to_owned())
        );
    }

    #[test]
    fn shuffle_queue_starts_with_the_selected_video_without_duplicates() {
        let mut app = App::new();
        app.active_playlist = Some("PL1234567890".to_owned());
        app.shuffle_enabled = true;
        app.videos = ["first", "second", "third", "fourth"]
            .into_iter()
            .map(|id| Video {
                id: id.to_owned(),
                ..Video::default()
            })
            .collect();
        let selected = app.videos[2].clone();

        app.prepare_queue(&selected);

        assert_eq!(
            app.queue.first().map(|video| video.id.as_str()),
            Some("third")
        );
        let mut ids = app
            .queue
            .iter()
            .map(|video| video.id.as_str())
            .collect::<Vec<_>>();
        ids.sort_unstable();
        assert_eq!(ids, vec!["first", "fourth", "second", "third"]);
        assert_eq!(app.queue_index, Some(0));
    }

    #[test]
    fn shuffled_page_append_preserves_the_browser_order() {
        let mut app = App::new();
        app.active_playlist = Some("PL1234567890".to_owned());
        app.shuffle_enabled = true;
        app.videos = ["first", "second"]
            .into_iter()
            .map(|id| Video {
                id: id.to_owned(),
                ..Video::default()
            })
            .collect();
        let first = app.videos[0].clone();
        app.prepare_queue(&first);

        app.append_catalog_page(CatalogPage {
            videos: ["third", "fourth"]
                .into_iter()
                .map(|id| Video {
                    id: id.to_owned(),
                    ..Video::default()
                })
                .collect(),
            next_page_token: None,
        });

        assert_eq!(
            app.videos
                .iter()
                .map(|video| video.id.as_str())
                .collect::<Vec<_>>(),
            vec!["first", "second", "third", "fourth"]
        );
        let mut queued_ids = app
            .queue
            .iter()
            .map(|video| video.id.as_str())
            .collect::<Vec<_>>();
        queued_ids.sort_unstable();
        assert_eq!(queued_ids, vec!["first", "fourth", "second", "third"]);
    }

    #[test]
    fn playback_mode_toggles_between_video_and_audio() {
        let mut app = App::new();

        assert_eq!(
            app.handle_key(key(KeyCode::Char('m'))),
            Action::SetMode(PlaybackMode::Audio)
        );
        assert_eq!(app.playback.mode, PlaybackMode::Audio);
        assert_eq!(
            app.handle_key(key(KeyCode::Char('m'))),
            Action::SetMode(PlaybackMode::Video)
        );
        assert_eq!(app.playback.mode, PlaybackMode::Video);
    }

    #[test]
    fn next_page_is_only_actionable_when_a_token_exists() {
        let mut app = App::new();

        assert_eq!(app.handle_key(key(KeyCode::Char('n'))), Action::None);
        app.next_page_token = Some("next-token".to_owned());
        assert_eq!(app.handle_key(key(KeyCode::Char('n'))), Action::NextPage);
    }
}
