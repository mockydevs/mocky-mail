package ke.co.mocky.mail

import android.app.Activity
import android.content.Intent
import android.net.Uri
import android.provider.OpenableColumns
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSArray
import app.tauri.plugin.JSObject
import java.io.File
import java.io.IOException
import java.util.UUID
import java.util.concurrent.Executors

/**
 * Mail, calendar and contact files (.eml, .ics, .vcf) opened with the app
 * or shared to it. MainActivity hands each intent here; the file behind its
 * `content://` link is copied into the app's cache (shared-files/), and
 * Rust takes the copies through FilesPlugin.waitSharedFiles, which waits
 * until there is one. Nothing else is read: only the link the intent names,
 * of these kinds, up to 100 MB.
 */
object SharedFiles {
  private const val MAX_BYTES = 100L * 1024 * 1024
  private const val KEEP_MS = 24L * 60 * 60 * 1000

  private val io = Executors.newSingleThreadExecutor()
  private val lock = Any()
  private val pending = mutableListOf<JSObject>()
  private var waiting: Invoke? = null

  private val byType = mapOf(
    "message/rfc822" to "eml",
    "text/calendar" to "ics",
    "text/x-vcalendar" to "ics",
    "application/ics" to "ics",
    "text/vcard" to "vcf",
    "text/x-vcard" to "vcf",
    "text/directory" to "vcf",
  )
  private val byExtension = mapOf("eml" to "eml", "ics" to "ics", "ical" to "ics", "vcf" to "vcf", "vcard" to "vcf")

  /** The files an intent (VIEW, SEND, SEND_MULTIPLE) hands over, if any. */
  fun receive(activity: Activity, intent: Intent?) {
    intent ?: return
    val uris: List<Uri> = when (intent.action) {
      Intent.ACTION_VIEW -> listOfNotNull(intent.data).filter { it.scheme == "content" }
      Intent.ACTION_SEND -> listOfNotNull(stream(intent))
      Intent.ACTION_SEND_MULTIPLE -> streams(intent)
      else -> emptyList()
    }
    if (uris.isEmpty()) return
    val type = intent.type
    io.execute {
      cleanUp(activity)
      for (uri in uris) {
        runCatching { copy(activity, uri, type) }.getOrNull()?.let(::add)
      }
    }
  }

  /** Waits for files (or takes those already waiting): FilesPlugin.waitSharedFiles. */
  fun wait(invoke: Invoke) {
    synchronized(lock) {
      if (pending.isNotEmpty()) {
        val files = JSArray()
        pending.forEach { files.put(it) }
        pending.clear()
        invoke.resolve(JSObject().put("files", files))
      } else {
        waiting = invoke
      }
    }
  }

  private fun add(file: JSObject) {
    synchronized(lock) {
      val w = waiting
      if (w != null) {
        waiting = null
        w.resolve(JSObject().put("files", JSArray().put(file)))
      } else {
        pending.add(file)
      }
    }
  }

  @Suppress("DEPRECATION")
  private fun stream(intent: Intent): Uri? = intent.getParcelableExtra(Intent.EXTRA_STREAM) as? Uri

  @Suppress("DEPRECATION")
  private fun streams(intent: Intent): List<Uri> =
    intent.getParcelableArrayListExtra<Uri>(Intent.EXTRA_STREAM)?.filterNotNull() ?: emptyList()

  /** Copies left from earlier days are removed. */
  private fun cleanUp(activity: Activity) {
    val root = File(activity.cacheDir, "shared-files")
    val old = System.currentTimeMillis() - KEEP_MS
    root.listFiles()?.filter { it.lastModified() < old }?.forEach { it.deleteRecursively() }
  }

  /** Copy one file into cache/shared-files/<random>/<its name>. */
  private fun copy(activity: Activity, uri: Uri, intentType: String?): JSObject? {
    val resolver = activity.contentResolver
    var name = "file"
    resolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use { cursor ->
      if (cursor.moveToFirst() && !cursor.isNull(0)) name = cursor.getString(0)
    }
    name = name.replace('/', '_').replace('\\', '_').trim().ifEmpty { "file" }
    val type = (resolver.getType(uri) ?: intentType ?: "").lowercase().substringBefore(';').trim()
    val ext = name.substringAfterLast('.', "").lowercase()
    val kind = byExtension[ext] ?: byType[type] ?: return null
    if (byExtension[ext] == null) name = "$name.$kind"
    val dir = File(activity.cacheDir, "shared-files/${UUID.randomUUID()}")
    if (!dir.mkdirs()) throw IOException("no space in the app's storage")
    val target = File(dir, name)
    val input = resolver.openInputStream(uri) ?: throw IOException("the file is not available")
    input.use { from ->
      target.outputStream().use { to ->
        val buffer = ByteArray(64 * 1024)
        var total = 0L
        while (true) {
          val n = from.read(buffer)
          if (n < 0) break
          total += n
          if (total > MAX_BYTES) {
            to.close()
            dir.deleteRecursively()
            return null
          }
          to.write(buffer, 0, n)
        }
      }
    }
    return JSObject().put("path", target.absolutePath).put("name", name).put("kind", kind)
  }
}
