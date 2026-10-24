package io.hookecho.HookEcho

import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.lifecycle.Lifecycle
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/** Runs the real native activity; requires an arm64 device and a built libhookecho.so. */
@RunWith(AndroidJUnit4::class)
class DeviceSmokeTest {
    @get:Rule val rule = createAndroidComposeRule<MainActivity>()

    @Test fun everyShelfTabOpensAndClosesOverTheNativeMap() {
        for (section in listOf("Radar", "Layers", "Alerts", "Tools")) {
            rule.runOnUiThread { rule.activity.showQuietShelf(section, "KDMX", "REF", 0) }
            rule.onNodeWithText("${section.uppercase()} / QUICK ACTIONS").assertIsDisplayed()
            rule.onNodeWithText("×").performClick()
            rule.onNodeWithText("HOOKECHO / MOBILE").assertDoesNotExist()
        }
    }

    @Test fun backgroundResumeRestoresTheActivityAndWorkingShelf() {
        rule.activityRule.scenario.moveToState(Lifecycle.State.CREATED)
        rule.activityRule.scenario.moveToState(Lifecycle.State.RESUMED)
        rule.runOnUiThread { rule.activity.showQuietShelf("Layers", "KDMX", "REF", 0) }
        rule.onNodeWithText("LAYERS / QUICK ACTIONS").assertIsDisplayed()
        rule.onNodeWithText("×").performClick()
        rule.onNodeWithText("HOOKECHO / MOBILE").assertDoesNotExist()
    }
}
