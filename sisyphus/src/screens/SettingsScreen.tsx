import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { platform as osPlatform } from "@tauri-apps/plugin-os";
import { enable, disable, isEnabled } from "@tauri-apps/plugin-autostart";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { Power, FolderOpen, Radar, Activity, Info, Eye, Plus, X, Bot, Database, ShieldCheck, KeyRound, FlaskConical, ListChecks } from "lucide-react";
import { Card, CardLabel } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { prettyApp, categoryLabel } from "@/lib/format";
import { cn } from "@/lib/utils";

interface MonitoredApp {
  id: string;
  category: string;
  platform: string;
  source: string;
}

interface RuntimeInfo {
  id: string;
  available: boolean;
  ready: boolean;
  path: string | null;
  version: string | null;
}

interface AgentRuntimeStatus {
  configured: "auto" | "pi" | "codex";
  resolved: "pi" | "codex" | null;
  pi: RuntimeInfo;
  codex: RuntimeInfo;
  skill_path: string;
  extensions: string[];
  read_only: boolean;
}

interface LlmConfig {
  format: string;
  base_url: string;
  model: string;
  has_key: boolean;
}

interface DetectionRule {
  id: string;
  name: string;
  enabled: boolean;
  trigger_json: string;
  response_json: string;
  severity: string;
  cooldown_minutes: number;
  created_by: string;
}

interface NotionConfigView {
  has_token: boolean;
  page_id: string;
  sync_enabled: boolean;
  ready: boolean;
}

