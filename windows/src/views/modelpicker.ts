// Provider + model menus for the chat, used in the island's chat view and in
// the settings window. Each provider's model list is asked of the provider
// once, then kept until the menus are refreshed (a key was added, say).

import { h, clear } from "./dom";
import { Bridge, type ProviderInfo } from "../core/bridge";
import {
  modelLabel, pickerModels, selectModel, switchProvider, type ModelChoice, type ModelInfo,
} from "../core/models";

export interface ModelPickerHost {
  /** The settings the picker reads. */
  get(): ModelChoice;
  /** Persists a change to provider, model or remembered models. */
  save(patch: Partial<ModelChoice>): void;
  /** Called after the chat moved to another model, with its readable name. */
  onSwitch?(label: string): void;
}

export interface ModelPicker {
  el: HTMLElement;
  /** Re-reads the settings (they may have changed elsewhere). */
  sync(): void;
  /** Re-asks which providers have keys and drops the model lists that failed. */
  refresh(): Promise<void>;
}

export function buildModelPicker(host: ModelPickerHost): ModelPicker {
  const providerSel = h("select", { class: "mp-select mp-provider", title: "AI provider" }) as HTMLSelectElement;
  const modelSel = h("select", { class: "mp-select mp-model", title: "Model" }) as HTMLSelectElement;
  const hint = h("span", { class: "mp-hint" });
  const el = h("div", { class: "mp" }, providerSel, modelSel, hint);

  let providers: ProviderInfo[] = [];
  /** null: asked and failed (the reason is in `failures`). */
  const lists = new Map<string, ModelInfo[] | null>();
  const failures = new Map<string, string>();
  const loading = new Set<string>();
  let shownKey = "";

  async function loadModels(provider: string): Promise<ModelInfo[] | null> {
    if (lists.has(provider)) return lists.get(provider) ?? null;
    loading.add(provider);
    render();
    try {
      const list = await Bridge.chatModels(provider);
      lists.set(provider, list);
      failures.delete(provider);
      return list;
    } catch (err) {
      lists.set(provider, null);
      failures.set(provider, String(err).replace(/^Error:\s*/, ""));
      return null;
    } finally {
      loading.delete(provider);
      shownKey = "";
      render();
    }
  }

  function render() {
    const s = host.get();
    const list = lists.get(s.provider) ?? null;
    const key = [
      s.provider, s.model, providers.map((p) => `${p.id}:${p.hasKey}`).join(","),
      lists.has(s.provider) ? (list?.length ?? -1) : "?", loading.has(s.provider),
    ].join("|");
    if (key === shownKey) return;
    shownKey = key;

    clear(providerSel);
    const known = providers.length ? providers : [{ id: s.provider, name: s.provider } as ProviderInfo];
    for (const p of known) {
      const missingKey = p.keyRequired && !p.hasKey;
      providerSel.append(h("option", { value: p.id, text: missingKey ? `${p.name} (no key)` : p.name }));
    }
    providerSel.value = s.provider;

    clear(modelSel);
    const models = pickerModels(list, s.provider, s.model);
    for (const m of models) modelSel.append(h("option", { value: m.id, text: m.label }));
    if (models.length === 0) modelSel.append(h("option", { value: "", text: loading.has(s.provider) ? "Loading…" : "No models" }));
    modelSel.value = s.model;
    modelSel.disabled = loading.has(s.provider);

    const failure = failures.get(s.provider);
    // The full reason is in the tooltip; the line itself says what to do.
    const short = failure && /api key/i.test(failure) ? "Needs an API key: click to add" : failure;
    hint.textContent = loading.has(s.provider) ? "Loading models…" : short ?? "";
    hint.title = failure ?? "";
    hint.classList.toggle("err", !!failure && !loading.has(s.provider));
  }

  providerSel.addEventListener("change", async () => {
    const next = providerSel.value;
    const list = await loadModels(next);
    const patch = switchProvider(host.get(), next, list ?? [], pickerModels(null, next, ""));
    host.save(patch);
    shownKey = "";
    render();
    if (patch.model) host.onSwitch?.(describe(next, modelLabel(list, patch.model)));
  });

  modelSel.addEventListener("change", () => {
    const current = host.get();
    if (!modelSel.value || modelSel.value === current.model) return;
    host.save(selectModel(current, modelSel.value));
    shownKey = "";
    render();
    host.onSwitch?.(describe(current.provider, modelLabel(lists.get(current.provider) ?? null, modelSel.value)));
  });

  /** "gpt-5 (OpenAI)", but just "Claude Sonnet 5.5" when the name says it already. */
  function describe(provider: string, label: string): string {
    const name = providers.find((p) => p.id === provider)?.name ?? provider;
    return label.toLowerCase().includes(name.toLowerCase()) ? label : `${label} (${name})`;
  }

  async function refresh() {
    providers = (await Bridge.chatProviders()) ?? providers;
    for (const [id, list] of lists) if (list === null) lists.delete(id);
    shownKey = "";
    render();
    void loadModels(host.get().provider);
  }

  void refresh();

  return {
    el,
    sync() {
      render();
      const provider = host.get().provider;
      if (!lists.has(provider) && !loading.has(provider)) void loadModels(provider);
    },
    refresh,
  };
}
