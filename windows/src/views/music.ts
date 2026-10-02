// Music tab: whatever plays on the machine (browser tabs, Spotify, players),
// with its transport controls and volume. The OS keeps the list; the island only
// asks for it once a second, and only while this view is on screen.

import { h, svg, clear, dot } from "./dom";
import { ICONS } from "./icons";
import { Bridge, type MediaAction, type MediaSession } from "../core/bridge";
import { formatTime, livePosition, pickSession } from "../core/media";
import type { ViewHost } from "./views";

const POLL_MS = 1000;
/** After the user moves the slider, polled values wait this long to take over. */
const VOLUME_HOLD_MS = 1500;

const APP_COLORS: [string, string][] = [
  ["spotify", "#1DB954"],
  ["edge", "#2C8CF4"],
  ["chrome", "#F4B400"],
  ["firefox", "#FF7139"],
  ["opera", "#FF1B2D"],
  ["brave", "#FB542B"],
  ["media player", "#E879F9"],
  ["vlc", "#F48B00"],
];

function appColor(app: string): string {
  const key = app.toLowerCase();
  return APP_COLORS.find(([k]) => key.includes(k))?.[1] ?? "#A78BFA";
}

export function buildMusic(): ViewHost {
  const sources = h("div", { class: "music-sources" });

  const art = h("div", { class: "music-art" }, svg(ICONS.music, 18));
  const title = h("div", { class: "music-title" });
  const artist = h("div", { class: "music-artist" });
  const fill = h("i");
  const bar = h("div", { class: "music-bar" }, fill);
  const time = h("span", { class: "music-time" });
  const meta = h("div", { class: "music-meta" }, title, artist, h("div", { class: "music-progress" }, bar, time));

  const prev = h("button", { class: "music-btn", title: "Previous", onclick: () => act("previous") }, svg(ICONS.previous, 13));
  const play = h("button", { class: "music-btn play", title: "Play / pause", onclick: () => act("playPause") });
  const next = h("button", { class: "music-btn", title: "Next", onclick: () => act("next") }, svg(ICONS.next, 13));
  const volume = h("input", { type: "range", min: "0", max: "1", step: "0.01", class: "music-volume" }) as HTMLInputElement;
  const volumeLabel = h("span", { class: "music-scope" });
  const controls = h(
    "div",
    { class: "music-controls" },
    prev, play, next,
    h("div", { class: "grow" }),
    svg(ICONS.speakerOn, 13),
    volume,
    volumeLabel,
  );

  const player = h("div", { class: "music-player" }, h("div", { class: "music-now" }, art, meta), controls);
  const emptyTitle = h("div", { class: "title", text: "Nothing is playing." });
  const emptySub = h("div", { class: "sub" });
  const empty = h("div", { class: "music-empty" }, emptyTitle, emptySub);
  const body = h("div", { class: "music-body" }, sources, player, empty);

  const cardEl = h("div", { class: "card wash" }, body);
  cardEl.style.setProperty("--wash", "rgba(244,114,182,0.42)");
  const el = h("div", { class: "view" }, cardEl);

  let sessions: MediaSession[] = [];
  let selectedId: string | null = null;
  let error: string | null = null;
  let loaded = false;
  let polling = false;
  let lastPoll = 0;
  let polledAt = 0;
  let volumeHoldUntil = 0;
  let sourcesKey = "";
  let artKey = "";
  let volumeTimer: number | null = null;

  const selected = () => pickSession(sessions, selectedId);

  async function poll() {
    if (polling) return;
    polling = true;
    lastPoll = performance.now();
    try {
      sessions = await Bridge.mediaSessions();
      error = null;
    } catch (err) {
      sessions = [];
      error = String(err).replace(/^Error:\s*/, "");
    } finally {
      polling = false;
      loaded = true;
      polledAt = performance.now();
    }
    selectedId = selected()?.id ?? null;
    render();
  }

  async function act(action: MediaAction) {
    const s = selected();
    if (!s) return;
    if (action === "playPause") {
      // Answer the click at once; the next poll says what really happened.
      s.playing = !s.playing;
      render();
    }
    try {
      await Bridge.mediaControl(s.id, action);
    } catch (err) {
      error = String(err).replace(/^Error:\s*/, "");
      render();
    }
    window.setTimeout(() => void poll(), action === "playPause" ? 300 : 600);
  }

  volume.addEventListener("input", () => {
    volumeHoldUntil = performance.now() + VOLUME_HOLD_MS;
    if (volumeTimer != null) return;
    // At most one call every 60 ms while dragging.
    volumeTimer = window.setTimeout(() => {
      volumeTimer = null;
      const s = selected();
      if (!s) return;
      const level = Number(volume.value);
      void Bridge.mediaSetVolume(s.id, level)
        .then((scope) => {
          s.volume = level;
          s.volumeScope = scope;
          renderVolumeLabel(s);
        })
        .catch((err) => {
          error = String(err).replace(/^Error:\s*/, "");
          render();
        });
    }, 60);
  });

  function renderSources() {
    const key = sessions.map((s) => `${s.id}:${s.playing}`).join("|") + `>${selectedId}`;
    if (key === sourcesKey) return;
    sourcesKey = key;
    clear(sources);
    for (const s of sessions) {
      const chip = h(
        "button",
        {
          class: s.id === selectedId ? "music-src on" : "music-src",
          title: s.title ? `${s.app}: ${s.title}` : s.app,
          onclick: () => {
            selectedId = s.id;
            artKey = "";
            render();
          },
        },
        dot(appColor(s.app), 6),
        h("span", { text: s.app }),
        s.playing ? h("span", { class: "music-eq" }, h("i"), h("i"), h("i")) : null,
      );
      sources.append(chip);
    }
  }

  function renderArt(s: MediaSession) {
    const key = `${s.id}|${s.title}|${s.artist}`;
    if (key === artKey) return;
    artKey = key;
    art.style.backgroundImage = "";
    art.classList.remove("has-art");
    void Bridge.mediaArtwork(s.id).then((url) => {
      if (artKey !== key || !url) return;
      art.style.backgroundImage = `url("${url}")`;
      art.classList.add("has-art");
    });
  }

  function renderVolumeLabel(s: MediaSession) {
    volumeLabel.textContent = s.volume == null ? "" : s.volumeScope === "app" ? s.app : "System";
    volumeLabel.title =
      s.volumeScope === "app"
        ? `${s.app}'s own volume`
        : `${s.app}'s audio could not be found on its own, so this is the whole system volume`;
  }

  function renderTime(s: MediaSession) {
    const pos = livePosition(s.position, s.duration, s.playing, performance.now() - polledAt);
    if (pos == null || s.duration == null) {
      time.textContent = "";
      bar.style.visibility = "hidden";
      return;
    }
    bar.style.visibility = "";
    fill.style.width = `${Math.min(100, (pos / s.duration) * 100)}%`;
    time.textContent = `${formatTime(pos)} / ${formatTime(s.duration)}`;
  }

  function render() {
    const s = selected();
    const hasSession = s != null;
    player.style.display = hasSession ? "" : "none";
    sources.style.display = sessions.length > 1 ? "" : "none";
    empty.style.display = hasSession || !loaded ? "none" : "";
    if (!hasSession) {
      emptyTitle.textContent = error ? "Can't read what is playing." : "Nothing is playing.";
      emptySub.textContent = error ?? "Play something in a browser, Spotify or any player and it shows up here.";
      return;
    }
    renderSources();
    renderArt(s);
    title.textContent = s.title || s.app;
    // Without a title the source's name is the title; don't repeat it below.
    artist.textContent = error ?? ([s.artist, s.album].filter(Boolean).join(" · ") || (s.title ? s.app : ""));
    artist.classList.toggle("err", error != null);

    clear(play);
    play.append(svg(s.playing ? ICONS.pause : ICONS.play, 15));
    play.disabled = !s.canPlayPause;
    prev.disabled = !s.canPrevious;
    next.disabled = !s.canNext;

    const canVolume = s.volume != null;
    volume.disabled = !canVolume;
    if (canVolume && performance.now() > volumeHoldUntil) volume.value = String(s.volume);
    renderVolumeLabel(s);
    renderTime(s);
  }

  return {
    el,
    sync() {
      if (performance.now() - lastPoll > POLL_MS) void poll();
    },
    tick(nowMs: number) {
      if (nowMs - lastPoll > POLL_MS) void poll();
      const s = selected();
      if (s?.playing) renderTime(s);
    },
  };
}
