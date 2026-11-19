package io.hookecho.HookEcho

import org.junit.Assert.*
import org.junit.Test
import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicInteger

class AlertPollerTest {
    @Test fun overlappingSchedulesShareRequestsAndSeenTransaction() {
        val poller = AlertPoller()
        val entered = CountDownLatch(1)
        val release = CountDownLatch(1)
        val runs = AtomicInteger()
        val notifications = AtomicInteger()
        var savedSeen: Set<String> = emptySet()
        val pool = Executors.newFixedThreadPool(3)
        try {
            fun poll() = poller.run("coverage", { true }) {
                runs.incrementAndGet()
                val seen = savedSeen.toMutableSet()
                entered.countDown()
                check(release.await(2, TimeUnit.SECONDS))
                if (seen.add("warning")) notifications.incrementAndGet()
                savedSeen = seen
                AlertPoller.Result(successfulRequests = 1, hot = true)
            }
            val service = pool.submit<AlertPoller.Result?> { poll() }
            assertTrue(entered.await(2, TimeUnit.SECONDS))
            val alarm = pool.submit<AlertPoller.Result?> { poll() }
            val widget = pool.submit<AlertPoller.Result?> { poll() }
            release.countDown()
            val result = service.get(2, TimeUnit.SECONDS)
            assertSame(result, alarm.get(2, TimeUnit.SECONDS))
            assertSame(result, widget.get(2, TimeUnit.SECONDS))
            assertEquals(1, runs.get())
            assertEquals(1, notifications.get())
            assertEquals(setOf("warning"), savedSeen)
        } finally { release.countDown(); pool.shutdownNow() }
    }

    @Test fun cancellationDoesNotBlockOtherCallersOrReuseAnAbandonedPass() {
        val poller = AlertPoller()
        val ownerEntered = CountDownLatch(1)
        val release = CountDownLatch(1)
        val pool = Executors.newFixedThreadPool(2)
        try {
            val owner = pool.submit<AlertPoller.Result?> {
                poller.run("same", { true }) {
                    ownerEntered.countDown()
                    release.await(2, TimeUnit.SECONDS)
                    AlertPoller.Result(cancelled = true)
                }
            }
            assertTrue(ownerEntered.await(2, TimeUnit.SECONDS))
            val waiter = pool.submit<AlertPoller.Result?> { poller.run("same", { false }) { error("cancelled waiter ran") } }
            assertNull(waiter.get(1, TimeUnit.SECONDS))
            release.countDown()
            assertTrue(owner.get(2, TimeUnit.SECONDS)!!.cancelled)
            val replacement = poller.run("same", { true }) { AlertPoller.Result(successfulRequests = 2) }
            assertEquals(2, replacement!!.successfulRequests)
        } finally { release.countDown(); pool.shutdownNow() }
    }

    @Test fun failuresReleaseThePollAndChangedCoverageDoesNotUseCachedAnswers() {
        var now = 1L
        val poller = AlertPoller { now }
        assertTrue(runCatching { poller.run("a", { true }) { error("provider failed") } }.isFailure)
        assertEquals(1, poller.run("a", { true }) { AlertPoller.Result(successfulRequests = 1) }!!.successfulRequests)
        assertEquals(2, poller.run("b", { true }) { AlertPoller.Result(successfulRequests = 2) }!!.successfulRequests)
        now += 6_000_000_000
        assertEquals(3, poller.run("b", { true }) { AlertPoller.Result(successfulRequests = 3) }!!.successfulRequests)
    }

    @Test fun deniedNotificationsCanBeDeliveredOncePermissionIsGranted() {
        val warning = Nws.Alert("warning", "Tornado Warning", "warning", Nws.TIER_WARNING)
        val seen = mutableSetOf<String>()
        val poller = AlertPoller { 0L }
        var attempts = 0

        fun poll(canNotify: Boolean) = poller.run("home:$canNotify", { true }) {
            if (canNotify) {
                AlertPoller.deliverIfNew(seen, warning, quiet = false) {
                    attempts++
                    true
                }
            }
            AlertPoller.Result(hot = true)
        }

        assertTrue(poll(false)!!.hot)
        assertEquals(0, attempts)
        assertTrue(seen.isEmpty())
        assertFalse(AlertPoller.deliverIfNew(seen, warning, quiet = false) { false })
        assertTrue(seen.isEmpty())

        assertTrue(poll(true)!!.hot)
        assertEquals(setOf(warning.id), seen)
        assertEquals(1, attempts)
        assertFalse(AlertPoller.deliverIfNew(seen, warning, quiet = false) { error("duplicate notification") })
    }

