package io.hookecho.HookEcho

import androidx.activity.OnBackPressedCallback
import androidx.activity.result.contract.ActivityResultContracts
import android.net.Uri
import android.provider.OpenableColumns
import com.google.androidgamesdk.GameActivity
import android.content.Intent
import android.content.ClipData
import android.widget.Toast
import androidx.appcompat.app.AlertDialog
import androidx.core.content.FileProvider
import android.os.Bundle
import android.os.Build
import android.view.WindowInsets
import android.view.View
import android.view.ViewGroup
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.platform.ComposeView
import android.window.OnBackInvokedCallback
import android.window.OnBackInvokedDispatcher
import java.io.File
import kotlin.concurrent.thread

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
 * anyway; so the stream is copied into app storage and `filesDir/import.txt` names it.
 */
class MainActivity : GameActivity() {
    companion object {
        const val EXTRA_GOTO = "hookecho.goto"
        @Volatile var foreground = false
        // Survives activity recreation while a cloud provider is still copying a document.
        private val importInProgress = java.util.concurrent.atomic.AtomicBoolean(false)
    }

    override fun onStart() { super.onStart(); foreground = true }
    override fun onStop() { foreground = false; super.onStop() }
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
        override fun handleOnBackPressed() = handleBack()
    }
    private var overlayBackCallback: Any? = null
    private var overlayBackRegistered = false
    private var rustBackConsumed = false
    private var shelfView: ComposeView? = null
    private val shelfSection = mutableStateOf("Tools")
    private val shelfSite = mutableStateOf("")
    private val shelfProduct = mutableStateOf("")
    private val shelfAlerts = mutableIntStateOf(0)
    private var shelfVisible = false

    private fun handleBack() {
        if (shelfVisible) { closeQuietShelf(); return }
        if (Build.VERSION.SDK_INT >= 30 &&
            window.decorView.rootWindowInsets?.isVisible(WindowInsets.Type.ime()) == true) {
            window.insetsController?.hide(WindowInsets.Type.ime())
        } else {
            nativeOnBack()
        }
    }

    /** What [openDocument] was asked for: `kind<TAB>tag`, echoed back in `import.txt`. */
    private var pendingImport: String = ""
    private var pendingExport: String? = null

    private val saveDocument =
        registerForActivityResult(ActivityResultContracts.StartActivityForResult()) { result ->
            val source = pendingExport
            pendingExport = null
            val uri = result.data?.data
            if (result.resultCode == RESULT_OK && source != null && uri != null) {
                thread(name = "save-export") {
                    val saved = runCatching {
                        val file = TransferFiles.exportFile(filesDir, source)
                        contentResolver.openOutputStream(uri, "w")?.use { output ->
                            file.inputStream().use { it.copyTo(output) }
                        } ?: error("Cannot open the selected destination")
                    }
                    runOnUiThread {
                        saved.onSuccess { Toast.makeText(this, "Export saved", Toast.LENGTH_SHORT).show() }
                            .onFailure { transferFailed("Export", it) }
                    }
                }
            }
        }

    private val openDocument =
        registerForActivityResult(ActivityResultContracts.OpenDocument()) { uri ->
            uri?.let { copyToImportFile(it) }
        }

    override fun onCreate(savedInstanceState: Bundle?) {
        pendingImport = savedInstanceState?.getString("pendingImport") ?: ""
        pendingExport = savedInstanceState?.getString("pendingExport")
        writeGoto(intent)
        super.onCreate(savedInstanceState)
        onBackPressedDispatcher.addCallback(this, backCallback)
        shelfView = ComposeView(this).apply {
            visibility = View.GONE
            setContent {
                QuietShelf(
                    shelfSection.value, shelfSite.value, shelfProduct.value, shelfAlerts.intValue,
                    onSection = { shelfSection.value = it },
                    onClose = { closeQuietShelf() },
                    onAction = { command -> nativeOnQuietShelfAction(command); closeQuietShelf() }
                )
            }
        }
        addContentView(shelfView, ViewGroup.LayoutParams(
            ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT))
    }

    /**
     * Called from Rust to open the system file picker. [what] is `kind<TAB>tag` and is opaque
     * here — it comes back unchanged so the Rust side knows which button is being answered.
     */
    @Suppress("unused")
    fun openDocument(what: String, mime: String) {
        runOnUiThread {
            if (importInProgress.get()) {
                Toast.makeText(this, "An import is still being copied", Toast.LENGTH_SHORT).show()
                return@runOnUiThread
            }
            pendingImport = what
            runCatching { openDocument.launch(arrayOf(mime)) }
        }
    }

    /**
     * Keep referenced assets in durable storage; one-shot documents can stay in cache. A content
     * URI is not a file path and the permission behind it does not outlive this activity, so
     * handing the URI itself to Rust would hand it something it cannot open.
     */
    private fun copyToImportFile(uri: Uri) {
        if (!importInProgress.compareAndSet(false, true)) {
            Toast.makeText(this, "An import is still being copied", Toast.LENGTH_SHORT).show()
            return
        }
        val request = pendingImport
        thread(name = "import-document") {
            val copied = runCatching {
                val name = contentResolver.query(uri, null, null, null, null)?.use { c ->
                    val i = c.getColumnIndex(OpenableColumns.DISPLAY_NAME)
                    if (i >= 0 && c.moveToFirst()) c.getString(i) else null
                } ?: "import"
                val dest = contentResolver.openInputStream(uri)?.use { input ->
                    TransferFiles.copyImport(filesDir, cacheDir, request.substringBefore('\t'), name, input)
                } ?: error("Cannot open the selected document")
                try {
                    // Rust can poll while the provider is copying; publish only the complete handoff.
                    val handoff = File(filesDir, "import.txt.tmp")
                    handoff.writeText("$request\t${dest.absolutePath}")
                    check(handoff.renameTo(File(filesDir, "import.txt"))) { "Cannot finish import" }
                } catch (error: Exception) {
                    dest.delete()
                    throw error
                }
            }
            importInProgress.set(false)
            runOnUiThread {
                copied.onFailure { transferFailed("Import", it) }
            }
        }
    }

    override fun onSaveInstanceState(outState: Bundle) {
        outState.putString("pendingImport", pendingImport)
        outState.putString("pendingExport", pendingExport)
        super.onSaveInstanceState(outState)
    }

    private fun transferFailed(action: String, error: Throwable) {
        android.util.Log.w("HookEchoActivity", "$action failed", error)
        Toast.makeText(this, "$action failed: ${error.message}", Toast.LENGTH_LONG).show()
    }

    /** Rust calls this only once the output is closed, including screenshot and loop encoders. */
    @Suppress("unused")
    fun offerExport(path: String) {
        runOnUiThread {
            runCatching {
                val file = TransferFiles.exportFile(filesDir, path)
                AlertDialog.Builder(this)
                    .setTitle("Export ${file.name}")
                    .setItems(arrayOf("Save to files…", "Share…")) { _, choice ->
                        runCatching {
                            if (choice == 0) {
                                pendingExport = file.absolutePath
                                saveDocument.launch(Intent(Intent.ACTION_CREATE_DOCUMENT).apply {
                                    addCategory(Intent.CATEGORY_OPENABLE)
                                    type = TransferFiles.mime(file)
                                    putExtra(Intent.EXTRA_TITLE, file.name)
                                })
                            } else {
                                val uri = FileProvider.getUriForFile(this, "$packageName.exports", file)
                                val send = Intent(Intent.ACTION_SEND).apply {
                                    type = TransferFiles.mime(file)
                                    putExtra(Intent.EXTRA_STREAM, uri)
                                    clipData = ClipData.newUri(contentResolver, file.name, uri)
                                    addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
                                }
                                startActivity(Intent.createChooser(send, "Share export"))
                            }
                        }.onFailure { transferFailed("Export", it) }
                    }
                    .setNegativeButton("Cancel", null)
                    .show()
            }.onFailure { transferFailed("Export", it) }
        }
    }

    /** Native Radar Focus menu. The ComposeView is gone when closed, so the map keeps touch input. */
    @Suppress("unused")
    fun showQuietShelf(section: String, site: String, product: String, alerts: Int) {
        runOnUiThread {
            shelfSection.value = section
            shelfSite.value = site
            shelfProduct.value = product
            shelfAlerts.intValue = alerts
            shelfVisible = true
            shelfView?.visibility = View.VISIBLE
            updateBackCallback()
        }
    }

    private fun closeQuietShelf() {
        shelfVisible = false
        shelfView?.visibility = View.GONE
        nativeOnQuietShelfClosed()
        updateBackCallback()
    }

    /** Called from Rust when what back would do changes. */
    @Suppress("unused")
    fun setBackConsumed(consumed: Boolean) {
        runOnUiThread {
            rustBackConsumed = consumed
            updateBackCallback()
        }
    }

    private fun updateBackCallback() {
        val consumed = rustBackConsumed || shelfVisible
        if (Build.VERSION.SDK_INT >= 33) {
            if (consumed && !overlayBackRegistered) {
                val callback = OnBackInvokedCallback { handleBack() }
                onBackInvokedDispatcher.registerOnBackInvokedCallback(
                    OnBackInvokedDispatcher.PRIORITY_OVERLAY, callback)
                overlayBackCallback = callback
                overlayBackRegistered = true
            } else if (!consumed && overlayBackRegistered) {
                (overlayBackCallback as? OnBackInvokedCallback)?.let {
                    onBackInvokedDispatcher.unregisterOnBackInvokedCallback(it)
                }
                overlayBackCallback = null
                overlayBackRegistered = false
            }
        } else {
            backCallback.isEnabled = consumed
        }
    }

    private external fun nativeOnQuietShelfAction(action: String)
    private external fun nativeOnQuietShelfClosed()

    private external fun nativeOnBack()

    /**
     * When GameActivity is destroyed, its native event loop has to go with it.
     *
     * `android_main` is a one-shot: the Rust event loop starts when the native thread does, and it
     * does not stop when the Java activity is destroyed — it keeps ticking frames against a window
     * that no longer exists, which measured at 100% of a core indefinitely, and the next launch
     * finds a process that already ran its entry point, so it sits on the splash screen forever.
     * Ending the process is the only exit that leaves the next launch a clean one.
     *
     * A configuration change is the exception: Android recreates the activity in the same
     * process. `isFinishing` alone misses some Back exits, leaving the old native loop spinning.
     * [AlertService] is `START_STICKY`, so background alerting restarts if enabled.
     */
    override fun onDestroy() {
        android.util.Log.i("HookEchoActivity", "destroy changingConfig=$isChangingConfigurations finishing=$isFinishing")
        // GameActivity's teardown can wait for a native loop that still calls its dead input
        // buffers. End the process before entering that teardown on a real exit.
        if (!isChangingConfigurations) android.os.Process.killProcess(android.os.Process.myPid())
        super.onDestroy()
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
}
