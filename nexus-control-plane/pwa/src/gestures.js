// Gesture handlers — swipe-back, pull-to-refresh

const EDGE_ZONE = 30;
const SWIPE_BACK_THRESHOLD = 80;
const PULL_THRESHOLD = 70;

/**
 * Edge swipe from left side pops the terminal view.
 * Attaches to the view container element.
 */
export function setupSwipeBack(viewEl, onBack) {
  let startX = 0, startY = 0, tracking = false;

  viewEl.addEventListener("touchstart", (e) => {
    const touch = e.touches[0];
    if (touch.clientX < EDGE_ZONE) {
      startX = touch.clientX;
      startY = touch.clientY;
      tracking = true;
    }
  }, { passive: true });

  viewEl.addEventListener("touchmove", (e) => {
    if (!tracking) return;
    const dx = e.touches[0].clientX - startX;
    const dy = Math.abs(e.touches[0].clientY - startY);
    if (dy > dx) { tracking = false; viewEl.style.transform = ""; return; }
    viewEl.style.transform = `translateX(${Math.max(0, dx)}px)`;
    viewEl.style.transition = "none";
  }, { passive: true });

  viewEl.addEventListener("touchend", (e) => {
    if (!tracking) return;
    tracking = false;
    const dx = e.changedTouches[0].clientX - startX;
    viewEl.style.transition = "transform 0.3s cubic-bezier(0.2, 0, 0, 1)";
    if (dx > SWIPE_BACK_THRESHOLD) {
      viewEl.style.transform = "translateX(100%)";
      viewEl.addEventListener("transitionend", () => onBack(), { once: true });
    } else {
      viewEl.style.transform = "translateX(0)";
    }
  }, { passive: true });
}

/**
 * Horizontal swipe on a card element.
 * Swipe right → archive indicator + callback.
 * Swipe left → action reveal + callback.
 */
export function setupCardSwipe(cardEl, { onSwipeRight, onSwipeLeft }) {
  let startX = 0, startY = 0, tracking = false, direction = null;

  const SWIPE_RIGHT_THRESHOLD = 100;
  const SWIPE_LEFT_THRESHOLD = 80;
  const MIN_DELTA = 20;

  cardEl.addEventListener("touchstart", (e) => {
    startX = e.touches[0].clientX;
    startY = e.touches[0].clientY;
    tracking = true;
    direction = null;
  }, { passive: true });

  cardEl.addEventListener("touchmove", (e) => {
    if (!tracking) return;
    const dx = e.touches[0].clientX - startX;
    const dy = Math.abs(e.touches[0].clientY - startY);

    // Lock direction after initial movement — require strong horizontal intent
    // to avoid hijacking diagonal scroll starts
    if (!direction && (Math.abs(dx) > 10 || dy > 10)) {
      direction = Math.abs(dx) > dy * 2 ? "horizontal" : "vertical";
    }
    if (direction !== "horizontal") { tracking = false; return; }

    cardEl.style.transition = "none";
    cardEl.style.transform = `translateX(${dx}px)`;

    // Show/update underlay
    const wrapper = cardEl.closest(".card-wrapper");
    if (!wrapper) return;
    let underlay = wrapper.querySelector(".card-swipe-underlay");
    if (dx > MIN_DELTA) {
      if (!underlay || !underlay.classList.contains("archive")) {
        if (underlay) underlay.remove();
        underlay = document.createElement("div");
        underlay.className = "card-swipe-underlay archive";
        underlay.textContent = "Archive";
        wrapper.insertBefore(underlay, cardEl);
      }
    } else if (dx < -MIN_DELTA) {
      if (!underlay || !underlay.classList.contains("actions")) {
        if (underlay) underlay.remove();
        // Actions underlay is created by board.js per card — just reveal via transform
      }
    } else {
      if (underlay) underlay.remove();
    }
  }, { passive: true });

  cardEl.addEventListener("touchend", (e) => {
    if (!tracking || direction !== "horizontal") {
      tracking = false;
      direction = null;
      return;
    }
    tracking = false;
    const dx = e.changedTouches[0].clientX - startX;

    // Clean up underlay
    const wrapper = cardEl.closest(".card-wrapper");
    const underlay = wrapper?.querySelector(".card-swipe-underlay");

    cardEl.style.transition = "transform 0.3s cubic-bezier(0.2, 0, 0, 1)";

    if (dx > SWIPE_RIGHT_THRESHOLD && onSwipeRight) {
      // Slide off screen right
      cardEl.style.transform = "translateX(100%)";
      cardEl.addEventListener("transitionend", () => {
        onSwipeRight();
        // Reset after action
        cardEl.style.transition = "";
        cardEl.style.transform = "";
        if (underlay) underlay.remove();
      }, { once: true });
    } else if (dx < -SWIPE_LEFT_THRESHOLD && onSwipeLeft) {
      // Snap back and open action sheet
      cardEl.style.transform = "translateX(0)";
      cardEl.addEventListener("transitionend", () => {
        cardEl.style.transition = "";
        cardEl.style.transform = "";
      }, { once: true });
      onSwipeLeft();
    } else {
      // Snap back
      cardEl.style.transform = "translateX(0)";
      cardEl.addEventListener("transitionend", () => {
        cardEl.style.transition = "";
        cardEl.style.transform = "";
        if (underlay) underlay.remove();
      }, { once: true });
    }

    direction = null;
  }, { passive: true });
}

/**
 * Pull-to-refresh on a scrollable container.
 * Safe to call multiple times — listeners are only attached once.
 * The indicator is looked up dynamically since innerHTML may destroy it.
 */
export function setupPullToRefresh(scrollContainer, onRefresh) {
  // Guard: only attach listeners once per container
  if (scrollContainer._pullToRefreshActive) return;
  scrollContainer._pullToRefreshActive = true;

  let startY = 0, pulling = false;

  function getIndicator() {
    let el = scrollContainer.querySelector(".pull-indicator");
    if (!el) {
      el = document.createElement("div");
      el.className = "pull-indicator";
      el.innerHTML = `<div class="pull-spinner"></div>`;
      scrollContainer.prepend(el);
    }
    return el;
  }

  // Create the initial indicator
  getIndicator();

  scrollContainer.addEventListener("touchstart", (e) => {
    if (scrollContainer.scrollTop === 0) {
      startY = e.touches[0].clientY;
      pulling = true;
    }
  }, { passive: true });

  scrollContainer.addEventListener("touchmove", (e) => {
    if (!pulling) return;
    const dy = e.touches[0].clientY - startY;
    if (dy > 0 && dy < 120) {
      const indicator = getIndicator();
      indicator.style.height = dy + "px";
      indicator.style.opacity = Math.min(dy / PULL_THRESHOLD, 1);
      if (dy > PULL_THRESHOLD) indicator.classList.add("pull-ready");
      else indicator.classList.remove("pull-ready");
    }
  }, { passive: true });

  scrollContainer.addEventListener("touchend", () => {
    if (!pulling) return;
    pulling = false;
    const indicator = scrollContainer.querySelector(".pull-indicator");
    if (!indicator) return;
    const ready = indicator.classList.contains("pull-ready");
    indicator.style.transition = "height 0.2s ease, opacity 0.2s ease";
    indicator.style.height = "0";
    indicator.style.opacity = "0";
    indicator.classList.remove("pull-ready");
    setTimeout(() => { indicator.style.transition = ""; }, 200);
    if (ready) onRefresh();
  }, { passive: true });
}
