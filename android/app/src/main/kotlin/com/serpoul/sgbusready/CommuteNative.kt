package com.serpoul.sgbusready

/** JNI surface implemented in Rust (libsgbusoready). */
object CommuteNative {
    init { System.loadLibrary("sgbusoready") }

    /** Live Update body for all active commutes (one per line); "" if none. */
    external fun renderActive(filesDir: String, epochSecs: Long): String

    /** Epoch millis of the next window boundary to wake at, or -1 if none. */
    external fun nextAlarmEpochMillis(filesDir: String, epochSecs: Long): Long
}
