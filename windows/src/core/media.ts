// Pure helpers for the Music tab, kept free of imports so tests can load them
// without a DOM or Tauri.

export interface SessionLike {
  id: string;
  playing: boolean;
  current: boolean;
}

/**
 * The session the tab shows: the one picked before while it still exists,
 * otherwise the one the media keys would drive, then whatever plays.
 */
export function pickSession<T extends SessionLike>(sessions: T[], selected: string | null): T | null {
  return (
    sessions.find((s) => s.id === selected) ??
    sessions.find((s) => s.current) ??
    sessions.find((s) => s.playing) ??
    sessions[0] ??
    null
  );
}

/** 75 → "1:15", 3725 → "1:02:05". */
export function formatTime(seconds: number): string {
  const total = Math.max(0, Math.floor(seconds));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = String(total % 60).padStart(2, "0");
  return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${s}` : `${m}:${s}`;
}

/**
 * Where a playing track is now: sources report a position and the island
 * polls once a second, so the bar moves on smoothly in between.
 */
export function livePosition(
  position: number | null,
  duration: number | null,
  playing: boolean,
  sincePollMs: number,
): number | null {
  if (position == null) return null;
  const moved = playing ? position + Math.max(0, sincePollMs) / 1000 : position;
  return duration != null ? Math.min(moved, duration) : moved;
}
