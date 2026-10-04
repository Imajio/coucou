// The sessions window: every agent session as an avatar in the sidebar, the
// selected one's conversation with its tool calls and approvals, a box to
// follow up, and the form that starts a new session with a role, a task, a
// folder, a model and how freely it may act.

import "./sessions.css";
import { h, clear } from "../views/dom";
import { Bridge, onEvent, type ProviderInfo } from "../core/bridge";
import type { ModelInfo } from "../core/models";
import { Agents } from "../core/agentApi";
import {
  AGENT_EVENTS, MODE_LABELS, botState, isBusy, planProgress, sessionTaskId, shortFolder, statusLabel,
  type Entry, type EntryEvent, type RoleInfo, type SessionMode, type SessionSummary,
} from "../core/agents";
import type { AgentTask } from "../core/state";
import { createMiniBot, pruneMiniBots, syncMiniBotStates, tickMiniBots } from "../mochi/minibots";
import { renderDetail, renderMarkdown } from "./render";

const root = document.getElementById("sessions-root")!;

let roles: RoleInfo[] = [];
let providers: ProviderInfo[] = [];
const sessions = new Map<string, SessionSummary>();
const logs = new Map<string, Entry[]>();
let current: string | null = null;

// ── Avatars ───────────────────────────────────────────────────────────────────

function avatarTask(s: SessionSummary): AgentTask {
  return {
    id: sessionTaskId(s.id), name: s.name, color: s.color, state: botState(s.status),
    stepIndex: 0, steps: [], source: "session", isIntegration: false,
  };
}

/** The canvas is drawn at 1/0.6 of the body (see createMiniBot): the slot holds all of it. */
function avatar(s: SessionSummary, size: number): HTMLElement {
  const box = Math.round(size / 0.6);
  return h("span", { class: "avatar", style: `width:${box}px;height:${box}px` }, createMiniBot(avatarTask(s), size));
}

let last = performance.now();
function frame(now: number) {
  const dt = Math.min(0.1, (now - last) / 1000);
  last = now;
  if (!document.hidden) tickMiniBots(dt);
  requestAnimationFrame(frame);
}

// ── Layout ────────────────────────────────────────────────────────────────────

const list = h("div", { class: "list" });
const main = h("main", { class: "main" });
const newBtn = h("button", { class: "new", onclick: () => select(null) }, h("span", { text: "+" }), h("span", { text: "New session" }));
root.replaceChildren(
  h("div", { class: "app" },
    h("aside", { class: "side" },
      h("div", { class: "side-head" }, h("b", { text: "Sessions" }), newBtn),
      list,
    ),
    main,
  ),
);

function ordered(): SessionSummary[] {
  // Delegated sessions sit right under the session that started them.
  const all = [...sessions.values()].sort((a, b) => b.updatedAt - a.updatedAt);
  const top = all.filter((s) => !s.parent || !sessions.has(s.parent));
  const out: SessionSummary[] = [];
  for (const s of top) {
    out.push(s);
    out.push(...all.filter((c) => c.parent === s.id).sort((a, b) => a.updatedAt - b.updatedAt));
  }
  return out;
}

function renderList() {
  clear(list);
  const items = ordered();
  if (items.length === 0) {
    list.append(h("div", { class: "list-empty", text: "No sessions yet. Start one to give Mochi some work." }));
  }
  for (const s of items) {
    const line = s.status === "waiting" ? "Waiting for your approval" : s.activity || statusLabel(s.status);
    const item = h("button", { class: `item${s.id === current ? " on" : ""}${s.parent ? " child" : ""}`, onclick: () => select(s.id) },
      avatar(s, 22),
      h("span", { class: "item-text" },
        h("span", { class: "item-name" }, h("b", { text: s.name }), h("span", { class: `badge ${s.status}`, text: statusLabel(s.status) })),
        h("span", { class: "item-line", text: line }),
      ),
    );
    list.append(item);
  }
  newBtn.classList.toggle("on", current === null);
  pruneMiniBots();
}

// ── Selecting ─────────────────────────────────────────────────────────────────

async function select(id: string | null) {
  current = id && sessions.has(id) ? id : null;
  renderList();
  if (current === null) {
    await renderForm();
    return;
  }
  const id2 = current;
  try {
    const view = await Agents.get(id2);
    sessions.set(id2, view.summary);
    logs.set(id2, view.log);
  } catch (err) {
    console.error(err);
  }
  if (current === id2) renderSession();
}

