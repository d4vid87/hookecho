export function applySmartCtas(userAgent, root = document) {
  if (!/android/i.test(userAgent || "")) return;

  root.querySelectorAll("[data-smart-primary]").forEach((link) => {
    const placement = link.getAttribute("data-placement") || "nav";
    link.setAttribute("href", `/go/android/${placement}`);
    link.textContent = "Get Android app";
  });
  root.querySelectorAll("[data-smart-secondary]").forEach((link) => {
    const placement = link.getAttribute("data-placement") || "hero";
    link.setAttribute("href", `/go/web/${placement}`);
    link.textContent = "Open in browser";
  });
  root.querySelectorAll("[data-android-note]").forEach((note) => note.removeAttribute("hidden"));
}
