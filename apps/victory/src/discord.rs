//! Discord Rich Presence (ported from realblastex/flamingo): shows the title's
//! name, whether it is in menus or open world, live FPS, and elapsed play time
//! over Discord's local IPC pipe (`\\.\pipe\discord-ipc-N`) with zero extra crates.
//!
//! Configuration (opt-in, controllable from GTA6-Launcher):
//! - `VICTORY_DISCORD=1` (or `FLAMINGO_DISCORD=1`) enables presence (`0` disables).
//! - `VICTORY_DISCORD_APP_ID=<id>` (or `FLAMINGO_DISCORD_APP_ID=<id>`), or saved in
//!   `C:\Users\gabri\Downloads\victory-data\discord_app_id.txt` or
//!   `%LOCALAPPDATA%\victory\discord_app_id.txt`.
//! - `VICTORY_DISCORD_HIDE_TITLE=1` (or `FLAMINGO_DISCORD_HIDE_TITLE=1`) omits the title name.

use std::io::{Read, Write};
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const OP_HANDSHAKE: u32 = 0;
const OP_FRAME: u32 = 1;
const OP_CLOSE: u32 = 2;

/// Discord accepts 5 activity updates per 20 seconds: throttle updates to at most every 4s.
const EVERY: Duration = Duration::from_secs(4);
/// Retry interval when Discord client is not running yet.
const RETRY: Duration = Duration::from_secs(3);

/// Starts the Discord Rich Presence background thread if enabled and an Application ID is available.
pub fn start(title_dir: &Path) {
    if !enabled() {
        return;
    }
    let Some(app) = app_id() else {
        println!("discord        off (defina VICTORY_DISCORD_APP_ID no Launcher)");
        return;
    };
    let hide_title = std::env::var_os("VICTORY_DISCORD_HIDE_TITLE").is_some_and(|v| v == "1")
        || std::env::var_os("FLAMINGO_DISCORD_HIDE_TITLE").is_some_and(|v| v == "1");
    let title = if hide_title {
        None
    } else {
        std::fs::read_to_string(title_dir.join("MicrosoftGame.config"))
            .ok()
            .and_then(|m| display_name(&m))
            .or_else(|| Some("GTA VI (Xbox Series)".to_owned()))
    };
    println!("discord        Rich Presence ativo (app {app})");
    let started = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let _ = std::thread::Builder::new()
        .name("discord-presence".into())
        .spawn(move || run(&app, title.as_deref(), started));
}

fn enabled() -> bool {
    if let Some(v) = std::env::var_os("VICTORY_DISCORD") {
        return v != "0";
    }
    if let Some(v) = std::env::var_os("FLAMINGO_DISCORD") {
        return v != "0";
    }
    // Default off unless explicitly requested or app id is set via env.
    std::env::var_os("VICTORY_DISCORD_APP_ID").is_some()
        || std::env::var_os("FLAMINGO_DISCORD_APP_ID").is_some()
}

fn app_id() -> Option<String> {
    let raw = std::env::var("VICTORY_DISCORD_APP_ID")
        .ok()
        .or_else(|| std::env::var("FLAMINGO_DISCORD_APP_ID").ok())
        .or_else(|| {
            std::fs::read_to_string(r"C:\Users\gabri\Downloads\victory-data\discord_app_id.txt")
                .ok()
        })
        .or_else(|| {
            let dir = std::env::var_os("LOCALAPPDATA")?;
            std::fs::read_to_string(Path::new(&dir).join("victory").join("discord_app_id.txt"))
                .ok()
                .or_else(|| {
                    std::fs::read_to_string(
                        Path::new(&dir).join("flamingo").join("discord_app_id.txt"),
                    )
                    .ok()
                })
        })?;
    let id = raw.trim();
    (!id.is_empty() && id.bytes().all(|b| b.is_ascii_digit())).then(|| id.to_owned())
}

/// The title's name in `MicrosoftGame.config` (`DefaultDisplayName`), unless it is a resource key.
fn display_name(manifest: &str) -> Option<String> {
    const KEY: &str = "DefaultDisplayName=\"";
    let at = manifest.find(KEY)? + KEY.len();
    let name = &manifest[at..at + manifest[at..].find('"')?];
    (!name.is_empty() && !name.starts_with("ms-resource:")).then(|| name.to_owned())
}

fn run(app: &str, title: Option<&str>, started: u64) {
    let mut nonce = 0u64;
    loop {
        if let Some(mut pipe) = connect(app) {
            let mut shown = String::new();
            let mut sent: Option<std::time::Instant> = None;
            loop {
                let (fps, draws) = victory_gpu::frame_rate();
                let st = state(fps, draws);
                if st != shown && sent.is_none_or(|at| at.elapsed() >= EVERY) {
                    nonce += 1;
                    let update = activity(std::process::id(), title, &st, started, nonce);
                    if send(&mut pipe, OP_FRAME, &update)
                        .and_then(|()| receive(&mut pipe))
                        .is_err()
                    {
                        break;
                    }
                    (shown, sent) = (st, Some(std::time::Instant::now()));
                }
                std::thread::sleep(Duration::from_secs(1));
            }
        }
        std::thread::sleep(RETRY);
    }
}

