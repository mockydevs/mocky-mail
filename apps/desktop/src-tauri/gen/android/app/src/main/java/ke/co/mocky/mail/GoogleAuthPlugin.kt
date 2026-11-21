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
import com.google.android.gms.common.api.ApiException
import com.google.android.gms.common.api.CommonStatusCodes
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
      .addOnFailureListener { error -> rejectFor(invoke, error) }
  }

  /**
   * Turn a Play services failure into something the user can act on. The
   * status code is the only clue to set-up problems, which otherwise look
   * exactly like the user saying no.
   */
  private fun rejectFor(invoke: Invoke, error: Exception) {
    val status = (error as? ApiException)?.statusCode
    when (status) {
      CommonStatusCodes.CANCELED -> invoke.reject("Sign-in was cancelled.", CANCELLED)
      CommonStatusCodes.DEVELOPER_ERROR -> invoke.reject(
        "Google refused this copy of Mocky Mail (code 10). Its Android sign-in registration doesn't match this app's package name and signing certificate.",
        NOT_SET_UP,
      )
      CommonStatusCodes.NETWORK_ERROR -> invoke.reject("Google couldn't be reached. Check the connection and try again.", FAILED)
      else -> {
        val code = status?.let { " (code $it)" } ?: ""
        invoke.reject("${error.message ?: "Google sign-in failed."}$code", FAILED)
      }
    }
  }

  /** The account waiting on the consent screen. */
  private var pending: String? = null

  @ActivityCallback
  fun consentGiven(invoke: Invoke, result: ActivityResult) {
    val email = pending
    pending = null
    if (email == null) {
      invoke.reject("Sign-in was cancelled.", CANCELLED)
      return
    }
    // Read the result even when the screen closed without "OK": it carries
    // the reason (a set-up error closes it at once, before the user sees a
    // thing), which a bare "declined" would hide.
    try {
      val granted = Identity.getAuthorizationClient(activity).getAuthorizationResultFromIntent(result.data)
      resolve(invoke, email, granted)
    } catch (error: Exception) {
      if (error is ApiException || result.resultCode == Activity.RESULT_OK) {
        rejectFor(invoke, error)
      } else {
        invoke.reject("Mail access wasn't allowed on Google's screen, so Mocky Mail can't read this mailbox.", DENIED)
      }
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
    const val NOT_SET_UP = "not_set_up"
    const val FAILED = "failed"
  }
}
