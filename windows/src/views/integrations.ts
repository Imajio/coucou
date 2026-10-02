// Integration cards shown in the overview's left card - DOM ports of
// IntegrationCardView and friends from IslandViewContent.swift.

import { h, svg, clear, dot } from "./dom";
import { ICONS } from "./icons";
import { pillDefinition } from "../core/catalog";
import { bookingsOn, dayKey, pageFor, pageRows, stepPage, type CalendarPage } from "../core/calendar";
import { State, type AgentTask } from "../core/state";
import { Bridge } from "../core/bridge";

/** Same shape as the Swift `timeAgo` computed properties. */
export function timeAgo(value: unknown): string {
  const date = typeof value === "number" ? new Date(value) : new Date(String(value));
  const diff = (Date.now() - date.getTime()) / 1000;
  if (!Number.isFinite(diff)) return "";
  if (diff < 60) return "just now";
  if (diff < 3600) return `${Math.floor(diff / 60)}m`;
  if (diff < 86400) return `${Math.floor(diff / 3600)}h`;
  return `${Math.floor(diff / 86400)}d`;
}

function header(color: string, name: string, kind: string, extra?: Node): HTMLElement {
  const row = h("div", { class: "int-head" }, dot(color, 7), h("b", { text: name }), h("span", { text: kind }));
  if (extra) row.append(extra);
  return row;
}

/** Highlighted first row + plain rows, the layout every list card shares. */
function listRow(accent: string, first: boolean, ...children: Node[]): HTMLElement {
  const row = h("div", { class: first ? "int-row first" : "int-row" }, dot(accent, 5), ...children);
  if (first) row.style.background = `${accent}14`;
  return row;
}

function get(id: string): Record<string, unknown> {
  return (State.integrations[id]?.data ?? {}) as Record<string, unknown>;
}

function arr(id: string, key: string): Record<string, unknown>[] {
  const v = get(id)[key];
  return Array.isArray(v) ? (v as Record<string, unknown>[]) : [];
}

// ── Not configured / idle ─────────────────────────────────────────────────────

const OPEN_URLS: Record<string, string> = {
  integration_resend: "https://resend.com/emails",
  integration_vercel: "https://vercel.com/dashboard",
  integration_github: "https://github.com",
  integration_stripe: "https://dashboard.stripe.com/payments",
  integration_notion: "https://notion.so",
  integration_calcom: "https://app.cal.com/bookings",
};

function idleCard(task: AgentTask, hooks: IntegrationCardHooks): HTMLElement {
  const openSettings = hooks.openSettings;
  const def = pillDefinition(task.id);
  const info = State.integrations[task.id];
  const configured = info?.configured ?? false;
  const error = info?.error ?? null;
  const isHooks = task.id === "integration_claude" || !!def?.hookAgent;
  const provider = def?.provider;
  // Hook pills are about hooks, AI pills about their key, the rest about a service.
  let label: string;
  if (def?.comingSoon) label = "Coming soon";
  else if (error) label = error;
  else if (isHooks) label = configured ? "Hooks installed" : "Hooks not installed";
  else if (provider) {
    const s = State.settings;
    const model = s.providerModels[provider] ?? (s.provider === provider ? s.model : "");
    const what = provider === "ollama" ? "Runs locally" : "Key configured";
    label = configured ? (model ? `${what} · ${model}` : what) : "Key not configured";
  } else label = configured ? "Connected · loading…" : "Key not configured";
  const statusColor = def?.comingSoon ? "#6B7079" : error || !configured ? "#F4505E" : "#22C55E";

  const actions = h("div", { class: "int-actions" });
  if (provider) {
    if (configured) {
      actions.append(
        h("button", {
          class: "link-btn",
          style: `color:${task.color}d9`,
          text: `Chat with ${task.name}`,
          onclick: () => hooks.chatWith(provider),
        }),
      );
    }
  } else if (task.id === "agent_cursor") {
    actions.append(
      h("button", {
        class: "link-btn",
        style: `color:${task.color}d9`,
        text: "Open Cursor",
        onclick: async () => {
          if (!(await Bridge.openApp("cursor"))) void Bridge.openUrl("https://cursor.com/download");
        },
      }),
    );
  } else if (task.id === "integration_claude") {
    actions.append(
      h("button", {
        class: "link-btn",
        style: `color:${task.color}b3`,
        text: "Open Visual Studio Code",
        onclick: () => void Bridge.openInVSCode(task.sessionCwd ?? null),
      }),
    );
  } else if (task.id === "integration_n8n") {
    actions.append(
      h("button", {
        class: "link-btn",
        style: `color:${task.color}d9`,
        text: "Open n8n",
        onclick: () => void Bridge.openN8n(),
      }),
    );
  } else if (OPEN_URLS[task.id]) {
    actions.append(
      h("button", {
        class: "link-btn",
        style: `color:${task.color}d9`,
        text: `Open ${task.name}`,
        onclick: () => void Bridge.openUrl(OPEN_URLS[task.id]),
      }),
    );
  }
  if (configured && def?.category === "service") {
    actions.append(
      h("button", {
        class: "link-btn",
        style: `color:${task.color}d9`,
        text: "Refresh",
        onclick: () => void Bridge.refreshIntegration(task.id),
      }),
    );
  } else if (!configured && !def?.comingSoon) {
    actions.append(
      h("button", { class: "link-btn", style: "color:#8e939c", text: "Settings…", onclick: openSettings }),
    );
  }

  return h(
    "div",
    { class: "int-card" },
    header(task.color, def?.name ?? task.name, def?.subtitle ?? "Integration"),
    h("div", { class: "int-status" }, dot(statusColor, 5), h("span", { text: label })),
    actions,
  );
}

