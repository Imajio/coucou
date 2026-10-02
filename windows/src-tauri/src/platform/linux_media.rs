// Linux side of the Music tab: MPRIS players, through the playerctl command.
//
// playerctl ships with every major distribution and already speaks to each
// player over D-Bus; calling it keeps a D-Bus stack out of the app. Without it
// the tab says how to get it.

use std::process::Command;

use crate::media::{
    data_url, parse_playerctl, percent_decode, Action, MediaSession, MAX_ARTWORK, PLAYERCTL_SEP,
};

const MISSING: &str = "Install playerctl to see what is playing (e.g. `sudo apt install playerctl`).";

fn playerctl(args: &[&str]) -> Result<String, String> {
    let out = Command::new("playerctl").args(args).output().map_err(|_| MISSING.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

pub fn sessions() -> Result<Vec<MediaSession>, String> {
    let players = match playerctl(&["--list-all"]) {
        Ok(list) => list,
        Err(e) if e == MISSING => return Err(e),
        // "No players found" is an error exit, and simply means nothing plays.
        Err(_) => return Ok(Vec::new()),
    };
    let format = ["{{status}}", "{{artist}}", "{{title}}", "{{album}}", "{{volume}}", "{{position}}", "{{mpris:length}}"]
        .join(&PLAYERCTL_SEP.to_string());
    let mut out = Vec::new();
    // playerctl lists the most recently active player first, which is the one
    // it would drive without `-p`.
    for (i, name) in players.lines().map(str::trim).filter(|l| !l.is_empty()).enumerate() {
        let Ok(line) = playerctl(&["--player", name, "metadata", "--format", &format]) else { continue };
        if let Some(mut session) = parse_playerctl(name, &line) {
            session.current = i == 0;
            out.push(session);
        }
    }
    Ok(out)
}

pub fn control(id: &str, action: Action) -> Result<(), String> {
    let verb = match action {
        Action::PlayPause => "play-pause",
        Action::Play => "play",
        Action::Pause => "pause",
        Action::Next => "next",
        Action::Previous => "previous",
    };
    playerctl(&["--player", id, verb]).map(|_| ())
}

pub fn set_volume(id: &str, level: f32) -> Result<String, String> {
    playerctl(&["--player", id, "volume", &format!("{:.2}", level.clamp(0.0, 1.0))])?;
    Ok("app".into())
}

/// Only local art can be shown: the island's CSP allows `data:` images, and
/// fetching a remote URL would be a network call nobody asked for.
pub fn artwork(id: &str) -> Option<String> {
    let url = playerctl(&["--player", id, "metadata", "mpris:artUrl"]).ok()?;
    let path = percent_decode(url.trim().strip_prefix("file://")?);
    let meta = std::fs::metadata(&path).ok()?;
    if meta.len() == 0 || meta.len() > MAX_ARTWORK {
        return None;
    }
    let bytes = std::fs::read(&path).ok()?;
    let lower = path.to_lowercase();
    let mime = if lower.ends_with(".png") {
        "image/png"
    } else if lower.ends_with(".webp") {
        "image/webp"
    } else {
        "image/jpeg"
    };
    Some(data_url(mime, &bytes))
}