/// Scene & FPS status string (>= 500 draws/frame = open world).
fn state(fps: f64, draws: u32) -> String {
    match (fps > 0.0, draws >= 500) {
        (false, _) => "Iniciando...".to_owned(),
        (true, true) => format!("No mundo aberto • {fps:.0} fps"),
        (true, false) => format!("Nos menus • {fps:.0} fps"),
    }
}

fn connect(app: &str) -> Option<std::fs::File> {
    (0..10).find_map(|n| {
        let mut pipe = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(format!(r"\\.\pipe\discord-ipc-{n}"))
            .ok()?;
        send(
            &mut pipe,
            OP_HANDSHAKE,
            &format!(r#"{{"v":1,"client_id":"{app}"}}"#),
        )
        .ok()?;
        receive(&mut pipe).ok()?;
        Some(pipe)
    })
}

fn send(pipe: &mut impl Write, op: u32, json: &str) -> std::io::Result<()> {
    pipe.write_all(&frame(op, json))
}

fn frame(op: u32, json: &str) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(8 + json.len());
    bytes.extend_from_slice(&op.to_le_bytes());
    bytes.extend_from_slice(&(json.len() as u32).to_le_bytes());
    bytes.extend_from_slice(json.as_bytes());
    bytes
}

fn receive(pipe: &mut impl Read) -> std::io::Result<()> {
    let mut head = [0u8; 8];
    pipe.read_exact(&mut head)?;
    let op = u32::from_le_bytes([head[0], head[1], head[2], head[3]]);
    let len = u32::from_le_bytes([head[4], head[5], head[6], head[7]]) as usize;
    if len > 1 << 20 {
        return Err(std::io::Error::other("discord reply too long"));
    }
    let mut body = vec![0u8; len];
    pipe.read_exact(&mut body)?;
    if op == OP_CLOSE {
        return Err(std::io::Error::other(
            String::from_utf8_lossy(&body).into_owned(),
        ));
    }
    Ok(())
}

fn activity(pid: u32, title: Option<&str>, state: &str, started: u64, nonce: u64) -> String {
    let details = title
        .map(|t| format!(r#""details":"{}","#, escape(t)))
        .unwrap_or_default();
    format!(
        concat!(
            r#"{{"cmd":"SET_ACTIVITY","args":{{"pid":{},"activity":{{{}"state":"{}","#,
            r#""timestamps":{{"start":{}}},"assets":{{"large_image":"victory","large_text":"VIctory"}}}}}},"nonce":"{}"}}"#
        ),
        pid,
        details,
        escape(state),
        started,
        nonce
    )
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if u32::from(c) < 0x20 => out.push_str(&format!("\\u{:04x}", u32::from(c))),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_title_name_comes_from_the_manifest() {
        let manifest = r#"<ShellVisuals DefaultDisplayName="Example Title" PublisherDisplayName="x"/>"#;
        assert_eq!(display_name(manifest).as_deref(), Some("Example Title"));
        assert_eq!(display_name(r#"DefaultDisplayName="ms-resource:Title""#), None);
        assert_eq!(display_name("<Game/>"), None);
    }

    #[test]
    fn frames_and_activity_are_well_formed() {
        let json = activity(42, Some(r#"A "quoted" \ name"#), &state(7.4, 4980), 1_700_000_000, 3);
        let bytes = frame(OP_FRAME, &json);
        assert_eq!(&bytes[..4], &1u32.to_le_bytes());
        assert_eq!(
            u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize,
            json.len()
        );
        assert!(json.contains(r#""details":"A \"quoted\" \\ name","#), "{json}");
        assert!(json.contains(r#""state":"No mundo aberto • 7 fps""#), "{json}");
        assert!(json.contains(r#""timestamps":{"start":1700000000}"#), "{json}");
        assert!(json.ends_with(r#""nonce":"3"}"#), "{json}");
        let opens = json.matches('{').count();
        assert_eq!(opens, json.matches('}').count(), "{json}");
        let json = activity(42, None, &state(0.0, 0), 5, 1);
        assert_eq!(state(29.6, 12), "Nos menus • 30 fps");
        assert!(!json.contains("details") && json.contains(r#""state":"Iniciando...""#), "{json}");
    }

    #[test]
    fn a_close_reply_is_an_error() {
        let mut close = std::io::Cursor::new(frame(
            OP_CLOSE,
            r#"{"code":4000,"message":"Invalid Client ID"}"#,
        ));
        assert!(receive(&mut close).is_err());
        let mut ready = std::io::Cursor::new(frame(OP_FRAME, r#"{"evt":"READY"}"#));
        assert!(receive(&mut ready).is_ok());
        assert_eq!(ready.position() as usize, ready.get_ref().len());
    }
}
