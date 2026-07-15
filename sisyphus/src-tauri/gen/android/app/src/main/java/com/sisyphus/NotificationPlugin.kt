package com.sisyphus

import android.app.Activity
import android.webkit.WebView
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import com.sisyphus.notification.InterventionNotification

@InvokeArg
class ShowInterventionArgs {
    lateinit var message: String
    lateinit var interventionId: String
}

@TauriPlugin
class NotificationPlugin(private val activity: Activity) : Plugin(activity) {

    override fun load(webView: WebView) {
        instance = this
    }

    /** JS 调用：invoke('plugin:notification|show_intervention', {message, intervention_id}) */
    @Command
    fun showIntervention(invoke: Invoke) {
        try {
            val args = invoke.parseArgs(ShowInterventionArgs::class.java)
            InterventionNotification(activity).show(args.message, args.interventionId)
            invoke.resolve()
        } catch (e: Exception) {
            invoke.reject(e.message, null as JSObject?)
        }
    }

    /** 由 NotificationActionReceiver 调用，将按钮点击事件传回 JS 层。 */
    fun emitActionTaken(interventionId: String, action: String) {
        val data = JSObject().apply {
            put("intervention_id", interventionId)
            put("action", action)
        }
        trigger("action_taken", data)
    }

    companion object {
        var instance: NotificationPlugin? = null
    }
}
