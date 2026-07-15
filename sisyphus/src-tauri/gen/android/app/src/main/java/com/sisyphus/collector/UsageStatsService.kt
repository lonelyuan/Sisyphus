package com.sisyphus.collector

import android.app.Service
import android.app.usage.UsageEvents
import android.app.usage.UsageStatsManager
import android.content.Context
import android.content.Intent
import android.os.IBinder
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

/**
 * 前台服务：每 10s 轮询 UsageStats，将前台 app 事件通过 UsagePlugin.trigger() 发送给 JS 层。
 * Kotlin 只做 OS API 桥接，业务逻辑（规则、存储）全在 Rust。
 */
class UsageStatsService : Service() {

    private val serviceScope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val usageStatsManager by lazy {
        getSystemService(Context.USAGE_STATS_SERVICE) as UsageStatsManager
    }

    // 内存状态：当前前台 app 及其启动时间（服务重启后从第一次 poll 恢复）
    private var currentPkg = ""
    private var currentStartMs = 0L
    private var lastPollMs = 0L

    override fun onCreate() {
        super.onCreate()
        startForeground(
            NOTIF_ID_COLLECTOR,
            InterventionNotification(this).buildCollectorNotification()
        )
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
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
            val ts  = event.timeStamp

            when (event.eventType) {
                UsageEvents.Event.ACTIVITY_RESUMED -> {
                    if (currentPkg.isNotEmpty() && currentPkg != pkg) {
                        // 旧 app 切出 — 不需要发事件，Rust 会通过 active_ms=0 推断
                    }
                    if (currentPkg != pkg) {
                        currentPkg     = pkg
                        currentStartMs = ts
                    }
                }
                UsageEvents.Event.ACTIVITY_PAUSED,
                UsageEvents.Event.ACTIVITY_STOPPED -> {
                    if (currentPkg == pkg) {
                        currentPkg     = ""
                        currentStartMs = 0L
                    }
                }
            }
        }

        lastPollMs = nowMs

        // 计算当前进行中的娱乐会话时长（防漏算）
        val activeMs = if (currentPkg.isNotEmpty() && currentStartMs > 0L) {
            nowMs - currentStartMs
        } else {
            0L
        }
        val category = ENTERTAINMENT_PACKAGES[currentPkg]

        // 只在有前台 app 时才向 JS 层发事件（减少无效触发）
        if (currentPkg.isNotEmpty()) {
            UsagePlugin.instance?.emitUsageEvent(currentPkg, category, activeMs)
        }
    }

    companion object {
        const val POLL_INTERVAL_MS = 10_000L

        /**
         * MVP 娱乐 app 包名 → 分类。
         * 此处仅用于 Kotlin 侧过滤/标注，Rust 侧维护同一份列表作为权威来源。
         */
        val ENTERTAINMENT_PACKAGES = mapOf(
            "tv.danmaku.bili"              to "entertainment.video",
            "com.ss.android.ugc.aweme"     to "entertainment.video",
            "com.kuaishou.nebula"          to "entertainment.video",
            "com.zhiliaoapp.musically"     to "entertainment.video",
            "com.google.android.youtube"   to "entertainment.video",
            "com.netflix.mediaclient"      to "entertainment.video",
            "com.tencent.qqlive"           to "entertainment.video",
            "com.qiyi.video"               to "entertainment.video",
            "com.youku.phone"              to "entertainment.video",
            "com.ss.android.article.news"  to "entertainment.news",
            "com.tencent.news"             to "entertainment.news",
            "com.sina.weibo"               to "entertainment.social",
            "com.reddit.frontpage"         to "entertainment.social",
            "com.instagram.android"        to "entertainment.social",
        )

        fun start(context: Context) {
            context.startForegroundService(Intent(context, UsageStatsService::class.java))
        }

        fun stop(context: Context) {
            context.stopService(Intent(context, UsageStatsService::class.java))
        }
    }
}
