package com.sgbusoready

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent

/** Re-arm commute alarms after a reboot (alarms don't survive reboot). */
class BootReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action == Intent.ACTION_BOOT_COMPLETED) {
            AlarmScheduler.arm(context)
        }
    }
}
