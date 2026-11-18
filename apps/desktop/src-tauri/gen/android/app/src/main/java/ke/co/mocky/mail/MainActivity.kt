package ke.co.mocky.mail

import android.content.Intent
import android.graphics.Color
import android.os.Bundle
import android.webkit.JavascriptInterface
import android.webkit.WebView
import androidx.activity.enableEdgeToEdge
import androidx.core.view.WindowCompat

class MainActivity : TauriActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    rememberMailto(intent)
  }

  override fun onNewIntent(intent: Intent) {
    super.onNewIntent(intent)
    rememberMailto(intent)
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
