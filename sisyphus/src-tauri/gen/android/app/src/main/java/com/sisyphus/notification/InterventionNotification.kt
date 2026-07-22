package com.sisyphus.notification

import android.app.Notification
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat

class InterventionNotification(private val context: Context) {

    fun show(message: String, interventionId: String) {
        val notification = NotificationCompat.Builder(context, CHANNEL_INTERVENTION)
            .setSmallIcon(android.R.drawable.ic_dialog_info)
            .setContentTitle("Sisyphus")
            .setContentText(message)
            .setStyle(NotificationCompat.BigTextStyle().bigText(message))
            .setPriority(NotificationCompat.PRIORITY_HIGH)
            .setAutoCancel(true)
            .addAction(buildAction(context, "开始任务",  "start_task",    interventionId))
            .addAction(buildAction(context, "休息一下",  "take_rest",     interventionId))
            .addAction(buildAction(context, "继续娱乐",  "continue",      interventionId))
            .addAction(buildAction(context, "今天放弃",  "abandon_today", interventionId))
            .build()

        try {
            NotificationManagerCompat.from(context).notify(NOTIF_ID_INTERVENTION, notification)
        } catch (_: SecurityException) {
            // POST_NOTIFICATIONS 未授权，静默失败
        }
    }

    /** 到点提醒（简单高优先通知，无干预按钮）。 */
    fun showReminder(text: String) {
        val notification = NotificationCompat.Builder(context, CHANNEL_INTERVENTION)
            .setSmallIcon(android.R.drawable.ic_dialog_info)
            .setContentTitle("⏰ 提醒")
            .setContentText(text)
            .setStyle(NotificationCompat.BigTextStyle().bigText(text))
            .setPriority(NotificationCompat.PRIORITY_HIGH)
            .setAutoCancel(true)
            .build()
        try {
            NotificationManagerCompat.from(context).notify(3000 + (text.hashCode() and 0xffff), notification)
        } catch (_: SecurityException) {
            // POST_NOTIFICATIONS 未授权，静默失败
        }
    }

    fun buildCollectorNotification(): Notification =
        NotificationCompat.Builder(context, CHANNEL_COLLECTOR)
            .setSmallIcon(android.R.drawable.ic_dialog_info)
            .setContentTitle("Sisyphus 运行中")
            .setContentText("正在保护你的专注时间")
            .setPriority(NotificationCompat.PRIORITY_MIN)
            .setOngoing(true)
            .build()

    private fun buildAction(
        context: Context,
        label: String,
        action: String,
        interventionId: String,
    ): NotificationCompat.Action {
        val intent = Intent(context, NotificationActionReceiver::class.java).apply {
            putExtra(NotificationActionReceiver.EXTRA_ACTION, action)
            putExtra(NotificationActionReceiver.EXTRA_INTERVENTION_ID, interventionId)
        }
        val pi = PendingIntent.getBroadcast(
            context,
            (action + interventionId).hashCode(),
            intent,
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )
        return NotificationCompat.Action(0, label, pi)
    }
}
