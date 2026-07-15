package com.sisyphus.collector

/**
 * JNI 桥：绕过 WebView 直调 Rust core（libsisyphus_lib.so）。实现见 src/android_jni.rs。
 *
 * 为什么：App 退到后台（用户在刷抖音）时 WebView 被系统挂起，Tauri 的 invoke() 失效。
 * 前台服务 / 通知按钮接收器改用这里的原生调用，使闭环在后台仍能评估+弹通知+回填反馈。
 * 业务逻辑仍全在 Rust core（单一来源），Kotlin 只做 OS 桥接。
 */
object NativeBridge {
    init {
        System.loadLibrary("sisyphus_lib")
    }

    /** 评估当前前台 app；命中返回 JSON {"message","interventionId"}，否则空串。 */
    external fun evaluate(dbPath: String, pkg: String, category: String, activeMs: Long): String

    /** 记录用户对干预通知的响应（按钮点击）。 */
    external fun recordFeedback(dbPath: String, interventionId: String, action: String)

    /** 写一段已结束的前台会话到 Event log（app 切换时调用）。 */
    external fun ingestForeground(
        dbPath: String,
        pkg: String,
        category: String,
        startMs: Long,
        endMs: Long,
    )

    /** 诊断：返回 DB 路径/目标/事件数 的 JSON，用于排查「手机 0 记录」。 */
    external fun debugState(dbPath: String): String
}
