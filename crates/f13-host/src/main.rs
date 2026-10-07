//! f13-host: the host's commands for a running f13-game started with `--host` (it listens on 127.0.0.1 only).
//!
//!     f13-host [--port n] <command...>     one command, prints the reply
//!     f13-host [--port n]                  commands from stdin, one per line
//!
//! Commands: status, give <Class>, teleport counselor|jason <x> <y> <z>, start, end, kick, jason.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;

/// f13-game's `--host` default.
const DEFAULT_PORT: u16 = 7713;

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut port = DEFAULT_PORT;
    if let Some(i) = args.iter().position(|a| a == "--port") {
        port = args.get(i + 1).and_then(|p| p.parse().ok()).unwrap_or_else(|| usage());
        args.drain(i..=i + 1);
    }
    if args.iter().any(|a| a == "-h" || a == "--help") {
        usage();
    }
    let stream = TcpStream::connect(("127.0.0.1", port)).unwrap_or_else(|e| {
        eprintln!("f13-host: can't reach f13-game on 127.0.0.1:{port} ({e}); start it with --host");
        std::process::exit(1);
    });
    let mut out = stream.try_clone().expect("socket");
    let mut replies = BufReader::new(stream).lines();
    let mut send = |line: &str| -> bool {
        if writeln!(out, "{line}").is_err() {
            return false;
        }
        match replies.next() {
            Some(Ok(reply)) => {
                println!("{reply}");
                true
            }
            _ => false,
        }
    };
    if !args.is_empty() {
        if !send(&args.join(" ")) {
            std::process::exit(1);
        }
        return;
    }
    for line in std::io::stdin().lock().lines().map_while(Result::ok) {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if !send(line) {
            eprintln!("f13-host: the game closed the connection");
            std::process::exit(1);
        }
    }
}

fn usage() -> ! {
    eprintln!("usage: f13-host [--port n] [command...]\ncommands: status, give <Class>, teleport counselor|jason <x> <y> <z>, start, end, kick, jason");
    std::process::exit(2);
}
