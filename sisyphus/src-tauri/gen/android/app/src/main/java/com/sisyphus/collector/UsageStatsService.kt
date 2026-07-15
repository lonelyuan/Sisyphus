package com.sisyphus.collector

import android.app.Service
import android.app.usage.UsageEvents
import android.app.usage.UsageStatsManager
import android.content.Context
import android.content.Intent
import android.os.IBinder
import android.util.Log
import com.sisyphus.UsagePlugin
import com.sisyphus.notification.InterventionNotification
import com.sisyphus.notification.NOTIF_ID_COLLECTOR
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import org.json.JSONObject
import java.io.File

private const val TAG = "Sisyphus"

/**
 * 前台服务：每 10s 轮询 UsageStats，**经 JNI（NativeBridge）直调 Rust core** 评估娱乐规则，
 * 命中就由 Kotlin 直接弹干预通知——全程不依赖 WebView，所以用户在刷短视频（App 后台）时也能弹。
 * Kotlin 只做 OS API 桥接，规则/存储/文案全在 Rust core（单一来源）。
 */
class UsageStatsService : Service() {

    private val serviceScope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val usageStatsManager by lazy {
        getSystemService(Context.USAGE_STATS_SERVICE) as UsageStatsManager
    }
    // 与 Tauri App 同一个 SQLite。Tauri 的 app_data_dir 在 Android = context.getDataDir()
    // (= /data/data/<pkg>)，**不是** filesDir(/files 子目录)——必须一致，否则读到空库、规则永不命中。
    private val dbPath by lazy { File(dataDir, "sisyphus.db").absolutePath }

    // 内存态：当前前台 app、其分类与启动时间（服务重启后从第一次 poll 恢复）
    private var currentPkg = ""
    private var currentCategory = ""
    private var currentStartMs = 0L
    private var lastPollMs = 0L

    override fun onCreate() {
        super.onCreate()
        startForeground(
            NOTIF_ID_COLLECTOR,
            InterventionNotification(this).buildCollectorNotification(),
        )
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val granted = UsagePlugin.hasUsageStatsPermission(this)
        Log.i(TAG, "采集服务启动 dbPath=$dbPath 使用情况权限=$granted")
        if (!granted) {
            Log.w(TAG, "⚠️ 未授予「使用情况访问」权限 → queryEvents 将为空、永远抓不到前台 app。去 设置→前往授权。")
        }
        try {
            Log.i(TAG, "debugState=" + NativeBridge.debugState(dbPath))
        } catch (e: Throwable) {
            Log.e(TAG, "debugState/JNI 调用失败（native 库没加载？）", e)
        }
        serviceScope.launch { runLoop() }
        return START_STICKY
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onDestroy() {
        serviceScope.cancel()
        super.onDestroy()
    }

    private suspend fun runLoop() {
        lastPollMs = System.currentTimeMillis() - POLL_INTERVAL_MS
        while (serviceScope.isActive) {
            try {
                poll()
            } catch (_: Exception) {
                // 不中断循环
            }
            delay(POLL_INTERVAL_MS)
        }
    }

    private fun poll() {
        val nowMs = System.currentTimeMillis()
        val events = usageStatsManager.queryEvents(lastPollMs, nowMs)
        val event = UsageEvents.Event()

        while (events.hasNextEvent()) {
            events.getNextEvent(event)
            val pkg = event.packageName ?: continue
            val ts = event.timeStamp

            when (event.eventType) {
                UsageEvents.Event.ACTIVITY_RESUMED -> {
                    if (currentPkg != pkg) {
                        closeSession(ts)          // 旧会话结束 → 写 Event log
                        currentPkg = pkg
                        currentCategory = ENTERTAINMENT_PACKAGES[pkg] ?: ""
                        currentStartMs = ts
                    }
                }
                UsageEvents.Event.ACTIVITY_PAUSED,
                UsageEvents.Event.ACTIVITY_STOPPED -> {
                    if (currentPkg == pkg) {
                        closeSession(ts)
                        currentPkg = ""
                        currentCategory = ""
                        currentStartMs = 0L
                    }
                }
            }
        }

        lastPollMs = nowMs

        // 进行中的娱乐时长（防漏算：未切走的会话不在 DB，作为 active_ms 注入规则）
        if (currentPkg.isEmpty()) {
            Log.d(TAG, "poll: 无前台 app（可能权限未授予，或本轮无 UsageEvents）")
            return
        }
        val activeMs = if (currentStartMs > 0L) nowMs - currentStartMs else 0L
        Log.d(TAG, "poll front=$currentPkg cat=${currentCategory.ifEmpty { "(非娱乐)" }} active=${activeMs / 1000}s")

        // 经 JNI 评估；命中就直接弹通知（不走 WebView）
        val result = try {
            NativeBridge.evaluate(dbPath, currentPkg, currentCategory, activeMs)
        } catch (e: Throwable) {
            Log.e(TAG, "NativeBridge.evaluate 失败", e)
            ""
        }
        if (result.isNotEmpty()) {
            Log.i(TAG, "🔔 规则命中，弹干预通知: $result")
            try {
                val obj = JSONObject(result)
                InterventionNotification(this).show(
                    obj.getString("message"),
                    obj.getString("interventionId"),
                )
            } catch (e: Exception) {
                Log.e(TAG, "展示干预通知失败", e)
            }
        }
    }

    /** 把 currentPkg 的会话（currentStartMs..endMs）写入 Event log。 */
    private fun closeSession(endMs: Long) {
        if (currentPkg.isEmpty() || currentStartMs <= 0L || endMs <= currentStartMs) return
        try {
            NativeBridge.ingestForeground(dbPath, currentPkg, currentCategory, currentStartMs, endMs)
        } catch (_: Throwable) {
            // 忽略，不中断轮询
        }
    }

    companion object {
        const val POLL_INTERVAL_MS = 10_000L

        /**
         * MVP 娱乐 app 包名 → 分类（Kotlin 侧标注；Rust 侧规则以 category 前缀 "entertainment" 判定）。
         */
        val ENTERTAINMENT_PACKAGES = mapOf(
            "tv.danmaku.bili" to "entertainment.video",
            "com.ss.android.ugc.aweme" to "entertainment.video",
            "com.kuaishou.nebula" to "entertainment.video",
            "com.zhiliaoapp.musically" to "entertainment.video",
            "com.google.android.youtube" to "entertainment.video",
            "com.netflix.mediaclient" to "entertainment.video",
            "com.tencent.qqlive" to "entertainment.video",
            "com.qiyi.video" to "entertainment.video",
            "com.youku.phone" to "entertainment.video",
            "com.ss.android.article.news" to "entertainment.news",
            "com.tencent.news" to "entertainment.news",
            "com.sina.weibo" to "entertainment.social",
            "com.reddit.frontpage" to "entertainment.social",
            "com.instagram.android" to "entertainment.social",
        )

        fun start(context: Context) {
            context.startForegroundService(Intent(context, UsageStatsService::class.java))
        }

        fun stop(context: Context) {
            context.stopService(Intent(context, UsageStatsService::class.java))
        }
    }
}
