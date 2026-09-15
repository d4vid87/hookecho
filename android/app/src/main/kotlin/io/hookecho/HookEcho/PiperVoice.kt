package io.hookecho.HookEcho

import ai.onnxruntime.OnnxTensor
import ai.onnxruntime.OrtEnvironment
import android.content.Context
import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioTrack
import org.json.JSONObject
import java.io.File
import java.nio.FloatBuffer
import java.nio.LongBuffer
import java.util.concurrent.atomic.AtomicReference

/** Offline en_US-amy-medium synthesis shared by foreground and background alerts. */
object PiperVoice {
    private val playing = AtomicReference<AudioTrack?>()
    private val environment by lazy { OrtEnvironment.getEnvironment() }
    init { System.loadLibrary("hookecho_phonemize") }
    @JvmStatic private external fun nativePhonemes(text: String, dataPath: String): String
    @JvmStatic fun stop() = playing.getAndSet(null)?.runCatching { pause(); flush(); release() }

    @JvmStatic fun speakBlocking(context: Context, text: String): Boolean = runCatching {
        val dir = File(context.filesDir, "piper-amy")
        installAssets(context, dir)
        val config = JSONObject(File(dir, "en_US-amy-medium.onnx.json").readText())
        val ids = phonemeIds(nativePhonemes(text, File(dir, "espeak-ng-data").path), config)
        environment.createSession(File(dir, "en_US-amy-medium.onnx").path).use { session ->
            OnnxTensor.createTensor(environment, LongBuffer.wrap(ids), longArrayOf(1, ids.size.toLong())).use { input ->
                OnnxTensor.createTensor(environment, LongBuffer.wrap(longArrayOf(ids.size.toLong())), longArrayOf(1)).use { lengths ->
                    OnnxTensor.createTensor(environment, FloatBuffer.wrap(floatArrayOf(0.667f, 1.0f, 0.8f)), longArrayOf(3)).use { scales ->
                        session.run(mapOf("input" to input, "input_lengths" to lengths, "scales" to scales)).use { output ->
                            play((output[0] as OnnxTensor).floatBuffer, config.getJSONObject("audio").getInt("sample_rate"))
                        }
                    }
                }
            }
        }
        true
    }.getOrDefault(false)

    private fun phonemeIds(phonemes: String, config: JSONObject): LongArray {
        val map = config.getJSONObject("phoneme_id_map")
        val out = arrayListOf(1L)
        phonemes.codePoints().forEach { cp ->
            val values = map.optJSONArray(String(Character.toChars(cp))) ?: return@forEach
            repeat(values.length()) { out += values.getLong(it); out += 0L }
        }
        out += 2L
        return out.toLongArray()
    }

    private fun installAssets(context: Context, dir: File) {
        if (File(dir, "en_US-amy-medium.onnx").exists()) return
        fun copy(asset: String, target: File) {
            val children = context.assets.list(asset).orEmpty()
            if (children.isEmpty()) {
                target.parentFile?.mkdirs()
                context.assets.open(asset).use { src -> target.outputStream().use(src::copyTo) }
            } else children.forEach { copy("$asset/$it", File(target, it)) }
        }
        copy("piper", dir)
    }

    private fun play(samples: FloatBuffer, sampleRate: Int) {
        val pcm = ShortArray(samples.remaining()) { (samples.get().coerceIn(-1f, 1f) * 32767).toInt().toShort() }
        val track = AudioTrack.Builder()
            .setAudioAttributes(AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_ASSISTANCE_ACCESSIBILITY).build())
            .setAudioFormat(AudioFormat.Builder().setEncoding(AudioFormat.ENCODING_PCM_16BIT).setSampleRate(sampleRate).setChannelMask(AudioFormat.CHANNEL_OUT_MONO).build())
            .setBufferSizeInBytes(pcm.size * 2).setTransferMode(AudioTrack.MODE_STATIC).build()
        stop(); playing.set(track); track.write(pcm, 0, pcm.size); track.play()
        while (playing.get() === track && track.playState == AudioTrack.PLAYSTATE_PLAYING) Thread.sleep(25)
        playing.compareAndSet(track, null); track.release()
    }
}
