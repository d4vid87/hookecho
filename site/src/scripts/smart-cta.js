export function applySmartCtas(userAgent, root = document) {
  if (!/android/i.test(userAgent || "")) return;

  root.querySelectorAll("[data-android-note]").forEach((note) => note.removeAttribute("hidden"));
}