// ── Vercel ────────────────────────────────────────────────────────────────────

function vercelCard(onDetail: () => void): HTMLElement {
  const deployments = arr("integration_vercel", "deployments");
  const rows = h("div", { class: "int-rows" });
  deployments.slice(0, 3).forEach((d, i) => {
    const accent = d.state === "READY" ? "#22C55E" : "#F4505E";
    const name = h("span", { class: "int-name", text: String(d.projectName ?? "") });
    const ago = h("span", { class: "int-ago", text: timeAgo(d.createdAt) });
    if (i === 0) {
      const more = h(
        "button",
        { class: "int-more", title: "Details", onclick: onDetail },
        svg(ICONS.ellipsis, 8),
      );
      rows.append(listRow(accent, true, name, ago, more));
    } else {
      rows.append(listRow(accent, false, name, ago));
    }
  });
  return h("div", { class: "int-card" }, header("#7C5CFF", "Vercel", "Deployments"), rows);
}

function vercelDetail(onBack: () => void): HTMLElement {
  const d = arr("integration_vercel", "deployments")[0] ?? {};
  const success = d.state === "READY";
  const accent = success ? "#22C55E" : "#F4505E";
  const status = success ? "Ready" : d.state === "CANCELED" ? "Canceled" : "Error";
  const body = h("div", { class: "int-detail-body" });
  if (d.commitMessage) body.append(h("div", { class: "int-commit", text: String(d.commitMessage) }));
  const meta = h("div", { class: "int-meta" });
  if (d.branch) meta.append(h("span", { text: String(d.branch) }));
  meta.append(h("span", { text: `${timeAgo(d.createdAt)} ago` }));
  body.append(meta);
  if (d.url) {
    body.append(
      h("button", {
        class: "int-link",
        text: String(d.url),
        onclick: () => void Bridge.openUrl(`https://${d.url}`),
      }),
    );
  }
  return h(
    "div",
    { class: "int-card detail" },
    h(
      "div",
      { class: "int-detail-head" },
      h("button", { class: "int-back", onclick: onBack }, svg(ICONS.chevronLeft, 10, { stroke: 2.4 })),
      dot(accent, 6),
      h("b", { text: String(d.projectName ?? "Deployment") }),
      h("span", { class: "int-badge", style: `color:${accent};background:${accent}24`, text: status }),
    ),
    body,
  );
}

// ── Resend ────────────────────────────────────────────────────────────────────