// ── A session ─────────────────────────────────────────────────────────────────

let feed: HTMLElement | null = null;
let feedNodes: HTMLElement[] = [];
let headerEl: HTMLElement | null = null;
let planEl: HTMLElement | null = null;
let approvalEl: HTMLElement | null = null;
let composer: HTMLTextAreaElement | null = null;
let sendBtn: HTMLButtonElement | null = null;
let stopBtn: HTMLButtonElement | null = null;
const openDetails = new Set<string>();

function renderSession() {
  const s = current ? sessions.get(current) : undefined;
  if (!s) return;
  headerEl = h("header", { class: "top" });
  planEl = h("div", { class: "plan" });
  feed = h("div", { class: "feed" });
  approvalEl = h("div", { class: "approval" });
  composer = h("textarea", { class: "composer-input", rows: 3, placeholder: "Follow up, or give it the next task (Ctrl+Enter to send)", spellcheck: "false" }) as HTMLTextAreaElement;
  sendBtn = h("button", { class: "btn primary", text: "Send" }) as HTMLButtonElement;
  stopBtn = h("button", { class: "btn danger", text: "Stop" }) as HTMLButtonElement;
  const composerError = h("div", { class: "composer-error" });
  const send = async () => {
    if (!current || !composer) return;
    const text = composer.value.trim();
    if (!text) return;
    composerError.textContent = "";
    try {
      await Agents.send(current, text);
      composer.value = "";
    } catch (err) {
      composerError.textContent = String(err);
    }
  };
  sendBtn.addEventListener("click", () => void send());
  stopBtn.addEventListener("click", () => current && void Agents.stop(current));
  composer.addEventListener("keydown", (e) => {
    if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
      e.preventDefault();
      void send();
    }
  });
  main.replaceChildren(
    headerEl,
    planEl,
    feed,
    approvalEl,
    h("div", { class: "composer" }, composer, h("div", { class: "composer-actions" }, composerError, stopBtn, sendBtn)),
  );
  feedNodes = [];
  for (const entry of logs.get(s.id) ?? []) feed.append(renderEntryNode(feedNodes.length, entry));
  updateSessionChrome();
  feed.scrollTop = feed.scrollHeight;
  composer.focus();
}

function renderEntryNode(index: number, entry: Entry): HTMLElement {
  const node = renderEntry(entry);
  feedNodes[index] = node;
  return node;
}

const TOOL_ICONS: Record<string, string> = {
  read_file: "R", write_file: "W", edit_file: "E", list_dir: "L", glob: "G", grep: "S",
  run_command: "$", web_fetch: "@", todo_write: "P", delegate: "D",
};

function renderEntry(entry: Entry): HTMLElement {
  switch (entry.kind) {
    case "user":
      return h("div", { class: "msg user" }, h("div", { class: "bubble", text: entry.text }));
    case "assistant":
      return h("div", { class: "msg assistant" }, renderMarkdown(entry.text));
    case "note":
      return h("div", { class: "msg note", text: entry.text });
    case "error":
      return h("div", { class: "msg error", text: entry.text });
    case "tool": {
      const key = `${entry.callId}`;
      const body = h("div", { class: "tool-body" });
      if (entry.detail) body.append(renderDetail(entry.detail));
      if (entry.output) body.append(h("pre", { class: "output", text: entry.output }));
      const open = openDetails.has(key);
      const card = h("div", { class: `tool ${entry.state}${open ? " open" : ""}` },
        h("button", { class: "tool-head", onclick: () => {
          card.classList.toggle("open");
          if (card.classList.contains("open")) openDetails.add(key);
          else openDetails.delete(key);
        } },
          h("span", { class: "tool-icon", text: TOOL_ICONS[entry.tool] ?? "*" }),
          h("span", { class: "tool-summary", text: entry.summary }),
          entry.outside ? h("span", { class: "tool-flag", text: "outside folder" }) : null,
          h("span", { class: `tool-state ${entry.state}`, text: TOOL_STATE[entry.state] }),
        ),
        body.childNodes.length ? body : null,
      );
      return card;
    }
  }
}

const TOOL_STATE: Record<string, string> = {
  running: "running", waiting: "needs approval", ok: "done", error: "failed", denied: "denied",
};

