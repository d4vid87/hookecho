package io.hookecho.HookEcho

import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createEmptyComposeRule
import androidx.lifecycle.Lifecycle
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File
import java.lang.reflect.InvocationTargetException

/** Runs the real native activity; requires an arm64 device and a built libhookecho.so. */
@RunWith(AndroidJUnit4::class)
class DeviceSmokeTest {
    @get:Rule val rule = createEmptyComposeRule()

    @Test fun nativeMenusResumeAndOfflineVoice() {
        // This one-shot native activity kills its process on finish; keep it alive until the runner exits.
        val scenario = ActivityScenario.launch(MainActivity::class.java)
        lateinit var activity: MainActivity
        scenario.onActivity { activity = it }
        for (section in listOf("Radar", "Layers", "Alerts", "Tools")) {
            rule.runOnUiThread { activity.showQuietShelf(section, "KDMX", "REF", 0) }
            rule.onNodeWithText("${section.uppercase()} / QUICK ACTIONS").assertIsDisplayed()
            rule.onNodeWithText("×").performClick()
            rule.onNodeWithText("HOOKECHO / MOBILE").assertIsNotDisplayed()
        }
        scenario.moveToState(Lifecycle.State.CREATED)
        scenario.moveToState(Lifecycle.State.RESUMED)
        rule.runOnUiThread { activity.showQuietShelf("Layers", "KDMX", "REF", 0) }
        rule.onNodeWithText("LAYERS / QUICK ACTIONS").assertIsDisplayed()
        rule.onNodeWithText("×").performClick()
        rule.onNodeWithText("HOOKECHO / MOBILE").assertIsNotDisplayed()

        assertTrue(PiperVoice.speakBlocking(activity, "HookEcho device test. This is only a test."))
        val phonemes = PiperVoice::class.java.getDeclaredMethod(
            "nativePhonemes", String::class.java, String::class.java
        ).apply { isAccessible = true }
        val error = assertThrows(InvocationTargetException::class.java) {
            phonemes.invoke(null, "test", File(activity.cacheDir, "missing-espeak-data").path)
        }
        assertTrue(error.cause is IllegalStateException)
    }
}
