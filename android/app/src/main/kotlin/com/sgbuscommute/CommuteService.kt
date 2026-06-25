package com.sgbuscommute

import android.app.Notification
import android.app.NotificationManager
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import androidx.core.app.NotificationCompat

/**
 * Foreground service that refreshes the commute Live Update every ~15s while at
 * least one commute window is open, then stops itself. The blocking LTA fetch
 * runs on a worker thread (never the main thread → no ANR).
 */
class CommuteService : Service() {
    @Volatile private var running = false
    private var worker: Thread? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        NotificationHelper.ensureChannel(this)
        startForegroundCompat(buildNotification("SG Bus Commute", "Updating…"))
        // Re-arm the next boundary (next window start, or this window's end).
        AlarmScheduler.arm(this)
        if (!running) {
            running = true
            worker = Thread { loop() }.also { it.start() }
        }
        return START_STICKY
    }

    private fun loop() {
        val nm = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
        while (running) {
            val now = System.currentTimeMillis() / 1000
            val body = CommuteNative.renderActive(filesDir.absolutePath, now)
            if (body.isEmpty()) {
                stopSelf()
                break
            }
            nm.notify(NotificationHelper.NOTIF_ID, buildNotification("Next buses", body))
            try {
                Thread.sleep(REFRESH_MS)
            } catch (e: InterruptedException) {
                break
            }
        }
        running = false
    }

    private fun buildNotification(title: String, text: String): Notification =
        NotificationCompat.Builder(this, NotificationHelper.CHANNEL_ID)
            .setContentTitle(title)
            .setStyle(NotificationCompat.BigTextStyle().bigText(text))
            .setContentText(text.substringBefore('\n'))
            .setSmallIcon(android.R.drawable.ic_dialog_info)
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            // Android 16 Live Update: surface as a status-bar/lock-screen chip.
            // NotificationCompat no-ops these on pre-36 devices (plain ongoing).
            .setRequestPromotedOngoing(true)
            .setShortCriticalText(chipText(text))
            .build()

    /**
     * Compact status-bar chip. The body is two lines per stop (name, then
     * arrivals), so the soonest arrival is the first segment of the 2nd line,
     * e.g. "Opp Blk 123\n2m (14), 4m (14e)" -> "2m (14)".
     */
    private fun chipText(body: String): String {
        val lines = body.split('\n')
        val arrivals = lines.getOrNull(1).orEmpty()
        return arrivals.substringBefore(", ").ifEmpty { lines.firstOrNull().orEmpty() }
    }

    private fun startForegroundCompat(notif: Notification) {
        if (Build.VERSION.SDK_INT >= 29) {
            startForeground(NotificationHelper.NOTIF_ID, notif, ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC)
        } else {
            startForeground(NotificationHelper.NOTIF_ID, notif)
        }
    }

    override fun onDestroy() {
        running = false
        worker?.interrupt()
    }

    override fun onBind(intent: Intent?): IBinder? = null

    companion object {
        private const val REFRESH_MS = 15_000L

        // @JvmStatic so the Rust JNI bridge can call it as a real static method.
        @JvmStatic
        fun start(context: Context) {
            context.startForegroundService(Intent(context, CommuteService::class.java))
        }
    }
}
