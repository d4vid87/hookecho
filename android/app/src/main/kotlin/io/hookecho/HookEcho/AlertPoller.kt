package io.hookecho.HookEcho

/** One in-process pass shared by the service, recovery worker and widget. */
internal class AlertPoller(private val clock: () -> Long = System::nanoTime) {
    data class Result(
        val watched: List<Nws.Watch> = emptyList(),
        val lines: List<String> = emptyList(),
        val hot: Boolean = false,
        val successfulRequests: Int = 0,
        val failedRequests: Int = 0,
        val incomplete: List<String> = emptyList(),
        val cancelled: Boolean = false,
        val error: String? = null,
    ) {
        val complete: Boolean get() = watched.isNotEmpty() && successfulRequests > 0 &&
            failedRequests == 0 && incomplete.isEmpty() && !cancelled && error == null
    }

    private val lock = Object()
    private var active = false
    private var lastKey: String? = null
    private var last: Result? = null
    private var finished = 0L

    fun run(key: String, shouldContinue: () -> Boolean, execute: () -> Result): Result? {
        synchronized(lock) {
            while (active) {
                if (!shouldContinue()) return null
                try { lock.wait(100) } catch (_: InterruptedException) {
                    Thread.currentThread().interrupt()
                    return null
                }
            }
            if (!shouldContinue()) return null
            // A worker starting the service may reach us just after that service's fast pass.
            if (key == lastKey && clock() - finished < 5_000_000_000L) return last
            active = true
        }
        try {
            val result = execute()
            synchronized(lock) {
                if (!result.cancelled) {
                    last = result
                    lastKey = key
                    finished = clock()
                }
            }
            return result
        } finally {
            synchronized(lock) { active = false; lock.notifyAll() }
        }
    }

    companion object {
        const val BUDGET_NANOS = 45_000_000_000L

        fun deliverIfNew(
            seen: MutableSet<String>,
            alert: Nws.Alert,
            quiet: Boolean,
            post: () -> Boolean,
        ): Boolean {
            if (alert.id in seen || (quiet && alert.tier < Nws.TIER_EMERGENCY)) return false
            return post() && seen.add(alert.id)
        }

        fun collect(
            watched: List<Nws.Watch>,
            deadline: Long,
            shouldContinue: () -> Boolean,
            onAlert: (Nws.Watch, Nws.Alert, Boolean) -> Unit,
            point: (Double, Double, Long, () -> Boolean) -> Nws.AlertResponse = Nws::alertsAt,
            area: (Double, Double, Double, Long, () -> Boolean) -> Nws.AlertResponse = Nws::alertsWithin,
            clock: () -> Long = System::nanoTime,
        ): Result {
            var hot = false
            var successful = 0
            var failed = 0
            val lines = linkedSetOf<String>()
            val incomplete = watched.filter { it.samples.size < it.requestedSamples }.mapTo(linkedSetOf()) { it.name }
            val remaining = watched.map { it.samples.size + if (it.home && it.samples.isNotEmpty()) 1 else 0 }.toIntArray()
            fun continuing() = shouldContinue() && clock() < deadline
            fun request(index: Int, sample: DoubleArray?): Boolean {
                if (!continuing()) return false
                val watch = watched[index]
                val response = if (sample == null) area(watch.lat, watch.lon, 30.0, deadline, shouldContinue)
                    else point(sample[0], sample[1], deadline, shouldContinue)
                if (!continuing()) return false
                remaining[index]--
                if (response.successful) successful++ else {
                    failed++
                    incomplete.add(watch.name)
                }
                for (alert in response.alerts) {
                    if (!continuing()) { incomplete.add(watch.name); return false }
                    if (alert.tier >= Nws.TIER_WARNING) hot = true
                    lines.add("${alert.event} — ${watch.name}")
                    onAlert(watch, alert, sample == null || Nws.speaksAt(watch, sample))
                }
                return true
            }
            // Homes retain notification/speech priority; other places check centres before rims.
            checking@ do {
                for ((index, watch) in watched.withIndex()) {
                    if (watch.home && watch.samples.isNotEmpty()) {
                        if (!request(index, null)) break@checking
                        for (sample in watch.samples) if (!request(index, sample)) break@checking
                    }
                }
                for ((index, watch) in watched.withIndex()) {
                    if (!watch.home && watch.samples.isNotEmpty() && !request(index, watch.samples.first())) break@checking
                }
                for (sample in 1 until (watched.maxOfOrNull { it.samples.size } ?: 0)) {
                    for ((index, watch) in watched.withIndex()) {
                        if (!watch.home && sample < watch.samples.size && !request(index, watch.samples[sample])) break@checking
                    }
                }
            } while (false)
            for ((index, count) in remaining.withIndex()) if (count > 0) incomplete.add(watched[index].name)
            return Result(watched, lines.toList(), hot, successful, failed, incomplete.toList(), !shouldContinue())
        }
    }
}
