# Spec: Android 采集层

Android 特权 API 无法从 Rust/WebView 直接调用，必须以 **Kotlin Tauri 插件**形式桥接。

Kotlin 只负责 OS API 调用，业务逻辑（规则、存储、冷却）全在 Rust。

---

## 环境准备

```bash
# Rust Android targets
rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android i686-linux-android

# 环境变量（加到 ~/.zshrc）
export ANDROID_HOME=$HOME/Library/Android/sdk
export NDK_HOME=$ANDROID_HOME/ndk/<version>

# 初始化 Android 工程（首次）
cd sisyphus && npm run tauri android init
```

生成路径：`sisyphus/src-tauri/gen/android/`（标准 Gradle 项目，Kotlin 插件写在此目录下）。

---

## Tauri 插件 API

### UsagePlugin

采集前台 app 使用情况。

```kotlin
@TauriPlugin
class UsagePlugin(activity: Activity) : Plugin(activity) {

    @Command
    fun startCollector(invoke: Invoke) {
        activity.startForegroundService(Intent(activity, UsageStatsService::class.java))
        invoke.resolve()
    }

    @Command
    fun stopCollector(invoke: Invoke) {
        activity.stopService(Intent(activity, UsageStatsService::class.java))
        invoke.resolve()
    }

    @Command
    fun checkPermission(invoke: Invoke) {
        val granted = hasUsageStatsPermission(activity)
        invoke.resolve(JSObject().put("granted", granted))
    }

    @Command
    fun requestPermission(invoke: Invoke) {
        activity.startActivity(Intent(Settings.ACTION_USAGE_ACCESS_SETTINGS))
        invoke.resolve()
    }
}
```

Rust 侧调用方式：
```rust
// 由 UsageStatsService 每 10s 通过 app_handle 调用
app_handle.emit("usage_event", &event_payload)?;
```

TypeScript 侧（设置页权限检查）：
```typescript
await invoke('plugin:usage|start_collector')
await invoke('plugin:usage|check_permission')
```

### NotificationPlugin

弹出干预通知。

```kotlin
@TauriPlugin
class NotificationPlugin(activity: Activity) : Plugin(activity) {

    @Command
    fun showIntervention(invoke: Invoke) {
        val message = invoke.getString("message") ?: return
        val interventionId = invoke.getString("intervention_id") ?: return
        InterventionNotification(activity).show(message, interventionId)
        invoke.resolve()
    }
}
```

---

## AndroidManifest.xml 必要声明

```xml
<!-- 权限 -->
<uses-permission android:name="android.permission.PACKAGE_USAGE_STATS"
    tools:ignore="ProtectedPermissions" />
<uses-permission android:name="android.permission.FOREGROUND_SERVICE" />
<uses-permission android:name="android.permission.FOREGROUND_SERVICE_DATA_SYNC" />
<uses-permission android:name="android.permission.POST_NOTIFICATIONS" />
<uses-permission android:name="android.permission.RECEIVE_BOOT_COMPLETED" />

<!-- Layer 1：采集服务 -->
<service
    android:name=".collector.UsageStatsService"
    android:foregroundServiceType="dataSync"
    android:exported="false" />

<!-- Layer 2（可选）：媒体状态检测 -->
<service
    android:name=".collector.MediaStateListener"
    android:exported="true"
    android:permission="android.permission.BIND_NOTIFICATION_LISTENER_SERVICE">
    <intent-filter>
        <action android:name="android.service.notification.NotificationListenerService" />
    </intent-filter>
</service>

<!-- Layer 3（可选）：滚动检测 -->
<service
    android:name=".collector.SisyphusAccessibilityService"
    android:permission="android.permission.BIND_ACCESSIBILITY_SERVICE"
    android:exported="true">
    <intent-filter>
        <action android:name="android.accessibilityservice.AccessibilityService" />
    </intent-filter>
    <meta-data android:name="android.accessibilityservice"
        android:resource="@xml/accessibility_service_config" />
</service>

<!-- 通知按钮接收器 -->
<receiver android:name=".NotificationActionReceiver" android:exported="false" />

<!-- 开机自启 -->
<receiver android:name=".BootReceiver" android:exported="true">
    <intent-filter>
        <action android:name="android.intent.action.BOOT_COMPLETED" />
    </intent-filter>
</receiver>
```

---

## 保活策略

- `UsageStatsService` 使用 `START_STICKY`，系统杀掉后自动重启
- `BootReceiver` 开机后重新启动 Service
- 设置页引导用户：**电池优化白名单** + **自启动权限**

