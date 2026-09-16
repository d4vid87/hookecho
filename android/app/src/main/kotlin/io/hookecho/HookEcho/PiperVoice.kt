package io.hookecho.HookEcho

import ai.onnxruntime.OnnxTensor
import ai.onnxruntime.OrtEnvironment
import ai.onnxruntime.OrtSession
import android.content.Context
import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioTrack
import org.json.JSONObject
import java.io.File
import java.nio.FloatBuffer
import java.nio.LongBuffer
import java.util.concurrent.PriorityBlockingQueue
import java.util.concurrent.atomic.AtomicLong
import java.util.concurrent.atomic.AtomicReference

/** Offline en_US-amy-medium synthesis shared by foreground and background alerts. */
object PiperVoice {
    private val playing = AtomicReference<AudioTrack?>()
    private val environment by lazy { OrtEnvironment.getEnvironment() }
    private var session: OrtSession? = null
    private val sequence = AtomicLong()
    private data class Job(
        val context: Context,
        val text: String,
        val priority: Int,
        val order: Long,
    ) : Comparable<Job> {
        override fun compareTo(other: Job): Int =
            compareValuesBy(this, other, { -it.priority }, { it.order })
    }
    private val jobs = PriorityBlockingQueue<Job>()
    init { System.loadLibrary("hookecho_phonemize") }
    init {
        Thread({
            while (true) {
                val job = jobs.take()
                speakBlocking(job.context, job.text)
            }
        }, "hookecho-piper").apply { isDaemon = true; start() }
    }
    @JvmStatic private external fun nativePhonemes(text: String, dataPath: String): String
    @JvmStatic fun stop() = playing.getAndSet(null)?.runCatching { pause(); flush(); release() }

    @JvmStatic fun enqueue(context: Context, text: String, priority: Int) {
        if (priority >= Nws.TIER_EMERGENCY) {
            jobs.removeIf { it.priority < priority }
            stop()
        }
        jobs.put(Job(context.applicationContext, text, priority, sequence.getAndIncrement()))
    }

    @JvmStatic @Synchronized fun speakBlocking(context: Context, text: String): Boolean = runCatching {
        val dir = File(context.filesDir, "piper-amy")
        installAssets(context, dir)
        val config = JSONObject(File(dir, "en_US-amy-medium.onnx.json").readText())
        val ids = phonemeIds(nativePhonemes(text, File(dir, "espeak-ng-data").path), config)
        val voice = session ?: environment.createSession(
            File(dir, "en_US-amy-medium.onnx").path
        ).also { session = it }
        OnnxTensor.createTensor(environment, LongBuffer.wrap(ids), longArrayOf(1, ids.size.toLong())).use { input ->
            OnnxTensor.createTensor(environment, LongBuffer.wrap(longArrayOf(ids.size.toLong())), longArrayOf(1)).use { lengths ->
                OnnxTensor.createTensor(environment, FloatBuffer.wrap(floatArrayOf(0.667f, 1.0f, 0.8f)), longArrayOf(3)).use { scales ->
                    voice.run(mapOf("input" to input, "input_lengths" to lengths, "scales" to scales)).use { output ->
                        play((output[0] as OnnxTensor).floatBuffer, config.getJSONObject("audio").getInt("sample_rate"))
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
        val deadline = System.currentTimeMillis() + (pcm.size * 1000L / sampleRate) + 2_000L
        while (playing.get() === track && track.playbackHeadPosition < pcm.size &&
            System.currentTimeMillis() < deadline
        ) Thread.sleep(25)
        if (playing.compareAndSet(track, null)) {
            track.stop()
            track.release()
        }
    }
}
