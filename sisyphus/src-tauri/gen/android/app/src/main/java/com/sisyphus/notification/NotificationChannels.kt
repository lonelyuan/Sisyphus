package com.sisyphus.notification

import android.app.NotificationChannel
import android.app.NotificationManager
import android.content.Context

const val CHANNEL_COLLECTOR    = "sisyphus_collector"
const val CHANNEL_INTERVENTION = "sisyphus_intervention"
const val NOTIF_ID_COLLECTOR    = 1001
const val NOTIF_ID_INTERVENTION = 1002

object NotificationChannels {
    fun createAll(context: Context) {
        val mgr = context.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
        mgr.createNotificationChannel(
            NotificationChannel(
                CHANNEL_COLLECTOR,
                "后台采集",
                NotificationManager.IMPORTANCE_MIN
            ).apply { description = "Sisyphus 后台运行指示" }
        )
        mgr.createNotificationChannel(
            NotificationChannel(
                CHANNEL_INTERVENTION,
                "习惯干预",
                NotificationManager.IMPORTANCE_HIGH
            ).apply { description = "专注目标提醒" }
        )
    }
}
