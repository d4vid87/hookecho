/*
  The site's whole motion system: scroll reveals, count-ups, and pointer tilt.

  IntersectionObserver drives the reveals rather than CSS scroll-driven animation — Safari still
  needs the JS path, so a second parallel system would only be dead weight. The CSS in global.css
  holds the reduced-motion backstop; the one matchMedia check below is the JS half.

  ponytail: hand-rolled, ~100 lines, no animation library.
*/
const still = matchMedia("(prefers-reduced-motion: reduce)").matches;

function initReveals(): void {
  const targets = document.querySelectorAll<HTMLElement>("[data-reveal]");
  if (!targets.length) return;
  if (still || !("IntersectionObserver" in window)) {
    targets.forEach((el) => el.classList.add("is-in"));
    return;
  }
  const io = new IntersectionObserver(
    (entries) => {
      for (const entry of entries) {
        if (!entry.isIntersecting) continue;
        const el = entry.target as HTMLElement;
        const delay = Number(el.dataset.revealDelay ?? 0);
        if (delay) el.style.transitionDelay = `${delay}ms`;
        el.classList.add("is-in");
        io.unobserve(el);
      }
    },
    { rootMargin: "0px 0px -10% 0px", threshold: 0.1 },
  );
  targets.forEach((el) => io.observe(el));
}

/** Formats to the same shape the markup shipped with, so "1,024" stays "1,024" mid-count. */
function format(value: number, template: string): string {
  return template.includes(",") ? value.toLocaleString("en-US") : String(value);
}

function countUp(el: HTMLElement): void {
  const text = el.textContent ?? "";
  const target = Number(el.dataset.countup ?? text.replace(/[^0-9.]/g, ""));
  if (!Number.isFinite(target)) return;
  const start = performance.now();
  const step = (now: number) => {
    const t = Math.min(1, (now - start) / 900);
    // Same ease-out curve as --ease-out, so JS and CSS motion feel like one thing.
    const eased = 1 - Math.pow(1 - t, 3);
    el.textContent = format(Math.round(target * eased), text);
    if (t < 1) requestAnimationFrame(step);
  };
  requestAnimationFrame(step);
}

let counter: IntersectionObserver | null = null;

/**
 * Counts an element up the next time it is on screen. Exported because the live numbers (warning
 * counts, GitHub stats) arrive from fetches that finish long after this module ran.
 */
export function watchCount(el: HTMLElement): void {
  if (still || !("IntersectionObserver" in window)) return; // the final value is already rendered
  counter ??= new IntersectionObserver(
    (entries) => {
      for (const entry of entries) {
        if (!entry.isIntersecting) continue;
        countUp(entry.target as HTMLElement);
        counter!.unobserve(entry.target);
      }
    },
    { threshold: 0.4 },
  );
  counter.observe(el);
}

function initCountups(): void {
  document.querySelectorAll<HTMLElement>("[data-countup]").forEach(watchCount);
}

/*
  Tilt plus a pointer-following gradient: --mx/--my are published as percentages so any element
  can use them in a radial-gradient without more JS.
*/
function initTilt(): void {
  if (still || matchMedia("(hover: none)").matches) return;
  for (const el of document.querySelectorAll<HTMLElement>("[data-tilt]")) {
    el.addEventListener("pointermove", (event) => {
      const rect = el.getBoundingClientRect();
      const x = (event.clientX - rect.left) / rect.width;
      const y = (event.clientY - rect.top) / rect.height;
      el.style.setProperty("--mx", `${(x * 100).toFixed(1)}%`);
      el.style.setProperty("--my", `${(y * 100).toFixed(1)}%`);
      el.style.transform = `perspective(800px) rotateY(${((x - 0.5) * 8).toFixed(2)}deg) rotateX(${((0.5 - y) * 8).toFixed(2)}deg)`;
    });
    el.addEventListener("pointerleave", () => {
      el.style.transform = "";
    });
    el.addEventListener("pointerdown", () => {
      el.style.scale = "0.98";
    });
    for (const done of ["pointerup", "pointerleave", "pointercancel"]) {
      el.addEventListener(done, () => {
        el.style.scale = "";
      });
    }
  }
}

initReveals();
initCountups();
initTilt();
