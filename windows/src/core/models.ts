// Chat model choice, shared by the island's chat and the settings window. Kept
// free of imports so tests can load it without a DOM or Tauri.

export interface ModelInfo {
  id: string;
  label: string;
}

export interface ModelChoice {
  provider: string;
  model: string;
  /** The last model picked on each provider. */
  providerModels: Record<string, string>;
}

/**
 * Shown when a provider's own list can't be fetched (no key yet, offline).
 * Only Claude has one: elsewhere the models change too often to hard-code.
 */
export const FALLBACK_MODELS: Record<string, ModelInfo[]> = {
  anthropic: [
    { id: "claude-opus-5-5", label: "Claude Opus 5.5" },
    { id: "claude-sonnet-5-5", label: "Claude Sonnet 5.5" },
    { id: "claude-haiku-4-5", label: "Claude Haiku 4.5" },
  ],
};

/** The model last used on a provider while it is still offered, else its first. */
export function chooseModel(models: ModelInfo[], remembered: string | undefined): string {
  if (remembered && (models.length === 0 || models.some((m) => m.id === remembered))) return remembered;
  return models[0]?.id ?? "";
}

/**
 * Settings change for moving the chat to another provider. `models` is the
 * provider's own list, empty when it couldn't be fetched; `fallback` is used
 * only when there is neither a list nor a model picked there before.
 */
export function switchProvider(
  current: ModelChoice,
  next: string,
  models: ModelInfo[],
  fallback: ModelInfo[] = [],
): ModelChoice {
  const providerModels = { ...current.providerModels };
  if (current.model) providerModels[current.provider] = current.model;
  const model = chooseModel(models, providerModels[next]) || chooseModel(fallback, undefined);
  if (model) providerModels[next] = model;
  return { provider: next, model, providerModels };
}

/** Settings change for picking another model on the same provider. */
export function selectModel(current: ModelChoice, model: string): Pick<ModelChoice, "model" | "providerModels"> {
  return { model, providerModels: { ...current.providerModels, [current.provider]: model } };
}

/**
 * What the model menu lists: the provider's models (or the fallback), with the
 * current model kept in even when the list doesn't have it.
 */
export function pickerModels(list: ModelInfo[] | null, provider: string, current: string): ModelInfo[] {
  const models = list ?? FALLBACK_MODELS[provider] ?? [];
  if (current && !models.some((m) => m.id === current)) return [{ id: current, label: current }, ...models];
  return models;
}

export function modelLabel(list: ModelInfo[] | null, id: string): string {
  return list?.find((m) => m.id === id)?.label ?? id;
}
