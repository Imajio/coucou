// Agent sessions as the windows see them (agent/mod.rs on the Rust side): the
// types the commands and events carry and the small rules both the island and
// the sessions window share. The calls themselves are in agentApi.ts.

import type { BotStateName } from "./layout";

export type SessionStatus = "idle" | "running" | "waiting" | "done" | "error" | "stopped";
export type SessionMode = "ask" | "edits" | "auto";
export type ToolState = "running" | "waiting" | "ok" | "error" | "denied";
export type Decision = "allow" | "always" | "deny";

export interface Todo {
  content: string;
  status: "pending" | "in_progress" | "completed" | string;
}

export interface ApprovalRequest {
  requestId: string;
  sessionId: string;
  sessionName: string;
  color: string;
  tool: string;
  summary: string;
  detail: string;
  outside: boolean;
}

export interface SessionSummary {
  id: string;
  name: string;
  role: string;
  color: string;
  provider: string;
  model: string;
  folder: string;
  mode: SessionMode;
  status: SessionStatus;
  error: string | null;
  parent: string | null;
  updatedAt: number;
  activity: string;
  todos: Todo[];
  approval: ApprovalRequest | null;
  entries: number;
}

export type Entry =
  | { kind: "user"; text: string; at: number }
  | { kind: "assistant"; text: string; at: number }
  | {
      kind: "tool";
      callId: string;
      tool: string;
      summary: string;
      detail: string;
      state: ToolState;
      output: string;
      outside: boolean;
      at: number;
    }
  | { kind: "note"; text: string; at: number }
  | { kind: "error"; text: string; at: number };

export interface SessionView {
  summary: SessionSummary;
  log: Entry[];
  rolePrompt: string;
  tools: string[];
}

export interface RoleInfo {
  id: string;
  name: string;
  color: string;
  summary: string;
  prompt: string;
  tools: string[];
}

export interface NewSession {
  name?: string;
  role: string;
  rolePrompt?: string;
  task: string;
  folder: string;
  provider?: string;
  model?: string;
  mode: SessionMode;
}

export interface SessionDefaults {
  folder: string;
  role: string;
  mode: SessionMode | null;
  provider: string;
  model: string;
}

export interface EntryEvent {
  sessionId: string;
  index: number;
  entry: Entry;
}

export const AGENT_EVENTS = {
  session: "agent-session",
  entry: "agent-entry",
  removed: "agent-removed",
  approval: "agent-approval",
  focus: "agent-focus",
} as const;

/** The pill id of a session in the island. */
export const SESSION_PREFIX = "session_";
export const sessionTaskId = (id: string) => `${SESSION_PREFIX}${id}`;
export const sessionIdOf = (taskId: string) =>
  taskId.startsWith(SESSION_PREFIX) ? taskId.slice(SESSION_PREFIX.length) : null;

export const isBusy = (s: SessionStatus) => s === "running" || s === "waiting";

/** How Mochi looks while a session is in this state. */
export function botState(s: SessionStatus): BotStateName {
  switch (s) {
    case "running":
      return "working";
    case "waiting":
      return "approval";
    case "done":
      return "finished";
    case "error":
      return "error";
    case "stopped":
      return "sleeping";
    default:
      return "idle";
  }
}

export function statusLabel(s: SessionStatus): string {
  return {
    idle: "Ready",
    running: "Working",
    waiting: "Needs you",
    done: "Done",
    error: "Error",
    stopped: "Stopped",
  }[s];
}

export const MODE_LABELS: Record<SessionMode, { name: string; hint: string }> = {
  ask: { name: "Ask first", hint: "Asks before every file change, command and web request." },
  edits: { name: "Edit files", hint: "Edits files in its folder on its own; asks before commands and web requests." },
  auto: { name: "Full auto", hint: "Does everything on its own, except writing outside its folder." },
};

/** The ticker lines for a session's avatar: its plan, or what it is doing. */
export function tickerSteps(s: SessionSummary): { steps: string[]; index: number } {
  if (s.todos.length > 0) {
    const current = s.todos.findIndex((t) => t.status === "in_progress");
    const firstOpen = s.todos.findIndex((t) => t.status !== "completed");
    const index = current >= 0 ? current : firstOpen >= 0 ? firstOpen : s.todos.length - 1;
    return { steps: s.todos.map((t) => t.content), index };
  }
  const line = s.status === "waiting" && s.approval ? s.approval.summary : s.activity;
  return { steps: line ? [line] : [], index: 0 };
}

/** "3/5" for a plan, empty without one. */
export function planProgress(todos: Todo[]): string {
  if (todos.length === 0) return "";
  return `${todos.filter((t) => t.status === "completed").length}/${todos.length}`;
}

/** A folder as the user reads it: its last two parts. */
export function shortFolder(path: string): string {
  const parts = path.split(/[\\/]+/).filter(Boolean);
  return parts.length <= 2 ? parts.join("/") || path : `…/${parts.slice(-2).join("/")}`;
}

/** Sessions for the island: the ones needing the user first, then the busy ones, then the latest. */
export function orderSessions(list: SessionSummary[]): SessionSummary[] {
  const rank = (s: SessionSummary) => (s.status === "waiting" ? 0 : s.status === "running" ? 1 : 2);
  return [...list].sort((a, b) => rank(a) - rank(b) || b.updatedAt - a.updatedAt);
}
