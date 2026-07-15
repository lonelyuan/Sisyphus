package com.sisyphus.notification

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import androidx.core.app.NotificationManagerCompat
import com.sisyphus.collector.NativeBridge
import java.io.File

/**
 * 干预通知的按钮点击接收器。经 JNI 直写反馈（不依赖 WebView，后台可用），并关掉这条通知。
 */
class NotificationActionReceiver : BroadcastReceiver() {

    override fun onReceive(context: Context, intent: Intent) {
        val action = intent.getStringExtra(EXTRA_ACTION) ?: return
        val interventionId = intent.getStringExtra(EXTRA_INTERVENTION_ID) ?: return
        val dbPath = File(context.applicationContext.dataDir, "sisyphus.db").absolutePath

        NotificationManagerCompat.from(context).cancel(NOTIF_ID_INTERVENTION)

        // DB 写入放后台线程，避免阻塞广播主线程。
        val pending = goAsync()
        Thread {
            try {
                NativeBridge.recordFeedback(dbPath, interventionId, action)
            } catch (_: Throwable) {
                // 忽略
            } finally {
                pending.finish()
            }
        }.start()
    }

    companion object {
        const val EXTRA_ACTION = "action"
        const val EXTRA_INTERVENTION_ID = "intervention_id"
    }
}
