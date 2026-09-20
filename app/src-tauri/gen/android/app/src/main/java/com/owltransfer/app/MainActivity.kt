package com.owltransfer.app

import android.content.ContentResolver
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.net.wifi.WifiManager
import android.os.Bundle
import android.os.Environment
import android.provider.OpenableColumns
import android.util.Log
import androidx.activity.enableEdgeToEdge
import java.io.File
import java.io.FileOutputStream
import java.io.IOException
import java.util.concurrent.ExecutorService
import java.util.concurrent.Executors
import org.json.JSONException
import org.json.JSONObject

private const val TAG = "owl-transfer"

/**
 * Where files land when nothing has written settings.json yet. The Rust side
 * uses the same default, so the two agree before anything has been saved.
 */
private const val DEFAULT_FOLDER = "/storage/emulated/0/OwlTransfer"

/** How many " (n)" suffixes to try before giving up on a colliding name. */
private const val MAX_NAME_ATTEMPTS = 1000

/**
 * One thread for every share in the process. Copies never interleave, so two
 * files sharing a display name cannot pick the same free name, and a long copy
 * cannot start a second one beside it.
 */
private val copies: ExecutorService = Executors.newSingleThreadExecutor()

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
 *
 * The copying itself lives in the file level functions below rather than in
 * this class, so that a copy in flight holds on to a resolver and two paths
 * instead of to an activity that may already have been destroyed.
 */
class MainActivity : TauriActivity() {
  private var multicastLock: WifiManager.MulticastLock? = null

  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    acquireMulticastLock()

    // Only on a genuinely new launch. Android keeps the launching intent when a
    // task is restored from recents after process death, so handling it
    // unconditionally would copy a shared file again every time the app came
    // back.
    if (savedInstanceState == null) {
      handleShare(intent)
    }
  }

  /** A share while the app is already open. The activity is singleTask. */
  override fun onNewIntent(intent: Intent) {
    super.onNewIntent(intent)
    // So that getIntent() stops returning the launching intent, which is what
    // anything reading it later would expect.
    setIntent(intent)
    handleShare(intent)
  }

  override fun onDestroy() {
    multicastLock?.let { if (it.isHeld) it.release() }
    multicastLock = null
    super.onDestroy()
  }

  private fun acquireMulticastLock() {
    val wifi = applicationContext.getSystemService(Context.WIFI_SERVICE) as? WifiManager
    if (wifi == null) {
      // A device with no Wi-Fi service, such as some of the TV boxes the
      // leanback filter lets this install on. Discovery stops working there,
      // pairing by address still does.
      Log.w(TAG, "no Wi-Fi service on this device, so nearby devices will not be found")
      return
    }
    try {
      multicastLock = wifi.createMulticastLock("owl-transfer-discovery").apply {
        // Not reference counted: this is taken once and released once, and a
        // counted lock released more often than it was taken throws.
        setReferenceCounted(false)
        acquire()
      }
      Log.i(TAG, "multicast lock held, so discovery beacons will be delivered")
    } catch (e: SecurityException) {
      Log.w(TAG, "could not take the multicast lock, so nearby devices will not be found", e)
    }
  }

  private fun handleShare(intent: Intent?) {
    val uris = sharedUris(intent ?: return)
    if (uris.isEmpty()) return

    // The application's resolver and a plain File, captured before the work is
    // queued: a large copy would otherwise keep a destroyed activity reachable
    // until it finished.
    val resolver = applicationContext.contentResolver
    val settings = File(filesDir, "settings.json")

    // Everything from here touches the file system, including reading
    // settings.json and stat-ing a FUSE backed path, so none of it belongs on
    // the thread that draws.
    copies.execute {
      val folder = File(syncFolder(settings))
      if (!storageIsReachable(folder)) return@execute
      uris.forEach { copyIntoFolder(resolver, it, folder) }
    }
  }

  @Suppress("DEPRECATION")
  private fun sharedUris(intent: Intent): List<Uri> = when (intent.action) {
    Intent.ACTION_SEND -> listOfNotNull(intent.getParcelableExtra(Intent.EXTRA_STREAM) as? Uri)
    Intent.ACTION_SEND_MULTIPLE ->
      intent.getParcelableArrayListExtra<Uri>(Intent.EXTRA_STREAM).orEmpty().filterNotNull()
    else -> emptyList()
  }
}

/**
 * Whether there is any point in trying to write. Without all files access the
 * folder is not ours to touch, and the app already shows a card explaining
 * that, so this only has to say why in the log and stop.
 */
private fun storageIsReachable(folder: File): Boolean {
  if (!Environment.isExternalStorageManager()) {
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
 * It is written to a .owl-tmp- name and renamed, which is what the engine does
 * for a download: the watcher ignores .owl-tmp-*, so a file only ever appears
 * in the index whole. The temp name carries a timestamp because two files
 * shared together can have the same display name, and two copies interleaving
 * into one temp file would give one corrupt file and no error.
 */
private fun copyIntoFolder(resolver: ContentResolver, uri: Uri, folder: File) {
  val name = displayName(resolver, uri)
  val target = freeName(folder, name)
  if (target == null) {
    Log.w(TAG, "too many files in $folder are already called $name")
    return
  }
  val tmp = File(folder, ".owl-tmp-${System.currentTimeMillis()}-$name")
  try {
    val stream = resolver.openInputStream(uri)
    if (stream == null) {
      Log.w(TAG, "nothing to read from $uri")
      return
    }
    stream.use { input -> FileOutputStream(tmp).use { output -> input.copyTo(output) } }
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

/**
 * A name in `folder` that nothing occupies: `name`, else `name (2)` and so on
 * before the extension. Sharing the same photo twice keeps both copies rather
 * than overwriting the first, which would otherwise propagate the overwrite to
 * the other device.
 */
private fun freeName(folder: File, name: String): File? {
  val first = File(folder, name)
  if (!first.exists()) return first

  val dot = name.lastIndexOf('.')
  val stem = if (dot > 0) name.substring(0, dot) else name
  val extension = if (dot > 0) name.substring(dot) else ""
  for (n in 2..MAX_NAME_ATTEMPTS) {
    val candidate = File(folder, "$stem ($n)$extension")
    if (!candidate.exists()) return candidate
  }
  return null
}

/** The name the sending app gave the file, with any path separator dropped. */
private fun displayName(resolver: ContentResolver, uri: Uri): String {
  try {
    resolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use { cursor ->
      val column = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME)
      if (column >= 0 && cursor.moveToFirst()) {
        val name = cursor.getString(column)?.substringAfterLast('/')
        // "." and ".." name a directory rather than a file, so they are not
        // names this can use even though they carry no separator.
        if (!name.isNullOrBlank() && name != "." && name != "..") return name
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
private fun syncFolder(settings: File): String {
  if (!settings.isFile) return DEFAULT_FOLDER
  return try {
    JSONObject(settings.readText()).optString("folder", DEFAULT_FOLDER).ifBlank { DEFAULT_FOLDER }
  } catch (e: JSONException) {
    Log.w(TAG, "settings.json is not readable, using $DEFAULT_FOLDER", e)
    DEFAULT_FOLDER
  } catch (e: IOException) {
    Log.w(TAG, "settings.json could not be read, using $DEFAULT_FOLDER", e)
    DEFAULT_FOLDER
  }
}
