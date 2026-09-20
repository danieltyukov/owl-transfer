package com.owltransfer.app

import android.app.Activity
import android.content.ActivityNotFoundException
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.Environment
import android.provider.Settings
import android.util.Log
import app.tauri.annotation.Command
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin

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
 * The Rust half is app/src-tauri/src/android.rs.
 */
@TauriPlugin
class OwlPlugin(private val activity: Activity) : Plugin(activity) {

  /** "granted" or "denied". A phone too old to have the permission has it. */
  @Command
  fun allFilesPermission(invoke: Invoke) {
    val result = JSObject()
    result.put("state", if (hasAllFilesAccess()) "granted" else "denied")
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
    if (Build.VERSION.SDK_INT < Build.VERSION_CODES.R) {
      // Before Android 11 there is no such screen, and no such permission to
      // grant: WRITE_EXTERNAL_STORAGE already covers the folder.
      invoke.resolve(JSObject())
      return
    }

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

  private fun hasAllFilesAccess(): Boolean =
    Build.VERSION.SDK_INT < Build.VERSION_CODES.R || Environment.isExternalStorageManager()

  private companion object {
    const val TAG = "owl-transfer"
  }
}
