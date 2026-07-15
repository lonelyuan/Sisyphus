package com.sisyphus

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import com.sisyphus.collector.UsageStatsService

class BootReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action == Intent.ACTION_BOOT_COMPLETED) {
            UsageStatsService.start(context)
        }
    }
}