function resendCard(): HTMLElement {
  const emails = arr("integration_resend", "emails");
  const total = get("integration_resend").total;
  const extra =
    total != null
      ? h("span", { class: "int-total" }, h("i", { class: "pulse" }), h("span", { text: String(total) }))
      : undefined;
  const rows = h("div", { class: "int-rows" });
  emails.slice(0, 3).forEach((e, i) => {
    const delivered = e.lastEvent === "delivered";
    const accent = delivered ? "#22C55E" : "#F4505E";
    const to = Array.isArray(e.to) ? String(e.to[0] ?? "?") : "?";
    const short = to.split("@")[0];
    const cells: Node[] = [
      h("span", { class: "int-name", text: short }),
      h("span", { class: "int-ago", text: timeAgo(e.createdAt) }),
    ];
    if (i === 0 && e.subject) cells.push(h("span", { class: "int-sub", text: String(e.subject) }));
    rows.append(listRow(accent, i === 0, ...cells));
  });
  return h("div", { class: "int-card" }, header("#22C55E", "Resend", "Emails", extra), rows);
}

// ── GitHub ────────────────────────────────────────────────────────────────────

function statRow(icon: string, color: string, label: string, value: string): HTMLElement {
  return h(
    "div",
    { class: "int-stat" },
    h("i", { class: "int-stat-icon", style: `color:${color}` }, svg(icon, 10)),
    h("span", { class: "int-stat-label", text: label }),
    h("span", { class: "int-stat-value", text: value }),
  );
}

function githubCard(): HTMLElement {
  const d = get("integration_github");
  const stars = Number(d.totalStars ?? 0);
  const repos = Number(d.totalRepos ?? 0);
  const fmt = (n: number) => (n >= 1000 ? `${(n / 1000).toFixed(1)}k` : String(n));
  return h(
    "div",
    { class: "int-card" },
    header("#F4505E", "GitHub", "Overview"),
    h(
      "div",
      { class: "int-stats" },
      statRow(ICONS.star, "#F5A524", "Total stars", fmt(stars)),
      statRow(ICONS.stack, "#6B7079", "Repositories", String(repos)),
    ),
  );
}

// ── Stripe ────────────────────────────────────────────────────────────────────

function stripeCard(): HTMLElement {
  const d = get("integration_stripe");
  const balance = (Number(d.balance ?? 0) / 100).toFixed(2);
  const currency = String(d.currency ?? "eur").toUpperCase();
  const rows = h("div", { class: "int-rows tight" });
  for (const p of arr("integration_stripe", "payments")) {
    const success = p.status === "succeeded";
    const accent = success ? "#22C55E" : "#F4505E";
    rows.append(
      h(
        "div",
        { class: "int-row" },
        dot(accent, 5),
        h("span", { class: "int-name", text: String(p.description ?? "Payment") }),
        h("span", {
          class: "int-amount",
          style: "color:#22c55e",
          text: `+${(Number(p.amount ?? 0) / 100).toFixed(2)}`,
        }),
        h("span", { class: "int-ago", text: timeAgo(p.createdAt) }),
      ),
    );
  }
  return h(
    "div",
    { class: "int-card" },
    header("#0570DE", "Stripe", "Payments"),
    h("div", { class: "int-balance" }, h("span", { text: balance }), h("i", { text: currency })),
    rows,
  );
}

// ── Notion ────────────────────────────────────────────────────────────────────

function notionCard(): HTMLElement {
  const rows = h("div", { class: "int-rows tight" });
  for (const p of arr("integration_notion", "pages").slice(0, 3)) {
    rows.append(
      h(
        "button",
        {
          class: "int-page",
          onclick: () => {
            if (typeof p.url === "string") void Bridge.openUrl(p.url);
          },
        },
        p.emoji
          ? h("span", { class: "int-emoji", text: String(p.emoji) })
          : h("i", { class: "int-emoji" }, svg(ICONS.doc, 9)),
        h("span", { class: "int-name", text: String(p.title ?? "Untitled") }),
        h("span", { class: "int-ago", text: timeAgo(p.lastEditedAt) }),
      ),
    );
  }
  return h("div", { class: "int-card" }, header("#E8E8E8", "Notion", "Recent"), rows);
}

// ── Cal.com ───────────────────────────────────────────────────────────────────

// Three levels as in CalcomCardView: the half-month calendar, a day's
// bookings, one booking. Where the card stands survives the polls' re-renders
// and resets when another pill takes the focus.

const CALCOM = "#C9956A";

type CalcomPlace = { page: CalendarPage; day: string | null; booking: string | null };

let calcom: CalcomPlace = { page: pageFor(new Date()), day: null, booking: null };

