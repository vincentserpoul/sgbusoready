package com.sgbuscommute

/** JNI surface implemented in Rust (libsgbusoready). */
object CommuteNative {
    init { System.loadLibrary("sgbusoready") }

    /** Live Update body for all active commutes (one per line); "" if none. */
    external fun renderActive(filesDir: String, epochSecs: Long): String

    /** Epoch millis of the next window boundary to wake at, or -1 if none. */
    external fun nextAlarmEpochMillis(filesDir: String, epochSecs: Long): Long

    /** Implemented in Rust: deliver a native time-picker result to the editor. */
    external fun onTimePicked(tag: String, hour: Int, minute: Int)

    /** Implemented in Rust: handle system Back. Returns true if consumed
     *  (navigated to the list); false to let the app finish. */
    external fun onBackPressed(): Boolean
}
