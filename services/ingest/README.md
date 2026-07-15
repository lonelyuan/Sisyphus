# services/ingest — Supabase 中央事实库（Phase 2 同步）

> **阶段:同步是 Phase 2,不在任何 MVP 闭环关键路径上**（每台设备本地闭环,见 [`../../docs/spec/architecture.md`](../../docs/spec/architecture.md) §5、[`../../docs/spec/sync.md`](../../docs/spec/sync.md)）。本服务是届时的中央事实库,现阶段不必启用。

底层是 **Postgres**——为了将来 Python ML 能直连同一份数据、零迁移而特意选的。

## 角色（Phase 2）

| 承担 | 实现 |
|---|---|
| 中央事实库：`raw_events` append-only，跨端联合分析基础 | Postgres 表 + 幂等摄取 |
| 跨端联动（更后期）：`command_queue` / `global_state` | Postgres + Supabase Realtime（暂不启用） |

> 端侧本地是 append-only Event log（SQLite）,靠 outbox 最终一致地批量上传到此处。重型 ML 是将来追加的 Python worker,直连此 Postgres。

## 目录

```
supabase/
  config.toml              本地开发配置
  migrations/0001_init.sql raw_events / global_state / command_queue + 索引
  functions/ingest/        幂等批量摄取 Edge Function
```

## 前置

```bash
# 安装 Supabase CLI (macOS)
brew install supabase/tap/supabase
# 或见 https://supabase.com/docs/guides/cli
```

## 本地开发

```bash
cd services/ingest
supabase init            # 若需补全 config（已有 config.toml 可跳过冲突项）
supabase start           # 拉起本地 Postgres + Studio + API (Docker)
supabase db reset        # 应用 migrations/ 到本地库
supabase functions serve ingest   # 本地跑摄取函数
```

Studio: http://localhost:54323　API: http://localhost:54321

## 连接云端项目（部署）

```bash
supabase login
supabase link --project-ref <your-project-ref>
supabase db push                 # 推送 migrations 到云端
supabase functions deploy ingest # 部署摄取函数
```

免费档注意：项目 7 天无活动会暂停；本项目天天采集不会触发。

## 端如何上报

两种方式，二选一：

1. **Edge Function**（含轻校验，推荐）
   ```
   POST {SUPABASE_URL}/functions/v1/ingest
   Authorization: Bearer <anon-or-service-key>
   Content-Type: application/json
   { "events": [ <BehaviorEvent>, ... ] }     // 最多 500 条/批
   ```

2. **PostgREST 直插**（省函数）
   ```
   POST {SUPABASE_URL}/rest/v1/raw_events
   Prefer: resolution=ignore-duplicates       // 幂等
   ```

两种都以 `event_id` 幂等，断网重传无副作用。

## 待办（超出 MVP 范围）

- pg_cron 定时跑 sessionizer / 日终汇总
- Edge Function 写 `global_state` 供端订阅
- 多用户时开启各表 RLS（见 migration 末尾注释）