function updateSessionChrome() {
  const s = current ? sessions.get(current) : undefined;
  if (!s || !headerEl || !planEl || !approvalEl || !stopBtn || !sendBtn) return;
  const role = roles.find((r) => r.id === s.role);

  // Header: who it is, what it runs on, how freely it acts.
  clear(headerEl);
  const providerSel = h("select", { class: "sel", title: "Provider" }) as HTMLSelectElement;
  for (const p of providers) providerSel.append(h("option", { value: p.id, text: p.name, selected: p.id === s.provider }));
  const modelSel = h("select", { class: "sel model", title: "Model" }) as HTMLSelectElement;
  modelSel.append(h("option", { value: s.model, text: s.model, selected: true }));
  void fillModels(modelSel, s.provider, s.model);
  providerSel.addEventListener("change", async () => {
    const models = await loadModels(providerSel.value);
    const model = models[0]?.id;
    if (!model) {
      providerSel.value = s.provider;
      alertLine(`No ${providers.find((p) => p.id === providerSel.value)?.name ?? "provider"} models: add its key in Settings > AI providers.`);
      return;
    }
    await Agents.update(s.id, { provider: providerSel.value, model });
  });
  modelSel.addEventListener("change", () => void Agents.update(s.id, { model: modelSel.value }));
  const modeSel = h("select", { class: "sel", title: MODE_LABELS[s.mode].hint }) as HTMLSelectElement;
  for (const m of Object.keys(MODE_LABELS) as SessionMode[]) {
    modeSel.append(h("option", { value: m, text: MODE_LABELS[m].name, selected: m === s.mode }));
  }
  modeSel.addEventListener("change", () => void Agents.update(s.id, { mode: modeSel.value as SessionMode }));
  // Two clicks: the first one asks, in place.
  const del = h("button", { class: "btn ghost", text: "Delete", title: "Delete this session and its history" }) as HTMLButtonElement;
  let armed = false;
  del.addEventListener("click", async () => {
    if (!armed) {
      armed = true;
      del.textContent = "Delete? Click again";
      del.classList.add("danger");
      window.setTimeout(() => {
        armed = false;
        del.textContent = "Delete";
        del.classList.remove("danger");
      }, 3000);
      return;
    }
    await Agents.remove(s.id);
  });
  const parent = s.parent ? sessions.get(s.parent) : undefined;
  headerEl.append(
    avatar(s, 30),
    h("div", { class: "who" },
      h("div", { class: "who-name" }, h("b", { text: s.name }), h("span", { class: "role", style: `color:${s.color}`, text: role?.name ?? s.role }), h("span", { class: `badge ${s.status}`, text: statusLabel(s.status) })),
      h("div", { class: "who-meta" },
        h("button", { class: "folder", title: `${s.folder}\nOpen in VS Code`, text: shortFolder(s.folder), onclick: () => void Bridge.openInVSCode(s.folder) }),
        parent ? h("button", { class: "folder", text: `from ${parent.name}`, onclick: () => void select(parent.id) }) : null,
      ),
    ),
    h("div", { class: "controls" }, providerSel, modelSel, modeSel, del),
  );

  // The plan, when it keeps one.
  clear(planEl);
  planEl.style.display = s.todos.length ? "" : "none";
  if (s.todos.length) {
    planEl.append(h("div", { class: "plan-head", text: `Plan ${planProgress(s.todos)}` }));
    for (const t of s.todos) {
      planEl.append(h("div", { class: `todo ${t.status}` }, h("i"), h("span", { text: t.content })));
    }
  }

  // The approval it waits for.
  clear(approvalEl);
  approvalEl.style.display = s.approval ? "" : "none";
  if (s.approval) {
    const req = s.approval;
    const decide = (d: "allow" | "always" | "deny") => async () => {
      try {
        await Agents.decide(req.requestId, d);
      } catch (err) {
        alertLine(String(err));
      }
    };
    approvalEl.append(h("div", { class: "approval-head" },
        h("b", { text: req.summary }),
        req.outside ? h("span", { class: "tool-flag", text: "outside its folder" }) : null,
    ));
    if (req.detail) approvalEl.append(renderDetail(req.detail));
    approvalEl.append(
      h("div", { class: "approval-actions" },
        h("button", { class: "btn danger", text: "Deny", onclick: decide("deny") }),
        req.outside && req.tool !== "run_command" && req.tool !== "web_fetch"
          ? null
          : h("button", { class: "btn ghost", text: `Always allow ${req.tool}`, onclick: decide("always") }),
        h("button", { class: "btn primary", text: "Allow", onclick: decide("allow") }),
      ),
    );
  }
  stopBtn.style.display = isBusy(s.status) ? "" : "none";
  sendBtn.disabled = isBusy(s.status);
  pruneMiniBots();
}

