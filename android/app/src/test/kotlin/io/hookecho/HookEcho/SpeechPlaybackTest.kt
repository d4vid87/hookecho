package io.hookecho.HookEcho

import org.junit.Assert.*
import org.junit.Test
import java.lang.reflect.Modifier
import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicInteger

class SpeechPlaybackTest {
    private class Output : SpeechPlayback.Output {
        val starts = AtomicInteger()
        val closes = AtomicInteger()
        private val closed = AtomicBoolean()
        override fun start() { check(!closed.get()); starts.incrementAndGet() }
        override fun isPlaying(): Boolean { check(!closed.get()); return true }
        override fun close() { if (closed.compareAndSet(false, true)) closes.incrementAndGet() }
    }

    @Test fun stopHasTheVoidStaticJniAbiWithoutInitializingNativeVoice() {
        val voice = Class.forName("io.hookecho.HookEcho.PiperVoice", false, javaClass.classLoader)
        val stop = voice.getDeclaredMethod("stop")
        assertEquals(Void.TYPE, stop.returnType)
        assertTrue(Modifier.isStatic(stop.modifiers))
        val speak = voice.getDeclaredMethod("speakBlocking", android.content.Context::class.java, String::class.java)
        assertEquals(java.lang.Boolean.TYPE, speak.returnType)
        assertFalse("ticket must be captured before waiting for synthesis", Modifier.isSynchronized(speak.modifiers))
    }

    @Test fun stopCancelsQueuedAndInFlightSpeechBeforeAudioPublication() {
        val gate = SpeechPlayback()
        val running = gate.begin(0)
        val queued = gate.begin(0) // Captured before a foreground/dequeued request waits for synthesis.
        val inference = CountDownLatch(1)
        val finishInference = CountDownLatch(1)
        val waiting = CountDownLatch(1)
        val queuedRan = AtomicBoolean()
        val output = Output()
        val pool = Executors.newFixedThreadPool(3)
        try {
            val first = pool.submit<Boolean> { gate.run(running) {
                inference.countDown()
                check(finishInference.await(5, TimeUnit.SECONDS))
                try { gate.publish(running, output) } finally { gate.finish(output) }
            } }
            assertTrue(inference.await(3, TimeUnit.SECONDS))
            val second = pool.submit<Boolean> {
                waiting.countDown()
                gate.run(queued) { queuedRan.set(true); true }
            }
            assertTrue(waiting.await(3, TimeUnit.SECONDS))
            pool.submit { gate.stop() }.get(2, TimeUnit.SECONDS) // Stop cannot wait for ONNX.
            finishInference.countDown()
            assertTrue("cancelled inference is handled, not a voice failure", first.get(3, TimeUnit.SECONDS))
            assertTrue("cancelled queued speech is handled without running", second.get(3, TimeUnit.SECONDS))
            assertFalse(queuedRan.get())
            assertEquals(0, output.starts.get())
            assertEquals(1, output.closes.get())
            assertTrue(gate.run(gate.begin(0)) { true }) // A later request is allowed.
        } finally { finishInference.countDown(); pool.shutdownNow() }
    }

    @Test fun stopSerializesWithAudioPublicationAndReleasesExactlyOnce() {
        repeat(50) {
            val gate = SpeechPlayback()
            val ticket = gate.begin(0)
            val output = Output()
            val start = CountDownLatch(1)
            val pool = Executors.newFixedThreadPool(2)
            try {
                val publish = pool.submit<Boolean> { start.await(); gate.publish(ticket, output) }
                val stop = pool.submit { start.await(); gate.stop() }
                start.countDown()
                publish.get(3, TimeUnit.SECONDS)
                stop.get(3, TimeUnit.SECONDS)
                assertFalse(gate.isPlaying(ticket, output))
                gate.finish(output)
                assertEquals(1, output.closes.get())
                assertTrue(output.starts.get() <= 1)
                assertFalse(gate.publish(ticket, Output()))
            } finally { pool.shutdownNow() }
        }
    }

    @Test fun emergenciesCancelLowerPriorityWorkButPreserveEqualPriorityAlerts() {
        val gate = SpeechPlayback()
        val normal = gate.begin(1)
        val pending = gate.begin(2)
        val emergency = gate.begin(3)
        val higher = gate.begin(4)
        val normalAudio = Output()
        assertTrue(gate.publish(normal, normalAudio))
        gate.interruptBelow(3)
        assertEquals(1, normalAudio.closes.get())
        assertFalse(gate.isCurrent(normal))
        assertTrue(gate.run(pending) { fail("lower-priority queued speech must be cancelled"); true })
        assertTrue(gate.isCurrent(emergency))
        assertTrue(gate.isCurrent(higher))
        val emergencyAudio = Output()
        assertTrue(gate.publish(emergency, emergencyAudio))
        gate.interruptBelow(3)
        assertTrue(gate.isPlaying(emergency, emergencyAudio))
        assertEquals(0, emergencyAudio.closes.get())
        gate.stop()
        assertFalse(gate.isCurrent(higher))
        gate.finish(emergencyAudio)
        assertEquals(1, emergencyAudio.closes.get())
    }

    @Test fun stoppedOrPreemptedPlaybackIsHandledWithoutReportingVoiceFailure() {
        for (emergency in listOf(false, true)) {
            val gate = SpeechPlayback()
            val ticket = gate.begin(1)
            val output = Output()
            assertTrue(gate.run(ticket) {
                try {
                    assertTrue(gate.publish(ticket, output))
                    if (emergency) gate.interruptBelow(3) else gate.stop()
                    assertFalse(gate.isPlaying(ticket, output))
                    gate.isCurrent(ticket) // Piper's playback returns false when interrupted.
                } finally { gate.finish(output) }
            })
            assertEquals(1, output.starts.get())
            assertEquals(1, output.closes.get())
        }
    }

    @Test fun anUncancelledVoiceFailureStillReturnsFalse() {
        val gate = SpeechPlayback()
        val ticket = gate.begin(0)
        assertFalse(gate.run(ticket) { false })
        assertTrue(gate.isCurrent(ticket))
    }
}