export default function SettingsScreen() {
  const [platform, setPlatform] = useState("");
  const [autostart, setAutostart] = useState<boolean | null>(null);
  const [vault, setVault] = useState("");
  const [usageGranted, setUsageGranted] = useState<boolean | null>(null);
  const [collecting, setCollecting] = useState(false);
  const [monitored, setMonitored] = useState<MonitoredApp[]>([]);
  const [rules, setRules] = useState<DetectionRule[]>([]);
  const [newId, setNewId] = useState("");
  const [newCat, setNewCat] = useState("entertainment.video");
  const [err, setErr] = useState("");
  const [runtimeStatus, setRuntimeStatus] = useState<AgentRuntimeStatus | null>(null);
  const [runtimeSaving, setRuntimeSaving] = useState(false);
  const [llm, setLlm] = useState<LlmConfig>({
    format: "openai",
    base_url: "",
    model: "",
    has_key: false,
  });
  const [apiKey, setApiKey] = useState("");
  const [llmSaving, setLlmSaving] = useState(false);
  const [llmTest, setLlmTest] = useState("");
  const [notion, setNotion] = useState<NotionConfigView>({
    has_token: false,
    page_id: "",
    sync_enabled: false,
    ready: false,
  });
  const [notionToken, setNotionToken] = useState("");
  const [notionSaving, setNotionSaving] = useState(false);
  const [notionMsg, setNotionMsg] = useState("");

  async function setRuntime(runtime: "auto" | "pi" | "codex") {
    setRuntimeSaving(true);
    try {
      await invoke("set_agent_runtime", { runtime });
      const status = await invoke<AgentRuntimeStatus>("get_agent_runtime_status");
      setRuntimeStatus(status);
    } catch (e) {
      setErr("保存 Agent runtime 失败: " + String(e));
    } finally {
      setRuntimeSaving(false);
    }
  }

  async function savePiConfig(testAfterSave = false) {
    setLlmSaving(true);
    setErr("");
    setLlmTest("");
    try {
      await invoke("set_llm_config", {
        format: llm.format,
        baseUrl: llm.base_url,
        model: llm.model,
        apiKey,
      });
      setApiKey("");
      const saved = await invoke<LlmConfig>("get_llm_config");
      setLlm(saved);
      const status = await invoke<AgentRuntimeStatus>("get_agent_runtime_status");
      setRuntimeStatus(status);
      if (testAfterSave) {
        const result = await invoke<{ runtime: string; text: string }>("run_agent", {
          prompt: "这是 Pi JS SDK 连通性检查。不要调用工具，只回复 OK。",
          runtime: "pi",
        });
        setLlmTest(`${result.runtime}: ${result.text.trim()}`);
      } else {
        setLlmTest("配置已保存");
      }
    } catch (e) {
      setErr((testAfterSave ? "Pi SDK 测试失败: " : "保存 Pi 配置失败: ") + String(e));
    } finally {
      setLlmSaving(false);
    }
  }

  function loadNotionConfig() {
    invoke<NotionConfigView>("get_notion_config").then(setNotion).catch(() => {});
  }

  async function saveNotionToken() {
    setNotionSaving(true);
    setNotionMsg("");
    try {
      await invoke("set_notion_config", {
        token: notionToken,
        pageId: notion.page_id,
        syncEnabled: notion.sync_enabled,
      });
      setNotionToken("");
      loadNotionConfig();
      setNotionMsg(notion.sync_enabled ? "已保存，将自动开始同步" : "配置已保存");
    } catch (e) {
      setErr("保存 Notion 配置失败: " + String(e));
    } finally {
      setNotionSaving(false);
    }
  }

  async function clearNotionToken() {
    setNotionSaving(true);
    setNotionMsg("");
    try {
      await invoke("clear_notion_config");
      loadNotionConfig();
      setNotionMsg("已断开");
    } catch (e) {
      setErr("断开 Notion 失败: " + String(e));
    } finally {
      setNotionSaving(false);
    }
  }

  useEffect(() => {
    (async () => {
      let p = "";
      try {
        // 官方 JS API（比裸 invoke 可靠）：返回 "android" | "macos" | "windows" | ...
        p = await osPlatform();
      } catch {
        // 兜底：os 插件不可用时，android-only 的 usage 命令能调通即安卓。
        try {
          await invoke("check_usage_permission");
          p = "android";
        } catch {
          p = "desktop";
        }
      }
      setPlatform(p);
      if (p === "android") await refreshPermission();
    })();
    isEnabled().then(setAutostart).catch(() => setAutostart(null));
    invoke<string>("get_vault_path").then(setVault).catch(() => {});
    invoke<AgentRuntimeStatus>("get_agent_runtime_status").then(setRuntimeStatus).catch(() => {});
    invoke<LlmConfig>("get_llm_config").then(setLlm).catch(() => {});
    loadNotionConfig();
    loadMonitored();
    loadRules();
  }, []);

  async function refreshPermission() {
    try {
      const granted = await invoke<boolean>("check_usage_permission");
      setUsageGranted(granted);
    } catch (e) {
      setErr("检查权限失败: " + String(e));
    }
  }

  async function toggleAutostart() {
    setErr("");
    try {
      if (autostart) {
        await disable();
        setAutostart(false);
      } else {
        await enable();
        setAutostart(true);
      }
    } catch (e) {
      setErr("开机自启切换失败: " + String(e));
    }
  }

  async function openVault() {
    try {
      await revealItemInDir(vault);
    } catch (e) {
      setErr("打开知识库失败: " + String(e));
    }
  }

  function loadMonitored() {
    invoke<MonitoredApp[]>("list_monitored_apps").then(setMonitored).catch(() => {});
  }

  function loadRules() {
    invoke<DetectionRule[]>("list_detection_rules").then(setRules).catch(() => {});
  }

  async function toggleRule(id: string, enabled: boolean) {
    try {
      await invoke("set_detection_rule_enabled", { id, enabled });
      loadRules();
    } catch (e) {
      setErr("更新规则失败: " + String(e));
    }
  }

  async function removeRule(id: string) {
    try {
      await invoke("delete_detection_rule", { id });
      loadRules();
    } catch (e) {
      setErr("删除规则失败: " + String(e));
    }
  }

  async function addApp() {
    const id = newId.trim();
    if (!id) return;
    setErr("");
    try {
      await invoke("add_monitored_app", { id, category: newCat });
      setNewId("");
      loadMonitored();
    } catch (e) {
      setErr("添加失败: " + String(e));
    }
  }

  async function removeApp(id: string) {
    try {
      await invoke("remove_monitored_app", { id });
      loadMonitored();
    } catch (e) {
      setErr("删除失败: " + String(e));
    }
  }

  async function requestPermission() {
    setErr("");
    try {
      await invoke("request_usage_permission");
      setTimeout(refreshPermission, 1500);
      setTimeout(refreshPermission, 3000);
    } catch (e) {
      setErr("跳转授权失败: " + String(e));
    }
  }

  async function toggleCollector() {
    setErr("");
    try {
      await invoke(collecting ? "stop_collector" : "start_collector");
      setCollecting(!collecting);
    } catch (e) {
      setErr("采集服务操作失败: " + String(e));
    }
  }

  const isAndroid = platform === "android";
  const apps = monitored.filter(
    (m) => m.platform === (isAndroid ? "android" : "desktop") || m.platform === "custom",
  );

  return (
    <div className="animate-in mx-auto flex max-w-2xl flex-col gap-3 p-5 md:p-8">
      {err && (
        <Card className="border-danger/40 bg-danger/10 p-3 text-xs text-danger">{err}</Card>
      )}

      {/* Agent runtime：主对话、宠物和 scheduler 共用。 */}
      <Card className="flex flex-col gap-3 p-4">
        <div className="flex items-center justify-between gap-2">
          <div className="flex items-center gap-2">
            <Bot size={14} strokeWidth={1.75} className="text-accent" />
            <CardLabel>Agent Runtime</CardLabel>
          </div>
          <span className="flex items-center gap-1 rounded-full bg-success/10 px-2 py-1 text-[10px] text-success">
            <ShieldCheck size={11} /> 权限分层
          </span>
        </div>
        <div className="grid grid-cols-3 gap-1 rounded-lg bg-muted p-1">
          {(["auto", "pi", "codex"] as const).map((runtime) => (
            <button
              key={runtime}
              disabled={runtimeSaving || (runtime !== "auto" && !runtimeStatus?.[runtime].ready)}
              onClick={() => void setRuntime(runtime)}
              className={
                "rounded-md px-2 py-2 text-xs transition " +
                (runtimeStatus?.configured === runtime
                  ? "bg-card text-foreground shadow-sm"
                  : "text-muted-foreground hover:text-foreground disabled:opacity-35")
              }
            >
              {runtime === "auto" ? "自动" : runtime === "pi" ? "Pi" : "Codex"}
            </button>
          ))}
        </div>
        <div className="grid grid-cols-2 gap-2 text-[11px] text-muted-foreground">
          {[runtimeStatus?.pi, runtimeStatus?.codex].map((item) => item && (
            <div key={item.id} className="rounded-md border border-border px-2.5 py-2">
              <div className="flex items-center justify-between text-foreground/80">
                <span>{item.id === "pi" ? "Pi" : "Codex"}</span>
                <span className={item.ready ? "text-success" : "text-warning"}>
                  {item.ready ? "可用" : item.available ? "未配置" : "缺失"}
                </span>
              </div>
              <code className="mt-1 block truncate">{item.version || item.path || "—"}</code>
            </div>
          ))}
        </div>
        <p className="text-[11px] leading-relaxed text-muted-foreground">
          当前实际使用 <b>{runtimeStatus?.resolved || "检测中"}</b>。Pi 由项目内 JS SDK 驱动；主动建议只读，交互会话可写 Core，LifeIndex 同步只能写 LifeDB 与指定 Notion 页面。
        </p>
      </Card>

      <Card className="flex flex-col gap-3 p-4">
        <div className="flex items-center justify-between gap-2">
          <div className="flex items-center gap-2">
            <KeyRound size={14} strokeWidth={1.75} className="text-accent" />
            <CardLabel>Pi JS SDK 模型配置</CardLabel>
          </div>
          <span className="text-[10px] text-muted-foreground">
            {llm.has_key ? "Key 已保存" : "需要 API Key"}
          </span>
        </div>
        <div className="rounded-lg border border-accent/15 bg-accent/5 px-3 py-2.5 text-[11px] leading-relaxed text-muted-foreground">
          <p className="m-0 text-foreground/80">按你的模型服务填写，不需要安装或登录 pi CLI：</p>
          <ol className="mb-0 mt-1.5 list-decimal space-y-1 pl-4">
            <li>选择服务实际支持的 API 协议；OpenAI 兼容接口通常选 Chat Completions。</li>
            <li>官方服务可先留空 Endpoint；代理、内网网关或自定义模型填写完整 base URL。</li>
            <li>Model ID 必须与服务端一致，粘贴该服务签发的 API Key。</li>
            <li>点“保存并测试”，看到 <code>pi: OK</code> 才表示配置完成。</li>
          </ol>
          <details className="mt-2">
            <summary className="cursor-pointer text-accent">填写示例与 401 排错</summary>
            <div className="mt-1.5 space-y-1">
              <p className="m-0"><b className="text-foreground/70">OpenAI 官方：</b>OpenAI Responses / Endpoint 留空 / 官方 Model ID / OpenAI Key。</p>
              <p className="m-0"><b className="text-foreground/70">Anthropic 官方：</b>Anthropic Messages / Endpoint 留空 / Claude Model ID / Anthropic Key。</p>
              <p className="m-0"><b className="text-foreground/70">兼容网关：</b>选网关声明的协议 / 填网关 base URL / 网关 Model ID / 网关 Key。</p>
              <p className="m-0 text-warning">401 Invalid bearer token 表示 Key 不属于当前 Endpoint/Provider、已失效或复制不完整；它不是 Pi SDK 的登录问题。</p>
            </div>
          </details>
        </div>
        <label className="flex flex-col gap-1 text-[11px] text-muted-foreground">
          Provider / API 协议
          <select
            value={llm.format}
            onChange={(e) => setLlm((value) => ({ ...value, format: e.target.value }))}
            className="h-9 rounded-md border border-input bg-input px-2 text-xs text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/40"
          >
            <option value="openai">OpenAI Responses</option>
            <option value="openai-completions">OpenAI Chat Completions</option>
            <option value="anthropic">Anthropic Messages</option>
            <option value="google">Google Generative AI</option>
            <option value="openrouter">OpenRouter</option>
            <option value="deepseek">DeepSeek</option>
          </select>
        </label>
        <label className="flex flex-col gap-1 text-[11px] text-muted-foreground">
          API Endpoint（官方地址可留空）
          <Input
            value={llm.base_url}
            onChange={(e) => setLlm((value) => ({ ...value, base_url: e.target.value }))}
            placeholder="https://api.example.com/v1"
            spellCheck={false}
          />
        </label>
        <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
          <label className="flex flex-col gap-1 text-[11px] text-muted-foreground">
            Model ID
            <Input
              value={llm.model}
              onChange={(e) => setLlm((value) => ({ ...value, model: e.target.value }))}
              placeholder="gpt-5.5"
              spellCheck={false}
            />
          </label>
          <label className="flex flex-col gap-1 text-[11px] text-muted-foreground">
            API Key
            <Input
              type="password"
              value={apiKey}
              onChange={(e) => setApiKey(e.target.value)}
              placeholder={llm.has_key ? "已保存，留空保持不变" : "输入 API Key"}
              autoComplete="off"
              spellCheck={false}
            />
          </label>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <Button size="sm" disabled={llmSaving} onClick={() => void savePiConfig(false)}>
            保存配置
          </Button>
          <Button variant="secondary" size="sm" disabled={llmSaving} onClick={() => void savePiConfig(true)}>
            <FlaskConical size={13} /> {llmSaving ? "检查中…" : "保存并测试"}
          </Button>
          {llmTest && <span className="text-[11px] text-success">{llmTest}</span>}
        </div>
        <p className="text-[11px] leading-relaxed text-muted-foreground">
          配置仅供西西弗斯的 Pi SDK runtime 使用。Key 不返回前端，运行时由 Rust 通过子进程环境传给 SDK；不需要执行 <code>pi /login</code>。
        </p>
      </Card>

      <Card className="flex flex-col gap-3 p-4">
        <div className="flex items-center justify-between gap-2">
          <div className="flex items-center gap-2">
            <Database size={14} strokeWidth={1.75} className="text-muted-foreground" />
            <CardLabel>Notion · LifeIndex 文本投影</CardLabel>
          </div>
          <span
            className={cn(
              "rounded-full px-2 py-1 text-[10px]",
              notion.ready ? "bg-success/10 text-success" : "bg-warning/10 text-warning",
            )}
          >
            {notion.ready ? "双向同步已开启" : notion.has_token ? "待完成配置" : "尚未连接"}
          </span>
        </div>
        <p className="text-xs leading-relaxed text-muted-foreground">
          SQLite LifeDB 是事实源，Notion 只是一张可自由编辑的普通文本页面。Agent 会理解页面修改并合并到本地，App 修改则投影回同一页。受限网关把写入固定到下面这一个 Page ID，模型不能改写其它页面。在{" "}
          <a
            href="https://www.notion.so/my-integrations"
            target="_blank"
            rel="noreferrer"
            className="text-accent underline"
          >
            notion.so/my-integrations
          </a>{" "}
          新建 integration 时勾选 <b>“Read content” 与 “Update content”</b>，并且只把 LifeIndex 页面连接给它。
        </p>
        <label className="flex flex-col gap-1 text-[11px] text-muted-foreground">
          Integration Token
          <Input
            type="password"
            value={notionToken}
            onChange={(e) => setNotionToken(e.target.value)}
            placeholder={notion.has_token ? "已保存，留空保持不变" : "ntn_…"}
            autoComplete="off"
            spellCheck={false}
          />
        </label>
        <label className="flex flex-col gap-1 text-[11px] text-muted-foreground">
          LifeIndex Page ID 或页面 URL
          <Input
            value={notion.page_id}
            onChange={(e) => setNotion((value) => ({ ...value, page_id: e.target.value, ready: false }))}
            placeholder="xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"
            autoComplete="off"
            spellCheck={false}
          />
        </label>
        <label className="flex items-center justify-between gap-3 rounded-md border border-border px-3 py-2 text-xs text-foreground">
          <span>
            开启定时双向同步
            <span className="mt-0.5 block text-[10px] text-muted-foreground">每天 8:30 拉取；本地修改后尽快推送</span>
          </span>
          <Switch
            checked={notion.sync_enabled}
            onCheckedChange={(checked) => setNotion((value) => ({ ...value, sync_enabled: checked, ready: false }))}
            aria-label="开启 LifeIndex 双向同步"
          />
        </label>
        <div className="flex flex-wrap items-center gap-2">
          <Button size="sm" disabled={notionSaving} onClick={() => void saveNotionToken()}>
            {notionSaving ? "保存中…" : "保存"}
          </Button>
          {notion.has_token && (
            <Button variant="secondary" size="sm" disabled={notionSaving} onClick={() => void clearNotionToken()}>
              断开连接
            </Button>
          )}
          {notionMsg && <span className="text-[11px] text-muted-foreground">{notionMsg}</span>}
        </div>
      </Card>

      {/* 监控名单（增删查改） */}
      <Card className="flex flex-col gap-3 p-4">
        <div className="flex items-center gap-2">
          <Eye size={14} strokeWidth={1.75} className="text-muted-foreground" />
          <CardLabel>监控名单（{apps.length}）</CardLabel>
        </div>

        {/* 增：包名 + 分类 */}
        <div className="flex gap-2">
          <Input
            value={newId}
            onChange={(e) => setNewId(e.target.value)}
            placeholder={isAndroid ? "包名，如 com.ss.android.ugc.aweme" : "bundle id，如 com.apple.TV"}
            onKeyDown={(e) => e.key === "Enter" && addApp()}
          />
          <select
            value={newCat}
            onChange={(e) => setNewCat(e.target.value)}
            className="h-9 shrink-0 rounded-md border border-input bg-input px-2 text-xs text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/40"
          >
            <option value="entertainment.video">视频</option>
            <option value="entertainment.game">游戏</option>
            <option value="entertainment.social">社交</option>
            <option value="entertainment.news">资讯</option>
          </select>
          <Button variant="secondary" size="icon" onClick={addApp} aria-label="添加监控 app">
            <Plus size={16} strokeWidth={2} />
          </Button>
        </div>

        {/* 查 + 删 */}
        {apps.length ? (
          <ul className="flex flex-col gap-1.5">
            {apps.map((m) => (
              <li key={m.platform + m.id} className="group flex items-center gap-2">
                <span className="flex-1 truncate text-sm">{prettyApp(m.id)}</span>
                <code className="max-w-[120px] truncate font-mono text-[10px] text-muted-foreground/60">
                  {m.id}
                </code>
                <span className="shrink-0 rounded bg-warning/15 px-1.5 py-0.5 text-[10px] text-warning">
                  {categoryLabel(m.category)}
                </span>
                {m.source === "user" ? (
                  <button
                    onClick={() => removeApp(m.id)}
                    className="shrink-0 text-muted-foreground/40 transition-colors hover:text-danger"
                    aria-label="删除"
                  >
                    <X size={14} strokeWidth={2} />
                  </button>
                ) : (
                  <span className="w-[14px] shrink-0 text-center text-[10px] text-muted-foreground/40">·</span>
                )}
              </li>
            ))}
          </ul>
        ) : (
          <p className="text-sm text-muted-foreground">当前平台暂无监控项。</p>
        )}
        <p className="text-[11px] leading-relaxed text-muted-foreground">
          停留在名单内的 app 超阈值触发干预。自定义项跨端即时生效（安卓无需重编）；内置项不可删，可用同名自定义项覆盖分类。
          {!isAndroid && " 桌面浏览器内刷视频需浏览器插件（延后）。"}
        </p>
      </Card>

      {/* 检测规则（智能体一句话建；此处查看 / 启停 / 删） */}
      <Card className="flex flex-col gap-3 p-4">
        <div className="flex items-center gap-2">
          <ListChecks size={14} strokeWidth={1.75} className="text-accent" />
          <CardLabel>检测规则（{rules.length}）</CardLabel>
        </div>
        {rules.length > 0 ? (
          <ul className="flex flex-col gap-2">
            {rules.map((r) => (
              <li key={r.id} className="flex items-center gap-2 rounded-lg bg-muted/50 p-2">
                <div className="flex min-w-0 flex-1 flex-col">
                  <span className="truncate text-sm text-foreground">
                    {r.name}
                    {r.severity === "high" && <span className="ml-1 text-danger">⚠️</span>}
                  </span>
                  <span className="truncate font-mono text-[10px] text-muted-foreground">{r.trigger_json}</span>
                </div>
                <Switch checked={r.enabled} onCheckedChange={(v) => void toggleRule(r.id, v)} aria-label="启用规则" />
                <Button variant="ghost" size="icon" onClick={() => removeRule(r.id)} aria-label="删除规则">
                  <X size={14} />
                </Button>
              </li>
            ))}
          </ul>
        ) : (
          <p className="text-sm text-muted-foreground">还没有自定义检测规则。</p>
        )}
        <p className="text-[11px] leading-relaxed text-muted-foreground">
          在 Agent 对话里说“帮我盯着…/我一到…就停不下来”，智能体会用一句话建出检测规则；命中后端侧自动弹通知或宠物气泡。这里可查看、临时停用或删除。
        </p>
      </Card>

      {!isAndroid && (
        <>
          {/* 后台常驻 */}
          <Card className="flex items-start gap-3 p-4">
            <Radar size={16} strokeWidth={1.75} className="mt-0.5 shrink-0 text-accent" />
            <div className="flex flex-col gap-1">
              <CardLabel>后台常驻</CardLabel>
              <p className="text-xs leading-relaxed text-muted-foreground">
                关窗后从程序坞隐藏、仅保留菜单栏图标（不占程序坞）；采集器持续在后台运行。点菜单栏图标唤回窗口，「退出」才结束进程。
              </p>
            </div>
          </Card>

          {/* 开机自启 */}
          <Card className="flex items-center justify-between gap-3 p-4">
            <div className="flex items-start gap-3">
              <Power size={16} strokeWidth={1.75} className="mt-0.5 shrink-0 text-muted-foreground" />
              <div className="flex flex-col gap-1">
                <CardLabel>开机自启</CardLabel>
                <p className="text-xs text-muted-foreground">登录时自动启动，跨重启常驻采集</p>
              </div>
            </div>
            <Switch
              checked={autostart ?? false}
              disabled={autostart === null}
              onCheckedChange={toggleAutostart}
            />
          </Card>

          {/* 知识库 */}
          <Card className="flex flex-col gap-3 p-4">
            <div className="flex items-center gap-2">
              <FolderOpen size={14} strokeWidth={1.75} className="text-muted-foreground" />
              <CardLabel>第二大脑知识库</CardLabel>
            </div>
            <code className="block truncate rounded-md border border-border bg-muted px-2.5 py-2 font-mono text-[11px] text-muted-foreground">
              {vault || "…"}
            </code>
            <Button variant="secondary" size="sm" className="self-start" onClick={openVault}>
              <FolderOpen size={14} strokeWidth={1.75} />
              在 Finder 打开（可作为 Obsidian 库）
            </Button>
          </Card>
        </>
      )}

      {isAndroid && (
        <>
          <Card className="flex flex-col gap-3 p-4">
            <div className="flex items-center gap-2">
              <Activity size={14} strokeWidth={1.75} className="text-muted-foreground" />
              <CardLabel>应用使用情况权限</CardLabel>
            </div>
            <div className="flex items-center justify-between">
              <span
                className={
                  "rounded-full px-2 py-0.5 text-[11px] " +
                  (usageGranted ? "bg-success/15 text-success" : "bg-danger/15 text-danger")
                }
              >
                {usageGranted === null ? "检查中…" : usageGranted ? "已授权" : "未授权"}
              </span>
              {!usageGranted && (
                <Button size="sm" onClick={requestPermission}>
                  前往授权
                </Button>
              )}
            </div>
          </Card>

          <Card className="flex items-center justify-between gap-3 p-4">
            <div className="flex flex-col gap-1">
              <CardLabel>采集服务</CardLabel>
              <p className="text-xs text-muted-foreground">后台持续采集（前台通知常驻）</p>
            </div>
            <Button
              variant={collecting ? "secondary" : "primary"}
              size="sm"
              disabled={!usageGranted}
              onClick={toggleCollector}
            >
              {collecting ? "停止" : "启动"}
            </Button>
          </Card>
        </>
      )}

      {/* 页脚 */}
      <div className="mt-1 flex items-center justify-between px-1 text-[11px] text-muted-foreground">
        <span className="flex items-center gap-1.5">
          <Info size={12} strokeWidth={1.75} />
          Sisyphus v0.1.0 · Phase 1
        </span>
        <span>{platform || "…"}</span>
      </div>
    </div>
  );
}