function alertLine(text: string) {
  if (!feed) return;
  feed.append(h("div", { class: "msg error", text }));
  feed.scrollTop = feed.scrollHeight;
}

// ── Models ────────────────────────────────────────────────────────────────────

const modelCache = new Map<string, ModelInfo[]>();

async function loadModels(provider: string): Promise<ModelInfo[]> {
  if (modelCache.has(provider)) return modelCache.get(provider)!;
  try {
    const list = await Bridge.chatModels(provider);
    modelCache.set(provider, list);
    return list;
  } catch {
    return [];
  }
}

async function fillModels(sel: HTMLSelectElement, provider: string, selected: string) {
  const models = await loadModels(provider);
  if (!models.length) return;
  clear(sel);
  if (selected && !models.some((m) => m.id === selected)) sel.append(h("option", { value: selected, text: selected }));
  for (const m of models) sel.append(h("option", { value: m.id, text: m.label }));
  sel.value = selected || models[0].id;
}

// ── New session ───────────────────────────────────────────────────────────────

async function renderForm() {
  const defaults = await Agents.defaults().catch(() => null);
  let role = roles.find((r) => r.id === defaults?.role) ?? roles[0];
  let mode: SessionMode = defaults?.mode ?? "ask";

  const roleGrid = h("div", { class: "roles" });
  const prompt = h("textarea", { class: "field prompt", rows: 4, spellcheck: "false" }) as HTMLTextAreaElement;
  const drawRoles = () => {
    clear(roleGrid);
    for (const r of roles) {
      roleGrid.append(h("button", { class: `role-card${r.id === role?.id ? " on" : ""}`, onclick: () => {
        role = r;
        prompt.value = r.prompt;
        drawRoles();
      } },
        h("span", { class: "role-name" }, h("i", { style: `background:${r.color}` }), h("b", { text: r.name })),
        h("span", { class: "role-summary", text: r.summary }),
      ));
    }
  };
  drawRoles();
  prompt.value = role?.prompt ?? "";

  const task = h("textarea", { class: "field task", rows: 6, placeholder: "What should this session do? Be as specific as you would with a colleague: the goal, the files, what done looks like.", spellcheck: "false" }) as HTMLTextAreaElement;
  const folder = h("input", { class: "field", placeholder: "C:\\path\\to\\project", spellcheck: "false", value: defaults?.folder ?? "" }) as HTMLInputElement;
  const folderNote = h("div", { class: "note-line" });
  const checkFolder = async () => {
    if (!folder.value.trim()) {
      folderNote.textContent = "";
      return;
    }
    try {
      const resolved = await Agents.checkFolder(folder.value);
      folderNote.textContent = `Works in ${resolved}`;
      folderNote.className = "note-line ok";
    } catch (err) {
      folderNote.textContent = String(err);
      folderNote.className = "note-line err";
    }
  };
  folder.addEventListener("change", () => void checkFolder());
  void checkFolder();
  const name = h("input", { class: "field", placeholder: "Named after its role when empty", spellcheck: "false" }) as HTMLInputElement;

  const providerSel = h("select", { class: "field" }) as HTMLSelectElement;
  for (const p of providers) {
    const ready = p.hasKey || !p.keyRequired;
    providerSel.append(h("option", { value: p.id, text: ready ? p.name : `${p.name} (no key yet)`, selected: p.id === (defaults?.provider || "anthropic") }));
  }
  const modelSel = h("select", { class: "field" }) as HTMLSelectElement;
  const syncModels = (keep: string) => {
    clear(modelSel);
    modelSel.append(h("option", { value: keep, text: keep || "Loading models..." }));
    void fillModels(modelSel, providerSel.value, keep);
  };
  providerSel.addEventListener("change", () => syncModels(""));
  syncModels(defaults && defaults.provider === providerSel.value ? defaults.model : "");

  const modes = h("div", { class: "modes" });
  const drawModes = () => {
    clear(modes);
    for (const m of Object.keys(MODE_LABELS) as SessionMode[]) {
      modes.append(h("button", { class: `mode-card${m === mode ? " on" : ""}`, onclick: () => {
        mode = m;
        drawModes();
      } }, h("b", { text: MODE_LABELS[m].name }), h("span", { text: MODE_LABELS[m].hint })));
    }
  };
  drawModes();

  const error = h("div", { class: "note-line err" });
  const start = h("button", { class: "btn primary big", text: "Start session" }) as HTMLButtonElement;
  const submit = async () => {
    if (!role) return;
    error.textContent = "";
    start.disabled = true;
    try {
      const id = await Agents.create({
        name: name.value.trim() || undefined,
        role: role.id,
        rolePrompt: prompt.value.trim() === role.prompt ? undefined : prompt.value,
        task: task.value,
        folder: folder.value.trim(),
        provider: providerSel.value,
        model: modelSel.value || undefined,
        mode,
      });
      const list = await Agents.list();
      for (const s of list) sessions.set(s.id, s);
      await select(id);
    } catch (err) {
      error.textContent = String(err);
    } finally {
      start.disabled = false;
    }
  };
  start.addEventListener("click", () => void submit());
  task.addEventListener("keydown", (e) => {
    if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
      e.preventDefault();
      void submit();
    }
  });

  const details = h("details", { class: "advanced" }, h("summary", { text: "Role instructions" }),
    h("p", { class: "hint", text: "What the session is told it is. Edit it to shape this one session." }), prompt);

  main.replaceChildren(
    h("div", { class: "form" },
      h("h1", { text: "New session" }),
      h("p", { class: "hint", text: "Each session is one of Mochi's avatars: give it a role and some work, it gets on with it and asks you before anything its mode doesn't allow." }),
      h("label", { class: "label", text: "Role" }), roleGrid,
      h("label", { class: "label", text: "Task" }), task,
      h("div", { class: "row2" },
        h("div", {}, h("label", { class: "label", text: "Folder" }), folder, folderNote),
        h("div", {}, h("label", { class: "label", text: "Name" }), name),
      ),
      h("div", { class: "row2" },
        h("div", {}, h("label", { class: "label", text: "Provider" }), providerSel),
        h("div", {}, h("label", { class: "label", text: "Model" }), modelSel),
      ),
      h("label", { class: "label", text: "Permissions" }), modes,
      details,
      h("div", { class: "form-actions" }, error, start),
    ),
  );
  feed = null;
  task.focus();
}

