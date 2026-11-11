// Captured frames are bounded before leaving Rust. GIF encoding uses a separate worker;
// browser-native video recording yields between frames and both jobs can be cancelled.
function validateLoopFrames(frames, width, height) {
  if (!frames?.length || !Number.isInteger(width) || !Number.isInteger(height) || width < 1 || height < 1 || Math.max(width, height) > 960) {
    throw new Error("Invalid export dimensions");
  }
  const bytes = width * height * 4;
  if (bytes * frames.length > 64 * 1024 * 1024) throw new Error("Loop exceeds the export memory limit; select fewer scans");
  if (frames.some(frame => !(frame instanceof Uint8Array) || frame.length !== bytes)) {
    throw new Error("Frame size changed during export");
  }
}

function showLoopDownload(blob, ext, count) {
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
  title.textContent = `${ext.toUpperCase()} ready · ${count} frames`;
  title.style.cssText = "font-weight:700;margin-bottom:8px";
  const save = document.createElement("a");
  save.textContent = `Save ${ext.toUpperCase()}`;
  save.href = panel.videoUrl;
  save.download = `hookecho-loop.${ext}`;
  save.style.cssText = "display:inline-block;background:#2872c3;color:white;text-decoration:none;border:1px solid #8abcf0;border-radius:6px;padding:7px 12px;cursor:pointer;font:inherit";
  const status = document.createElement("div");
  status.style.cssText = "font-size:12px;margin-top:8px;color:#cadcf1";
  save.addEventListener("click", () => {
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
  return `${ext.toUpperCase()} ready to save (${count} frames)`;
}

globalThis.__hookechoExportGif = (frames, width, height, delayMs, onprogress) => {
  let worker;
  let finish;
  const count = frames.length;
  const promise = new Promise((resolve, reject) => {
    let settled = false;
    finish = (error, output) => {
      if (settled) return;
      settled = true;
      if (worker) {
        worker.onmessage = null;
        worker.onerror = null;
        worker.terminate();
      }
      frames.length = 0;
      if (error) reject(error);
      else {
        try { resolve(showLoopDownload(new Blob([output], { type: "image/gif" }), "gif", count)); }
        catch (error) { reject(error); }
      }
    };
    try {
      validateLoopFrames(frames, width, height);
      if (!globalThis.__hookechoCreateExportWorker) throw new Error("GIF export worker is unavailable; reload this page");
      worker = globalThis.__hookechoCreateExportWorker();
      worker.onerror = event => finish(new Error(event.message || "GIF encoder failed"));
      worker.onmessage = ({ data }) => {
        if (settled) return;
        if (data.progress !== undefined) onprogress(data.progress);
        else if (data.err) finish(new Error(data.err));
        else if (data.ok) finish(null, data.ok);
      };
      worker.postMessage({ id: 1, op: "gif", frames, width, height, delayMs }, frames.map(frame => frame.buffer));
    } catch (error) { finish(error); }
  });
  promise.cancel = () => finish(new Error("Loop export cancelled"));
  return promise;
};

globalThis.__hookechoExportVideo = (frames, width, height, fps, onprogress) => {
  let cancelled = false;
  let wake;
  let cancelRecording;
  const wait = milliseconds => new Promise(resolve => {
    const timer = setTimeout(() => { wake = null; resolve(); }, milliseconds);
    wake = () => { clearTimeout(timer); wake = null; resolve(); };
  });
  const run = async () => {
    validateLoopFrames(frames, width, height);
    if (!globalThis.MediaRecorder || !HTMLCanvasElement.prototype.captureStream) {
      throw new Error("Video recording is not available in this browser");
    }
    const formats = [
      ["video/mp4;codecs=avc1.42E01E", "mp4"], ["video/mp4", "mp4"],
      ["video/webm;codecs=vp9", "webm"], ["video/webm;codecs=vp8", "webm"], ["video/webm", "webm"],
    ];
    const selected = formats.find(([mime]) => MediaRecorder.isTypeSupported(mime));
    if (!selected) throw new Error("This browser has no supported video encoder");
    const canvas = document.createElement("canvas");
    canvas.width = width + (width & 1);
    canvas.height = height + (height & 1);
    const ctx = canvas.getContext("2d", { alpha: false, willReadFrequently: false });
    if (!ctx) throw new Error("Cannot create a video canvas");
    const stream = canvas.captureStream(0);
    const track = stream.getVideoTracks()[0];
    const chunks = [];
    const count = frames.length;
    let recorder;
    let encoderError;
    let finishTimeout;
    const check = () => {
      if (cancelled) throw new Error("Loop export cancelled");
      if (encoderError) throw encoderError;
    };
    try {
      if (!track?.requestFrame) throw new Error("Frame-controlled video capture is unavailable in this browser");
      recorder = new MediaRecorder(stream, { mimeType: selected[0], videoBitsPerSecond: 4_000_000 });
      const finished = new Promise((resolve, reject) => {
        cancelRecording = () => reject(new Error("Loop export cancelled"));
        recorder.ondataavailable = event => { if (event.data.size) chunks.push(event.data); };
        recorder.onerror = event => {
          encoderError = event.error || new Error("Video encoder failed");
          reject(encoderError);
          wake?.();
        };
        recorder.onstop = resolve;
      });
      finished.catch(() => {});
      recorder.start();
      const delay = Math.round(1000 / Math.max(1, Math.min(15, fps)));
      for (let index = 0; index < count; index++) {
        check();
        const frame = frames[index];
        ctx.putImageData(new ImageData(new Uint8ClampedArray(frame.buffer, frame.byteOffset, frame.byteLength), width, height), 0, 0);
        frames[index] = null;
        track.requestFrame();
        onprogress(index + 1);
        check();
        await wait(delay);
      }
      check();
      // Repeat the last image with a corner-pixel change to retain its final playback interval.
      ctx.fillStyle = "#071421";
      ctx.fillRect(canvas.width - 1, canvas.height - 1, 1, 1);
      track.requestFrame();
      await wait(Math.min(delay, 80));
      check();
      recorder.stop();
      const timeout = new Promise((resolve, reject) => {
        finishTimeout = setTimeout(() => reject(new Error("Video recorder did not finish; try again")), 10_000);
      });
      await Promise.race([finished, timeout]);
      check();
      const mime = recorder.mimeType || selected[0];
      const ext = mime.startsWith("video/mp4") ? "mp4" : "webm";
      const blob = new Blob(chunks, { type: mime });
      if (blob.size < 100) throw new Error("Video encoder returned an empty file");
      return showLoopDownload(blob, ext, count);
    } finally {
      clearTimeout(finishTimeout);
      cancelRecording = null;
      try { if (recorder?.state === "recording") recorder.stop(); } catch {}
      stream.getTracks().forEach(track => track.stop());
      frames.length = 0;
      chunks.length = 0;
    }
  };
  const promise = run();
  promise.cancel = () => { cancelled = true; cancelRecording?.(); wake?.(); };
  return promise;
};
