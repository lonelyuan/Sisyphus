package com.sisyphus.notification

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import com.sisyphus.NotificationPlugin

class NotificationActionReceiver : BroadcastReceiver() {

    override fun onReceive(context: Context, intent: Intent) {
        val action         = intent.getStringExtra(EXTRA_ACTION)          ?: return
        val interventionId = intent.getStringExtra(EXTRA_INTERVENTION_ID) ?: return
        // 通过 NotificationPlugin 将按钮点击事件传回 JS 层
        NotificationPlugin.instance?.emitActionTaken(interventionId, action)
    }

    companion object {
        const val EXTRA_ACTION          = "action"
        const val EXTRA_INTERVENTION_ID = "intervention_id"
    }
}
