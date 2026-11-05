package io.hookecho.HookEcho

/** Cancellation and AudioTrack ownership, independent of synthesis and Android for race tests. */
internal class SpeechPlayback {
    class Ticket internal constructor(internal val generation: Long, val priority: Int) {
        internal var cancelled = false
    }
    interface Output {
        fun start()
        fun isPlaying(): Boolean
        /** Idempotent: an output can finish after stop already released it. */
        fun close()
    }

    private val synthesis = Any()
    private var generation = 0L
    private val pending = mutableSetOf<Ticket>()
    private var active: Pair<Ticket, Output>? = null

    @Synchronized fun begin(priority: Int): Ticket =
        Ticket(generation, priority).also { pending.add(it) }

    @Synchronized fun isCurrent(ticket: Ticket): Boolean =
        ticket.generation == generation && !ticket.cancelled

    /**
     * Capture the ticket before waiting for synthesis. Cancellation is handled success, so the
     * JNI Boolean reports false only for a real voice failure rather than Stop or preemption.
     */
    fun run(ticket: Ticket, body: () -> Boolean): Boolean = try {
        synchronized(synthesis) {
            if (!isCurrent(ticket)) true else {
                val succeeded = body()
                succeeded || !isCurrent(ticket)
            }
        }
    } finally {
        synchronized(this) { pending.remove(ticket) }
    }

    @Synchronized fun stop(): Unit {
        generation++
        pending.clear()
        closeActive()
    }

    /** An emergency cancels lower-priority work, retaining other queued emergencies. */
    @Synchronized fun interruptBelow(priority: Int): Unit {
        pending.removeAll {
            if (it.priority < priority) { it.cancelled = true; true } else false
        }
        if (active?.first?.priority?.let { it < priority } == true) closeActive()
    }

    @Synchronized fun publish(ticket: Ticket, output: Output): Boolean {
        if (!isCurrent(ticket)) return false
        closeActive()
        active = ticket to output
        try { output.start() } catch (error: Throwable) { closeActive(); throw error }
        return true
    }

    @Synchronized fun isPlaying(ticket: Ticket, output: Output): Boolean =
        isCurrent(ticket) && active?.second === output && output.isPlaying()

    @Synchronized fun finish(output: Output): Unit {
        if (active?.second === output) active = null
        runCatching { output.close() }
    }

    private fun closeActive() {
        val previous = active
        active = null
        previous?.second?.let { runCatching { it.close() } }
    }
}
