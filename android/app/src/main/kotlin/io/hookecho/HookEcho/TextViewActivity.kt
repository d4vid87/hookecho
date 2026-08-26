package io.hookecho.HookEcho

import android.app.Activity
import android.os.Bundle
import android.view.ViewGroup
import android.webkit.WebResourceRequest
import android.webkit.WebView
import android.webkit.WebViewClient

/**
 * Reads one NWS text product in a WebView.
 *
 * The document arrives whole, in the intent, already typeset by `textview.rs` — this activity
 * adds no content of its own and fetches nothing. JavaScript is off, the document has a
 * `default-src 'none'` policy, and the client below refuses every navigation: there is nowhere
 * for a forecast discussion to navigate to, so any attempt is a bug or an attack.
 *
 * A separate activity rather than a Compose surface over the game view: the Rust event loop owns
 * `MainActivity`'s window, and the reader is a full screen the user backs out of, which is what an
 * activity already is.
 */
class TextViewActivity : Activity() {
    override fun onCreate(state: Bundle?) {
        super.onCreate(state)
        title = intent.getStringExtra("title") ?: getString(R.string.app_name)
        val web = WebView(this)
        web.layoutParams = ViewGroup.LayoutParams(
            ViewGroup.LayoutParams.MATCH_PARENT,
            ViewGroup.LayoutParams.MATCH_PARENT,
        )
        web.settings.javaScriptEnabled = false
        web.settings.allowFileAccess = false
        web.settings.allowContentAccess = false
        web.webViewClient = object : WebViewClient() {
            override fun shouldOverrideUrlLoading(v: WebView, r: WebResourceRequest) = true
        }
        // No base URL: the document is its own origin and can reach nothing else.
        web.loadDataWithBaseURL(null, intent.getStringExtra("html") ?: "", "text/html", "utf-8", null)
        setContentView(web)
    }
}
