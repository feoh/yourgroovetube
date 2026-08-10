use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Gauge, List, ListItem, ListState, Paragraph, Wrap};

use crate::app::App;
use crate::artwork::ArtworkState;
use crate::models::PlaybackMode;

pub fn draw(frame: &mut Frame<'_>, app: &App, artwork: Option<&mut ArtworkState>) {
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(10),
            Constraint::Length(5),
            Constraint::Length(1),
        ])
        .split(frame.area());

    render_header(frame, sections[0], app);
    render_browser(frame, sections[1], app, artwork);
    render_player(frame, sections[2], app);
    frame.render_widget(
        Paragraph::new(app.status.as_str()).style(Style::default().fg(Color::DarkGray)),
        sections[3],
    );

    if app.search_active {
        render_input(
            frame,
            centered_rect(70, 5, frame.area()),
            " Search title or tags ",
            &app.search_query,
        );
    }
    if app.playlist_active {
        render_input(
            frame,
            centered_rect(70, 5, frame.area()),
            " Open playlist URL or ID ",
            &app.playlist_query,
        );
    }
    if app.help_visible {
        render_help(frame, centered_rect(60, 15, frame.area()));
    }
}

fn render_header(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let controls = if area.width >= 100 {
        "  / search  P playlist  n more  [/] track  m mode  Space pause  s save  ? help  q quit"
    } else {
        "  / search  ? help  q quit"
    };
    let title = Line::from(vec![
        Span::styled(
            " yourgroovetube ",
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(controls),
    ]);
    let mode = format!("mode: {} ", app.playback.mode.label());
    let header = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min(10),
            Constraint::Length(mode.chars().count() as u16),
        ])
        .split(area);
    frame.render_widget(
        Paragraph::new(title)
            .block(Block::default().borders(Borders::BOTTOM))
            .alignment(Alignment::Left),
        header[0],
    );
    frame.render_widget(
        Paragraph::new(mode)
            .style(Style::default().fg(Color::Yellow))
            .alignment(Alignment::Right)
            .block(Block::default().borders(Borders::BOTTOM)),
        header[1],
    );
}

fn render_browser(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &App,
    artwork: Option<&mut ArtworkState>,
) {
    let panes = Layout::default()
        .direction(if area.width >= 80 {
            Direction::Horizontal
        } else {
            Direction::Vertical
        })
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(area);

    let items = if app.videos.is_empty() {
        vec![ListItem::new("No videos loaded. Press / to search.")]
    } else {
        app.videos
            .iter()
            .map(|video| {
                ListItem::new(Line::from(vec![
                    Span::styled(&video.title, Style::default().add_modifier(Modifier::BOLD)),
                    Span::raw(format!("  — {}", video.channel_title)),
                ]))
            })
            .collect()
    };
    let list_title = format!(" {} ", app.feed_label);
    let list = List::new(items)
        .block(Block::default().title(list_title).borders(Borders::ALL))
        .highlight_style(
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");
    let mut state = ListState::default();
    if !app.videos.is_empty() {
        state.select(Some(app.selected));
    }
    frame.render_stateful_widget(list, panes[0], &mut state);

    let details = app.selected_video().map_or_else(
        || "Select a search result to view its metadata.".to_owned(),
        |video| {
            format!(
                "{}\n\nChannel: {}\nDuration: {}\n\n{}",
                video.title,
                video.channel_title,
                format_duration(video.duration_seconds),
                video.description
            )
        },
    );
    let show_artwork = artwork.is_some() && panes[1].width >= 24 && panes[1].height >= 10;
    let detail_panes = Layout::default()
        .direction(Direction::Vertical)
        .constraints(if show_artwork {
            [Constraint::Percentage(60), Constraint::Percentage(40)]
        } else {
            [Constraint::Length(0), Constraint::Percentage(100)]
        })
        .split(panes[1]);
    if let Some(artwork) = artwork.filter(|_| show_artwork) {
        let title = if app.playback.mode == PlaybackMode::Audio && app.playback.current.is_some() {
            " Now playing thumbnail "
        } else {
            " Thumbnail "
        };
        let block = Block::default().title(title).borders(Borders::ALL);
        let inner = block.inner(detail_panes[0]);
        frame.render_widget(block, detail_panes[0]);
        artwork.render(frame, inner);
    }
    frame.render_widget(
        Paragraph::new(details)
            .wrap(Wrap { trim: true })
            .block(Block::default().title(" Details ").borders(Borders::ALL)),
        detail_panes[1],
    );
}

fn render_player(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let current = app
        .playback
        .current
        .as_ref()
        .map_or("Nothing playing", |video| video.title.as_str());
    let state = if app.playback.current.is_none() {
        "idle"
    } else if app.playback.paused {
        "paused"
    } else if app.playback.eof_reached {
        "finished"
    } else {
        "playing"
    };
    let label = format!(
        "[{state}] {current}  {} / {}",
        format_clock(app.playback.position_seconds),
        format_clock(app.playback.duration_seconds)
    );
    let gauge = Gauge::default()
        .block(
            Block::default()
                .title(" Now playing ")
                .borders(Borders::ALL),
        )
        .gauge_style(Style::default().fg(Color::Cyan))
        .ratio(app.playback.progress_ratio())
        .label(label);
    frame.render_widget(gauge, area);
}

fn render_input(frame: &mut Frame<'_>, area: Rect, title: &str, value: &str) {
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(value).block(Block::default().title(title).borders(Borders::ALL)),
        area,
    );
    let cursor_offset = value.chars().count() as u16;
    frame.set_cursor_position((area.x + cursor_offset + 1, area.y + 1));
}

