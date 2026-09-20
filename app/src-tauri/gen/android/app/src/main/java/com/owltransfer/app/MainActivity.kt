package com.owltransfer.app

import android.content.Context
import android.content.Intent
import android.net.Uri
import android.net.wifi.WifiManager
import android.os.Build
import android.os.Bundle
import android.os.Environment
import android.provider.OpenableColumns
import android.util.Log
import androidx.activity.enableEdgeToEdge
import java.io.File
import java.io.FileOutputStream
import java.io.IOException
import org.json.JSONException
import org.json.JSONObject

private const val TAG = "owl-transfer"

/**
 * Where files land when nothing has written settings.json yet. The Rust side
 * uses the same default, so the two agree before anything has been saved.
 */
private const val DEFAULT_FOLDER = "/storage/emulated/0/OwlTransfer"

/**
 * The activity, plus the two things the engine cannot do for itself on Android.
 *
 * The first is the multicast lock. Android's Wi-Fi driver drops broadcast and
 * multicast frames that are not addressed to this device, which is every
 * discovery beacon, unless something holds a MulticastLock.
 *
 * The second is the share sheet. A file shared to the app arrives as a
 * content:// URI that only this process can read, so it is copied into the sync
 * folder here and the engine's watcher picks it up from there.
 */
class MainActivity : TauriActivity() {
  private var multicastLock: WifiManager.MulticastLock? = null

  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    acquireMulticastLock()
    handleShare(intent)
  }

  /** A share while the app is already open. The activity is singleTask. */
  override fun onNewIntent(intent: Intent) {
    super.onNewIntent(intent)
    handleShare(intent)
  }

  override fun onDestroy() {
    multicastLock?.let { if (it.isHeld) it.release() }
    multicastLock = null
    super.onDestroy()
  }

  private fun acquireMulticastLock() {
    try {
      val wifi = applicationContext.getSystemService(Context.WIFI_SERVICE) as WifiManager
      multicastLock = wifi.createMulticastLock("owl-transfer-discovery").apply {
        // Not reference counted: this is taken once and released once, and a
        // counted lock released more often than it was taken throws.
        setReferenceCounted(false)
        acquire()
      }
      Log.i(TAG, "multicast lock held, so discovery beacons will be delivered")
    } catch (e: SecurityException) {
      // Discovery stops working, pairing by address still does, so this is
      // worth a line in the log rather than a crash.
      Log.w(TAG, "could not take the multicast lock, so nearby devices will not be found", e)
    }
  }

  private fun handleShare(intent: Intent?) {
    val uris = sharedUris(intent ?: return)
    if (uris.isEmpty()) return

    val folder = File(syncFolder())
    if (!storageIsReachable(folder)) return

    // Copying can be tens of megabytes, and this is the thread that draws.
    Thread { uris.forEach { copyIntoFolder(it, folder) } }.start()
  }

  @Suppress("DEPRECATION")
  private fun sharedUris(intent: Intent): List<Uri> = when (intent.action) {
    Intent.ACTION_SEND -> listOfNotNull(intent.getParcelableExtra(Intent.EXTRA_STREAM) as? Uri)
    Intent.ACTION_SEND_MULTIPLE ->
      intent.getParcelableArrayListExtra<Uri>(Intent.EXTRA_STREAM).orEmpty().filterNotNull()
    else -> emptyList()
  }

  /**
   * Whether there is any point in trying to write. Without all files access the
   * folder is not ours to touch, and the app already shows a card explaining
   * that, so this only has to say why in the log and stop.
   */
  private fun storageIsReachable(folder: File): Boolean {
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R && !Environment.isExternalStorageManager()) {
      Log.i(TAG, "ignoring the shared files: all files access has not been granted yet")
      return false
    }
    if (!folder.isDirectory && !folder.mkdirs()) {
      Log.w(TAG, "ignoring the shared files: cannot create $folder")
      return false
    }
    return true
  }

  /**
   * Copies one shared stream in under its display name.
   *
   * It is written to a .owl-tmp- name and renamed, which is what the engine
   * does for a download: the watcher ignores .owl-tmp-*, so a file only ever
   * appears in the index whole.
   */
  private fun copyIntoFolder(uri: Uri, folder: File) {
    val name = displayName(uri)
    val tmp = File(folder, ".owl-tmp-$name")
    try {
      val stream = contentResolver.openInputStream(uri)
      if (stream == null) {
        Log.w(TAG, "nothing to read from $uri")
        return
      }
      stream.use { input -> FileOutputStream(tmp).use { output -> input.copyTo(output) } }
      val target = File(folder, name)
      if (tmp.renameTo(target)) {
        Log.i(TAG, "shared file copied to $target")
      } else {
        Log.w(TAG, "could not rename $tmp to $target")
        tmp.delete()
      }
    } catch (e: IOException) {
      Log.w(TAG, "could not copy $uri into $folder", e)
      tmp.delete()
    } catch (e: SecurityException) {
      Log.w(TAG, "not allowed to read $uri", e)
      tmp.delete()
    }
  }

  /** The name the sending app gave the file, with any path separator dropped. */
  private fun displayName(uri: Uri): String {
    try {
      contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)
        ?.use { cursor ->
          val column = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME)
          if (column >= 0 && cursor.moveToFirst()) {
            val name = cursor.getString(column)?.substringAfterLast('/')
            if (!name.isNullOrBlank()) return name
          }
        }
    } catch (e: SecurityException) {
      Log.w(TAG, "could not read the name of $uri", e)
    }
    return "shared-${System.currentTimeMillis()}"
  }

  /**
   * The sync folder, read from the file the Rust side writes.
   *
   * settings.json lives in filesDir and holds { "folder": ..., "device_name":
   * ... }. Reading it here rather than asking the engine means a share works on
   * the very first launch, before any of the Rust side is awake.
   */
  private fun syncFolder(): String {
    val file = File(filesDir, "settings.json")
    if (!file.isFile) return DEFAULT_FOLDER
    return try {
      JSONObject(file.readText()).optString("folder", DEFAULT_FOLDER).ifBlank { DEFAULT_FOLDER }
    } catch (e: JSONException) {
      Log.w(TAG, "settings.json is not readable, using $DEFAULT_FOLDER", e)
      DEFAULT_FOLDER
    } catch (e: IOException) {
      Log.w(TAG, "settings.json could not be read, using $DEFAULT_FOLDER", e)
      DEFAULT_FOLDER
    }
  }
}
