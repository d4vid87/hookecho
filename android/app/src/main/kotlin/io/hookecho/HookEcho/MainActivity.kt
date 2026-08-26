package io.hookecho.HookEcho

import androidx.activity.OnBackPressedCallback
import androidx.activity.result.contract.ActivityResultContracts
import android.net.Uri
import android.provider.OpenableColumns
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
 *
 * File import works the same way. The Storage Access Framework hands back a content URI through
 * an activity result, which Rust has no way to receive and which is not a path anything can open
 * anyway; so the stream is copied into the cache and `filesDir/import.txt` names it.
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

    /** What [openDocument] was asked for: `kind<TAB>tag`, echoed back in `import.txt`. */
    private var pendingImport: String = ""

    private val openDocument =
        registerForActivityResult(ActivityResultContracts.OpenDocument()) { uri ->
            uri?.let { copyToImportFile(it) }
        }

    override fun onCreate(savedInstanceState: Bundle?) {
        writeGoto(intent)
        super.onCreate(savedInstanceState)
        onBackPressedDispatcher.addCallback(this, backCallback)
    }

    /**
     * Called from Rust to open the system file picker. [what] is `kind<TAB>tag` and is opaque
     * here — it comes back unchanged so the Rust side knows which button is being answered.
     */
    @Suppress("unused")
    fun openDocument(what: String, mime: String) {
        runOnUiThread {
            pendingImport = what
            runCatching { openDocument.launch(arrayOf(mime)) }
        }
    }

    /**
     * Copy the chosen document into the cache under its display name and announce it. A content
     * URI is not a file path and the permission behind it does not outlive this activity, so
     * handing the URI itself to Rust would hand it something it cannot open.
     */
    private fun copyToImportFile(uri: Uri) = runCatching {
        val name = contentResolver.query(uri, null, null, null, null)?.use { c ->
            val i = c.getColumnIndex(OpenableColumns.DISPLAY_NAME)
            if (i >= 0 && c.moveToFirst()) c.getString(i) else null
        } ?: "import"
        val dir = File(cacheDir, "import").apply { mkdirs() }
        val dest = File(dir, name)
        contentResolver.openInputStream(uri)?.use { input ->
            dest.outputStream().use { input.copyTo(it) }
        } ?: return@runCatching
        File(filesDir, "import.txt").writeText("$pendingImport\t${dest.absolutePath}")
    }

    /** Called from Rust when what back would do changes. */
    @Suppress("unused")
    fun setBackConsumed(consumed: Boolean) {
        runOnUiThread { backCallback.isEnabled = consumed }
    }

    private external fun nativeOnBack()

    /**
     * Back out of the app and the process has to go with it.
     *
     * `android_main` is a one-shot: the Rust event loop starts when the native thread does, and it
     * does not stop when the Java activity is destroyed — it keeps ticking frames against a window
     * that no longer exists, which measured at 100% of a core indefinitely, and the next launch
     * finds a process that already ran its entry point, so it sits on the splash screen forever.
     * Ending the process is the only exit that leaves the next launch a clean one.
     *
     * Only when the user is actually leaving: a destroy for a configuration change must not take
     * the process with it. [AlertService] is `START_STICKY`, so background alerting comes back on
     * its own for anyone who has it switched on.
     */
    override fun onDestroy() {
        super.onDestroy()
        if (isFinishing) android.os.Process.killProcess(android.os.Process.myPid())
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
