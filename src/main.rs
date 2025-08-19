use skim::prelude::*;
use std::io::Cursor;

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

fn main() {
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
    let mut stmt = conn.prepare("SELECT id, url, title, visit_count, typed_count, last_visit_time, hidden FROM urls WHERE length(url) < 60 ORDER BY last_visit_time DESC, visit_count DESC").expect("Failed to prepare statement");

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

    // Collect rows into a vector so we can present them to skim and map back to the Url
    let mut urls: Vec<Url> = Vec::new();
    for url in url_iter {
        match url {
            Ok(u) => urls.push(u),
            Err(e) => eprintln!("Error reading row: {}", e),
        }
    }

    if urls.is_empty() {
        eprintln!("No URLs found");
        return;
    }

    // Build the input lines for skim. Prefix each line with the index so we can find the selected item.
    let mut input = String::new();
    for (i, u) in urls.iter().enumerate() {
        let safe_url = u.url.replace('\n', " ");
        let safe_title = u.title.replace('\n', " ");
        input.push_str(&format!("{}\t{} ... {}\n", i, safe_title, safe_url));
    }

    // Configure skim options: single-select, reasonable height
    let options = SkimOptionsBuilder::default()
        .height("50%".to_string())
        .multi(false)
        .build()
        .unwrap();

    // Run skim with our input
    let item_reader = SkimItemReader::default();
    let items = item_reader.of_bufread(Cursor::new(input));
    let selected_items = Skim::run_with(&options, Some(items))
        .map(|out| out.selected_items)
        .unwrap_or_default();

    if selected_items.is_empty() {
        eprintln!("No selection made");
        return;
    }

    // Parse the selected line to get the index and open the corresponding URL in the default browser
    let selected_output = selected_items[0].output();
    let parts: Vec<&str> = selected_output.split("\t").collect();
    let idx: usize = parts.get(0).and_then(|s| s.parse().ok()).unwrap_or(0);
    if let Some(u) = urls.get(idx) {
        // Open the URL in the default browser
        match webbrowser::open(&u.url) {
            Ok(_) => {}
            Err(e) => eprintln!("Failed to open URL {}: {}", u.url, e),
        }
    }
}
