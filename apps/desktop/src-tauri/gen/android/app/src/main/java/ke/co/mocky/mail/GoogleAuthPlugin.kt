package ke.co.mocky.mail

import android.accounts.Account
import android.accounts.AccountManager
import android.app.Activity
import androidx.activity.result.ActivityResult
import androidx.activity.result.IntentSenderRequest
import app.tauri.annotation.ActivityCallback
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import com.google.android.gms.auth.api.identity.AuthorizationRequest
import com.google.android.gms.auth.api.identity.AuthorizationResult
import com.google.android.gms.auth.api.identity.Identity
import com.google.android.gms.common.api.Scope

@InvokeArg
class TokenArgs {
  lateinit var email: String
}

/**
 * Gmail sign-in the Android way: the system "Choose an account" sheet lists
 * the Google accounts already on the phone, then Google Play services asks
 * for mail access (once) and hands back an access token. No browser, no
 * local callback page. Later tokens come from the same call without any
 * screen, so the app never stores a refresh token for these accounts.
 *
 * Tokens are returned to Rust only, never to the web page.
 */
@TauriPlugin
class GoogleAuthPlugin(private val activity: Activity) : Plugin(activity) {
  private val scopes = listOf(Scope(GMAIL_SCOPE), Scope("email"))

  /** Pick an account on the phone, then ask for Gmail access to it. */
  @Command
  fun signIn(invoke: Invoke) {
    val chooser = AccountManager.newChooseAccountIntent(
      null, null, arrayOf(GOOGLE_ACCOUNT_TYPE), null, null, null, null,
    )
    startActivityForResult(invoke, chooser, "accountChosen")
  }

  @ActivityCallback
  fun accountChosen(invoke: Invoke, result: ActivityResult) {
    val email = result.data?.getStringExtra(AccountManager.KEY_ACCOUNT_NAME)
    if (result.resultCode != Activity.RESULT_OK || email.isNullOrBlank()) {
      invoke.reject("Sign-in was cancelled.", CANCELLED)
      return
    }
    authorize(invoke, email, interactive = true)
  }

  /** A fresh access token for an account that already granted access. */
  @Command
  fun token(invoke: Invoke) {
    val args = invoke.parseArgs(TokenArgs::class.java)
    authorize(invoke, args.email, interactive = false)
  }

  private fun authorize(invoke: Invoke, email: String, interactive: Boolean) {
    val request = AuthorizationRequest.builder()
      .setRequestedScopes(scopes)
      .setAccount(Account(email, GOOGLE_ACCOUNT_TYPE))
      .build()
    Identity.getAuthorizationClient(activity).authorize(request)
      .addOnSuccessListener { result ->
        val consent = result.pendingIntent
        when {
          !result.hasResolution() -> resolve(invoke, email, result)
          // Background renewal must never pop up a screen.
          !interactive || consent == null -> invoke.reject("Sign in again to reconnect this account.", NEEDS_CONSENT)
          else -> {
            pending = email
            startIntentSenderForResult(
              invoke,
              IntentSenderRequest.Builder(consent.intentSender).build(),
              "consentGiven",
            )
          }
        }
      }
      .addOnFailureListener { error ->
        invoke.reject(error.message ?: "Google sign-in failed.", FAILED)
      }
  }

  /** The account waiting on the consent screen. */
  private var pending: String? = null

  @ActivityCallback
  fun consentGiven(invoke: Invoke, result: ActivityResult) {
    val email = pending
    pending = null
    if (result.resultCode != Activity.RESULT_OK || email == null) {
      invoke.reject("Sign-in was declined.", DENIED)
      return
    }
    try {
      val granted = Identity.getAuthorizationClient(activity).getAuthorizationResultFromIntent(result.data)
      resolve(invoke, email, granted)
    } catch (error: Exception) {
      invoke.reject(error.message ?: "Google sign-in failed.", FAILED)
    }
  }

  private fun resolve(invoke: Invoke, email: String, result: AuthorizationResult) {
    val token = result.accessToken
    if (token.isNullOrEmpty() || !result.grantedScopes.contains(GMAIL_SCOPE)) {
      invoke.reject("Gmail access wasn't granted.", DENIED)
      return
    }
    val out = JSObject()
    out.put("email", email)
    out.put("accessToken", token)
    invoke.resolve(out)
  }

  private companion object {
    const val GOOGLE_ACCOUNT_TYPE = "com.google"
    const val GMAIL_SCOPE = "https://mail.google.com/"
    const val CANCELLED = "cancelled"
    const val DENIED = "denied"
    const val NEEDS_CONSENT = "needs_consent"
    const val FAILED = "failed"
  }
}
