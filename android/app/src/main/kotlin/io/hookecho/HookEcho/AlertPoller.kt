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
        val nextRequest: Int = 0,
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
        private const val REQUEST_CAP = 40
        private const val HOME_LEAD_NANOS = 5_000_000_000L

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
            startAt: Int = 0,
        ): Result {
            var hot = false
            var successful = 0
            var failed = 0
            val lines = linkedSetOf<String>()
            val incomplete = linkedSetOf<String>()
            val requests = buildList<Pair<Int, DoubleArray?>>() {
                // Home checks retain their speech priority. Elsewhere, check centres before rims.
                for ((index, watch) in watched.withIndex()) {
                    if (watch.home && watch.samples.isNotEmpty()) {
                        add(index to null)
                        for (sample in watch.samples) add(index to sample)
                    }
                }
                val others = watched.withIndex().filter { !it.value.home }
                for (sample in 0 until (others.maxOfOrNull { it.value.samples.size } ?: 0)) {
                    for ((index, watch) in others) {
                        watch.samples.getOrNull(sample)?.let { add(index to it) }
                    }
                }
            }
            val remaining = IntArray(watched.size)
            for ((index, _) in requests) remaining[index]++
            fun continuing() = shouldContinue() && clock() < deadline
            var attempted = 0
            fun checkRequest(position: Int, requestDeadline: Long): Boolean {
                val (index, sample) = requests[position]
                attempted++
                val watch = watched[index]
                val response = if (sample == null) area(watch.lat, watch.lon, 30.0, requestDeadline, shouldContinue)
                    else point(sample[0], sample[1], requestDeadline, shouldContinue)
                if (!continuing()) return false
                if (response.successful) successful++ else {
                    failed++
                    incomplete.add(watch.name)
                }
                for (alert in response.alerts) {
                    if (!continuing()) return false
                    if (alert.tier >= Nws.TIER_WARNING) hot = true
                    lines.add("${alert.event} — ${watch.name}")
                    onAlert(watch, alert, sample == null || Nws.speaksAt(watch, sample))
                }
                remaining[index]--
                return true
            }
            val start = startAt.takeIf { it in requests.indices } ?: 0
            // Give earlier home checks another chance to speak first, reserving time and slots
            // for continuation. Count these checks against the same cap and skip them on wrap.
            val homeDeadline = minOf(deadline, clock() + HOME_LEAD_NANOS)
            val homeLimit = minOf(start, REQUEST_CAP / 2)
            var homePrefix = 0
            while (homePrefix < homeLimit && watched[requests[homePrefix].first].home &&
                shouldContinue() && clock() < homeDeadline) {
                if (!checkRequest(homePrefix, homeDeadline)) break
                homePrefix++
            }
            var advanced = 0
            var attemptedContinuation = false
            while (advanced < requests.size && continuing()) {
                val position = (start + advanced) % requests.size
                if (position >= homePrefix) {
                    if (attempted == REQUEST_CAP) break
                    attemptedContinuation = true
                    if (!checkRequest(position, deadline)) break
                }
                advanced++
            }
            for ((index, count) in remaining.withIndex()) if (count > 0) incomplete.add(watched[index].name)
            // Retry the first unfinished request next time. If one request used the entire budget,
            // advance anyway so a persistently slow point cannot block the rest of the list.
            val nextRequest = when {
                advanced == requests.size -> 0
                advanced == 0 && attemptedContinuation -> (start + 1) % requests.size
                else -> (start + advanced) % requests.size
            }
            return Result(watched, lines.toList(), hot, successful, failed, incomplete.toList(),
                !shouldContinue(), nextRequest = nextRequest)
        }
    }
}