// ── Events ────────────────────────────────────────────────────────────────────

async function boot() {
  [roles, providers] = await Promise.all([
    Agents.roles().catch(() => [] as RoleInfo[]),
    Bridge.chatProviders().then((p) => p ?? []),
  ]);
  for (const s of await Agents.list().catch(() => [] as SessionSummary[])) sessions.set(s.id, s);

  await onEvent<SessionSummary>(AGENT_EVENTS.session, (s) => {
    sessions.set(s.id, s);
    renderList();
    syncMiniBotStates([...sessions.values()].map(avatarTask));
    if (s.id === current && feed) updateSessionChrome();
  });
  await onEvent<EntryEvent>(AGENT_EVENTS.entry, (e) => {
    const log = logs.get(e.sessionId);
    if (!log) return;
    log[e.index] = e.entry;
    if (e.sessionId !== current || !feed) return;
    const atBottom = feed.scrollHeight - feed.scrollTop - feed.clientHeight < 80;
    const node = renderEntry(e.entry);
    const old = feedNodes[e.index];
    if (old) old.replaceWith(node);
    else feed.append(node);
    feedNodes[e.index] = node;
    if (atBottom) feed.scrollTop = feed.scrollHeight;
  });
  await onEvent<string>(AGENT_EVENTS.removed, (id) => {
    sessions.delete(id);
    logs.delete(id);
    if (current === id) void select(ordered()[0]?.id ?? null);
    else renderList();
  });
  await onEvent<string | null>(AGENT_EVENTS.focus, (id) => {
    if (id) logs.delete(id);
    void select(id);
  });
  // Keys may have been added in Settings meanwhile.
  window.addEventListener("focus", () => {
    void Bridge.chatProviders().then((p) => {
      if (p) providers = p;
    });
  });

  requestAnimationFrame(frame);
  await select(ordered()[0]?.id ?? null);
}

void boot();
