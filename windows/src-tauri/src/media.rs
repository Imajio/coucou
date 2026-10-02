// Music tab: what is playing on this machine, and its transport controls.
//
// The OS already keeps the list. On Windows it is the system media transport
// controls, which every browser tab with a media session, Spotify, Media Player,
// VLC and most players report to; on Linux it is MPRIS, read through playerctl.
// This file holds the shared shape, the pure helpers and the Tauri commands; the
// OS side lives in `platform::media`.

use serde::Serialize;

use crate::platform;

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MediaSession {
    /// Stable for as long as the source plays: the app's id on Windows, the
    /// MPRIS player name on Linux.
    pub id: String,
    /// Readable name of the source: "Spotify", "Microsoft Edge"…
    pub app: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub playing: bool,
    pub can_previous: bool,
    pub can_next: bool,
    pub can_play_pause: bool,
    /// 0 to 1, or None when no volume could be read at all.
    pub volume: Option<f32>,
    /// "app" when `volume` is the source's own level, "system" when it is the
    /// whole output (the source's audio could not be told apart).
    pub volume_scope: String,
    /// Seconds into the track, when the source reports it.
    pub position: Option<f64>,
    /// Track length in seconds, when the source reports it.
    pub duration: Option<f64>,
    /// The session the OS itself would drive with the keyboard's media keys.
    pub current: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    PlayPause,
    Play,
    Pause,
    Next,
    Previous,
}

impl Action {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "playPause" => Some(Self::PlayPause),
            "play" => Some(Self::Play),
            "pause" => Some(Self::Pause),
            "next" => Some(Self::Next),
            "previous" => Some(Self::Previous),
            _ => None,
        }
    }
}

/// Known sources: a fragment of the id (lowercase), the name to show, and the
/// executable stems whose audio belongs to it.
const KNOWN_APPS: &[(&str, &str, &[&str])] = &[
    ("spotify", "Spotify", &["spotify"]),
    ("msedge", "Microsoft Edge", &["msedge"]),
    ("chrome", "Google Chrome", &["chrome"]),
    ("chromium", "Chromium", &["chromium"]),
    // Firefox registers under a hash of its install path on Windows.
    ("308046b0af4a39cb", "Firefox", &["firefox"]),
    ("firefox", "Firefox", &["firefox"]),
    ("opera", "Opera", &["opera"]),
    ("brave", "Brave", &["brave"]),
    ("vivaldi", "Vivaldi", &["vivaldi"]),
    ("yandex", "Yandex Browser", &["browser"]),
    ("zunemusic", "Media Player", &["microsoft.media.player"]),
    ("vlc", "VLC", &["vlc"]),
    ("applemusic", "Apple Music", &["applemusic", "itunes"]),
    ("itunes", "iTunes", &["itunes"]),
    ("deezer", "Deezer", &["deezer"]),
    ("tidal", "TIDAL", &["tidal"]),
    ("foobar2000", "foobar2000", &["foobar2000"]),
    ("aimp", "AIMP", &["aimp"]),
    ("musicbee", "MusicBee", &["musicbee"]),
    ("telegram", "Telegram", &["telegram"]),
    ("discord", "Discord", &["discord"]),
];

fn known(id: &str) -> Option<&'static (&'static str, &'static str, &'static [&'static str])> {
    let lower = id.to_lowercase();
    KNOWN_APPS.iter().find(|(fragment, _, _)| lower.contains(fragment))
}

/// The bare program name inside an id: `C:\…\Foo.exe` → `Foo`,
/// `Pub.App_8wekyb3d8bbwe!App` → `App`, `vlc.instance123` → `vlc`.
fn bare_name(id: &str) -> String {
    let mut s = id.split('!').next().unwrap_or(id);
    s = s.rsplit(['\\', '/']).next().unwrap_or(s);
    if s.len() > 4 && s[s.len() - 4..].eq_ignore_ascii_case(".exe") {
        s = &s[..s.len() - 4];
    }
    // MPRIS adds `.instance<pid>` to players that run more than once.
    if let Some(i) = s.find(".instance") {
        s = &s[..i];
    }
    // Store apps: `Publisher.Name_<publisher hash>`.
    if let Some(i) = s.find('_') {
        s = &s[..i];
    }
    s.rsplit('.').next().unwrap_or(s).to_string()
}

