// Agent sessions in the island: each one is an avatar pill that works, waits
// for a yes or no, and finishes, like the Claude Code pill. The island shows
// the sessions that are busy or recent; the sessions window has all of them.

import { Agents } from "../core/agentApi";
import {
  AGENT_EVENTS, botState, isBusy, orderSessions, sessionIdOf, sessionTaskId, statusLabel, tickerSteps,
  type ApprovalRequest, type SessionStatus, type SessionSummary,
} from "../core/agents";
import { onEvent } from "../core/bridge";
import { Sound } from "../core/sound";
import { State, type AgentTask } from "../core/state";
import type { Island } from "./island";

/** Sessions the island carries at most; the window has the rest. */
const MAX_ISLAND_SESSIONS = 6;
/** A finished session stays in the island this long. */
const RECENT_MS = 12 * 60 * 60 * 1000;
/**
 * How long a session's request holds the island open. A session waits as long
 * as it takes, the island doesn't: after this it can fold (Esc, or on its own)
 * and the pill's badge keeps the request in sight. Reopening shows it again.
 */
const PIN_MS = 20_000;

const all = new Map<string, SessionSummary>();
const lastStatus = new Map<string, SessionStatus>();

function toTask(s: SessionSummary): AgentTask {
  const ticker = tickerSteps(s);
  const steps = ticker.steps.length ? ticker.steps : [statusLabel(s.status)];
  return {
    id: sessionTaskId(s.id),
    name: s.name,
    color: s.color,
    state: botState(s.status),
    steps,
    stepIndex: Math.min(ticker.index, steps.length - 1),
    source: "session",
    isIntegration: false,
    sessionCwd: s.folder,
  };
}

function shown(now = Date.now()): SessionSummary[] {
  return orderSessions([...all.values()])
    .filter((s) => isBusy(s.status) || now - s.updatedAt < RECENT_MS)
    .slice(0, MAX_ISLAND_SESSIONS);
}

/** Puts the island's session pills in line with the sessions. */
function syncPills() {
  const keep = new Set(shown().map((s) => sessionTaskId(s.id)));
  for (const t of [...State.tasks]) {
    if (t.source === "session" && !keep.has(t.id)) State.removeTask(t.id);
  }
  for (const s of shown()) State.upsertSession(toTask(s));
}

/** After an answer anywhere: the next request, or back to what was there. */
function afterApproval(island: Island) {
  if (State.view !== "approval" || State.pendingApproval) return;
  const next = State.sessionApproval;
  if (next) {
    State.setFocus(sessionTaskId(next.sessionId));
    State.notify();
    return;
  }
  State.isPinned = false;
  island.dropPin();
  island.setView(island.homeView());
}

function dropApproval(island: Island, requestId: string) {
  const req = State.sessionApprovals.find((r) => r.requestId === requestId);
  if (!req) return;
  State.sessionApprovals = State.sessionApprovals.filter((r) => r.requestId !== requestId);
  if (!State.sessionApprovals.some((r) => r.sessionId === req.sessionId)) {
    State.setPillBadge(sessionTaskId(req.sessionId), null);
  }
  afterApproval(island);
}

function onApproval(island: Island, req: ApprovalRequest) {
  if (State.paused || State.sessionApprovals.some((r) => r.requestId === req.requestId)) return;
  State.sessionApprovals.push(req);
  Sound.play("approval");
  const taskId = sessionTaskId(req.sessionId);
  // Nothing else on the card: open on this request. Otherwise the badge says it.
  if (!State.pendingApproval && State.view !== "approval") {
    State.setFocus(taskId);
    State.isPinned = true;
    island.alert("approval");
    window.setTimeout(() => {
      const still = State.sessionApprovals.some((r) => r.requestId === req.requestId);
      if (still && State.isPinned && !State.pendingApproval) {
        State.isPinned = false;
        island.dropPin();
      }
    }, PIN_MS);
  } else {
    island.reveal(true);
  }
  // The badge outlives a folded island.
  State.setPillBadge(taskId, "approval");
}

function onSession(island: Island, s: SessionSummary) {
  const before = lastStatus.get(s.id);
  all.set(s.id, s);
  lastStatus.set(s.id, s.status);
  syncPills();
  const taskId = sessionTaskId(s.id);

  // Answered or stopped somewhere else: its card goes.
  for (const r of State.sessionApprovals.filter((r) => r.sessionId === s.id)) {
    if (s.approval?.requestId !== r.requestId) dropApproval(island, r.requestId);
  }
  if (!before || !isBusy(before) || isBusy(s.status) || State.paused) return;

  // It just stopped working.
  const focused = State.focusId === taskId && State.mode === "expanded";
  if (s.status === "done") {
    Sound.play("finish");
    if (!focused) State.setPillBadge(taskId, "finished");
    island.reveal(false);
  } else if (s.status === "error") {
    Sound.play("error");
    if (!focused) State.setPillBadge(taskId, "error");
    island.reveal(false);
  }
}

export function registerAgentHandlers(island: Island) {
  void onEvent<SessionSummary>(AGENT_EVENTS.session, (s) => onSession(island, s));
  void onEvent<ApprovalRequest>(AGENT_EVENTS.approval, (r) => onApproval(island, r));
  void onEvent<string>(AGENT_EVENTS.removed, (id) => {
    all.delete(id);
    lastStatus.delete(id);
    for (const r of State.sessionApprovals.filter((r) => r.sessionId === id)) dropApproval(island, r.requestId);
    State.removeTask(sessionTaskId(id));
  });
  // Events sent before this page loaded are gone: start from the list.
  void Agents.list()
    .then((list) => {
      for (const s of list) {
        all.set(s.id, s);
        lastStatus.set(s.id, s.status);
      }
      syncPills();
      return Agents.approvals();
    })
    .then((pending) => {
      for (const r of pending) onApproval(island, r);
    })
    .catch(() => {});
  // Finished sessions leave the island after a while.
  window.setInterval(syncPills, 10 * 60 * 1000);
}

/** The island's Allow / Deny on a session's request. */
export async function decideSession(island: Island, d: "allow" | "deny") {
  const req = State.sessionApproval;
  if (!req) return;
  Sound.play(d === "deny" ? "blip" : "approve");
  State.sessionApprovals = State.sessionApprovals.filter((r) => r.requestId !== req.requestId);
  State.setPillBadge(sessionTaskId(req.sessionId), null);
  try {
    await Agents.decide(req.requestId, d);
  } catch {
    // Already answered in the sessions window.
  }
  afterApproval(island);
}

export { sessionIdOf };