fn render_help(frame: &mut Frame<'_>, area: Rect) {
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(
            "/       Search by title or tags\n\
             j/k     Move through videos\n\
             Enter   Play selected video\n\
             n       Load the next result page\n\
             P       Open a YouTube playlist URL or ID\n\
             [ / ]   Previous / next playlist video\n\
             m       Toggle video / audio + thumbnail\n\
             Space   Pause or resume\n\
             s       Save current video to Plex directory\n\
             q       Quit\n\n\
             Press any key to close help.",
        )
        .block(Block::default().title(" Keys ").borders(Borders::ALL)),
        area,
    );
}

fn centered_rect(percent_x: u16, height: u16, area: Rect) -> Rect {
    let width = area.width.saturating_mul(percent_x) / 100;
    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height: height.min(area.height),
    }
}

fn format_duration(seconds: Option<u64>) -> String {
    seconds.map_or_else(|| "unknown".to_owned(), |value| format_clock(value as f64))
}

fn format_clock(seconds: f64) -> String {
    let total = seconds.max(0.0).round() as u64;
    format!("{}:{:02}", total / 60, total % 60)
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;

    #[test]
    fn shell_renders_requested_controls() {
        let backend = TestBackend::new(100, 24);
        let mut terminal = match Terminal::new(backend) {
            Ok(terminal) => terminal,
            Err(never) => match never {},
        };
        let app = App::new(false);

        if terminal.draw(|frame| draw(frame, &app, None)).is_err() {
            panic!("frame should render");
        }
        let rendered = terminal.backend().to_string();

        assert!(rendered.contains("yourgroovetube"));
        assert!(rendered.contains("audio + thumbnail") || rendered.contains("mode: video"));
        assert!(rendered.contains("Configure a YouTube Data API key"));
    }

    #[test]
    fn wide_terminals_render_the_thumbnail_pane() {
        let backend = TestBackend::new(120, 30);
        let mut terminal = match Terminal::new(backend) {
            Ok(terminal) => terminal,
            Err(never) => match never {},
        };
        let app = App::new(false);
        let Ok(mut artwork) = ArtworkState::halfblocks() else {
            panic!("half-block artwork should initialize");
        };

        if terminal
            .draw(|frame| draw(frame, &app, Some(&mut artwork)))
            .is_err()
        {
            panic!("artwork frame should render");
        }
        let rendered = terminal.backend().to_string();

        assert!(rendered.contains("Thumbnail"));
        assert!(rendered.contains("Details"));
    }

    #[test]
    fn narrow_terminals_stack_browser_and_details() {
        let backend = TestBackend::new(60, 24);
        let mut terminal = match Terminal::new(backend) {
            Ok(terminal) => terminal,
            Err(never) => match never {},
        };
        let app = App::new(false);

        if terminal.draw(|frame| draw(frame, &app, None)).is_err() {
            panic!("narrow frame should render");
        }
        let rendered = terminal.backend().to_string();

        assert!(rendered.contains("Popular videos"));
        assert!(rendered.contains("Details"));
        assert!(rendered.contains("Now playing"));
    }
}