/** Back to the calendar on today's half month, for a fresh focus. */
export function resetIntegrationCards() {
  calcom = { page: pageFor(new Date()), day: null, booking: null };
}

const pad2 = (n: number) => String(n).padStart(2, "0");
const hhmm = (d: Date) => `${pad2(d.getHours())}:${pad2(d.getMinutes())}`;

function fromKey(key: string): Date {
  const [y, m, d] = key.split("-").map(Number);
  return new Date(y, m - 1, d);
}

function calcomCard(): HTMLElement {
  const card = h("div", { class: "int-card cal" });
  const bookings = arr("integration_calcom", "bookings");

  const go = (place: Partial<CalcomPlace>) => {
    calcom = { ...calcom, ...place };
    render();
  };
  const back = (onclick: () => void) =>
    h("button", { class: "int-back", title: "Back", onclick }, svg(ICONS.chevronLeft, 10, { stroke: 2.4 }));

  function calendar(): Node[] {
    const { page } = calcom;
    const month = new Date(page.year, page.month, 1).toLocaleDateString(undefined, { month: "long", year: "numeric" });
    const today = dayKey(new Date());
    const busy = new Set(
      bookings.map((b) => new Date(String(b.start))).filter((d) => !Number.isNaN(d.getTime())).map(dayKey),
    );
    const nav = h(
      "div",
      { class: "cal-nav" },
      h("button", { class: "cal-step", title: "Previous", onclick: () => go({ page: stepPage(page, -1) }) },
        svg(ICONS.chevronLeft, 9, { stroke: 2.6 })),
      h("span", { text: `${month} Q${page.half}` }),
      h("button", { class: "cal-step", title: "Next", onclick: () => go({ page: stepPage(page, 1) }) },
        svg(ICONS.chevronRight, 9, { stroke: 2.6 })),
    );
    const grid = h("div", { class: "cal-grid" });
    for (const row of pageRows(page)) {
      const first = row[0]!;
      const line = h("div", { class: "cal-row" },
        h("span", { class: "cal-week", text: `${pad2(first.getDate())}/${pad2(first.getMonth() + 1)}` }));
      for (const day of row) {
        if (!day) {
          line.append(h("span", { class: "cal-day empty" }));
          continue;
        }
        const key = dayKey(day);
        line.append(
          h("button", { class: key === today ? "cal-day today" : "cal-day", onclick: () => go({ day: key }) },
            h("b", { text: String(day.getDate()) }),
            h("i", { class: busy.has(key) ? "on" : "" })),
        );
      }
      grid.append(line);
    }
    return [header(CALCOM, "Cal.com", "Schedule"), nav, grid];
  }

  function day(key: string): Node[] {
    const list = bookingsOn(bookings, key);
    const label = fromKey(key).toLocaleDateString(undefined, { weekday: "long", day: "numeric", month: "long" });
    const head = h("div", { class: "int-detail-head cal-head" }, back(() => go({ day: null })), h("b", { text: label }));
    const rows = h("div", { class: "cal-bookings" });
    if (list.length === 0) rows.append(h("div", { class: "int-empty", text: "No calls scheduled" }));
    for (const b of list) {
      rows.append(
        h("button", { class: "cal-booking", onclick: () => go({ booking: String(b.id) }) },
          dot(CALCOM, 4),
          h("span", { class: "int-time", text: hhmm(new Date(String(b.start))) }),
          h("span", { class: "int-name", text: String(b.title ?? "Meeting") }),
          svg(ICONS.chevronRight, 8, { stroke: 2.2 })),
      );
    }
    return [head, rows];
  }

  function booking(b: Record<string, unknown>): Node[] {
    const head = h("div", { class: "int-detail-head cal-head" },
      back(() => go({ booking: null })),
      h("span", { class: "int-time", text: hhmm(new Date(String(b.start))) }));
    const body = h("div", { class: "cal-detail" }, h("b", { text: String(b.title ?? "Meeting") }));
    const line = (icon: string, value: unknown, kind: string) => {
      if (typeof value === "string" && value.trim()) {
        body.append(h("div", { class: `cal-line ${kind}` }, svg(icon, 10), h("span", { text: value })));
      }
    };
    line(ICONS.person, b.attendeeName, "name");
    line(ICONS.envelope, b.attendeeEmail, "email");
    line(ICONS.note, b.attendeeNotes, "notes");
    return [head, body];
  }

  function render() {
    clear(card);
    // A booking cancelled meanwhile falls back to its day.
    const picked = calcom.booking ? bookings.find((b) => String(b.id) === calcom.booking) : undefined;
    if (calcom.booking && !picked) calcom = { ...calcom, booking: null };
    if (picked) card.append(...booking(picked));
    else if (calcom.day) card.append(...day(calcom.day));
    else card.append(...calendar());
  }

  render();
  return card;
}

