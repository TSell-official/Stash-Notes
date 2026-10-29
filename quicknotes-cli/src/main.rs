//! `quicknotes` — a global CLI over the shared QuickNotes SQLite + FTS5 store.
//!
//! Works from PowerShell, CMD, bash, or scripts. Because it uses the same
//! `quicknotes-core` as the desktop app and MCP server, everything you write
//! here is instantly available everywhere else.

use clap::{Parser, Subcommand};
use quicknotes_core as core;

#[derive(Parser)]
#[command(
    name = "quicknotes",
    version,
    about = "QuickNotes — local-first sticky board with FTS5 search",
    long_about = "Create, search, list and export notes from the terminal.\n\
                  All surfaces (desktop app, MCP server, CLI) share one database."
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Create a new note
    New {
        #[arg(short = 't', long, default_value = "")]
        title: String,
        #[arg(short = 'c', long, default_value = "")]
        content: String,
        /// Comma-separated tags, e.g. --tags mcp,setup
        #[arg(short = 'g', long, default_value = "")]
        tags: String,
        /// Read content from stdin instead of --content (pipe friendly)
        #[arg(long)]
        stdin: bool,
    },
    /// List recent notes (pinned first)
    List {
        #[arg(short, long, default_value_t = 20)]
        limit: i64,
        /// Emit JSON instead of human-readable lines
        #[arg(long)]
        json: bool,
    },
    /// Full-text search (FTS5, BM25 ranked)
    Search {
        query: String,
        #[arg(short, long, default_value_t = 20)]
        limit: i64,
        #[arg(long)]
        json: bool,
    },
    /// Show one note in full
    Show { id: i64 },
    /// Update a note's title/content/tags
    Edit {
        id: i64,
        #[arg(short = 't', long)]
        title: Option<String>,
        #[arg(short = 'c', long)]
        content: Option<String>,
        #[arg(short = 'g', long)]
        tags: Option<String>,
    },
    /// Pin or unpin a note
    Pin {
        id: i64,
        #[arg(long, default_value_t = true)]
        on: bool,
    },
    /// Delete a note by ID
    Delete { id: i64 },
    /// Export all notes as Markdown (one file, headings per note)
    Export {
        #[arg(short, long, default_value = "notes-export.md")]
        out: String,
    },
    /// Print the database path
    Path,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let db_path = core::default_db_path();
    let conn = core::open_db(&db_path)?;

    match cli.command {
        Commands::New {
            title,
            content,
            tags,
            stdin,
        } => {
            let content = if stdin {
                use std::io::Read;
                let mut buf = String::new();
                std::io::stdin().read_to_string(&mut buf)?;
                buf.trim_end().to_string()
            } else {
                content
            };
            let id = core::create_note(&conn, &title, &content, &tags)?;
            println!("Created note {id}");
        }
        Commands::List { limit, json } => {
            let items = core::list_notes(&conn, limit)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&items)?);
            } else {
                print_briefs(&items);
            }
        }
        Commands::Search { query, limit, json } => {
            let items = core::search_notes(&conn, &query, limit)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&items)?);
            } else {
                print_briefs(&items);
            }
        }
        Commands::Show { id } => match core::get_note(&conn, id)? {
            Some(n) => {
                println!("# {}  [id {id}]  tags: {}", n.title, n.tags);
                println!("{}", n.content);
            }
            None => anyhow::bail!("note {id} not found"),
        },
        Commands::Edit {
            id,
            title,
            content,
            tags,
        } => {
            let existing =
                core::get_note(&conn, id)?.ok_or_else(|| anyhow::anyhow!("note {id} not found"))?;
            core::update_note(
                &conn,
                id,
                title.as_deref().unwrap_or(&existing.title),
                content.as_deref().unwrap_or(&existing.content),
                tags.as_deref().unwrap_or(&existing.tags),
            )?;
            println!("Updated note {id}");
        }
        Commands::Pin { id, on } => {
            core::set_pinned(&conn, id, on)?;
            println!("{} note {id}", if on { "Pinned" } else { "Unpinned" });
        }
        Commands::Delete { id } => {
            core::delete_note(&conn, id)?;
            println!("Deleted note {id}");
        }
        Commands::Export { out } => {
            let items = core::list_notes(&conn, 100_000)?;
            let mut md = String::from("# QuickNotes Export\n\n");
            for b in items {
                if let Some(n) = core::get_note(&conn, b.id)? {
                    md.push_str(&format!(
                        "## {}\n\n",
                        if n.title.is_empty() {
                            "(untitled)"
                        } else {
                            &n.title
                        }
                    ));
                    if !n.tags.is_empty() {
                        md.push_str(&format!("*Tags: {}*\n\n", n.tags));
                    }
                    md.push_str(&n.content);
                    md.push_str("\n\n---\n\n");
                }
            }
            std::fs::write(&out, md)?;
            println!("Exported to {out}");
        }
        Commands::Path => println!("{}", db_path.display()),
    }
    Ok(())
}

fn print_briefs(items: &[core::NoteBrief]) {
    if items.is_empty() {
        println!("(no notes)");
        return;
    }
    for n in items {
        let title = if n.title.is_empty() {
            "(untitled)"
        } else {
            &n.title
        };
        println!("[{}] {} — {}", n.id, title, n.snippet.replace('\n', " "));
    }
}
