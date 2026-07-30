# 检测规则（一句话建规则）

当用户说“帮我盯着 X”“我一到 X 就停不下来”“晚上老是刷 Y，提醒我”这类**想让系统主动监控某类行为并提醒**的话时，把它落成一条**检测规则**：`create_detection_rule`。规则命中后，端侧（桌面/安卓）会自动弹通知或宠物气泡——不需要用户当时在跟你对话。

先 `list_detection_rules` 看有没有重复或可复用的规则，再决定新建还是改。

## 何时用规则 vs 用监控名单

- **只是想“少刷某个 app”**：`add_monitored_app(包名, 分类)` + `set_goal` 就够了（内置的“设了目标又超时刷娱乐”规则会管）。
- **想要自定义的触发条件**（特定分类、特定时段、特定阈值、不依赖今日目标等）：用 `create_detection_rule`。

## trigger 声明式 schema

`create_detection_rule(name, trigger, response?, severity?, cooldown_minutes?)`。`trigger` 是一个对象：

| 字段 | 说明 | 默认 |
|---|---|---|
| `category_prefix` | 分类前缀匹配，如 `"entertainment"`、`"entertainment.game"` | — |
| `category_in` | 精确分类白名单数组（与 prefix 或关系） | `[]` |
| `app_in` | 目标 app 包名/bundle id 数组 | `[]` |
| `window_minutes` | 统计窗口（分钟） | `30` |
| `min_minutes_in_window` | **阈值**：窗口内匹配时长 ≥ 此值则命中（必填正数） | — |
| `requires_active_goal` | 是否要求今日有未完成目标才触发 | `true` |
| `time_of_day` | 生效时段 `{"from":"HH:MM","to":"HH:MM"}`（本地时间，`from>to` 视为跨午夜） | 全天 |

**至少要指定一个** `category_prefix` / `category_in` / `app_in`，否则规则会命中一切、会被拒绝。

分类取值参考（与采集器一致）：`entertainment.video` / `entertainment.game` / `entertainment.social` / `entertainment.news`。

## response 策略（可选）

`response` 决定命中后怎么回应，默认 `{"policy":"immediate","kind":"notify"}`：

- `{"policy":"immediate","kind":"notify"}` — 立即系统通知。
- `{"policy":"immediate","kind":"pet_message"}` — 立即宠物气泡。
- `{"policy":"deferred","after_ms":600000}` — 延后 10 分钟再提醒（先观察一会儿）。
- `{"policy":"debounce","window_ms":2700000,"dedup_key":"night-game"}` — 窗口内只提醒一次，防打扰。
- `{"policy":"suppress"}` — 不打扰（占位，用于临时静音某规则）。

`severity`: `"medium"`（默认）/ `"high"`（文案带 ⚠️）。`cooldown_minutes`: 同规则两次提醒最小间隔，默认 30。

## 例子

- “我一到晚上就打游戏停不下来，帮我盯着” →
  `create_detection_rule("夜间游戏", {"category_prefix":"entertainment.game","time_of_day":{"from":"20:00","to":"02:00"},"window_minutes":30,"min_minutes_in_window":20,"requires_active_goal":false})`
- “工作日只要刷抖音超过 10 分钟就提醒我，哪怕没设目标” →
  `create_detection_rule("抖音超时", {"app_in":["com.ss.android.ugc.aweme"],"window_minutes":30,"min_minutes_in_window":10,"requires_active_goal":false})`
- “我刷视频太多了，先别急着弹，刷够 25 分钟再温柔提醒一次” →
  `create_detection_rule("视频节制", {"category_prefix":"entertainment.video","min_minutes_in_window":25}, {"policy":"debounce","window_ms":3600000,"dedup_key":"video-limit"})`

建完复述一句让用户确认：规则名 + 触发条件 + 会怎么提醒。用户想关掉/删掉时用 `set_detection_rule_enabled` / `delete_detection_rule`。

## response 策略（都已可用，2026-07-30 修复）

| policy | 语义 | 何时选它 |
|---|---|---|
| `immediate` | 命中即弹（`kind=notify` 或 `pet_message`）| 默认；需要当场打断 |
| `deferred` | `now + after_ms` 再弹 | "先别烦我，过 10 分钟还在刷再说" |
| `debounce` | `window_ms` 内同 `dedup_key` 只提醒一次 | 高频命中场景防打扰 |
| `suppress` | 不打扰 | 夜间免打扰、明确不想被提醒的时段 |

> `deferred` / `debounce` 此前有 bug（冷却按"通知弹出过"判断，而这两种策略命中时只入队 →
> 冷却永远 ready → 采集器每 5–15s 重复入队 → 十分钟后几十条通知一起炸）。现在冷却与去重
> 都看 `rule_fires`（命中当拍就记），`window_ms` 也真的生效。可以放心用。

## 命中优先级

同一拍多条规则命中时，按 `severity` 取最高；同级时**用户/智能体建的动态规则优先于内置规则**。
所以要让某条规则更容易胜出，给它 `severity: "high"`。