/// A readable name for a source id.
pub fn friendly_app_name(id: &str) -> String {
    if let Some((_, name, _)) = known(id) {
        return (*name).to_string();
    }
    let bare = bare_name(id);
    let mut chars = bare.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => id.to_string(),
    }
}

/// Executable stems (lowercase, no extension) whose audio sessions belong to
/// this source, for its own volume slider.
pub fn exe_hints(id: &str) -> Vec<String> {
    if let Some((_, _, stems)) = known(id) {
        return stems.iter().map(|s| (*s).to_string()).collect();
    }
    let bare = bare_name(id).to_lowercase();
    if bare.is_empty() {
        Vec::new()
    } else {
        vec![bare]
    }
}

/// Separator between the fields asked of `playerctl metadata --format`.
pub const PLAYERCTL_SEP: char = '\u{1f}';

/// One player's line from `playerctl metadata --format` with the fields
/// status, artist, title, album, volume, position (µs) and mpris:length (µs).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn parse_playerctl(player: &str, line: &str) -> Option<MediaSession> {
    let f: Vec<&str> = line.trim_end_matches(['\r', '\n']).split(PLAYERCTL_SEP).collect();
    if f.len() < 7 || f[0].trim().is_empty() {
        return None;
    }
    let micros = |s: &str| s.trim().parse::<f64>().ok().map(|v| v / 1_000_000.0);
    Some(MediaSession {
        id: player.to_string(),
        app: friendly_app_name(player),
        title: f[2].trim().to_string(),
        artist: f[1].trim().to_string(),
        album: f[3].trim().to_string(),
        playing: f[0].trim() == "Playing",
        // MPRIS players without a capability ignore the call, so offer them all.
        can_previous: true,
        can_next: true,
        can_play_pause: true,
        volume: f[4].trim().parse::<f32>().ok().map(|v| v.clamp(0.0, 1.0)),
        volume_scope: "app".into(),
        position: micros(f[5]),
        duration: micros(f[6]).filter(|d| *d > 0.0),
        current: false,
    })
}

/// `data:` URL for album art; the island's CSP allows nothing else for images.
#[cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))]
pub fn data_url(mime: &str, bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len() * 4 / 3 + mime.len() + 16);
    out.push_str("data:");
    out.push_str(mime);
    out.push_str(";base64,");
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// `%20` and friends in a `file://` art URL.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let hex = |b: u8| (b as char).to_digit(16).map(|d| d as u8);
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push(hi << 4 | lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Biggest album art accepted, in bytes.
pub const MAX_ARTWORK: u64 = 4 * 1024 * 1024;

/// OS media calls block (WinRT `get()`, a playerctl process): keep them off
/// both the UI thread and the async workers.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| e.to_string())
}

// ── Commands ──────────────────────────────────────────────────────────────────

/// Everything playing now, the OS's current session first.
#[tauri::command]
pub async fn media_sessions() -> Result<Vec<MediaSession>, String> {
    let mut list = blocking(platform::media::sessions).await??;
    list.sort_by(|a, b| {
        b.current
            .cmp(&a.current)
            .then(b.playing.cmp(&a.playing))
            .then(a.app.to_lowercase().cmp(&b.app.to_lowercase()))
    });
    Ok(list)
}

#[tauri::command]
pub async fn media_control(id: String, action: String) -> Result<(), String> {
    let action = Action::parse(&action).ok_or_else(|| format!("unknown action {action}"))?;
    blocking(move || platform::media::control(&id, action)).await?
}

/// Sets the source's own volume when its audio can be found, the whole
/// output's otherwise. Returns which one it was ("app" or "system").
#[tauri::command]
pub async fn media_set_volume(id: String, volume: f32) -> Result<String, String> {
    if !volume.is_finite() {
        return Err("invalid volume".into());
    }
    let volume = volume.clamp(0.0, 1.0);
    blocking(move || platform::media::set_volume(&id, volume)).await?
}

