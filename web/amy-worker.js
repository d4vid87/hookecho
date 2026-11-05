let session;
self.onmessage = async ({ data }) => {
  try {
    if (data.type === "prepare") {
      const { TtsSession } = await import("/voice/runtime/piper-tts-web.js?module=1");
      session = await TtsSession.create({
        voiceId: "en_US-amy-medium",
        wasmPaths: {
          onnxWasm: "/voice/runtime/",
          piperData: "/voice/runtime/piper_phonemize.data",
          piperWasm: "/voice/runtime/piper_phonemize.wasm",
        },
      });
      self.postMessage({ type: "ready" });
    } else if (data.type === "speak") {
      if (!session) throw new Error("Voice is not ready");
      const wav = await session.predict(data.text);
      const bytes = await wav.arrayBuffer();
      self.postMessage({ type: "audio", id: data.id, bytes, volume: data.volume }, [bytes]);
    }
  } catch (error) {
    self.postMessage({ type: "error", phase: data.type, id: data.id, message: String(error?.message || error) });
  }
};
