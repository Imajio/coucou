// Preferences, stored as plain JSON in settings.json under platform::config_dir().
// No secret ever lands here — API keys live in the OS keychain (see secrets.rs).

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub sound_enabled: bool,
    pub sound_volume: f64,
    pub auto_close_interval: f64,
    pub absence_interval: f64,
    pub active_integrations: Vec<String>,
    /// The pill in the island's big card: VS Code, or a declared workspace pill.
    #[serde(default = "default_main_pill")]
    pub main_pill: String,
    /// "primary" = the main display, "cursor" = whichever display the mouse is on.
    pub screen: String,
    pub autostart: bool,
    pub hooks_installed: bool,
    /// AI provider the chat talks to (see llm::PROVIDERS).
    #[serde(default = "default_provider")]
    pub provider: String,
    /// Model used by the chat, on `provider`. Changeable in the chat and in the
    /// settings window. Defaulted explicitly so a settings.json written by an
    /// older build still loads.
    #[serde(default = "default_model")]
    pub model: String,
    /// The last model picked on each provider, so switching back restores it.
    #[serde(default)]
    pub provider_models: std::collections::BTreeMap<String, String>,
    /// Address of the custom OpenAI-compatible endpoint, e.g. LM Studio or Groq.
    #[serde(default)]
    pub custom_base_url: String,
    /// Keep the compact island on screen at all times. Off, the island stays fully
    /// hidden until the cursor rests on the top edge of the screen.
    #[serde(default)]
    pub always_visible: bool,
    /// Seconds the cursor must rest on the top edge to bring a hidden island out.
    #[serde(default = "default_hover_reveal_delay")]
    pub hover_reveal_delay: f64,
}

fn default_main_pill() -> String {
    "integration_claude".to_string()
}

fn default_provider() -> String {
    crate::llm::DEFAULT_PROVIDER.to_string()
}

fn default_model() -> String {
    crate::llm::DEFAULT_MODEL.to_string()
}

fn default_hover_reveal_delay() -> f64 {
    1.0
}

/// Longest hover delay the settings window offers.
const MAX_HOVER_REVEAL_DELAY: f64 = 10.0;

impl Settings {
    /// Pulls values the settings window could not produce back into range, so a
    /// hand-edited settings.json cannot make the island unreachable.
    pub fn sanitized(mut self) -> Self {
        if !self.hover_reveal_delay.is_finite() {
            self.hover_reveal_delay = default_hover_reveal_delay();
        }
        self.hover_reveal_delay = self.hover_reveal_delay.clamp(0.0, MAX_HOVER_REVEAL_DELAY);
        if crate::llm::provider(&self.provider).is_none() {
            self.provider = default_provider();
            self.model = default_model();
        }
        self.provider_models.retain(|p, m| crate::llm::provider(p).is_some() && !m.trim().is_empty());
        self.custom_base_url = if self.custom_base_url.trim().is_empty() {
            String::new()
        } else {
            crate::llm::normalize_base_url(&self.custom_base_url).unwrap_or_default()
        };
        self
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            sound_enabled: true,
            sound_volume: 0.12,
            auto_close_interval: 15.0,
            absence_interval: 180.0,
            main_pill: default_main_pill(),
            active_integrations: vec![
                "integration_resend".into(),
                "integration_n8n".into(),
                "integration_vercel".into(),
                "integration_github".into(),
            ],
            screen: "primary".into(),
            autostart: false,
            hooks_installed: false,
            provider: default_provider(),
            model: default_model(),
            provider_models: Default::default(),
            custom_base_url: String::new(),
            always_visible: false,
            hover_reveal_delay: default_hover_reveal_delay(),
        }
    }
}

pub use crate::platform::{config_dir, local_dir};

pub fn hook_exe_path() -> PathBuf {
    local_dir().join("bin").join(crate::platform::HOOK_EXE)
}

fn settings_path() -> PathBuf {
    config_dir().join("settings.json")
}

pub fn load() -> Settings {
    match std::fs::read(settings_path()) {
        Ok(bytes) => serde_json::from_slice::<Settings>(&bytes)
            .map(Settings::sanitized)
            .unwrap_or_default(),
        Err(_) => Settings::default(),
    }
}

pub fn save(settings: &Settings) -> std::io::Result<()> {
    let dir = config_dir();
    crate::platform::ensure_private_dir(&dir)?;
    let json = serde_json::to_vec_pretty(settings)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(settings_path(), json)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_from_an_older_build_still_load() {
        // Written before the visibility settings existed: losing it would reset
        // every preference the user had.
        let old = r#"{
            "soundEnabled": true, "soundVolume": 0.12, "autoCloseInterval": 5.0,
            "absenceInterval": 180.0, "activeIntegrations": ["integration_github"],
            "screen": "primary", "autostart": false, "hooksInstalled": false,
            "model": "claude-sonnet-5"
        }"#;
        let s: Settings = serde_json::from_str(old).unwrap();
        assert!(!s.always_visible);
        assert_eq!(s.hover_reveal_delay, 1.0);
        assert_eq!(s.auto_close_interval, 5.0);
        assert_eq!(s.model, "claude-sonnet-5");
    }

    #[test]
    fn chat_provider_settings_are_kept_sane() {
        let old: Settings = serde_json::from_str(r#"{
            "soundEnabled": true, "soundVolume": 0.1, "autoCloseInterval": 15.0,
            "absenceInterval": 180.0, "activeIntegrations": [], "screen": "primary",
            "autostart": false, "hooksInstalled": false, "model": "claude-sonnet-5"
        }"#).unwrap();
        // An older build only knew Claude: its model stays, on Anthropic.
        assert_eq!(old.provider, "anthropic");
        assert_eq!(old.model, "claude-sonnet-5");

        let mut s = Settings { provider: "nope".into(), model: "x".into(), ..Settings::default() };
        s.provider_models.insert("openai".into(), "gpt-5".into());
        s.provider_models.insert("ghost".into(), "y".into());
        s.custom_base_url = " http://localhost:1234/v1/ ".into();
        let s = s.sanitized();
        assert_eq!(s.provider, "anthropic");
        assert_eq!(s.model, crate::llm::DEFAULT_MODEL);
        assert_eq!(s.provider_models.keys().collect::<Vec<_>>(), ["openai"]);
        assert_eq!(s.custom_base_url, "http://localhost:1234/v1");
        let bad = Settings { custom_base_url: "file:///etc".into(), ..Settings::default() }.sanitized();
        assert_eq!(bad.custom_base_url, "");
    }

    #[test]
    fn hover_delay_is_pulled_back_into_range() {
        let with = |delay: f64| Settings { hover_reveal_delay: delay, ..Settings::default() }.sanitized();
        assert_eq!(with(2.5).hover_reveal_delay, 2.5);
        assert_eq!(with(99.0).hover_reveal_delay, 10.0);
        assert_eq!(with(-3.0).hover_reveal_delay, 0.0);
        assert_eq!(with(f64::NAN).hover_reveal_delay, 1.0);
        assert_eq!(with(f64::INFINITY).hover_reveal_delay, 1.0);
    }
}
