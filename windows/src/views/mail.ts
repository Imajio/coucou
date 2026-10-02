// Mail card: send the dropped file by email (MailView on macOS). Nothing leaves
// before the Send click; Rust picks Resend, the default mail app or mailto.

import { h } from "./dom";
import { Bridge } from "../core/bridge";
import { Sound } from "../core/sound";
import { State } from "../core/state";
import type { ViewActions, ViewHost } from "./views";

export function buildMail(actions: ViewActions): ViewHost {
  const heading = h("div", { class: "mail-heading" });
  const to = h("input", { class: "mail-input", type: "email", placeholder: "address@example.com", spellcheck: "false" }) as HTMLInputElement;
  const subject = h("input", { class: "mail-input", type: "text", spellcheck: "false" }) as HTMLInputElement;
  const body = h("textarea", { class: "mail-body", placeholder: "Message", spellcheck: "false" }) as HTMLTextAreaElement;
  const status = h("div", { class: "mail-status" });
  const send = h("button", { class: "btn primary", text: "Send" }) as HTMLButtonElement;
  const cancel = h("button", { class: "btn secondary", text: "Cancel", onclick: () => actions.setView("choose") });

  const el = h(
    "div",
    { class: "view" },
    h("div", { class: "card" },
      h("div", { class: "mail-stack" },
        heading,
        h("div", { class: "mail-row" }, h("label", { text: "To" }), to),
        h("div", { class: "mail-row" }, h("label", { text: "Subject" }), subject),
        body,
        h("div", { class: "actions" }, send, cancel, status),
      ),
    ),
  );

  let sending = false;
  let shownFor: string | null = null;

  function setStatus(text: string, kind: "" | "err" | "ok" = "") {
    status.textContent = text;
    status.className = kind ? `mail-status ${kind}` : "mail-status";
  }

  async function submit() {
    if (sending) return;
    const file = State.droppedFile;
    if (!to.value.trim()) {
      setStatus("Missing recipient.", "err");
      return;
    }
    sending = true;
    send.disabled = true;
    send.textContent = "Sending…";
    setStatus("");
    try {
      const how = await Bridge.mailSend(to.value, subject.value, body.value, file?.path ?? null);
      Sound.play("approve");
      if (how === "sent") {
        State.noteMessage = `Sent to ${to.value.trim()}.`;
      } else if (how === "compose") {
        State.noteMessage = "Your mail app has the message, file attached.";
      } else if (how === "cancelled") {
        State.noteMessage = "Message closed without sending.";
      } else {
        State.noteMessage = `Attach ${file?.name ?? "the file"} in your mail app: its folder is open.`;
      }
      actions.setView("note");
      window.setTimeout(() => {
        if (State.view === "note") actions.setView(State.defaultView());
      }, 2600);
    } catch (err) {
      setStatus(String(err).replace(/^Error:\s*/, ""), "err");
      Sound.play("error");
    } finally {
      sending = false;
      send.disabled = false;
      send.textContent = "Send";
    }
  }

  send.addEventListener("click", () => void submit());
  for (const field of [to, subject]) {
    field.addEventListener("keydown", (e) => {
      if (e.key === "Enter") {
        e.preventDefault();
        void submit();
      }
    });
  }

  return {
    el,
    sync() {
      const name = State.droppedFile?.name ?? "";
      // A new file: fresh fields, the file's name as the subject.
      if (name !== shownFor) {
        shownFor = name;
        heading.textContent = name ? `New email with ${name}` : "New email";
        subject.value = name;
        subject.placeholder = name || "Subject";
        body.value = "";
        setStatus("");
      }
    },
    focus() {
      to.focus();
    },
  };
}
