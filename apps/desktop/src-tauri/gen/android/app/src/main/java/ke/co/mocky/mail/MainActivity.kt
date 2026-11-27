package ke.co.mocky.mail

import android.content.Intent
import android.graphics.Color
import android.os.Bundle
import android.webkit.JavascriptInterface
import android.webkit.WebView
import androidx.activity.enableEdgeToEdge
import androidx.core.view.ViewCompat
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import java.util.Locale

class MainActivity : TauriActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    rememberMailto(intent)
    // .eml, .ics and .vcf files opened with the app or shared to it.
    SharedFiles.receive(this, intent)
  }

  override fun onNewIntent(intent: Intent) {
    super.onNewIntent(intent)
    rememberMailto(intent)
    SharedFiles.receive(this, intent)
  }

  /** A mailto: link this app was opened with, until the page takes it. */
  @Volatile private var pendingMailto: String? = null

  private fun rememberMailto(intent: Intent?) {
    val data = intent?.data ?: return
    if (data.scheme.equals("mailto", ignoreCase = true)) {
      pendingMailto = data.toString()
    }
  }

  override fun onWebViewCreate(webView: WebView) {
    // Lets the page match the system bars to its own light/dark theme.
    // It can only change bar colours; email is shown in sandboxed frames
    // that cannot run scripts, so mail content can't reach it.
    webView.addJavascriptInterface(SystemBars(), "MockySystemBars")
    // The page asks for an email link the app was opened with; the link is
    // only ever read (parsed by Rust) to fill in a new message.
    webView.addJavascriptInterface(Intents(), "MockyIntents")
    // The app draws under the status and navigation bars (edge to edge), and
    // the WebView reports CSS env(safe-area-inset-*) as 0, so the page gets
    // the bars' sizes from here: as --safe-* CSS variables, pushed on every
    // change and read once at start-up through MockyInsets.
    webView.addJavascriptInterface(Insets(), "MockyInsets")
    ViewCompat.setOnApplyWindowInsetsListener(webView) { view, insets ->
      val bars = insets.getInsets(WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout())
      val density = view.resources.displayMetrics.density
      fun css(px: Int) = String.format(Locale.US, "\"%.1fpx\"", px / density)
      insetsJson = "{\"top\":${css(bars.top)},\"right\":${css(bars.right)},\"bottom\":${css(bars.bottom)},\"left\":${css(bars.left)}}"
      (view as WebView).evaluateJavascript("window.__mockyInsets && window.__mockyInsets($insetsJson)", null)
      insets
    }
  }

  /** The system bars' sizes in CSS pixels, as JSON (see onWebViewCreate). */
  @Volatile private var insetsJson = "{}"

  private inner class Insets {
    @JavascriptInterface
    fun get(): String = insetsJson
  }

  private inner class Intents {
    @JavascriptInterface
    fun takeMailto(): String? {
      val link = pendingMailto
      pendingMailto = null
      return link
    }
  }

  private inner class SystemBars {
    @JavascriptInterface
    fun set(dark: Boolean, background: String) {
      runOnUiThread {
        val controller = WindowCompat.getInsetsController(window, window.decorView)
        controller.isAppearanceLightStatusBars = !dark
        controller.isAppearanceLightNavigationBars = !dark
        runCatching { Color.parseColor(background) }.onSuccess { window.decorView.setBackgroundColor(it) }
      }
    }
  }
}
