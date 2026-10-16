// Encode the exact annotated radar frames captured by the app, rather than recording the
// live canvas while its timeline is being fetched. MediaRecorder chooses the container the
// current browser can actually write; the extension always follows the resulting MIME type.
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
    const url = URL.createObjectURL(blob);
    const link = document.createElement("a");
    link.href = url;
    link.download = `hookecho-loop.${ext}`;
    link.click();
    setTimeout(() => URL.revokeObjectURL(url), 60_000);
    return `${ext.toUpperCase()} video saved (${frames.length} frames)`;
  } finally {
    if (recorder?.state === "recording") recorder.stop();
    stream.getTracks().forEach((t) => t.stop());
  }
};
