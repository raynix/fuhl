use std::{io, time::Duration};

use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event as CEvent, KeyCode},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use tui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Modifier, Style},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
};

use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config as NmConfig, Matcher};
use webbrowser;

#[derive(Debug)]
struct Url {
    id: i64,
    url: String,
    title: String,
    visit_count: i64,
    typed_count: i64,
    last_visit_time: i64,
    hidden: i64,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // --- Load DB and collect URLs ---
    let database_file = std::env::var("FUHL_DB").unwrap_or_else(|_| {
        if cfg!(target_os = "macos") {
            let path = "~/Library/Application Support/Google/Chrome/Default/History";
            shellexpand::tilde(path).to_string()
        } else {
            "none".to_string()
        }
    });
    if std::path::Path::new(&database_file).exists() {
        std::fs::copy(&database_file, "/tmp/fuhl").expect("Failed to copy database file");
    } else {
        eprintln!("History DB not found at path {}", database_file);
        std::process::exit(1);
    }

    let conn = rusqlite::Connection::open("/tmp/fuhl").expect("Failed to open database");
    let mut stmt = conn
        .prepare("SELECT id, url, title, visit_count, typed_count, last_visit_time, hidden FROM urls ORDER BY id DESC LIMIT 200")
        .expect("Failed to prepare statement");

    let url_iter = stmt
        .query_map([], |row| {
            Ok(Url {
                id: row.get(0)?,
                url: row.get(1)?,
                title: row.get(2)?,
                visit_count: row.get(3)?,
                typed_count: row.get(4)?,
                last_visit_time: row.get(5)?,
                hidden: row.get(6)?,
            })
        })
        .expect("Failed to query urls");

    let mut urls: Vec<Url> = Vec::new();
    for url in url_iter {
        match url {
            Ok(u) => urls.push(u),
            Err(e) => eprintln!("Error reading row: {}", e),
        }
    }

    if urls.is_empty() {
        eprintln!("No URLs found");
        return Ok(());
    }

    // Build display strings: "title - url" or just "url"
    let displays: Vec<String> = urls
        .iter()
        .map(|u| {
            if u.title.trim().is_empty() {
                u.url.clone()
            } else {
                format!("{} - {}", u.title.trim(), u.url)
            }
        })
        .collect();

    // Prepare matcher (nucleo-matcher) used each frame
    let mut matcher = Matcher::new(NmConfig::DEFAULT);

    // Terminal / TUI setup
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    // UI state
    let mut query = String::new();
    let mut selected = 0usize;
    let mut list_state = ListState::default();

    // Precompute candidate &str slice referencing displays
    let candidates: Vec<&str> = displays.iter().map(|s| s.as_str()).collect();

    loop {
        // Compute matches for current query
        let items_for_display: Vec<String> = if query.trim().is_empty() {
            // show top N items (no ranking) when query empty
            candidates.iter().take(50).map(|s| s.to_string()).collect()
        } else {
            // Use pattern parsing & matcher to score and rank candidates
            let pattern = Pattern::parse(&query, CaseMatching::Ignore, Normalization::Smart);
            let mut matches = pattern.match_list(&candidates, &mut matcher);
            // match_list returns Vec<(&str, u32)> where &str is the candidate
            // We take up to 50 top results
            matches
                .drain(..std::cmp::min(matches.len(), 50))
                .map(|(s, _score)| s.to_string())
                .collect()
        };

        // Ensure selected is in bounds
        if items_for_display.is_empty() {
            selected = 0;
        } else if selected >= items_for_display.len() {
            selected = items_for_display.len() - 1;
        }

        // Update list state for rendering
        list_state.select(Some(selected));

        // Draw UI
        terminal.draw(|f| {
            let size = f.size();
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .margin(1)
                .constraints([Constraint::Length(3), Constraint::Min(1)].as_ref())
                .split(size);

            let input = Paragraph::new(query.as_ref())
                .block(Block::default().borders(Borders::ALL).title("Query"));
            f.render_widget(input, chunks[0]);

            let list_items: Vec<ListItem> = items_for_display
                .iter()
                .map(|s| ListItem::new(s.as_str()))
                .collect();
            let list = List::new(list_items)
                .block(Block::default().borders(Borders::ALL).title("Matches"))
                .highlight_style(Style::default().add_modifier(Modifier::BOLD));
            f.render_stateful_widget(list, chunks[1], &mut list_state);
        })?;

        // Handle input (non-blocking poll)
        if event::poll(Duration::from_millis(200))? {
            if let CEvent::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char(c) => {
                        query.push(c);
                        selected = 0;
                    }
                    KeyCode::Backspace => {
                        query.pop();
                        selected = 0;
                    }
                    KeyCode::Up => {
                        if selected > 0 {
                            selected -= 1;
                        }
                    }
                    KeyCode::Down => {
                        if selected + 1 < items_for_display.len() {
                            selected += 1;
                        }
                    }
                    KeyCode::Esc => {
                        // exit without printing
                        break;
                    }
                    KeyCode::Enter => {
                        // On enter, open the selected URL in the default browser.
                        // Restore the terminal first so the browser can open cleanly.
                        if let Some(choice) = items_for_display.get(selected) {
                            let url = if let Some(pos) = choice.rfind(" - ") {
                                &choice[pos + 3..]
                            } else {
                                choice.as_str()
                            };
                            // restore terminal before opening the browser
                            disable_raw_mode()?;
                            execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture)?;
                            terminal.show_cursor()?;
                            if webbrowser::open(url).is_err() {
                                // fallback to printing the URL if opening fails
                                println!("{}", url);
                            }
                        }
                        // return early because we've already restored the terminal
                        return Ok(());
                    }
                    _ => {}
                }
            }
        }
    }

    // restore terminal
    disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture)?;
    terminal.show_cursor()?;

    Ok(())
}
