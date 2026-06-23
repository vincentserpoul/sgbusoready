package com.serpoul.sgbusready

import android.app.AlarmManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent

/** Schedules an exact alarm at the next commute window boundary. */
object AlarmScheduler {
    private const val REQUEST_CODE = 1001

    @JvmStatic
    fun arm(context: Context) {
        val now = System.currentTimeMillis() / 1000
        val at = CommuteNative.nextAlarmEpochMillis(context.filesDir.absolutePath, now)
        if (at < 0) return
        val am = context.getSystemService(Context.ALARM_SERVICE) as AlarmManager
        val pi = PendingIntent.getBroadcast(
            context,
            REQUEST_CODE,
            Intent(context, AlarmReceiver::class.java),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )
        // USE_EXACT_ALARM (manifest) lets an alarm-clock-style app schedule exact
        // alarms without the user toggle. Fall back if it's somehow unavailable.
        if (am.canScheduleExactAlarms()) {
            am.setExactAndAllowWhileIdle(AlarmManager.RTC_WAKEUP, at, pi)
        } else {
            am.setAndAllowWhileIdle(AlarmManager.RTC_WAKEUP, at, pi)
        }
    }
}