    @Test fun ordinaryWarningsStayUnseenDuringQuietHours() {
        val warning = Nws.Alert("warning", "Flash Flood Warning", "warning", Nws.TIER_WARNING)
        val seen = mutableSetOf<String>()
        assertFalse(AlertPoller.deliverIfNew(seen, warning, quiet = true) { error("quiet hours") })
        assertTrue(seen.isEmpty())
        assertTrue(AlertPoller.deliverIfNew(seen, warning.copy(tier = Nws.TIER_EMERGENCY), quiet = true) { true })
        assertEquals(setOf(warning.id), seen)
    }

    @Test fun centresRunBeforeRimsAndCoverageStaysPartial() {
        val watches = (1..7).map { i -> Nws.Watch("Place $i", i.toDouble(), 0.0,
            listOf(doubleArrayOf(i.toDouble(), 0.0), doubleArrayOf(i.toDouble(), 1.0)), requestedSamples = 7) }
        val points = mutableListOf<Pair<Double, Double>>()
        val result = AlertPoller.collect(watches, 100, { true }, { _, _, _ -> },
            point = { lat, lon, _, _ -> points.add(lat to lon); Nws.AlertResponse(emptyList()) },
            clock = { 0 })
        assertEquals((1..7).map { it.toDouble() to 0.0 }, points.take(7))
        assertEquals(14, result.successfulRequests)
        assertFalse(result.complete)
        assertEquals(watches.map { it.name }, result.incomplete)
    }

    @Test fun homeSpeechWinsDedupEvenWhenAnotherLocationsCentreHasTheSameAlert() {
        val watches = listOf(
            Nws.Watch("Home", 32.0, -97.0, listOf(doubleArrayOf(32.0, -97.0), doubleArrayOf(32.01, -97.0)), home = true),
            Nws.Watch("Work", 32.1, -97.0, listOf(doubleArrayOf(32.1, -97.0))),
        )
        val alert = Nws.Alert("shared", "Tornado Warning", "warning", Nws.TIER_WARNING)
        val seen = mutableSetOf<String>()
        val notified = mutableListOf<Pair<String, Boolean>>()
        val result = AlertPoller.collect(watches, Long.MAX_VALUE, { true }, { watch, value, speak ->
            if (seen.add(value.id)) notified.add(watch.name to speak)
        }, point = { lat, _, _, _ ->
            Nws.AlertResponse(if (lat == 32.0) emptyList() else listOf(alert))
        }, area = { _, _, _, _, _ -> Nws.AlertResponse(emptyList()) })
        assertTrue(result.complete)
        assertEquals(listOf("Home" to true), notified)
    }

    @Test fun deadlineOrCancellationLeavesUncheckedPlacesVisibleAndNeverAnnouncesLateResults() {
        val watches = (1..3).map { i -> Nws.Watch("Place $i", i.toDouble(), 0.0,
            listOf(doubleArrayOf(i.toDouble(), 0.0))) }
        var now = 0L
        val calls = AtomicInteger()
        val result = AlertPoller.collect(watches, 10, { true }, { _, _, _ -> },
            point = { _, _, _, _ -> calls.incrementAndGet(); now += 6; Nws.AlertResponse(emptyList()) }, clock = { now })
        assertEquals(2, calls.get())
        assertEquals(1, result.successfulRequests)
        assertEquals(listOf("Place 2", "Place 3"), result.incomplete)
        assertFalse(result.complete)

        val active = AtomicBoolean(true)
        val cancelled = AlertPoller.collect(watches, Long.MAX_VALUE, active::get, { _, _, _ -> error("late warning announced") },
            point = { _, _, _, _ -> active.set(false); Nws.AlertResponse(listOf(Nws.Alert("a", "Tornado Warning", "warning", 2))) })
        assertTrue(cancelled.cancelled)
        assertEquals(0, cancelled.successfulRequests)
        assertEquals(3, cancelled.incomplete.size)
    }
}
