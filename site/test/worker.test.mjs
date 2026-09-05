import assert from "node:assert/strict";
import test from "node:test";
import worker, { GO_TARGETS, trackedRedirect } from "../public/_worker.js";
import { applySmartCtas } from "../src/scripts/smart-cta.js";

const placements = {
  web: "hero",
  download: "final",
  android: "nav",
  weatherdesk: "homepage",
  "weatherdesk-release": "weatherdesk-hero",
  "weatherdesk-source": "weatherdesk-final",
};

test("allowlisted CTA redirects keep working and record only aggregate fields", () => {
  for (const [target, destination] of Object.entries(GO_TARGETS)) {
    const points = [];
    const placement = placements[target];
    const response = trackedRedirect(
      new Request(`https://hookecho.io/go/${target}/${placement}`),
      { CTA_ANALYTICS: { writeDataPoint: (point) => points.push(point) } },
    );

    assert.equal(response.status, 302);
    assert.equal(response.headers.get("location"), new URL(destination, "https://hookecho.io").href);
    assert.deepEqual(points, [{ blobs: [target, placement], doubles: [1] }]);
  }
});

test("invalid routes are blocked and analytics is optional", async () => {
  assert.equal(
    trackedRedirect(new Request("https://hookecho.io/go/web/unknown"), {}).status,
    404,
  );
  assert.equal(
    trackedRedirect(new Request("https://hookecho.io/go/unknown/hero"), {}).status,
    404,
  );
  assert.equal(
    trackedRedirect(new Request("https://hookecho.io/go/web/hero"), {}).status,
    302,
  );

  const points = [];
  const head = await worker.fetch(new Request("https://hookecho.io/go/web/hero", { method: "HEAD" }), {
    CTA_ANALYTICS: { writeDataPoint: (point) => points.push(point) },
  });
  assert.equal(head.status, 302);
  assert.deepEqual(points, []);
});

test("Android visitors get the app CTA while other visitors keep the web default", () => {
  const node = (placement) => ({
    attrs: new Map([["data-placement", placement], ["hidden", ""]]),
    textContent: "unchanged",
    getAttribute(name) { return this.attrs.get(name); },
    setAttribute(name, value) { this.attrs.set(name, value); },
    removeAttribute(name) { this.attrs.delete(name); },
  });
  const primary = node("hero");
  const secondary = node("hero");
  const note = node("hero");
  const root = {
    querySelectorAll(selector) {
      return selector === "[data-smart-primary]"
        ? [primary]
        : selector === "[data-smart-secondary]" ? [secondary] : [note];
    },
  };

  applySmartCtas("Mozilla/5.0 (Linux; Android 16)", root);
  assert.equal(primary.attrs.get("href"), "/go/android/hero");
  assert.equal(primary.textContent, "Get Android app");
  assert.equal(secondary.attrs.get("href"), "/go/web/hero");
  assert.equal(secondary.textContent, "Open in browser");
  assert.equal(note.attrs.has("hidden"), false);

  const unchanged = node("final");
  applySmartCtas("Mozilla/5.0 (iPhone)", {
    querySelectorAll: () => [unchanged],
  });
  assert.equal(unchanged.textContent, "unchanged");
});
