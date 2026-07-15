//! 应用分类（bundle id / 包名 → category）。MVP 用硬编码白名单，与规则逻辑分离。
//! Android 娱乐包名见 [`crate::rule_engine::entertainment::entertainment_packages`]。

/// 桌面（macOS）娱乐应用 bundle id → 分类。
///
/// **注意**：桌面上真正的刷视频/信息流多发生在浏览器内，准确信号需要浏览器插件（延后的采集源）。
/// 这里只覆盖原生娱乐应用。要验证/自用：看 collector 打到 stderr 的 `(bundle, name)`，
/// 把你认定为时间黑洞的 app bundle id 加进来即可。刻意不含浏览器/音乐，避免误报。
pub fn categorize_desktop(bundle_id: &str) -> Option<&'static str> {
    match bundle_id {
        "com.apple.TV" => Some("entertainment.video"),
        "com.netflix.Netflix" => Some("entertainment.video"),
        "com.colliderli.iina" => Some("entertainment.video"), // IINA 播放器
        "tv.plex.plex-desktop" => Some("entertainment.video"),
        "com.valvesoftware.steam" => Some("entertainment.game"),
        _ => None,
    }
}
