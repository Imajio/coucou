// Every pill the island can show, and the rules for declaring them (Settings →
// Active pills): a port of PillCatalog.swift and AppState's pill logic. Kept
// free of imports so tests can load it without a DOM or Tauri.

export type PillCategory = "workspace" | "agent" | "ai" | "service";

export const CATEGORY_TITLES: Record<PillCategory, string> = {
  workspace: "Where you code",
  agent: "Agents",
  ai: "AI for the chat",
  service: "Services",
};

export type PillSource = "claudeCode" | "n8n" | "agent";

export interface PillDefinition {
  id: string;
  name: string;
  color: string;
  category: PillCategory;
  /** Shown next to the name on the idle card. */
  subtitle: string;
  source: PillSource;
  comingSoon?: boolean;
  /** AI pills: the chat provider they stand for. */
  provider?: string;
  /** Agent pills: the hooks that feed them. */
  hookAgent?: "gemini" | "antigravity";
}

export const MAIN_DEFAULT = "integration_claude";
/** Pills besides VS Code that can be on at once. */
export const MAX_ACTIVE = 4;

export const PILL_CATALOG: readonly PillDefinition[] = [
  // Where you code
  { id: "integration_claude", name: "VS Code", color: "#F5F6F8", category: "workspace", subtitle: "Integration", source: "claudeCode" },
  { id: "agent_cursor", name: "Cursor", color: "#C0C4CC", category: "workspace", subtitle: "Integration", source: "agent", comingSoon: true },
  { id: "agent_codex", name: "Codex", color: "#2DD4BF", category: "workspace", subtitle: "Integration", source: "agent", comingSoon: true },
  // Agents
  { id: "agent_gemini", name: "Gemini CLI", color: "#8AB4F8", category: "agent", subtitle: "Agent", source: "agent", hookAgent: "gemini" },
  { id: "agent_antigravity", name: "Antigravity", color: "#E879F9", category: "agent", subtitle: "Agent", source: "agent", hookAgent: "antigravity" },
  // AI for the chat
  { id: "ai_anthropic", name: "Anthropic", color: "#E07950", category: "ai", subtitle: "Chat", source: "n8n", provider: "anthropic" },
  { id: "ai_google", name: "Google AI", color: "#4285F4", category: "ai", subtitle: "Chat", source: "n8n", provider: "google" },
  { id: "ai_openai", name: "OpenAI", color: "#10A37F", category: "ai", subtitle: "Chat", source: "n8n", provider: "openai" },
  { id: "ai_openrouter", name: "OpenRouter", color: "#94A3B8", category: "ai", subtitle: "Chat", source: "n8n", provider: "openrouter" },
  { id: "ai_ollama", name: "Ollama", color: "#D4D4D8", category: "ai", subtitle: "Chat", source: "n8n", provider: "ollama" },
  // Services
  { id: "integration_resend", name: "Resend", color: "#22C55E", category: "service", subtitle: "Integration", source: "n8n" },
  { id: "integration_n8n", name: "n8n", color: "#F29B38", category: "service", subtitle: "Integration", source: "n8n" },
  { id: "integration_vercel", name: "Vercel", color: "#7C5CFF", category: "service", subtitle: "Integration", source: "n8n" },
  { id: "integration_github", name: "GitHub", color: "#F4505E", category: "service", subtitle: "Integration", source: "n8n" },
  { id: "integration_notion", name: "Notion", color: "#8C8C8C", category: "service", subtitle: "Integration", source: "n8n" },
  { id: "integration_calcom", name: "Cal.com", color: "#C9956A", category: "service", subtitle: "Integration", source: "n8n" },
  { id: "integration_stripe", name: "Stripe", color: "#0570DE", category: "service", subtitle: "Integration", source: "n8n" },
];

export function pillDefinition(id: string): PillDefinition | undefined {
  return PILL_CATALOG.find((p) => p.id === id);
}

export interface PillChoice {
  /** Declared pills besides VS Code, at most MAX_ACTIVE. */
  active: string[];
  /** The pill in the big card: VS Code, or a declared workspace pill. */
  mainPill: string;
}

/** Drops ids the catalog doesn't know, and a main pill that isn't allowed. */
export function sanitizePills(choice: PillChoice): PillChoice {
  const active = [...new Set(choice.active)]
    .filter((id) => id !== MAIN_DEFAULT && pillDefinition(id))
    .slice(0, MAX_ACTIVE);
  const main = pillDefinition(choice.mainPill);
  const mainPill =
    choice.mainPill === MAIN_DEFAULT || (main?.category === "workspace" && active.includes(choice.mainPill))
      ? choice.mainPill
      : MAIN_DEFAULT;
  return { active, mainPill };
}

/** Turning a pill on or off. VS Code always stays; turning off the main pill gives VS Code the big card back. */
export function togglePill(choice: PillChoice, id: string): PillChoice {
  if (id === MAIN_DEFAULT || !pillDefinition(id)) return choice;
  if (choice.active.includes(id)) {
    return {
      active: choice.active.filter((x) => x !== id),
      mainPill: choice.mainPill === id ? MAIN_DEFAULT : choice.mainPill,
    };
  }
  if (choice.active.length >= MAX_ACTIVE) return choice;
  return { active: [...choice.active, id], mainPill: choice.mainPill };
}

/** Whether a catalog pill belongs in the island. */
export function pillShown(choice: PillChoice, id: string): boolean {
  return id === MAIN_DEFAULT || id === choice.mainPill || choice.active.includes(id);
}

/** Whether ending a session removes the pill, or only resets it (declared pills stay). */
export function pillStays(choice: PillChoice, id: string): boolean {
  return id === MAIN_DEFAULT || id === choice.mainPill || (!!pillDefinition(id) && choice.active.includes(id));
}

/**
 * Island order: catalog order, with pills the catalog doesn't know (an agent
 * that tagged its own events) right after VS Code.
 */
export function orderPills<T extends { id: string }>(tasks: T[]): T[] {
  const rank = new Map(PILL_CATALOG.map((p, i) => [p.id, i]));
  const known = tasks.filter((t) => rank.has(t.id)).sort((a, b) => rank.get(a.id)! - rank.get(b.id)!);
  const others = tasks.filter((t) => !rank.has(t.id));
  const vsCode = known.findIndex((t) => t.id === MAIN_DEFAULT);
  if (vsCode < 0) return [...others, ...known];
  return [...known.slice(0, vsCode + 1), ...others, ...known.slice(vsCode + 1)];
}
