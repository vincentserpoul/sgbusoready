package com.sgbuscommute

import android.app.Activity
import android.app.TimePickerDialog
import android.content.Context
import android.util.Log

/** Shows the native 24-hour time picker; the result is delivered back to Rust
 *  via [CommuteNative.onTimePicked]. Called from Rust over JNI. */
object TimePicker {
    @JvmStatic
    fun show(context: Context, tag: String, hour: Int, minute: Int) {
        val activity = context as? Activity
        if (activity == null) {
            Log.e("sgbr", "TimePicker: context is not an Activity")
            return
        }
        activity.runOnUiThread {
            try {
                TimePickerDialog(
                    activity,
                    { _, h, m -> CommuteNative.onTimePicked(tag, h, m) },
                    hour,
                    minute,
                    true,
                ).show()
            } catch (e: Throwable) {
                Log.e("sgbr", "TimePicker: dialog failed", e)
            }
        }
    }
}
