// Windows side of the Music tab.
//
// Sessions and transport controls come from the system media transport controls
// (the same list as the volume flyout's media card). They know nothing about
// volume, so the source's own level is found among the default output's Core
// Audio sessions by matching the process that plays; when that fails, the whole
// output's volume stands in.

use std::time::{SystemTime, UNIX_EPOCH};

use ::windows::core::{Interface, PWSTR};
use ::windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession as Session,
    GlobalSystemMediaTransportControlsSessionManager as Manager,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status,
};
use ::windows::Storage::Streams::DataReader;
use ::windows::Win32::Foundation::CloseHandle;
use ::windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use ::windows::Win32::Media::Audio::{
    eMultimedia, eRender, AudioSessionStateExpired, IAudioSessionControl2,
    IAudioSessionManager2, IMMDevice, IMMDeviceEnumerator, ISimpleAudioVolume, MMDeviceEnumerator,
};
use ::windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED};
use ::windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};

use crate::media::{data_url, exe_hints, friendly_app_name, Action, MediaSession, MAX_ARTWORK};

/// Runs on a blocking worker that may never have touched COM. Already
/// initialised (or in another mode) is fine: the call is then a no-op.
fn com() {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
}

fn manager() -> Result<Manager, String> {
    Manager::RequestAsync()
        .and_then(|op| op.get())
        .map_err(|e| format!("media controls unavailable: {}", e.message()))
}

fn all_sessions(manager: &Manager) -> Vec<Session> {
    let Ok(list) = manager.GetSessions() else { return Vec::new() };
    let count = list.Size().unwrap_or(0);
    (0..count).filter_map(|i| list.GetAt(i).ok()).collect()
}

fn id_of(session: &Session) -> String {
    session.SourceAppUserModelId().map(|h| h.to_string()).unwrap_or_default()
}

fn find(id: &str) -> Result<Session, String> {
    all_sessions(&manager()?)
        .into_iter()
        .find(|s| id_of(s) == id)
        .ok_or_else(|| format!("{} is no longer playing", friendly_app_name(id)))
}

/// 100 ns ticks to seconds.
fn seconds(ticks: i64) -> f64 {
    ticks as f64 / 10_000_000.0
}

/// Now, in WinRT DateTime ticks (100 ns since 1601-01-01).
fn now_ticks() -> i64 {
    const EPOCH_GAP_SECONDS: i64 = 11_644_473_600;
    let since_unix = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    (since_unix.as_secs() as i64 + EPOCH_GAP_SECONDS) * 10_000_000 + i64::from(since_unix.subsec_nanos() / 100)
}

/// Where the track is now. Sources only report the position when something
/// changes, together with when they did, so a playing track has moved on since.
fn timeline(session: &Session, playing: bool) -> (Option<f64>, Option<f64>) {
    let Ok(t) = session.GetTimelineProperties() else { return (None, None) };
    let end = t.EndTime().map(|d| d.Duration).unwrap_or(0) - t.StartTime().map(|d| d.Duration).unwrap_or(0);
    if end <= 0 {
        return (None, None);
    }
    let mut position = t.Position().map(|d| d.Duration).unwrap_or(0);
    if playing {
        if let Ok(updated) = t.LastUpdatedTime() {
            let elapsed = now_ticks() - updated.UniversalTime;
            if elapsed > 0 {
                position += elapsed;
            }
        }
    }
    (Some(seconds(position.clamp(0, end))), Some(seconds(end)))
}

pub fn sessions() -> Result<Vec<MediaSession>, String> {
    com();
    let manager = manager()?;
    let current = manager.GetCurrentSession().ok().map(|s| id_of(&s));
    let audio = Audio::read();

    let mut out = Vec::new();
    for session in all_sessions(&manager) {
        let id = id_of(&session);
        if id.is_empty() {
            continue;
        }
        let props = session.TryGetMediaPropertiesAsync().and_then(|op| op.get()).ok();
        let text = |f: fn(&::windows::Media::Control::GlobalSystemMediaTransportControlsSessionMediaProperties) -> ::windows::core::Result<::windows::core::HSTRING>| {
            props.as_ref().and_then(|p| f(p).ok()).map(|h| h.to_string()).unwrap_or_default()
        };
        let info = session.GetPlaybackInfo().ok();
        let playing = info
            .as_ref()
            .and_then(|i| i.PlaybackStatus().ok())
            .map(|s| s == Status::Playing)
            .unwrap_or(false);
        let controls = info.as_ref().and_then(|i| i.Controls().ok());
        let flag = |f: fn(&::windows::Media::Control::GlobalSystemMediaTransportControlsSessionPlaybackControls) -> ::windows::core::Result<bool>| {
            controls.as_ref().and_then(|c| f(c).ok()).unwrap_or(false)
        };
        let can_play_pause = flag(|c| c.IsPlayPauseToggleEnabled()) || flag(|c| c.IsPlayEnabled()) || flag(|c| c.IsPauseEnabled());
        let (position, duration) = timeline(&session, playing);
        let (volume, volume_scope) = match audio.app_level(&exe_hints(&id)) {
            Some(level) => (Some(level), "app"),
            None => (audio.system_level(), "system"),
        };

        out.push(MediaSession {
            app: friendly_app_name(&id),
            title: text(|p| p.Title()),
            artist: text(|p| p.Artist()),
            album: text(|p| p.AlbumTitle()),
            playing,
            can_previous: flag(|c| c.IsPreviousEnabled()),
            can_next: flag(|c| c.IsNextEnabled()),
            can_play_pause,
            volume,
            volume_scope: volume_scope.into(),
            position,
            duration,
            current: current.as_deref() == Some(id.as_str()),
            id,
        });
    }
    Ok(out)
}

