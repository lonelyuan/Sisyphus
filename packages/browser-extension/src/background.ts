// Sisyphus 浏览器插件 — MV3 background service worker
//
// 角色（见 docs/dev-browser-extension.md）：插件**仅**借浏览器 API 采集
// tab 的 domain/title/访问时间，通过 Native Messaging 转交桌面端。
// 规则判定与提醒触发都在桌面端，不在插件内。
//
// 注意：MV3 service worker 会被浏览器随时回收，不能持有长期状态——
// 每个事件即时通过 Native Messaging 发出，本地缓冲只在桌面端做。

const NATIVE_HOST = "com.sisyphus.desktop";

// 与桌面端约定的轻量消息（桌面端负责归一化为 BehaviorEvent 并入 outbox）。
interface TabSignal {
  kind: "tab_active" | "tab_updated" | "idle_state";
  domain?: string;
  title?: string; // L1，桌面端按授权等级决定是否落库
  idle_state?: chrome.idle.IdleState;
  at: string; // ISO8601
}

function send(signal: TabSignal): void {
  try {
    chrome.runtime.sendNativeMessage(NATIVE_HOST, signal, () => {
      if (chrome.runtime.lastError) {
        // 桌面端未运行：丢弃即可，桌面端是 PC 行为的事实来源。
      }
    });
  } catch {
    /* 忽略：native host 不可用 */
  }
}

function domainOf(url: string | undefined): string | undefined {
  if (!url) return undefined;
  try {
    return new URL(url).hostname;
  } catch {
    return undefined;
  }
}

chrome.tabs.onActivated.addListener(async (info) => {
  const tab = await chrome.tabs.get(info.tabId);
  send({
    kind: "tab_active",
    domain: domainOf(tab.url),
    title: tab.title,
    at: new Date().toISOString(),
  });
});

chrome.tabs.onUpdated.addListener((_id, changeInfo, tab) => {
  if (changeInfo.status === "complete" && tab.active) {
    send({
      kind: "tab_updated",
      domain: domainOf(tab.url),
      title: tab.title,
      at: new Date().toISOString(),
    });
  }
});

chrome.idle.setDetectionInterval(60);
chrome.idle.onStateChanged.addListener((state) => {
  send({ kind: "idle_state", idle_state: state, at: new Date().toISOString() });
});
