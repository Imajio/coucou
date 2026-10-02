# Changelog

## Unreleased

- Windows and Linux: the chat works with Claude, OpenAI (GPT and Codex), Gemini, OpenRouter, a local Ollama or any OpenAI-compatible endpoint; switch provider or model in the middle of a conversation and the new model picks it up. The default Claude model is now Claude Opus 5.5
- Windows and Linux: changing the text or the languages while a translation is on its way now translates the newest text instead of leaving an old result on screen
- Windows and Linux: the Translate tab is free with no setup, through Google Translate's public web service; a Google Cloud key stays optional, for the official API
- Windows and Linux: the island reopens on the tab you left it on, including after a restart, and goes back to it once a permission request is answered
- Windows and Linux: the open island no longer folds away while you type in the chat or the translator; auto-close counts from your last key
- Windows and Linux: a Translate tab in the island translates with Google Translate, using your own Google Cloud Translation key from Settings, or opens the text on translate.google.com when there is no key
- Windows and Linux: a Music tab in the island shows what is playing in browsers, Spotify and other players, with previous, play/pause, next and a volume slider for the chosen source
- Windows: clicking the island gives it the keyboard, so `Esc` works there: an open island folds to compact, a compact one hides, and the keyboard goes back to the app you were in
- Windows and Linux: Settings → General has an **Always show island** switch. Off, the island stays fully hidden until the pointer rests on the top edge for the configurable **Hover delay**; on, it stays on screen
- Windows: hovering the top edge brings a hidden island back again; the wake strip could be left click-through after the island folded away, and then nothing woke it
- Declare the tools you use in Settings: Gemini CLI, Antigravity, Anthropic, Google AI and OpenAI pills join the existing ones (Cursor and Codex pills are coming soon), and you pick the main pill.
- Chat now supports Google AI (Gemini) and OpenAI in addition to Anthropic; switch provider and model by clicking the model name in the chat view, on macOS.
- Linux version: the Tauri app now builds for Linux too (AppImage, .deb, .rpm), with the island as a layer-shell overlay on Wayland and Claude Code hooks over a private Unix socket (#21) — thanks @Davy133
- Compact island on screens without a notch (#22) — thanks @Kamasoutra
- Only web links (http/https) open from the notch; other kinds of links from Claude or integrations are ignored (#16) — thanks @Cris1670
- Hook socket limited to your own user account, with size and time limits; logs no longer keep commands, n8n data or full URLs, and stay under 1 MB (#16) — thanks @Cris1670 and @Vignesh-Thangamariappan
- The island always reopens after folding, and Settings opens below it, resizable — thanks @rouderz
- Choose the Claude model for the chat in Settings; the list comes from your Anthropic account, and Claude Sonnet 4.6 stays the default — thanks @rouderz
- Windows build artifacts are now downloadable from a manual CI run — thanks @MysJofR
- Any agent can talk to Mochi: tag a hook payload with `coucou_agent` (e.g. `nb-hook --agent my-agent`) and it gets its own pill in the island (#7, #9) — thanks @lacatu5
- Gemini CLI and Antigravity (agy) hook support on macOS: install from Settings and their sessions show up in the island — thanks @corefusiion
