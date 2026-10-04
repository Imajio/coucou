// App state — mirror of AppState.swift (the parts the island needs).

import type { BotEmoteName, BotStateName, IslandMode, IslandViewName } from "./layout";
import type { ApprovalRequest } from "./agents";
import type { EyeShape } from "../mochi/engine";
import {
  MAIN_DEFAULT, PILL_CATALOG, orderPills, pillDefinition, pillShown, pillStays, sanitizePills, togglePill,
  type PillChoice, type PillDefinition,
} from "./catalog";

/** "session": one of Coucou's own agent sessions (core/agents.ts). */
export type AgentSource = "claudeCode" | "n8n" | "agent" | "session";
export type PillBadge = "approval" | "finished" | "error";

export interface AgentTask {
  id: string;
  name: string;
  color: string;
  state: BotStateName;
  stepIndex: number;
  steps: string[];
  source: AgentSource;
  isIntegration: boolean;
  emote?: BotEmoteName | null;
  miniEye?: EyeShape | null;
  pillBadge?: PillBadge | null;
  sessionCwd?: string | null;
  /** The processes the session runs under, nearest first (from the relay). */
  sessionPids?: number[] | null;
}

export interface ApprovalInfo {
  requestId: string;
  sessionId: string;
  tool: string;
  command: string;
}

export interface ChatMessage {
  id: number;
  /** "note": a line from the app itself, e.g. that the chat moved to another model. */
  role: "user" | "assistant" | "note";
  content: string;
}

export type PromptContext =
  | { kind: "window"; appName: string; title: string; url?: string }
  | { kind: "file"; name: string; path?: string };

export interface ResultItem {
  label: string;
  detail: string;
  url?: string;
}

export interface SearchResult {
  title: string;
  items: ResultItem[];
  note?: string;
}

/** A catalog pill as a fresh, idle task. */
function catalogTask(def: PillDefinition): AgentTask {
  return {
    id: def.id, name: def.name, color: def.color, state: "idle", stepIndex: 0, steps: [],
    source: def.source, isIntegration: true,
  };
}

/** What an integration poller last reported. */
export interface IntegrationInfo {
  data: Record<string, unknown>;
  error: string | null;
  loaded: boolean;
  configured: boolean;
}

export interface Settings {
  soundEnabled: boolean;
  soundVolume: number;
  autoCloseInterval: number;
  absenceInterval: number;
  /** Declared pills besides VS Code (Settings → Active pills). */
  activeIntegrations: string[];
  /** The pill in the big card: VS Code, or a declared workspace pill. */
  mainPill: string;
  screen: "primary" | "cursor";
  autostart: boolean;
  hooksInstalled: boolean;
  /** AI provider the chat talks to: anthropic, openai, google, openrouter, ollama, custom. */
  provider: string;
  /** Model used by the chat, on `provider`. */
  model: string;
  /** The last model picked on each provider. */
  providerModels: Record<string, string>;
  /** Address of the custom OpenAI-compatible endpoint. */
  customBaseUrl: string;
  /** Keep the compact island on screen at all times; off, it hides until hovered. */
  alwaysVisible: boolean;
  /** Seconds the cursor must rest on the top edge to bring a hidden island out. */
  hoverRevealDelay: number;
}

export const DEFAULT_SETTINGS: Settings = {
  soundEnabled: true,
  soundVolume: 0.12,
  autoCloseInterval: 15,
  absenceInterval: 180,
  activeIntegrations: [
    "integration_resend", "integration_n8n", "integration_vercel", "integration_github",
  ],
  mainPill: MAIN_DEFAULT,
  screen: "primary",
  autostart: false,
  hooksInstalled: false,
  provider: "anthropic",
  model: "claude-opus-5-5",
  providerModels: {},
  customBaseUrl: "",
  alwaysVisible: false,
  hoverRevealDelay: 1,
};

type Listener = () => void;

class AppState {
  mode: IslandMode = "hidden";
  view: IslandViewName = "overview";

  tasks: AgentTask[] = [];
  focusId: string | null = null;

  stateOverride: BotStateName | null = null;

  /** Cursor in logical screen pixels, origin top-left (like AppState.mousePosition). */
  mouse = { x: 0, y: 0 };
  /** Cursor relative to the island's top-left corner. */
  mouseInIsland = { x: 0, y: 0 };

  isPinned = false;
  paused = false;

  uploadProgress = 0;
  uploadDuration = 2.4;
  fileDragOver = false;

  promptContext: PromptContext | null = null;
  droppedFile: { name: string; path: string } | null = null;
  noteMessage: string | null = null;
  searchResult: SearchResult | null = null;
  chatHistory: ChatMessage[] = [];
  pendingApproval: ApprovalInfo | null = null;
  /** Agent sessions waiting for a yes or no, oldest first. */
  sessionApprovals: ApprovalRequest[] = [];

  integrations: Record<string, IntegrationInfo> = {};

  lastActivity = performance.now();

  settings: Settings = { ...DEFAULT_SETTINGS };

  private listeners = new Set<Listener>();

