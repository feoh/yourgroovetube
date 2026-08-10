use crossterm::event::{KeyCode, KeyEvent, KeyEventKind};

use crate::models::Video;
use crate::playback::PlaybackSnapshot;
use crate::provider::CatalogPage;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Action {
    None,
    Quit,
    Search(String),
    NextPage,
    Play(Video),
    TogglePause,
    SaveToPlex,
}

#[derive(Debug)]
pub struct App {
    pub videos: Vec<Video>,
    pub selected: usize,
    pub search_active: bool,
    pub search_query: String,
    pub help_visible: bool,
    pub should_quit: bool,
    pub status: String,
    pub feed_label: String,
    pub active_search: Option<String>,
    pub next_page_token: Option<String>,
    pub playback: PlaybackSnapshot,
}

impl App {
    pub fn new(api_configured: bool) -> Self {
        let status = if api_configured {
            "Press / to search YouTube".to_owned()
        } else {
            "Configure a YouTube Data API key, then press / to search".to_owned()
        };
        Self {
            videos: Vec::new(),
            selected: 0,
            search_active: false,
            search_query: String::new(),
            help_visible: false,
            should_quit: false,
            status,
            feed_label: "Popular videos".to_owned(),
            active_search: None,
            next_page_token: None,
            playback: PlaybackSnapshot::default(),
        }
    }

    pub fn selected_video(&self) -> Option<&Video> {
        self.videos.get(self.selected)
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
        self.status = format!("Loaded {} videos", self.videos.len());
    }

    pub fn append_catalog_page(&mut self, page: CatalogPage) {
        let added = page.videos.len();
        self.videos.extend(page.videos);
        self.next_page_token = page.next_page_token;
        self.status = format!("Loaded {added} more videos ({} total)", self.videos.len());
    }

    pub fn start_playback(&mut self, video: Video) {
        self.playback.current = Some(video);
        self.playback.position_seconds = 0.0;
        self.playback.duration_seconds = 0.0;
        self.playback.paused = false;
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Action {
        if key.kind != KeyEventKind::Press {
            return Action::None;
        }
        if self.search_active {
            return self.handle_search_key(key.code);
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
                Action::None
            }
            KeyCode::Char('n') if self.next_page_token.is_some() => Action::NextPage,
            KeyCode::Char(' ') => Action::TogglePause,
            KeyCode::Char('s') => Action::SaveToPlex,
            _ => Action::None,
        }
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
    use crate::models::PlaybackMode;

    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::from(code)
    }

    #[test]
    fn search_captures_text_before_global_shortcuts() {
        let mut app = App::new(true);

        app.handle_key(key(KeyCode::Char('/')));
        app.handle_key(key(KeyCode::Char('q')));
        let action = app.handle_key(key(KeyCode::Enter));

        assert_eq!(action, Action::Search("q".to_owned()));
        assert!(!app.should_quit);
    }

    #[test]
    fn playback_mode_toggles_between_video_and_audio() {
        let mut app = App::new(true);

        app.handle_key(key(KeyCode::Char('m')));
        assert_eq!(app.playback.mode, PlaybackMode::Audio);
        app.handle_key(key(KeyCode::Char('m')));
        assert_eq!(app.playback.mode, PlaybackMode::Video);
    }

    #[test]
    fn next_page_is_only_actionable_when_a_token_exists() {
        let mut app = App::new(true);

        assert_eq!(app.handle_key(key(KeyCode::Char('n'))), Action::None);
        app.next_page_token = Some("next-token".to_owned());
        assert_eq!(app.handle_key(key(KeyCode::Char('n'))), Action::NextPage);
    }
}
