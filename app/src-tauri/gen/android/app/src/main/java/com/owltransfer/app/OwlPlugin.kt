package com.owltransfer.app

import android.app.Activity
import android.content.ActivityNotFoundException
import android.content.Intent
import android.graphics.drawable.ColorDrawable
import android.net.Uri
import android.os.Environment
import android.provider.DocumentsContract
import android.provider.Settings
import android.util.Log
import android.webkit.MimeTypeMap
import android.widget.Toast
import androidx.core.content.ContextCompat
import androidx.core.content.FileProvider
import androidx.core.view.WindowCompat
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import java.io.File

/** The one argument `displayName` takes: the content URI to look up. */
@InvokeArg
class UriArgs {
  lateinit var uri: String
}

/** The one argument `openPath` takes: the absolute path of the file to open. */
@InvokeArg
class PathArgs {
  lateinit var path: String
}

/** The one argument `setWindowTheme` takes: whether the page is dark. */
@InvokeArg
class ThemeArgs {
  var dark: Boolean = false
}

/**
 * All files access, asked about and asked for.
 *
 * The sync folder is in shared storage so that every file manager on the phone
 * can see it, which needs MANAGE_EXTERNAL_STORAGE. That permission has no
 * runtime dialog: the only way to get it is to send the person to a system
 * settings screen and let them turn it on. So the interface needs to be able to
 * ask whether it is on, and to open that screen. Both are here because neither
 * is reachable from Rust.
 *
 * The third is the name of a picked file. The file picker hands the interface a
 * content:// URI, which carries no name; the name is a column in the content
 * resolver, which is Kotlin's to read.
 *
 * The fourth is opening a file and the fifth is opening a folder, both of which
 * need a URI of our own making, and the sixth is the window background, which
 * is the only part of the screen the web layer does not paint.
 *
 * The Rust half is app/src-tauri/src/android.rs.
 */
@TauriPlugin
class OwlPlugin(private val activity: Activity) : Plugin(activity) {

  /** "granted" or "denied". minSdk is 30, so the permission always exists. */
  @Command
  fun allFilesPermission(invoke: Invoke) {
    val result = JSObject()
    result.put("state", if (Environment.isExternalStorageManager()) "granted" else "denied")
    invoke.resolve(result)
  }

  /**
   * The name the sending application published for a content:// URI.
   *
   * Resolves with `{ "name": null }` rather than rejecting when there is no
   * name to be had: the file still gets imported, under a made up name, and a
   * missing display name is not a failure worth stopping an import over.
   */
  @Command
  fun displayName(invoke: Invoke) {
    val args = invoke.parseArgs(UriArgs::class.java)
    val result = JSObject()
    result.put("name", displayName(activity.contentResolver, Uri.parse(args.uri)))
    invoke.resolve(result)
  }

  /**
   * Opens a file in the sync folder with whatever application handles it.
   *
   * Another application cannot read /storage/emulated/0/OwlTransfer/... by
   * path, and handing it a file:// URI throws FileUriExposedException, so the
   * file goes out as a FileProvider content URI with read permission granted
   * for the life of the intent. The provider and the paths it may serve are in
   * AndroidManifest.xml and res/xml/file_paths.xml.
   *
   * A chooser rather than a bare ACTION_VIEW: a person opening a file from a
   * sync folder often wants a particular application rather than whichever one
   * claimed the type first.
   */
  @Command
  fun openPath(invoke: Invoke) {
    val args = invoke.parseArgs(PathArgs::class.java)
    val file = File(args.path)

    val uri = try {
      FileProvider.getUriForFile(activity, "${activity.packageName}.fileprovider", file)
    } catch (e: IllegalArgumentException) {
      // The path is outside every <paths> entry the provider declares, which
      // means the sync folder has been moved somewhere it cannot serve.
      Log.w(TAG, "cannot share $file through the file provider", e)
      invoke.reject("that file is somewhere OwlTransfer cannot share from")
      return
    }

    val view = Intent(Intent.ACTION_VIEW).apply {
      setDataAndType(uri, mimeTypeOf(file.name))
      addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
    }
    // The flag has to be on the chooser as well: it is the intent the system
    // actually starts, and the grant travels with it.
    val chooser = Intent.createChooser(view, null).apply {
      addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
    }

    try {
      activity.startActivity(chooser)
    } catch (e: ActivityNotFoundException) {
      Log.w(TAG, "nothing on this phone opens ${file.name}", e)
      invoke.reject("nothing on this phone opens that kind of file")
      return
    }
    invoke.resolve(JSObject())
  }

  /**
   * Opens a folder in whatever the phone uses as a file manager.
   *
   * Three ways down, because no one of them works on every phone. The system
   * documents provider has a name for everything in shared storage, and the
   * stock Files application opens a document URI built from that name, which
   * is the one that lands the person inside the folder. Failing that, the
   * folder goes out as a FileProvider URI, which some third party managers
   * take. Failing that too, the path itself is worth more than silence, so it
   * goes in a toast for the person to type into whatever they do have.
   *
   * Resolving in the last case is deliberate: the app has said where the
   * folder is, which is all it promised, and an error toast on top of the one
   * carrying the answer would only be in the way.
   */
  @Command
  fun openFolder(invoke: Invoke) {
    val args = invoke.parseArgs(PathArgs::class.java)
    val folder = File(args.path)

    if (!openAsDocument(folder) && !openAsProvidedFile(folder)) {
      Log.i(TAG, "nothing on this phone opens a folder, showing the path instead")
      activity.runOnUiThread {
        Toast.makeText(activity, folder.absolutePath, Toast.LENGTH_LONG).show()
      }
    }
    invoke.resolve(JSObject())
  }

