package com.sgbusoready

import android.app.NativeActivity
import android.os.Build
import android.os.Bundle
import android.window.OnBackInvokedDispatcher

/**
 * NativeActivity subclass that routes the system Back action (button or
 * predictive-back gesture) into the Slint UI: if the app is on a sub-screen
 * (editor/search) it navigates back to the commute list; only on the list does
 * Back fall through to the default (finish the app). Slint's android-activity
 * backend can't handle Back itself (slint-ui/slint#8323), so we intercept here.
 */
class MainActivity : NativeActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
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
}
