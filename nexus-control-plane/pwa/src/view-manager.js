// ViewManager — UINavigationController-style view stack with animated transitions

export class ViewManager {
  constructor(rootEl) {
    this.root = rootEl;
    this.stack = []; // [{ name, el, hooks }]
    this.overlay = null; // { name, el, hooks, backdropEl }
  }

  get currentViewName() {
    return this.stack[this.stack.length - 1]?.name;
  }

  push(name, renderFn, { transition = "slideRight", onEnter, onExit, onCleanup } = {}) {
    const outgoing = this.stack[this.stack.length - 1];

    const el = document.createElement("div");
    el.className = "view";
    el.dataset.viewName = name;
    renderFn(el);
    this.root.appendChild(el);

    if (outgoing && transition) {
      this._animateTransition(outgoing.el, el, transition, "forward");
      if (outgoing.hooks?.onExit) outgoing.hooks.onExit();
    } else {
      el.style.position = "absolute";
    }

    this.stack.push({ name, el, hooks: { onEnter, onExit, onCleanup }, transition });
    if (onEnter) onEnter(el);
  }

  async pop() {
    if (this.stack.length <= 1) return;
    const outgoing = this.stack.pop();
    const incoming = this.stack[this.stack.length - 1];

    const popTransition = outgoing.transition || "slideRight";
    this._animateTransition(incoming.el, outgoing.el, popTransition, "back", async () => {
      if (outgoing.hooks?.onCleanup) await outgoing.hooks.onCleanup();
      outgoing.el.remove();
    });

    if (outgoing.hooks?.onExit) outgoing.hooks.onExit();
    if (incoming.hooks?.onEnter) incoming.hooks.onEnter(incoming.el);
  }

  present(name, renderFn, { onEnter, onExit, onCleanup } = {}) {
    if (this.overlay) this.dismiss();

    const backdropEl = document.createElement("div");
    backdropEl.className = "view-backdrop";
    backdropEl.addEventListener("click", () => this.dismiss());
    this.root.appendChild(backdropEl);
    requestAnimationFrame(() => backdropEl.classList.add("visible"));

    const el = document.createElement("div");
    el.className = "view-overlay";
    el.dataset.viewName = name;
    renderFn(el);
    this.root.appendChild(el);

    requestAnimationFrame(() => {
      el.classList.add("view-overlay-active");
    });

    this.overlay = { name, el, hooks: { onEnter, onExit, onCleanup }, backdropEl };
    if (onEnter) onEnter(el);
  }

  dismiss() {
    if (!this.overlay) return;
    const { el, hooks, backdropEl } = this.overlay;

    el.classList.remove("view-overlay-active");
    backdropEl.classList.remove("visible");

    const cleanup = () => {
      el.remove();
      backdropEl.remove();
      if (hooks?.onCleanup) hooks.onCleanup();
    };
    const fallback = setTimeout(cleanup, 400);
    el.addEventListener("transitionend", () => {
      clearTimeout(fallback);
      cleanup();
    }, { once: true });

    if (hooks?.onExit) hooks.onExit();
    this.overlay = null;
  }

  update(renderFn) {
    const current = this.stack[this.stack.length - 1];
    if (!current) return;
    renderFn(current.el);
  }

  get activeEl() {
    return this.stack[this.stack.length - 1]?.el ?? null;
  }

  reset() {
    this.stack = [];
    this.overlay = null;
  }

  _animateTransition(stayEl, moveEl, type, direction, onComplete) {
    const isForward = direction === "forward";
    const TRANSITION_MS = 300;
    const FALLBACK_MS = TRANSITION_MS + 50;

    // Normalize: slideLeft forward is the mirror of slideRight
    // slideLeft pushes new view from the left; slideRight pushes from the right
    // For pop(), we always use the stored transition type to reverse correctly
    const effectiveType = type === "slideLeft" ? "slideRight" : type;
    const flipDir = type === "slideLeft";

    if (effectiveType === "slideRight") {
      const offStart = flipDir ? "-100%" : "100%";
      const peekAway = flipDir ? "30%" : "-30%";
      const offEnd = flipDir ? "-100%" : "100%";

      if (isForward) {
        moveEl.style.transform = `translateX(${offStart})`;
        requestAnimationFrame(() => {
          moveEl.style.transition = `transform ${TRANSITION_MS}ms cubic-bezier(0.2, 0, 0, 1)`;
          stayEl.style.transition = `transform ${TRANSITION_MS}ms cubic-bezier(0.2, 0, 0, 1), opacity ${TRANSITION_MS}ms ease`;
          moveEl.style.transform = "translateX(0)";
          stayEl.style.transform = `translateX(${peekAway})`;
          stayEl.style.opacity = "0.5";
        });
        const cleanupStyles = () => {
          moveEl.style.transition = "";
          moveEl.style.transform = "";
          stayEl.style.display = "none";
          stayEl.style.transition = "";
          stayEl.style.transform = "";
          stayEl.style.opacity = "";
        };
        const fallback = setTimeout(cleanupStyles, FALLBACK_MS);
        moveEl.addEventListener("transitionend", () => {
          clearTimeout(fallback);
          cleanupStyles();
        }, { once: true });
      } else {
        stayEl.style.display = "";
        stayEl.style.transform = `translateX(${peekAway})`;
        stayEl.style.opacity = "0.5";
        requestAnimationFrame(() => {
          stayEl.style.transition = `transform ${TRANSITION_MS}ms cubic-bezier(0.2, 0, 0, 1), opacity ${TRANSITION_MS}ms ease`;
          moveEl.style.transition = `transform ${TRANSITION_MS}ms cubic-bezier(0.2, 0, 0, 1)`;
          stayEl.style.transform = "translateX(0)";
          stayEl.style.opacity = "1";
          moveEl.style.transform = `translateX(${offEnd})`;
        });
        const fallback = setTimeout(() => { if (onComplete) onComplete(); }, FALLBACK_MS);
        moveEl.addEventListener("transitionend", () => {
          clearTimeout(fallback);
          if (onComplete) onComplete();
        }, { once: true });
      }
    }
  }
}