  /**
   * Paints the window background the colour the page is drawn on.
   *
   * The web layer is padded in by the system bar insets, so the strips behind
   * the status bar and the gesture handle are this. The theme resource follows
   * the system's dark mode, which is the right guess before the page has
   * painted and the wrong one afterwards for anyone who chose otherwise.
   *
   * The clock, the signal icons and the gesture handle are drawn by the system
   * on top of those strips, and the system is told which way round they go
   * separately from the colour. Without that, a light phone showing a dark
   * page draws dark icons on the dark strip and the clock disappears.
   */
  @Command
  fun setWindowTheme(invoke: Invoke) {
    val args = invoke.parseArgs(ThemeArgs::class.java)
    val colour = ContextCompat.getColor(
      activity,
      if (args.dark) R.color.owl_ground_dark else R.color.owl_ground_light,
    )
    activity.runOnUiThread {
      activity.window.setBackgroundDrawable(ColorDrawable(colour))
      val bars = WindowCompat.getInsetsController(activity.window, activity.window.decorView)
      bars.isAppearanceLightStatusBars = !args.dark
      bars.isAppearanceLightNavigationBars = !args.dark
    }
    invoke.resolve(JSObject())
  }

  /**
   * Opens the system screen that grants all files access.
   *
   * It returns as soon as the screen has been asked for, not when the person
   * decides, so the caller asks again when the app comes back to the front.
   */
  @Command
  fun openAllFilesSettings(invoke: Invoke) {
    val target = Intent(
      Settings.ACTION_MANAGE_APP_ALL_FILES_ACCESS_PERMISSION,
      Uri.fromParts("package", activity.packageName, null),
    )
    try {
      activity.startActivity(target)
    } catch (e: ActivityNotFoundException) {
      // Some builds have no per app screen. The list of every app that can ask
      // is the next best thing; the person finds OwlTransfer in it.
      Log.w(TAG, "no per app screen for all files access, opening the full list", e)
      try {
        activity.startActivity(Intent(Settings.ACTION_MANAGE_ALL_FILES_ACCESS_PERMISSION))
      } catch (e2: ActivityNotFoundException) {
        invoke.reject("this device has no screen for granting all files access")
        return
      }
    }
    invoke.resolve(JSObject())
  }

  /**
   * The folder as the system documents provider names it.
   *
   * Everything in primary shared storage has a document id of the form
   * "primary:<path under /storage/emulated/0>", and the stock Files
   * application opens a view of one. A folder somewhere else, which a sync
   * folder moved onto an SD card would be, has no such name here and falls
   * through to the next way down.
   */
  private fun openAsDocument(folder: File): Boolean {
    val relative = underPrimaryStorage(folder) ?: return false
    val uri = DocumentsContract.buildDocumentUri(EXTERNAL_STORAGE, "primary:$relative")
    return start(viewOf(uri, DIRECTORY_TYPE))
  }

  /** The folder through this app's own provider, for a manager that takes one. */
  private fun openAsProvidedFile(folder: File): Boolean {
    val uri = try {
      FileProvider.getUriForFile(activity, "${activity.packageName}.fileprovider", folder)
    } catch (e: IllegalArgumentException) {
      Log.w(TAG, "cannot share $folder through the file provider", e)
      return false
    }
    return start(viewOf(uri, DIRECTORY_TYPE))
  }

  /**
   * The path under /storage/emulated/0, with forward slashes, or null when the
   * folder is not under it at all. The storage root itself is the empty
   * string, which names the whole of primary storage.
   */
  private fun underPrimaryStorage(folder: File): String? {
    val root = Environment.getExternalStorageDirectory().absolutePath
    val path = folder.absolutePath
    if (path == root) return ""
    if (!path.startsWith("$root/")) return null
    return path.substring(root.length + 1)
  }

  private fun viewOf(uri: Uri, type: String): Intent =
    Intent(Intent.ACTION_VIEW).apply {
      setDataAndType(uri, type)
      addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
    }

  /** Starts an intent, answering whether anything took it. */
  private fun start(intent: Intent): Boolean =
    try {
      activity.startActivity(intent)
      true
    } catch (e: ActivityNotFoundException) {
      Log.i(TAG, "nothing handled ${intent.data}", e)
      false
    }

  /**
   * What kind of file this is, as far as the extension says.
   *
   * MimeTypeMap wants a lowercase extension and nothing else; a name with a
   * space in it defeats getFileExtensionFromUrl, so the extension is taken
   * here. Anything unrecognised goes out as star slash star, which makes the
   * chooser offer everything rather than nothing.
   */
  private fun mimeTypeOf(name: String): String {
    val extension = name.substringAfterLast('.', "").lowercase()
    if (extension.isEmpty()) return ANY_TYPE
    return MimeTypeMap.getSingleton().getMimeTypeFromExtension(extension) ?: ANY_TYPE
  }

  private companion object {
    const val TAG = "owl-transfer"
    const val ANY_TYPE = "*/*"

    /** The provider that names everything in shared storage. */
    const val EXTERNAL_STORAGE = "com.android.externalstorage.documents"

    /** What a file manager looks for when it is asked to show a folder. */
    const val DIRECTORY_TYPE = "vnd.android.document/directory"
  }
}