pub fn control(id: &str, action: Action) -> Result<(), String> {
    com();
    let session = find(id)?;
    let op = match action {
        Action::PlayPause => session.TryTogglePlayPauseAsync(),
        Action::Play => session.TryPlayAsync(),
        Action::Pause => session.TryPauseAsync(),
        Action::Next => session.TrySkipNextAsync(),
        Action::Previous => session.TrySkipPreviousAsync(),
    };
    match op.and_then(|op| op.get()) {
        Ok(true) => Ok(()),
        Ok(false) => Err(format!("{} did not accept that", friendly_app_name(id))),
        Err(e) => Err(e.message().to_string()),
    }
}

pub fn set_volume(id: &str, level: f32) -> Result<String, String> {
    com();
    let audio = Audio::read();
    if audio.set_app_level(&exe_hints(id), level) {
        return Ok("app".into());
    }
    audio.set_system_level(level)?;
    Ok("system".into())
}

pub fn artwork(id: &str) -> Option<String> {
    com();
    let session = find(id).ok()?;
    let props = session.TryGetMediaPropertiesAsync().ok()?.get().ok()?;
    let stream = props.Thumbnail().ok()?.OpenReadAsync().ok()?.get().ok()?;
    let size = stream.Size().ok()?;
    if size == 0 || size > MAX_ARTWORK {
        return None;
    }
    let reader = DataReader::CreateDataReader(&stream).ok()?;
    reader.LoadAsync(size as u32).ok()?.get().ok()?;
    let mut bytes = vec![0u8; size as usize];
    reader.ReadBytes(&mut bytes).ok()?;
    let mime = stream
        .ContentType()
        .map(|h| h.to_string())
        .ok()
        .filter(|m| m.starts_with("image/"))
        .unwrap_or_else(|| "image/png".into());
    Some(data_url(&mime, &bytes))
}

// ── Core Audio ────────────────────────────────────────────────────────────────

/// The default output's audio sessions, by the executable that owns them.
struct Audio {
    apps: Vec<(String, ISimpleAudioVolume)>,
    endpoint: Option<IAudioEndpointVolume>,
}

impl Audio {
    fn read() -> Self {
        let mut audio = Audio { apps: Vec::new(), endpoint: None };
        let Some(device) = default_output() else { return audio };
        unsafe {
            audio.endpoint = device.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None).ok();
            let Ok(manager) = device.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) else {
                return audio;
            };
            let Ok(list) = manager.GetSessionEnumerator() else { return audio };
            for i in 0..list.GetCount().unwrap_or(0) {
                let Ok(control) = list.GetSession(i) else { continue };
                if control.GetState().map(|s| s == AudioSessionStateExpired).unwrap_or(true) {
                    continue;
                }
                let Ok(control2) = control.cast::<IAudioSessionControl2>() else { continue };
                let Ok(pid) = control2.GetProcessId() else { continue };
                let Some(stem) = process_stem(pid) else { continue };
                if let Ok(volume) = control.cast::<ISimpleAudioVolume>() {
                    audio.apps.push((stem, volume));
                }
            }
        }
        audio
    }

    fn matching<'a>(&'a self, hints: &'a [String]) -> impl Iterator<Item = &'a ISimpleAudioVolume> + 'a {
        self.apps.iter().filter(|(stem, _)| hints.contains(stem)).map(|(_, v)| v)
    }

    /// A browser plays through several sessions; the loudest one is the level shown.
    fn app_level(&self, hints: &[String]) -> Option<f32> {
        self.matching(hints)
            .filter_map(|v| unsafe { v.GetMasterVolume() }.ok())
            .fold(None, |best, level| Some(best.map_or(level, |b: f32| b.max(level))))
    }

    fn set_app_level(&self, hints: &[String], level: f32) -> bool {
        let mut any = false;
        for v in self.matching(hints) {
            if unsafe { v.SetMasterVolume(level, std::ptr::null()) }.is_ok() {
                any = true;
            }
        }
        any
    }

    fn system_level(&self) -> Option<f32> {
        self.endpoint.as_ref().and_then(|e| unsafe { e.GetMasterVolumeLevelScalar() }.ok())
    }

    fn set_system_level(&self, level: f32) -> Result<(), String> {
        let endpoint = self.endpoint.as_ref().ok_or("no audio output found")?;
        unsafe { endpoint.SetMasterVolumeLevelScalar(level, std::ptr::null()) }.map_err(|e| e.message().to_string())
    }
}

fn default_output() -> Option<IMMDevice> {
    unsafe {
        let enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
        enumerator.GetDefaultAudioEndpoint(eRender, eMultimedia).ok()
    }
}

/// Lowercase file stem of a process's executable: `C:\…\Spotify.exe` → `spotify`.
fn process_stem(pid: u32) -> Option<String> {
    if pid == 0 {
        return None;
    }
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len).is_ok();
        let _ = CloseHandle(handle);
        if !ok {
            return None;
        }
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        std::path::Path::new(&path)
            .file_stem()
            .map(|s| s.to_string_lossy().to_lowercase())
    }
}