/// Album art of the source's current track, as a `data:` URL.
#[tauri::command]
pub async fn media_artwork(id: String) -> Option<String> {
    blocking(move || platform::media::artwork(&id)).await.ok().flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_sources_get_their_real_names() {
        assert_eq!(friendly_app_name("Spotify.exe"), "Spotify");
        assert_eq!(friendly_app_name("MSEdge"), "Microsoft Edge");
        assert_eq!(friendly_app_name("Chrome"), "Google Chrome");
        assert_eq!(friendly_app_name("308046B0AF4A39CB"), "Firefox");
        assert_eq!(friendly_app_name("firefox.instance_1_42"), "Firefox");
        assert_eq!(
            friendly_app_name("Microsoft.ZuneMusic_8wekyb3d8bbwe!Microsoft.ZuneMusic"),
            "Media Player"
        );
    }

    #[test]
    fn unknown_sources_get_a_readable_name() {
        assert_eq!(friendly_app_name(r"C:\Program Files\Tunes\tunebox.exe"), "Tunebox");
        assert_eq!(friendly_app_name("Acme.Radio_abcdef123!App"), "Radio");
        assert_eq!(friendly_app_name("cmus"), "Cmus");
    }

    #[test]
    fn volume_hints_name_the_process_that_plays() {
        assert_eq!(exe_hints("Spotify.exe"), vec!["spotify"]);
        assert_eq!(exe_hints("MSEdge"), vec!["msedge"]);
        assert_eq!(exe_hints("308046B0AF4A39CB"), vec!["firefox"]);
        assert_eq!(exe_hints(r"C:\Apps\TuneBox.exe"), vec!["tunebox"]);
        assert!(exe_hints("").is_empty());
    }

    #[test]
    fn playerctl_lines_become_sessions() {
        let line = "Playing\u{1f}Daft Punk\u{1f}One More Time\u{1f}Discovery\u{1f}0.65\u{1f}61500000\u{1f}320000000\n";
        let s = parse_playerctl("spotify", line).unwrap();
        assert_eq!(s.app, "Spotify");
        assert_eq!(s.title, "One More Time");
        assert_eq!(s.artist, "Daft Punk");
        assert!(s.playing);
        assert_eq!(s.volume, Some(0.65));
        assert_eq!(s.position, Some(61.5));
        assert_eq!(s.duration, Some(320.0));
    }

    #[test]
    fn playerctl_lines_with_missing_fields_still_parse() {
        let s = parse_playerctl("vlc", "Paused\u{1f}\u{1f}Radio\u{1f}\u{1f}\u{1f}\u{1f}").unwrap();
        assert!(!s.playing);
        assert_eq!(s.volume, None);
        assert_eq!(s.duration, None);
        assert!(parse_playerctl("vlc", "garbage").is_none());
        assert!(parse_playerctl("vlc", "\u{1f}\u{1f}\u{1f}\u{1f}\u{1f}\u{1f}").is_none());
    }

    #[test]
    fn actions_parse_from_the_front_end_names() {
        assert_eq!(Action::parse("playPause"), Some(Action::PlayPause));
        assert_eq!(Action::parse("previous"), Some(Action::Previous));
        assert_eq!(Action::parse("rewind"), None);
    }

    #[test]
    fn art_paths_are_percent_decoded() {
        assert_eq!(percent_decode("/home/a/My%20Music/c%C3%A9.jpg"), "/home/a/My Music/cé.jpg");
        assert_eq!(percent_decode("/x/100%"), "/x/100%");
        assert_eq!(percent_decode("/x/%zz%é"), "/x/%zz%é");
    }

    #[test]
    fn data_urls_are_standard_base64() {
        assert_eq!(data_url("image/png", b""), "data:image/png;base64,");
        assert_eq!(data_url("image/png", b"f"), "data:image/png;base64,Zg==");
        assert_eq!(data_url("image/png", b"fo"), "data:image/png;base64,Zm8=");
        assert_eq!(data_url("image/png", b"foobar"), "data:image/png;base64,Zm9vYmFy");
    }
}
