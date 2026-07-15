# Spec: 同步协议

> **阶段标注：同步是 Phase 2，不在任何 MVP 闭环的关键路径上。**
> 干预天然本地触发（在哪台设备产生的行为就在哪台弹提醒），每台设备跑自己的本地 Event log + 本地引擎 + 本地通知，闭环**不需要同步**。同步只服务后期的跨端联合分析。见 [architecture.md](architecture.md) §5。本文件描述届时的目标协议，现阶段不实现。

---

## 职责分工

| 角色 | 技术 | 职责 |
|---|---|---|
| Rust（src-tauri）| `reqwest` | outbox 批量 POST → Supabase REST API |
| TypeScript（src/）| `@supabase/supabase-js` | Realtime 订阅（command_queue）、Agent 上下文查询 |

Supabase 没有官方 Rust SDK，但其 REST API（PostgREST）是标准 HTTP，`reqwest` 直接调用即可。

---

## Rust → Supabase：outbox drain

### 触发时机

- Android：WorkManager 定时任务（不依赖前台服务，15min+ 粒度可接受）
- Desktop：系统定时器，每 ~2min 执行一次

### 上传流程

```
1. SELECT * FROM outbox WHERE sync_status = 'pending' LIMIT 500
2. UPDATE outbox SET sync_status = 'uploading' WHERE event_id IN (...)
3. POST https://<project>.supabase.co/rest/v1/raw_events
   Headers:
     apikey: <anon_key>
     Authorization: Bearer <anon_key>
     Content-Type: application/json
     Prefer: resolution=ignore-duplicates   ← 幂等：event_id 重复则跳过
   Body: JSON array of BehaviorEvent
4a. 成功（2xx）→ UPDATE sync_status = 'done'
4b. 失败      → UPDATE retry_count = retry_count + 1, sync_status = 'pending'
               退避时间：min(2^retry_count * 30s, 1h)
5. DELETE FROM outbox WHERE sync_status = 'done'  ← 定期清理
```

### 幂等保证

Supabase 端：`raw_events.event_id` 为主键，重复 insert 自动忽略（`ON CONFLICT DO NOTHING`）。  
客户端：`Prefer: resolution=ignore-duplicates`。  
网络重试无副作用。

---

## TypeScript → Supabase：Realtime + 查询

TypeScript 侧使用 `@supabase/supabase-js`，在 Tauri WebView 内运行（浏览器兼容构建）。

### command_queue 订阅（干预指令接收）

```typescript
import { createClient } from '@supabase/supabase-js'

const supabase = createClient(SUPABASE_URL, ANON_KEY)

supabase
  .channel('commands')
  .on('postgres_changes', {
    event: 'INSERT',
    schema: 'public',
    table: 'command_queue',
    filter: `target_device_id=eq.${deviceId}`,
  }, (payload) => {
    // 收到干预指令 → 调用 Rust show_notification command
    invoke('show_notification', { command: payload.new })
  })
  .subscribe()
```

### Agent 上下文查询

见 [agent.md](agent.md)——由 Rust 侧 `get_today_context` command 从本地 SQLite 构建，不走 Supabase（离线可用）。  
Supabase 查询仅用于跨端视图（如时间线展示所有设备的事件）。

---

## API Key 管理

| Key | 存储位置 | 用途 |
|---|---|---|
| Supabase `anon_key` | Tauri Store（用户设置页填入）或环境变量 | Rust outbox drain + TS Realtime |
| Anthropic API Key | Tauri Store（用户设置页填入）| Agent 调用 |

严禁硬编码到代码或配置文件中。
