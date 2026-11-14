package ke.co.mocky.mail

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
  }

  override fun onWebViewCreate(webView: WebView) {
    // Lets the page match the system bars to its own light/dark theme.
    // It can only change bar colours; email is shown in sandboxed frames
    // that cannot run scripts, so mail content can't reach it.
    webView.addJavascriptInterface(SystemBars(), "MockySystemBars")
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