  subscribe(fn: Listener): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }

  /** Marks the UI dirty; the island re-renders on the next frame. */
  notify() {
    for (const fn of this.listeners) fn();
  }

  get focusTask(): AgentTask | null {
    return this.tasks.find((t) => t.id === this.focusId) ?? this.tasks[0] ?? null;
  }

  get effectiveState(): BotStateName {
    return this.stateOverride ?? this.focusTask?.state ?? "idle";
  }

  get otherTasks(): AgentTask[] {
    return this.tasks.filter((t) => t.id !== this.focusId);
  }

  setFocus(id: string) {
    const t = this.tasks.find((x) => x.id === id);
    if (!t) return;
    this.focusId = id;
    t.pillBadge = null;
    this.notify();
  }

  updateTask(id: string, state: BotStateName) {
    const t = this.tasks.find((x) => x.id === id);
    if (!t) return;
    t.state = state;
    this.notify();
  }

  appendStep(id: string, step: string) {
    const t = this.tasks.find((x) => x.id === id);
    if (!t) return;
    t.steps.push(step);
    if (t.steps.length > 20) t.steps.shift();
    t.stepIndex = t.steps.length - 1;
    this.notify();
  }

  setPillBadge(id: string, badge: PillBadge | null) {
    const t = this.tasks.find((x) => x.id === id);
    if (!t) return;
    t.pillBadge = badge;
    this.notify();
  }

  /** The declared pills (Settings → Active pills), sanitized. */
  get pills(): PillChoice {
    return sanitizePills({ active: this.settings.activeIntegrations, mainPill: this.settings.mainPill });
  }

  /**
   * Puts the declared catalog pills in the island: VS Code and the main pill
   * always, the rest as declared. A session pill that isn't declared stays
   * while its session runs. Safe to call again whenever the settings change.
   */
  loadIntegrationTasks() {
    const pills = this.pills;
    this.settings.activeIntegrations = pills.active;
    this.settings.mainPill = pills.mainPill;
    for (const def of PILL_CATALOG) {
      const shown = pillShown(pills, def.id);
      const idx = this.tasks.findIndex((t) => t.id === def.id);
      if (shown && idx < 0) this.tasks.push(catalogTask(def));
      // A session that is running keeps its pill until it ends.
      const busy = idx >= 0 && (this.tasks[idx].state !== "idle" || this.tasks[idx].steps.length > 0);
      if (!shown && idx >= 0 && !(def.source === "agent" && busy)) this.tasks.splice(idx, 1);
    }
    this.tasks = orderPills(this.tasks);
    // A new main pill takes the big card, as on macOS.
    const mainChanged = this.shownMainPill !== null && this.shownMainPill !== pills.mainPill;
    this.shownMainPill = pills.mainPill;
    if (mainChanged || !this.focusId || !this.tasks.some((t) => t.id === this.focusId)) {
      this.focusId = pills.mainPill;
    }
    this.notify();
  }

  /** The main pill as last loaded, to notice a change. */
  private shownMainPill: string | null = null;

  /**
   * A session ended. A declared pill (and VS Code, and the main pill) goes
   * back to idle; any other pill leaves the island.
   */
  removeTask(id: string) {
    const idx = this.tasks.findIndex((t) => t.id === id);
    if (idx < 0) return;
    if (pillStays(this.pills, id)) {
      const t = this.tasks[idx];
      Object.assign(t, { state: "idle", steps: [], stepIndex: 0, pillBadge: null });
      t.name = pillDefinition(id)?.name ?? t.name;
    } else {
      this.tasks.splice(idx, 1);
      if (this.focusId === id) this.focusId = this.pills.mainPill;
    }
    this.notify();
  }

  /**
   * The pill for an agent's session, on its first event. A catalog agent
   * (Gemini CLI, Antigravity) takes its catalog look; any other tagged agent
   * gets its own pill right after VS Code. No-op when the pill is there.
   */
  upsertExternalAgent(id: string, name: string, color: string) {
    if (this.tasks.some((t) => t.id === id)) return;
    const def = pillDefinition(id);
    this.tasks.push(def ? catalogTask(def) : {
      id, name, color,
      state: "idle", stepIndex: 0, steps: [],
      source: "agent", isIntegration: false,
    });
    this.tasks = orderPills(this.tasks);
    if (!this.focusId) this.focusId = id;
    this.notify();
  }

  /**
   * An agent session's pill: added, or brought up to date. Sessions sit right
   * after VS Code, with the other agents.
   */
  upsertSession(task: AgentTask) {
    const existing = this.tasks.find((t) => t.id === task.id);
    if (existing) {
      const badge = existing.pillBadge;
      Object.assign(existing, task, { pillBadge: task.pillBadge === undefined ? badge : task.pillBadge });
    } else {
      this.tasks.push(task);
      this.tasks = orderPills(this.tasks);
      if (!this.focusId) this.focusId = task.id;
    }
    this.notify();
  }

  /** The approval the island shows: Claude Code's first, then the sessions'. */
  get sessionApproval(): ApprovalRequest | null {
    return this.pendingApproval ? null : (this.sessionApprovals[0] ?? null);
  }

  /** Settings → Active pills: on or off, within the rules of the catalog. */
  toggleIntegration(id: string) {
    const next = togglePill(this.pills, id);
    this.settings.activeIntegrations = next.active;
    this.settings.mainPill = next.mainPill;
    if (this.focusId === id && !pillShown(next, id)) this.focusId = next.mainPill;
    this.loadIntegrationTasks();
  }

  defaultView(): IslandViewName {
    return this.tasks.length === 0 ? "empty" : "overview";
  }
}

export const State = new AppState();
