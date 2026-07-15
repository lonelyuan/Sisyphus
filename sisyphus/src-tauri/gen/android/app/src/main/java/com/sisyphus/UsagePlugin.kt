package com.sisyphus

import android.app.Activity
import android.app.AppOpsManager
import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.provider.Settings
import android.webkit.WebView
import app.tauri.annotation.Command
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import com.sisyphus.collector.UsageStatsService

@TauriPlugin
class UsagePlugin(private val activity: Activity) : Plugin(activity) {

    override fun load(webView: WebView) {
        instance = this
    }

    @Command
    fun startCollector(invoke: Invoke) {
        UsageStatsService.start(activity)
        invoke.resolve()
    }

    @Command
    fun stopCollector(invoke: Invoke) {
        UsageStatsService.stop(activity)
        invoke.resolve()
    }

    @Command
    fun checkPermission(invoke: Invoke) {
        val granted = hasUsageStatsPermission(activity)
        invoke.resolve(JSObject().put("granted", granted))
    }

    @Command
    fun requestPermission(invoke: Invoke) {
        // 优先尝试直达本 app 的使用权限页（部分机型/API 支持）
        val directIntent = Intent(Settings.ACTION_USAGE_ACCESS_SETTINGS).apply {
            data = Uri.fromParts("package", activity.packageName, null)
            addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        }
        try {
            activity.startActivity(directIntent)
        } catch (_: ActivityNotFoundException) {
            // 降级到通用使用权限列表页
            activity.startActivity(
                Intent(Settings.ACTION_USAGE_ACCESS_SETTINGS)
                    .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            )
        }
        invoke.resolve()
    }

    /** 由 UsageStatsService 每 10s 调用，将前台 app 信息推送给 JS 层。 */
    fun emitUsageEvent(pkg: String, category: String?, activeMs: Long) {
        val data = JSObject().apply {
            put("pkg", pkg)
            put("category", category ?: "")
            put("active_ms", activeMs)
        }
        trigger("usage_event", data)
    }

    companion object {
        var instance: UsagePlugin? = null

        fun hasUsageStatsPermission(context: Context): Boolean {
            val appOps = context.getSystemService(Context.APP_OPS_SERVICE) as AppOpsManager
            val mode = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                appOps.unsafeCheckOpNoThrow(
                    AppOpsManager.OPSTR_GET_USAGE_STATS,
                    android.os.Process.myUid(),
                    context.packageName
                )
            } else {
                @Suppress("DEPRECATION")
                appOps.checkOpNoThrow(
                    AppOpsManager.OPSTR_GET_USAGE_STATS,
                    android.os.Process.myUid(),
                    context.packageName
                )
            }
            return mode == AppOpsManager.MODE_ALLOWED
        }
    }
}
