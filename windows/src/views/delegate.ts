// The Delegate tab: hand a task to a new agent session without leaving what
// you are doing. Pick a role, say what to do, and it starts in the last folder
// used, on the last model, as freely as last time. Everything else (another
// folder, the role's instructions) is in the sessions window.

import { h, clear } from "./dom";
import { Agents } from "../core/agentApi";
import { MODE_LABELS, sessionTaskId, shortFolder, type RoleInfo, type SessionDefaults, type SessionMode } from "../core/agents";
import { Sound } from "../core/sound";
import { State } from "../core/state";
import type { ViewActions, ViewHost } from "./views";

const QUICK_ROLES = ["engineer", "lead", "reviewer", "researcher", "tester", "writer"];
const MODES: SessionMode[] = ["ask", "edits", "auto"];

export function buildDelegate(actions: ViewActions): ViewHost {
  const roleRow = h("div", { class: "dg-roles" });
  const task = h("textarea", {
    class: "dg-task",
    rows: 3,
    placeholder: "What should it do? It works on its own and asks you before anything risky.",
    spellcheck: "false",
  }) as HTMLTextAreaElement;
  const folderBtn = h("button", { class: "dg-chip", title: "Change it in the sessions window" });
  const modeBtn = h("button", { class: "dg-chip" });
  const status = h("div", { class: "dg-status" });
  const start = h("button", { class: "btn primary", text: "Start" }) as HTMLButtonElement;
  const all = h("button", { class: "btn secondary", text: "Sessions", onclick: () => void Agents.openWindow(null).catch(() => {}) });

  const el = h("div", { class: "view" },
    h("div", { class: "card" },
      h("div", { class: "dg-stack" },
        roleRow,
        task,
        h("div", { class: "dg-row" }, folderBtn, modeBtn, status, all, start),
      ),
    ),
  );

  let roles: RoleInfo[] = [];
  let defaults: SessionDefaults | null = null;
  let role = "engineer";
  let mode: SessionMode = "ask";
  let busy = false;

  function drawRoles() {
    clear(roleRow);
    for (const id of QUICK_ROLES) {
      const r = roles.find((x) => x.id === id);
      if (!r) continue;
      const chip = h("button", { class: `dg-role${id === role ? " on" : ""}`, title: r.summary, onclick: () => {
        role = id;
        drawRoles();
        task.focus();
      } }, h("i", { style: `background:${r.color}` }), h("span", { text: r.name }));
      if (id === role) chip.style.borderColor = `${r.color}99`;
      roleRow.append(chip);
    }
  }

  function drawChips() {
    const folder = defaults?.folder ?? "";
    folderBtn.textContent = folder ? shortFolder(folder) : "Pick a folder";
    folderBtn.title = folder ? `${folder}\nChange it in the sessions window` : "Pick a folder in the sessions window";
    modeBtn.textContent = MODE_LABELS[mode].name;
    modeBtn.title = `${MODE_LABELS[mode].hint} Click to change.`;
  }

  folderBtn.addEventListener("click", () => void Agents.openWindow(null).catch(() => {}));
  modeBtn.addEventListener("click", () => {
    mode = MODES[(MODES.indexOf(mode) + 1) % MODES.length];
    drawChips();
  });

  async function load() {
    try {
      [roles, defaults] = await Promise.all([Agents.roles(), Agents.defaults()]);
      if (defaults.role && QUICK_ROLES.includes(defaults.role)) role = defaults.role;
      if (defaults.mode) mode = defaults.mode;
    } catch {
      // Outside Coucou: nothing to show.
    }
    drawRoles();
    drawChips();
  }

  async function submit() {
    if (busy) return;
    if (!task.value.trim()) {
      status.textContent = "Say what it should do.";
      return;
    }
    if (!defaults?.folder) {
      // First session: the folder is chosen in the window.
      void Agents.openWindow(null).catch(() => {});
      return;
    }
    busy = true;
    start.disabled = true;
    status.textContent = "";
    try {
      const id = await Agents.create({
        role,
        task: task.value,
        folder: defaults.folder,
        provider: defaults.provider || undefined,
        model: defaults.model || undefined,
        mode,
      });
      Sound.play("send");
      task.value = "";
      defaults = await Agents.defaults().catch(() => defaults);
      // Its pill arrives with its first event; then it takes the big card.
      window.setTimeout(() => {
        if (State.tasks.some((t) => t.id === sessionTaskId(id))) State.setFocus(sessionTaskId(id));
        actions.setView("overview");
      }, 150);
    } catch (err) {
      status.textContent = String(err).replace(/^Error:\s*/, "");
      Sound.play("error");
    } finally {
      busy = false;
      start.disabled = false;
    }
  }

  start.addEventListener("click", () => void submit());
  task.addEventListener("keydown", (e) => {
    if (e.key === "Enter" && (e.ctrlKey || !e.shiftKey)) {
      e.preventDefault();
      void submit();
    }
  });

  void load();

  return {
    el,
    sync() {},
    enter() {
      void load();
    },
    focus() {
      task.focus();
    },
  };
}
