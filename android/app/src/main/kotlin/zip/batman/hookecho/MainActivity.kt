package zip.batman.hookecho

import androidx.activity.OnBackPressedCallback
import com.google.androidgamesdk.GameActivity
import android.content.Intent
import android.os.Bundle
import java.io.File

/**
 * Rust draws every pixel; this is the `GameActivity` it runs inside. The subclass exists for one
 * reason beyond naming the native library:
 * a notification tap has to reach the Rust side, and the Rust side has no Java callback to receive
 * it. So the target is written to `filesDir/goto.txt` and picked up by `paths::goto_file()` at
 * startup and on the next resume. A file, not an IPC channel, because the activity may be starting
 * cold and there is nothing on the other end to talk to yet.
 */
class MainActivity : GameActivity() {
    /**
     * Predictive back. The callback is *disabled* by default, which is the whole point: with
     * nothing open in the app, Android owns the gesture and can draw its live preview of the home
     * screen as the user drags. The Rust side enables it (via [setBackConsumed]) exactly while it
     * has something to dismiss — a sheet, a full-screen surface — and then `handleOnBackPressed`
     * hands the press to `mobile_back`.
     *
     * KEYCODE_BACK stops being delivered as a key event on Android 16, so this is also how back
     * keeps working at all once the app targets it.
     */
    private val backCallback = object : OnBackPressedCallback(false) {
        override fun handleOnBackPressed() = nativeOnBack()
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        writeGoto(intent)
        super.onCreate(savedInstanceState)
        onBackPressedDispatcher.addCallback(this, backCallback)
    }

    /** Called from Rust when what back would do changes. */
    @Suppress("unused")
    fun setBackConsumed(consumed: Boolean) {
        runOnUiThread { backCallback.isEnabled = consumed }
    }

    private external fun nativeOnBack()

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        writeGoto(intent)
    }

    /** A notification tap carries the target as an extra; a tapped `hookecho://goto/…` link
     *  carries it in the intent data. Both end up as the same string in the same file. */
    private fun writeGoto(intent: Intent?) {
        val goto = intent?.getStringExtra(EXTRA_GOTO)
            ?: intent?.data?.takeIf { it.scheme == "hookecho" }?.toString()
            ?: return
        runCatching { File(filesDir, "goto.txt").writeText(goto) }
    }

    companion object {
        const val EXTRA_GOTO = "hookecho.goto"
    }
}
