package com.sgbuscommute

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent

/**
 * Fires at a commute window boundary. Starting the service re-evaluates which
 * commutes are active: it re-arms the next alarm and stops itself if nothing is
 * active (window just ended), or begins refreshing (window just opened).
 */
class AlarmReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        CommuteService.start(context)
    }
}
