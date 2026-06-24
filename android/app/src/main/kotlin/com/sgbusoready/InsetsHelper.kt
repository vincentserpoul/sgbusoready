package com.sgbusoready

import android.content.Context

/** Exposes system-bar insets to the Rust UI (the app draws edge-to-edge). */
object InsetsHelper {
    /** Status-bar height in dp (Slint's logical unit on Android ≈ dp). */
    @JvmStatic
    fun statusBarTopDp(context: Context): Int {
        val res = context.resources
        val id = res.getIdentifier("status_bar_height", "dimen", "android")
        val px = if (id > 0) res.getDimensionPixelSize(id) else 0
        val density = res.displayMetrics.density
        return if (density > 0f) Math.round(px / density) else px
    }
}
