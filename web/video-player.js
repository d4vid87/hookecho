// HLS is loaded only when someone opens a live camera. The radar startup path stays unchanged.
// The pinned Apache-2.0 hls.js distribution and its license live in /vendor/.
let library;
let closeActive;

function loadHls() {
  if (globalThis.Hls) return Promise.resolve(globalThis.Hls);
  if (!library) {
    library = new Promise((resolve, reject) => {
      const script = document.createElement("script");
      script.src = "/vendor/hls-1.7.3.min.js";
      script.onload = () => globalThis.Hls ? resolve(globalThis.Hls) : reject(new Error("HLS player did not initialize"));
      script.onerror = () => reject(new Error("Could not load HLS player"));
      document.head.append(script);
    }).catch((error) => {
      library = undefined;
      throw error;
    });
  }
  return library;
}

function make(tag, className, text) {
  const element = document.createElement(tag);
  element.className = className;
  if (text !== undefined) element.textContent = text;
  return element;
}

function ensureStyle() {
  if (document.getElementById("hookecho-video-style")) return;
  const style = document.createElement("style");
  style.id = "hookecho-video-style";
  style.textContent = `
    .hookecho-video-shade { position: fixed; inset: 0; z-index: 30; display: grid;
      place-items: center; padding: 16px; background: #030b16c9; }
    .hookecho-video-card { width: min(720px, 100%); max-height: min(90vh, 820px);
      overflow: auto; box-sizing: border-box; padding: 16px; border: 1px solid #37668b;
      border-radius: 14px; background: #10283d; color: #eaf5ff;
      box-shadow: 0 20px 60px #000b; font: 14px system-ui, sans-serif; }
    .hookecho-video-head { display: flex; align-items: center; justify-content: space-between;
      gap: 12px; margin-bottom: 12px; }
    .hookecho-video-head strong { font-size: 17px; }
    .hookecho-video-card button, .hookecho-video-card a { min-height: 40px; padding: 8px 12px;
      box-sizing: border-box; border: 1px solid #4b82ab; border-radius: 8px;
      background: #19466a; color: #eaf5ff; font: inherit; text-decoration: none; cursor: pointer; }
    .hookecho-video-card button:hover, .hookecho-video-card a:hover { background: #21608f; }
    .hookecho-video-card video { display: block; width: 100%; max-height: 60vh;
      aspect-ratio: 4 / 3; background: #050b12; border-radius: 8px; }
    .hookecho-video-status { margin: 10px 0; min-height: 20px; color: #bdd4e5; }
    .hookecho-video-foot { display: flex; align-items: center; gap: 12px; flex-wrap: wrap; }
    .hookecho-video-foot span { color: #bdd4e5; font-size: 12px; }
  `;
  document.head.append(style);
}

async function showHls(title, url) {
  closeActive?.();
  ensureStyle();
  const shade = make("div", "hookecho-video-shade");
  shade.setAttribute("role", "dialog");
  shade.setAttribute("aria-modal", "true");
  shade.setAttribute("aria-label", `${title} live camera`);
  const card = make("div", "hookecho-video-card");
  const head = make("div", "hookecho-video-head");
  head.append(make("strong", "", `${title} · Live`));
  const closeButton = make("button", "", "Close ×");
  closeButton.type = "button";
  head.append(closeButton);
  const video = make("video", "");
  video.controls = true;
  video.autoplay = true;
  video.muted = true;
  video.playsInline = true;
  const status = make("p", "hookecho-video-status", "Connecting to camera…");
  status.setAttribute("role", "status");
  const foot = make("div", "hookecho-video-foot");
  const source = make("a", "", "Open source stream");
  source.href = url;
  source.target = "_blank";
  source.rel = "noopener noreferrer";
  foot.append(source, make("span", "", "Public fixed camera · video availability depends on the provider"));
  card.append(head, video, status, foot);
  shade.append(card);
  document.body.append(shade);
  let hls;
  let closed = false;
  const onKey = (event) => { if (event.key === "Escape") close(); };
  function close() {
    if (closed) return;
    closed = true;
    hls?.destroy();
    video.pause();
    video.removeAttribute("src");
    video.load();
    shade.remove();
    document.removeEventListener("keydown", onKey, true);
    if (closeActive === close) closeActive = undefined;
  }
  closeActive = close;
  closeButton.addEventListener("click", close);
  shade.addEventListener("click", (event) => { if (event.target === shade) close(); });
  document.addEventListener("keydown", onKey, true);
  closeButton.focus();

  try {
    const Hls = await loadHls();
    if (closed) return;
    if (Hls.isSupported()) {
      hls = new Hls({ enableWorker: true, lowLatencyMode: true, backBufferLength: 30 });
      hls.on(Hls.Events.MANIFEST_PARSED, () => {
        status.textContent = "Live stream ready";
        video.play().catch(() => { status.textContent = "Ready — press Play to start video"; });
      });
      hls.on(Hls.Events.ERROR, (_event, data) => {
        if (data.fatal) status.textContent = `Stream unavailable: ${data.details || data.type}`;
      });
      hls.loadSource(url);
      hls.attachMedia(video);
    } else if (video.canPlayType("application/vnd.apple.mpegurl")) {
      video.src = url;
      video.play().catch(() => { status.textContent = "Ready — press Play to start video"; });
    } else {
      status.textContent = "This browser cannot play HLS video. Open the source stream in a compatible player.";
    }
  } catch (error) {
    if (!closed) status.textContent = `Stream unavailable: ${error.message}`;
  }
}

globalThis.__hookechoPlayHls = (title, url) => {
  void showHls(title, url);
  return true;
};
