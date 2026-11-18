package ke.co.mocky.mail

import android.app.Activity
import android.content.ActivityNotFoundException
import android.content.Intent
import android.net.Uri
import android.provider.OpenableColumns
import androidx.activity.result.ActivityResult
import androidx.core.content.FileProvider
import app.tauri.annotation.ActivityCallback
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSArray
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import java.io.File
import java.io.IOException
import java.util.UUID
import java.util.concurrent.Executors

@InvokeArg
class PickArgs {
  var maxBytes: Long = 0
}

@InvokeArg
class FileArgs {
  lateinit var path: String
  var fileName: String = ""
  var mimeType: String = "application/octet-stream"
}

/**
 * Files the Android way. The phone's file screens hand back `content://`
 * links, not paths, so picking, saving and opening attachments go through
 * here:
 *
 * - pickFiles: the system file picker; each chosen file is copied into the
 *   app's cache so Rust can read it like any other file.
 * - saveFile: the system "Save to" screen, then the file is copied there.
 * - openFile: hands a file to another app (PDF reader, Word, Gallery) with
 *   read access to that one file only.
 *
 * Only files inside the app's own cache are ever read, saved or opened.
 * Called from Rust only.
 */
@TauriPlugin
class FilesPlugin(private val activity: Activity) : Plugin(activity) {
  private val io = Executors.newSingleThreadExecutor()
  private var pickLimit: Long = 0
  private var saving: File? = null

  @Command
  fun pickFiles(invoke: Invoke) {
    pickLimit = invoke.parseArgs(PickArgs::class.java).maxBytes
    val intent = Intent(Intent.ACTION_OPEN_DOCUMENT)
      .addCategory(Intent.CATEGORY_OPENABLE)
      .setType("*/*")
      .putExtra(Intent.EXTRA_ALLOW_MULTIPLE, true)
    startActivityForResult(invoke, intent, "filesPicked")
  }

  @ActivityCallback
  fun filesPicked(invoke: Invoke, result: ActivityResult) {
    val data = result.data
    if (result.resultCode != Activity.RESULT_OK || data == null) {
      invoke.resolve(JSObject().put("files", JSArray()))
      return
    }
    val uris = buildList {
      data.clipData?.let { clip -> for (i in 0 until clip.itemCount) add(clip.getItemAt(i).uri) }
      if (isEmpty()) data.data?.let(::add)
    }
    io.execute {
      try {
        val files = JSArray()
        for (uri in uris) files.put(copyIn(uri))
        invoke.resolve(JSObject().put("files", files))
      } catch (e: TooLarge) {
        invoke.reject(e.message, "too_large")
      } catch (e: Exception) {
        invoke.reject("That file couldn't be read: ${e.message}")
      }
    }
  }

  private class TooLarge(message: String) : Exception(message)

  /** Copy one picked file into cache/picked/<random>/<its name>. */
  private fun copyIn(uri: Uri): JSObject {
    val resolver = activity.contentResolver
    var name = "attachment"
    resolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use { cursor ->
      if (cursor.moveToFirst() && !cursor.isNull(0)) name = cursor.getString(0)
    }
    name = name.replace('/', '_').replace('\\', '_').trim().ifEmpty { "attachment" }
    val dir = File(activity.cacheDir, "picked/${UUID.randomUUID()}")
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
          if (pickLimit > 0 && total > pickLimit) {
            to.close()
            dir.deleteRecursively()
            throw TooLarge("$name is too large to attach.")
          }
          to.write(buffer, 0, n)
        }
      }
    }
    return JSObject().put("path", target.absolutePath).put("fileName", name)
  }

  @Command
  fun saveFile(invoke: Invoke) {
    val args = invoke.parseArgs(FileArgs::class.java)
    val source = cached(args.path) ?: return invoke.reject("That file isn't available.")
    saving = source
    val intent = Intent(Intent.ACTION_CREATE_DOCUMENT)
      .addCategory(Intent.CATEGORY_OPENABLE)
      .setType(args.mimeType.ifBlank { "application/octet-stream" })
      .putExtra(Intent.EXTRA_TITLE, args.fileName)
    startActivityForResult(invoke, intent, "saveChosen")
  }

  @ActivityCallback
  fun saveChosen(invoke: Invoke, result: ActivityResult) {
    val source = saving
    saving = null
    val uri = result.data?.data
    if (result.resultCode != Activity.RESULT_OK || uri == null || source == null) {
      invoke.resolve(JSObject().put("saved", false))
      return
    }
    io.execute {
      try {
        val out = activity.contentResolver.openOutputStream(uri, "wt")
          ?: throw IOException("that place can't be written to")
        out.use { to -> source.inputStream().use { it.copyTo(to) } }
        var name = source.name
        activity.contentResolver
          .query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)
          ?.use { c -> if (c.moveToFirst() && !c.isNull(0)) name = c.getString(0) }
        invoke.resolve(JSObject().put("saved", true).put("name", name))
      } catch (e: Exception) {
        invoke.reject("The file couldn't be saved: ${e.message}")
      }
    }
  }

  @Command
  fun openFile(invoke: Invoke) {
    val args = invoke.parseArgs(FileArgs::class.java)
    val file = cached(args.path) ?: return invoke.reject("That file isn't available.")
    try {
      val uri = FileProvider.getUriForFile(activity, "${activity.packageName}.fileprovider", file)
      val intent = Intent(Intent.ACTION_VIEW)
        .setDataAndType(uri, args.mimeType.ifBlank { "application/octet-stream" })
        .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
      activity.startActivity(Intent.createChooser(intent, args.fileName.ifBlank { file.name }))
      invoke.resolve()
    } catch (e: ActivityNotFoundException) {
      invoke.reject("No app on this phone can open this file. Save it instead.", "no_app")
    } catch (e: Exception) {
      invoke.reject("The file couldn't be opened: ${e.message}")
    }
  }

  /** The file at `path` if it is inside the app's cache, else null. */
  private fun cached(path: String): File? {
    val cache = activity.cacheDir.canonicalFile
    val file = File(path).canonicalFile
    val inside = file.path.startsWith(cache.path + File.separator)
    return if (inside && file.isFile) file else null
  }
}
