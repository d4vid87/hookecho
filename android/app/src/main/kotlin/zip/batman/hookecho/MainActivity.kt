package zip.batman.hookecho

import android.app.NativeActivity
import android.content.Intent
import android.os.Bundle
import java.io.File

/**
 * The app is still a NativeActivity — Rust draws every pixel. This subclass exists for one reason:
 * a notification tap has to reach the Rust side, and the Rust side has no Java callback to receive
 * it. So the target is written to `filesDir/goto.txt` and picked up by `paths::goto_file()` at
 * startup and on the next resume. A file, not an IPC channel, because the activity may be starting
 * cold and there is nothing on the other end to talk to yet.
 */
class MainActivity : NativeActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        writeGoto(intent)
        super.onCreate(savedInstanceState)
    }

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
