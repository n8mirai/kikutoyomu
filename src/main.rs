#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod dict;
mod library;
mod models;
mod morph;
mod server;
mod subs;
mod synopsis;
mod timeutil;
mod toggl;
mod vocab;

use std::env;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use server::SharedState;

fn is_kiku_already_running(port: u16) -> bool {
    let addr = format!("127.0.0.1:{}", port);
    let Ok(mut stream) = TcpStream::connect_timeout(&addr.parse().unwrap(), Duration::from_millis(250)) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_millis(400)));
    let req = format!("GET /api/state HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n", addr);
    if stream.write_all(req.as_bytes()).is_err() {
        return false;
    }
    let mut buf = [0u8; 256];
    if let Ok(n) = stream.read(&mut buf) {
        let head = String::from_utf8_lossy(&buf[..n]);
        return head.contains("200 OK") && head.contains("application/json");
    }
    false
}

fn resolve_app_root() -> PathBuf {
    // 1. Explicit --root <dir> or KIKU_ROOT env var
    if let Ok(r) = env::var("KIKU_ROOT") {
        let p = PathBuf::from(r);
        if p.exists() {
            return p;
        }
    }
    // 2. Next to executable if media/ exists there (or target/release/../../)
    if let Ok(exe) = env::current_exe() {
        if let Some(dir) = exe.parent() {
            if dir.join("media").exists() {
                return dir.to_path_buf();
            }
            if let Some(parent2) = dir.parent().and_then(|p| p.parent()) {
                if parent2.join("media").exists() {
                    return parent2.to_path_buf();
                }
            }
        }
    }
    // 3. Current working directory if media/ exists
    if let Ok(cwd) = env::current_dir() {
        if cwd.join("media").exists() {
            return cwd;
        }
    }
    // 4. Fallback next to exe
    env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
}

fn launch_desktop_window(url: &str, app_root: &PathBuf) {
    let profile_dir = app_root.join("data").join("edge-profile");
    let _ = std::fs::create_dir_all(&profile_dir);

    let edge_candidates = [
        r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
        r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
    ];
    for edge in edge_candidates {
        if std::path::Path::new(edge).exists() {
            let launched = Command::new(edge)
                .arg(format!("--app={}", url))
                .arg("--window-size=980,780")
                .arg(format!("--user-data-dir={}", profile_dir.to_string_lossy()))
                .arg("--class=KikuImmersionDesk")
                .spawn();
            if launched.is_ok() {
                return;
            }
        }
    }
    // Fallback: open in default browser
    let _ = Command::new("cmd").args(["/C", "start", "", url]).spawn();
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let mut port: u16 = 41937;
    let mut no_open = false;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--no-open" => no_open = true,
            "--port" => {
                if let Some(val) = args.get(i + 1) {
                    if let Ok(p) = val.parse::<u16>() {
                        port = p;
                    }
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }

    let app_root = resolve_app_root();

    // If Kiku is already running on the default port, just reopen the desktop window.
    if !no_open && is_kiku_already_running(port) {
        let url = format!("http://127.0.0.1:{}", port);
        launch_desktop_window(&url, &app_root);
        return;
    }

    let state = Arc::new(Mutex::new(SharedState::load(&app_root)));

    let (bound_port, srv) = match server::run_server(port, Arc::clone(&state)) {
        Ok(res) => res,
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    };

    let url = format!("http://127.0.0.1:{}", bound_port);
    println!("Kiku — Japanese Immersion Audio Desk");
    println!("  URL:   {}", url);
    println!("  Media: {}", app_root.join("media").display());

    if !no_open {
        launch_desktop_window(&url, &app_root);
    }

    server::serve_loop(srv, state);
}
