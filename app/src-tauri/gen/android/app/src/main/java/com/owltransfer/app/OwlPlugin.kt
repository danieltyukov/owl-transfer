package com.owltransfer.app

import android.app.Activity
import android.content.ActivityNotFoundException
import android.content.Intent
import android.net.Uri
import android.os.Environment
import android.provider.Settings
import android.util.Log
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin

/** The one argument `displayName` takes: the content URI to look up. */
@InvokeArg
class UriArgs {
  lateinit var uri: String
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
      // is the next best thing; the person finds Owl Transfer in it.
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

  private companion object {
    const val TAG = "owl-transfer"
  }
}
