// The agent session commands (agent/mod.rs). Errors come back as the text
// the user should read.

import { invoke } from "@tauri-apps/api/core";
import { IS_TAURI } from "./bridge";
import type {
  ApprovalRequest, Decision, NewSession, RoleInfo, SessionDefaults, SessionMode, SessionSummary, SessionView,
} from "./agents";

async function run<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (!IS_TAURI) throw new Error("not running inside Coucou");
  return invoke<T>(cmd, args);
}

export const Agents = {
  roles: () => run<RoleInfo[]>("agent_roles"),
  list: () => run<SessionSummary[]>("agent_list"),
  get: (id: string) => run<SessionView>("agent_get", { id }),
  approvals: () => run<ApprovalRequest[]>("agent_approvals"),
  defaults: () => run<SessionDefaults>("agent_defaults"),
  create: (spec: NewSession) => run<string>("agent_create", { spec }),
  send: (id: string, text: string) => run<void>("agent_send", { id, text }),
  stop: (id: string) => run<void>("agent_stop", { id }),
  remove: (id: string) => run<void>("agent_delete", { id }),
  update: (id: string, patch: { name?: string; provider?: string; model?: string; mode?: SessionMode }) =>
    run<void>("agent_update", { id, patch }),
  decide: (requestId: string, decision: Decision) => run<void>("agent_decide", { requestId, decision }),
  checkFolder: (folder: string) => run<string>("agent_check_folder", { folder }),
  /** Shows the sessions window, on one session or on the new-session form. */
  openWindow: (id: string | null) => run<void>("open_sessions_window", { id }),
};
