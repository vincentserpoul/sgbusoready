package com.sgbuscommute

import android.Manifest
import android.app.NativeActivity
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import android.window.OnBackInvokedDispatcher

/**
 * NativeActivity subclass that (1) requests the notification permission and
 * (2) routes the system Back action (button or predictive-back gesture) into the
 * Slint UI: on a sub-screen (editor/search) it navigates back to the commute
 * list; only on the list does Back background the app. Slint's android-activity
 * backend can't handle Back itself (slint-ui/slint#8323), so we intercept here.
 */
class MainActivity : NativeActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        requestNotificationPermission()
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            onBackInvokedDispatcher.registerOnBackInvokedCallback(
                OnBackInvokedDispatcher.PRIORITY_DEFAULT,
            ) {
                // Rust returns true if it consumed Back (navigated to the list);
                // otherwise behave like Back on a root screen: go to the home
                // screen (backgrounding the app) rather than trapping the user.
                if (!CommuteNative.onBackPressed()) {
                    moveTaskToBack(true)
                }
            }
        }
    }

    /** Android 13+ needs runtime consent to post notifications (the Live Update). */
    private fun requestNotificationPermission() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU &&
            checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) !=
            PackageManager.PERMISSION_GRANTED
        ) {
            requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS), 1001)
        }
    }
}
