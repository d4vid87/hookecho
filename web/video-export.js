// Encode the exact annotated radar frames captured by the app, rather than recording the
// live canvas while its timeline is being fetched. MediaRecorder chooses the container the
// current browser can actually write; the extension always follows the resulting MIME type.
// Browsers may reject a programmatic download after asynchronous recording. Offer the finished
// blob through a real button so the download begins with a fresh user gesture.
globalThis.__hookechoExportVideo = async (frames, width, height, fps) => {
  if (!frames?.length || !width || !height) throw new Error("No video frames were captured");
  if (!globalThis.MediaRecorder || !HTMLCanvasElement.prototype.captureStream) {
    throw new Error("Video recording is not available in this browser");
  }
  const formats = [
    ["video/mp4;codecs=avc1.42E01E", "mp4"],
    ["video/mp4", "mp4"],
    ["video/webm;codecs=vp9", "webm"],
    ["video/webm;codecs=vp8", "webm"],
    ["video/webm", "webm"],
  ];
  const selected = formats.find(([mime]) => MediaRecorder.isTypeSupported(mime));
  if (!selected) throw new Error("This browser has no supported video encoder");

  const canvas = document.createElement("canvas");
  // H.264 encoders often require even dimensions. The original frame stays aligned at 0,0.
  canvas.width = width + (width & 1);
  canvas.height = height + (height & 1);
  const ctx = canvas.getContext("2d", { alpha: false, willReadFrequently: false });
  if (!ctx) throw new Error("Cannot create a video canvas");
  const stream = canvas.captureStream(0);
  const track = stream.getVideoTracks()[0];
  if (!track?.requestFrame) {
    stream.getTracks().forEach((t) => t.stop());
    throw new Error("Frame-controlled video capture is unavailable in this browser");
  }
  const chunks = [];
  let recorder;
  try {
    recorder = new MediaRecorder(stream, {
      mimeType: selected[0],
      videoBitsPerSecond: 4_000_000,
    });
    const finished = new Promise((resolve, reject) => {
      recorder.ondataavailable = (event) => { if (event.data.size) chunks.push(event.data); };
      recorder.onerror = (event) => reject(event.error || new Error("Video encoder failed"));
      recorder.onstop = resolve;
    });
    recorder.start();
    const delay = Math.round(1000 / Math.max(1, Math.min(15, fps)));
    for (const frame of frames) {
      if (frame.length !== width * height * 4) throw new Error("Video frame size changed during export");
      ctx.putImageData(new ImageData(new Uint8ClampedArray(frame), width, height), 0, 0);
      track.requestFrame();
      await new Promise((resolve) => setTimeout(resolve, delay));
    }
    // Container duration is measured to the last encoded timestamp, not to the time we
    // stop recording. Repeat the final image so its intended final interval is retained.
    // Some recorders coalesce identical frames. Touch one corner pixel to force a final
    // encoded timestamp while keeping the visible radar picture unchanged in practice.
    ctx.fillStyle = "#071421";
    ctx.fillRect(canvas.width - 1, canvas.height - 1, 1, 1);
    track.requestFrame();
    await new Promise((resolve) => setTimeout(resolve, Math.min(delay, 80)));
    recorder.stop();
    await finished;
    const mime = recorder.mimeType || selected[0];
    const ext = mime.startsWith("video/mp4") ? "mp4" : "webm";
    const blob = new Blob(chunks, { type: mime });
    if (blob.size < 100) throw new Error("Video encoder returned an empty file");
    const prior = document.getElementById("hookecho-video-save");
    if (prior) {
      if (prior.videoUrl) URL.revokeObjectURL(prior.videoUrl);
      prior.remove();
    }
    const panel = document.createElement("div");
    panel.id = "hookecho-video-save";
    panel.videoUrl = URL.createObjectURL(blob);
    panel.style.cssText = "position:fixed;right:16px;bottom:16px;z-index:9999;padding:12px 14px;max-width:300px;background:#102840;color:#f2f7ff;border:1px solid #6ba7d8;border-radius:10px;box-shadow:0 8px 28px #0009;font:14px system-ui,sans-serif";
    const title = document.createElement("div");
    title.textContent = `${ext.toUpperCase()} video ready · ${frames.length} frames`;
    title.style.cssText = "font-weight:700;margin-bottom:8px";
    const save = document.createElement("button");
    save.textContent = `Save ${ext.toUpperCase()}`;
    save.style.cssText = "background:#2872c3;color:white;border:1px solid #8abcf0;border-radius:6px;padding:7px 12px;cursor:pointer;font:inherit";
    const status = document.createElement("div");
    status.style.cssText = "font-size:12px;margin-top:8px;color:#cadcf1";
    save.addEventListener("click", () => {
      const link = document.createElement("a");
      link.href = panel.videoUrl;
      link.download = `hookecho-loop.${ext}`;
      link.click();
      status.textContent = "Download requested. You can click Save again if needed.";
    });
    const close = document.createElement("button");
    close.textContent = "Close";
    close.style.cssText = "margin-left:8px;background:transparent;color:#cadcf1;border:0;cursor:pointer;font:inherit";
    close.addEventListener("click", () => {
      URL.revokeObjectURL(panel.videoUrl);
      panel.remove();
    });
    panel.append(title, save, close, status);
    document.body.append(panel);
    return `${ext.toUpperCase()} video ready (${frames.length} frames)`;
  } finally {
    if (recorder?.state === "recording") recorder.stop();
    stream.getTracks().forEach((t) => t.stop());
  }
};
