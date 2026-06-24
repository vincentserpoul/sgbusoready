package com.sgbuscommute

import android.app.NotificationChannel
import android.app.NotificationManager
import android.content.Context
import androidx.core.app.NotificationCompat

/** Posts the commute notification. Called from Rust via JNI. */
object NotificationHelper {
    const val CHANNEL_ID = "sgbr_commute"
    const val NOTIF_ID = 1

    @JvmStatic
    fun ensureChannel(context: Context) {
        val nm = context.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
        nm.createNotificationChannel(
            NotificationChannel(
                CHANNEL_ID,
                "Commute arrivals",
                NotificationManager.IMPORTANCE_DEFAULT,
            )
        )
    }

    @JvmStatic
    fun showNow(context: Context, title: String, text: String) {
        ensureChannel(context)
        val nm = context.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
        val notif = NotificationCompat.Builder(context, CHANNEL_ID)
            .setContentTitle(title)
            .setContentText(text)
            .setSmallIcon(android.R.drawable.ic_dialog_info)
            .setOngoing(true)
            .build()
        nm.notify(NOTIF_ID, notif)
    }
}
