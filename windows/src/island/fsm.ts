// Island open/close FSM — port of IslandStateMachine.swift.
// No DOM, no Tauri: it only reports transitions.

export type FsmState = "hidden" | "petit" | "home" | "coucou";

export class IslandStateMachine {
  state: FsmState = "hidden";

  onTransition: ((from: FsmState, to: FsmState) => void) | null = null;

  /** home → petit delay, seconds. */
  homeToPetitDelay = 15;
  /** petit → hidden delay, seconds. Only for an island that was shown for a reason. */
  petitToHiddenDelay = 60;
  /** petit → hidden delay once the cursor has left it, seconds. */
  peekLingerDelay = 3;
  /**
   * Keep the compact island on screen all the time. Off, it stays fully hidden
   * and only comes out when the cursor rests on the top edge, or for an alert.
   */
  alwaysVisible = false;
  /** How long the cursor must rest on the top edge before a hidden island comes out, seconds. */
  hoverRevealDelay = 1;
  /** coucou → petit once the greeting animation ends (no hover). */
  greetAutoCollapseDelay = 0.6;
  /** coucou → petit while the mouse hovers the greeting. */
  greetHoverCollapseDelay = 10;
  /** An alert waiting for an answer stays open, even when the mouse leaves. */
  pinned = false;

  private petitHide: number | null = null;
  private homeCollapse: number | null = null;
  private greetCollapse: number | null = null;
  private hoverReveal: number | null = null;
  private launched = false;

  // ── Inputs ──────────────────────────────────────────────────────────────────

  launch() {
    this.launched = true;
    this.cancelTimers();
    this.transition("coucou");
  }

  mouseEntered() {
    switch (this.state) {
      case "hidden":
        this.cancelTimers();
        if (this.hoverRevealDelay <= 0) this.transition("petit");
        else this.scheduleHoverReveal();
        break;
      case "petit":
        this.clear("petitHide");
        break;
      case "home":
        this.clear("homeCollapse");
        break;
      case "coucou":
        this.scheduleGreetCollapse(this.greetHoverCollapseDelay);
        break;
    }
  }

  mouseLeft() {
    switch (this.state) {
      case "hidden":
        // Left before the delay was up: the island stays hidden.
        this.clear("hoverReveal");
        break;
      case "petit":
        this.schedulePetitHide();
        break;
      case "home":
        this.scheduleHomeCollapse();
        break;
      case "coucou":
        this.clear("greetCollapse");
        this.transition("petit");
        break;
    }
  }

  click() {
    if (this.state !== "petit") return;
    this.cancelTimers();
    this.transition("home");
  }

  /** Greeting animation finished (T.end). Doesn't override a running hover timer. */
  greetComplete() {
    if (this.state !== "coucou") return;
    if (this.greetCollapse == null) this.scheduleGreetCollapse(this.greetAutoCollapseDelay);
  }

  /**
   * Non-alert work event: show compact from hidden. Without "always visible" the
   * island is meant to stay out of sight, so only `urgent` events (something a
   * human has to act on) get through.
   */
  reveal(urgent = false) {
    if (this.state !== "hidden") return;
    if (!this.alwaysVisible && !urgent) return;
    this.cancelTimers();
    this.transition("petit");
    this.schedulePetitHide(urgent ? this.petitToHiddenDelay : this.peekLingerDelay);
  }

  /**
   * "Always visible" was switched while the app runs. `hovering`: the cursor is on
   * the island. `paused`: a paused island must not pop up because of a setting.
   */
  setAlwaysVisible(on: boolean, hovering: boolean, paused: boolean) {
    if (on === this.alwaysVisible) return;
    this.alwaysVisible = on;
    // Before launch the greeting owns the island; it ends in petit either way.
    if (!this.launched) return;
    if (on) {
      this.clear("petitHide");
      this.clear("hoverReveal");
      if (this.state === "hidden" && !paused) this.transition("petit");
    } else if (this.state === "petit" && !hovering) {
      this.schedulePetitHide();
    }
  }

  /** Alert or explicit request: open straight to expanded. */
  forceHome() {
    this.cancelTimers();
    this.transition("home");
  }

  /// Explicit close (OK button, Escape, an alert being answered).
  forcePetit() {
    this.cancelTimers();
    this.transition("petit");
  }

  forceHidden() {
    this.cancelTimers();
    this.transition("hidden");
  }

  // ── Timers ──────────────────────────────────────────────────────────────────

  private schedulePetitHide(delay = this.peekLingerDelay) {
    this.clear("petitHide");
    if (this.alwaysVisible) return;
    this.petitHide = window.setTimeout(() => {
      this.petitHide = null;
      if (this.state === "petit") this.transition("hidden");
    }, delay * 1000);
  }

  private scheduleHoverReveal() {
    this.clear("hoverReveal");
    this.hoverReveal = window.setTimeout(() => {
      this.hoverReveal = null;
      if (this.state !== "hidden") return;
      this.transition("petit");
    }, this.hoverRevealDelay * 1000);
  }

  private scheduleHomeCollapse() {
    this.clear("homeCollapse");
    if (this.pinned) return;
    this.homeCollapse = window.setTimeout(() => {
      this.homeCollapse = null;
      if (this.state === "home") this.transition("petit");
    }, this.homeToPetitDelay * 1000);
  }

  private scheduleGreetCollapse(delay: number) {
    this.clear("greetCollapse");
    this.greetCollapse = window.setTimeout(() => {
      this.greetCollapse = null;
      if (this.state === "coucou") this.transition("petit");
    }, delay * 1000);
  }

  private clear(which: "petitHide" | "homeCollapse" | "greetCollapse" | "hoverReveal") {
    const id = this[which];
    if (id != null) window.clearTimeout(id);
    this[which] = null;
  }

  cancelTimers() {
    this.clear("petitHide");
    this.clear("homeCollapse");
    this.clear("greetCollapse");
    this.clear("hoverReveal");
  }

  private transition(next: FsmState) {
    if (next === this.state) return;
    const from = this.state;
    this.state = next;
    this.onTransition?.(from, next);
  }
}