// ── n8n ───────────────────────────────────────────────────────────────────────

function n8nCard(task: AgentTask, onDetail: () => void, hooks: IntegrationCardHooks): HTMLElement {
  const hasActivity = task.steps.length > 0 && (task.state === "finished" || task.state === "error");
  if (!hasActivity) return idleCard(task, hooks);
  const success = task.state === "finished";
  const accent = success ? "#22C55E" : "#F4505E";
  return h(
    "div",
    { class: "int-card" },
    header("#F29B38", "n8n", "Workflow"),
    h(
      "div",
      { class: "int-actions" },
      h(
        "button",
        {
          class: "int-pill",
          style: `background:${accent}1a;border-color:${accent}38`,
          onclick: onDetail,
        },
        dot(accent, 5),
        h("span", { class: "int-name", text: task.steps[0] ?? "Workflow" }),
        svg(ICONS.ellipsis, 8),
      ),
    ),
  );
}

function n8nDetail(task: AgentTask, onBack: () => void): HTMLElement {
  const success = task.state === "finished";
  const accent = success ? "#22C55E" : "#F4505E";
  const detail = task.steps[1];
  return h(
    "div",
    { class: "int-card detail" },
    h(
      "div",
      { class: "int-detail-head" },
      h("button", { class: "int-back", onclick: onBack }, svg(ICONS.chevronLeft, 10, { stroke: 2.4 })),
      dot(accent, 6),
      h("b", { text: task.steps[0] ?? "Workflow" }),
      h("span", {
        class: "int-badge",
        style: `color:${accent};background:${accent}24`,
        text: success ? "Success" : "Failed",
      }),
    ),
    detail
      ? h("pre", { class: "int-detail-text", text: detail })
      : h("div", {
          class: "int-status",
          text: success ? "Completed successfully." : "No error details available.",
        }),
  );
}

// ── Dispatch ──────────────────────────────────────────────────────────────────

export interface IntegrationCardHooks {
  detailOpen: boolean;
  openDetail(): void;
  closeDetail(): void;
  openSettings(): void;
  /** AI pills: move the chat to this provider and open it. */
  chatWith(provider: string): void;
}

/** True when this integration has data worth showing instead of the idle card. */
export function hasIntegrationData(id: string): boolean {
  const info = State.integrations[id];
  if (!info || info.error) return false;
  switch (id) {
    case "integration_vercel":
      return arr(id, "deployments").length > 0;
    case "integration_resend":
      return arr(id, "emails").length > 0;
    case "integration_github":
      return get(id).totalRepos != null;
    case "integration_stripe":
      return info.loaded;
    case "integration_notion":
      return arr(id, "pages").length > 0;
    case "integration_calcom":
      return info.loaded;
    default:
      return false;
  }
}

export function renderIntegrationCard(task: AgentTask, hooks: IntegrationCardHooks): HTMLElement {
  if (task.id === "integration_n8n") {
    const hasActivity = task.steps.length > 0 && (task.state === "finished" || task.state === "error");
    return hooks.detailOpen && hasActivity
      ? n8nDetail(task, hooks.closeDetail)
      : n8nCard(task, hooks.openDetail, hooks);
  }
  if (task.id === "integration_vercel" && hasIntegrationData(task.id)) {
    return hooks.detailOpen ? vercelDetail(hooks.closeDetail) : vercelCard(hooks.openDetail);
  }
  if (!hasIntegrationData(task.id)) return idleCard(task, hooks);

  switch (task.id) {
    case "integration_resend":
      return resendCard();
    case "integration_github":
      return githubCard();
    case "integration_stripe":
      return stripeCard();
    case "integration_notion":
      return notionCard();
    case "integration_calcom":
      return calcomCard();
    default:
      return idleCard(task, hooks);
  }
}

export { clear };
