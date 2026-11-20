//! A tiny interactive shell for poking at the engine.
//!
//! ```text
//! cargo run -- ./data
//! > put language rust
//! > get language
//! rust
//! ```

use std::io::{self, BufRead, Write};
use tiny_lsm::Db;

const HELP: &str = "\
commands:
  put <key> <value>     store a value (the value may contain spaces)
  get <key>             print a value
  del <key>             delete a key
  scan [start] [end]    list keys in [start, end)
  stats                 memtable size and recovery info
  help                  this text
  quit                  exit (Ctrl+D works too)";

fn main() {
    let dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "tiny-lsm-data".into());
    // `match` on a Result: handle the error here instead of with `?`,
    // because `main` wants to print a friendly message.
    let mut db = match Db::open(&dir) {
        Ok(db) => db,
        Err(e) => {
            eprintln!("could not open {dir}: {e}");
            std::process::exit(1);
        }
    };

    println!("tiny-lsm · data in {dir} · type `help`");
    if db.recovered_dropped_bytes() > 0 {
        println!(
            "note: dropped {} bytes of an incomplete write from the last run",
            db.recovered_dropped_bytes()
        );
    }

    let stdin = io::stdin();
    loop {
        print!("> ");
        io::stdout().flush().ok();

        let mut line = String::new();
        match stdin.lock().read_line(&mut line) {
            Ok(0) => break, // end of input (Ctrl+D)
            Ok(_) => {}
            Err(e) => {
                eprintln!("read error: {e}");
                break;
            }
        }

        if let Err(e) = run(&mut db, line.trim()) {
            eprintln!("error: {e}");
        }
        if matches!(line.trim(), "quit" | "exit") {
            break;
        }
    }
}

fn run(db: &mut Db, line: &str) -> tiny_lsm::Result<()> {
    let mut parts = line.splitn(3, ' ');
    let cmd = parts.next().unwrap_or("");
    let a = parts.next();
    let b = parts.next();

    match (cmd, a, b) {
        ("", _, _) | ("quit" | "exit", _, _) => {}
        ("help", _, _) => println!("{HELP}"),
        ("put", Some(k), Some(v)) => {
            db.put(k.as_bytes(), v.as_bytes())?;
            println!("ok");
        }
        ("get", Some(k), None) => match db.get(k.as_bytes()) {
            Some(v) => println!("{}", String::from_utf8_lossy(&v)),
            None => println!("(not found)"),
        },
        ("del", Some(k), None) => {
            db.delete(k.as_bytes())?;
            println!("ok");
        }
        ("scan", start, end) => {
            let rows = db.scan(start.map(str::as_bytes), end.map(str::as_bytes));
            for (k, v) in &rows {
                println!(
                    "{} = {}",
                    String::from_utf8_lossy(k),
                    String::from_utf8_lossy(v)
                );
            }
            println!("({} keys)", rows.len());
        }
        ("stats", _, _) => println!(
            "memtable: {} entries, ~{} bytes · dropped on recovery: {} bytes",
            db.memtable_len(),
            db.memtable_bytes(),
            db.recovered_dropped_bytes()
        ),
        _ => println!("unknown or incomplete command, type `help`"),
    }
    Ok(())
}