各厂商设置入口（深链接）：

| ROM | 电池白名单 |
|---|---|
| 小米 MIUI | `Intent("miui.intent.action.POWER_HIDE_MODE_APP_LIST")` |
| 华为 EMUI | `Intent("huawei.intent.action.POWER_MANAGER_SETTINGS")` |
| OPPO | `Intent(Settings.ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS)` |
| 通用 | `Settings.ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS` |

---

## 娱乐 app 分类白名单（MVP）

```kotlin
val ENTERTAINMENT_PACKAGES = mapOf(
    "tv.danmaku.bili"              to "entertainment.video",   // B 站
    "com.ss.android.ugc.aweme"     to "entertainment.video",   // 抖音
    "com.kuaishou.nebula"          to "entertainment.video",   // 快手
    "com.zhiliaoapp.musically"     to "entertainment.video",   // TikTok
    "com.google.android.youtube"   to "entertainment.video",
    "com.netflix.mediaclient"      to "entertainment.video",
    "com.sina.weibo"               to "entertainment.social",
    "com.reddit.frontpage"         to "entertainment.social",
)
```

用户可在设置页追加自定义条目。

---

## Layer 2 / Layer 3 参考实现（延后；迁移自旧原生工程，已适配 Tauri 桥）

> 状态：**Layer 1（UsageStats）已在 `gen/android` 落地并适配 Tauri**（`UsageStatsService` 轮询 `queryEvents` → `UsagePlugin.emitUsageEvent` → Rust）。下面 Layer 2/3 是从旧原生工程抢救出来的 OS 采集机制，**开工前只作参考**。核心适配原则：Kotlin 只采集 OS 信号并经插件桥给 Rust，**不写 Room、不跑规则**（那些在 `sisyphus-core`）。旧工程里它们直接写 Room + 喂 Kotlin 引擎，迁移时改成 `trigger(...)` / 调用 `ingest_event`。

### Layer 2 — 媒体播放检测（`NotificationListenerService`，误报抑制）

判断是否在“被动看视频”。机制：监听娱乐包名的通知，检测 `android.mediaSession` extra。

```kotlin
class MediaStateListener : NotificationListenerService() {
    override fun onNotificationPosted(sbn: StatusBarNotification) {
        val pkg = sbn.packageName ?: return
        if (pkg !in ENTERTAINMENT_PACKAGES) return
        if (!sbn.notification.extras.containsKey("android.mediaSession")) return
        // 适配：不写 prefs/Room，改为桥给 Rust —— 由 Rust 记 media_playing_since_ms 注入 RuleContext
        MediaPlugin.instance?.emitMediaState(pkg, playing = true)
    }
    override fun onNotificationRemoved(sbn: StatusBarNotification) {
        val pkg = sbn.packageName ?: return
        if (pkg in ENTERTAINMENT_PACKAGES) MediaPlugin.instance?.emitMediaState(pkg, playing = false)
    }
}
```

Rust 侧把 `media_playing_since_ms` 注入 `RuleContext`，规则据此抑制“稳定播放≥5min 且低滚动”的误报（见 [rule-engine.md](rule-engine.md) 误报抑制）。

### Layer 3 — 滚动检测（`AccessibilityService`，用户可选）

端侧预聚合 `scroll_burst`，**不逐次上报**。`AccessibilityServiceInfo` 限 `TYPE_VIEW_SCROLLED` + XML 限定包名，功耗极低。聚合算法（抢救自旧实现）：

- 3 秒滑动窗口计数；窗口内 `count ≥ 5` 才计入本分钟；
- 每 60 秒 flush 一次，输出 `scroll_burst`（`scroll_count` / `window_sec` / `avg_interval_ms`）。

```kotlin
override fun onServiceConnected() {
    serviceInfo = serviceInfo?.also { it.eventTypes = TYPE_VIEW_SCROLLED; it.notificationTimeout = 150 }
}
// 3s 窗口累加 → 每 60s：适配为经插件桥调 Rust ingest_event(type="scroll_burst", ...)，不写 Room
```

Rust 侧把过去 10min 的 `scroll_burst` 汇总为 `recent_scroll_count` 注入 `RuleContext`（对应预留的 `ScrollBurstRule`，见 [rule-engine.md](rule-engine.md)）。

> Manifest 里 `MediaStateListener` / `SisyphusAccessibilityService` 的 service 声明见上文 §AndroidManifest.xml。
