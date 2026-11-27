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
class SignInArgs {
  /** Permissions beyond mail to ask for too (an account signing in again). */
  var scopes: Array<String>? = null
}

@InvokeArg
class TokenArgs {
  lateinit var email: String
  /** Permissions beyond mail (calendar, contacts, tasks) the token must carry. */
  var scopes: Array<String>? = null
  /** Google's consent sheet may be shown (connecting calendar and contacts). */
  var interactive: Boolean = false
}

/**
 * Gmail sign-in the Android way: the system "Choose an account" sheet lists
 * the Google accounts already on the phone, then Google Play services asks
 * for mail access (once) and hands back an access token. No browser, no
 * local callback page. Later tokens come from the same call without any
 * screen, so the app never stores a refresh token for these accounts.
 *
 * Connecting calendar and contacts asks again for the same account with
 * more scopes (incremental authorization): Google's consent sheet shows the
 * new ones only, and the token that comes back covers mail and them. Later
 * tokens are asked for with every scope the account was granted, so one
 * token serves IMAP, CalDAV, CardDAV and Google's APIs.
 *
 * Tokens are returned to Rust only, never to the web page.
 */
@TauriPlugin
class GoogleAuthPlugin(private val activity: Activity) : Plugin(activity) {
  private val mailScopes = listOf(GMAIL_SCOPE, "email")

  /** Mail's scopes and the extra ones, once each. */
  private fun scopesWith(extra: Array<String>?): List<Scope> =
    (mailScopes + (extra?.toList() ?: emptyList()))
      .map { it.trim() }
      .filter { it.isNotEmpty() }
      .distinct()
      .map { Scope(it) }

  /** Extra scopes for the account being chosen. */
  private var chosenExtra: Array<String>? = null

  /** Pick an account on the phone, then ask for Gmail access to it. */
  @Command
  fun signIn(invoke: Invoke) {
    chosenExtra = invoke.parseArgs(SignInArgs::class.java).scopes
    val chooser = AccountManager.newChooseAccountIntent(
      null, null, arrayOf(GOOGLE_ACCOUNT_TYPE), null, null, null, null,
    )
    startActivityForResult(invoke, chooser, "accountChosen")
  }

  @ActivityCallback
  fun accountChosen(invoke: Invoke, result: ActivityResult) {
    val email = result.data?.getStringExtra(AccountManager.KEY_ACCOUNT_NAME)
    val extra = chosenExtra
    chosenExtra = null
    if (result.resultCode != Activity.RESULT_OK || email.isNullOrBlank()) {
      invoke.reject("Sign-in was cancelled.", CANCELLED)
      return
    }
    authorize(invoke, email, interactive = true, extra = extra)
  }

  /**
   * A fresh access token for an account that already granted access (no
   * screen), or with `interactive` and more `scopes`, Google's consent
   * sheet for them first.
   */
  @Command
  fun token(invoke: Invoke) {
    val args = invoke.parseArgs(TokenArgs::class.java)
    authorize(invoke, args.email, interactive = args.interactive, extra = args.scopes)
  }

  private fun authorize(invoke: Invoke, email: String, interactive: Boolean, extra: Array<String>? = null) {
    val request = AuthorizationRequest.builder()
      .setRequestedScopes(scopesWith(extra))
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
    // Which scopes this token carries, so the app knows what was allowed.
    out.put("grantedScopes", org.json.JSONArray(result.grantedScopes))
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
