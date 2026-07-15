-- Sisyphus 中央事实库 v1.0
-- 事件协议见 packages/protocol/SPEC.md
-- 原则：raw_events append-only、不可变、event_id 幂等。它是唯一事实来源。

-- ---------------------------------------------------------------------------
-- raw_events：所有层（raw/session/finding/decision/intervention/outcome/feedback）
-- 的统一落点。一张表、一条 append-only 流。
-- ---------------------------------------------------------------------------
create table if not exists raw_events (
  event_id          uuid        primary key,           -- 端侧生成，幂等主键
  schema_version    text        not null default '1.0',
  user_id           text        not null,
  device_id         text        not null,
  seq_no            bigint      not null,              -- 单设备单调递增
  source            text        not null,
  layer             text        not null,
  type              text        not null,
  time_mode         text        not null check (time_mode in ('point','interval')),
  event_time        timestamptz,                       -- point
  start_time        timestamptz,                       -- interval
  end_time          timestamptz,
  entity            text,
  category          text,
  payload           jsonb       not null default '{}'::jsonb,
  parent_event_ids  uuid[]      not null default '{}',
  privacy_level     text        not null default 'L0',
  produced_at       timestamptz not null,
  ingested_at       timestamptz not null default now(),

  constraint time_mode_fields check (
    (time_mode = 'point'    and event_time is not null) or
    (time_mode = 'interval' and start_time is not null)
  )
);

-- 增量拉取 / 查漏：按设备的 seq_no。
create unique index if not exists raw_events_device_seq
  on raw_events (device_id, seq_no);

-- 引擎按用户 + 层 + 时间窗查询（会话聚合、规则窗口）。
create index if not exists raw_events_user_layer_time
  on raw_events (user_id, layer, coalesce(start_time, event_time) desc);

-- 按类别/实体查（娱乐会话识别）。
create index if not exists raw_events_user_category
  on raw_events (user_id, category)
  where category is not null;

-- 血缘反查。
create index if not exists raw_events_parents
  on raw_events using gin (parent_event_ids);

-- ---------------------------------------------------------------------------
-- global_state：联动平面（每用户一行）。MVP 仅作“状态刷新”，非事实来源。
-- 端通过 Supabase Realtime 订阅本表即可获得统一全局视野。
-- ---------------------------------------------------------------------------
create table if not exists global_state (
  user_id              text        primary key,
  today_goal           jsonb       not null default '{}'::jsonb,
  active_sessions      jsonb       not null default '[]'::jsonb,
  recent_findings      jsonb       not null default '[]'::jsonb,
  cooldowns            jsonb       not null default '{}'::jsonb,
  recommended_action   jsonb,
  last_intervention_at timestamptz,
  devices              jsonb       not null default '[]'::jsonb,
  updated_at           timestamptz not null default now()
);

-- ---------------------------------------------------------------------------
-- command_queue：下发到具体端的干预指令（端轮询或订阅后执行并回执）。
-- ---------------------------------------------------------------------------
create table if not exists command_queue (
  id               uuid        primary key default gen_random_uuid(),
  user_id          text        not null,
  target_device_id text        not null,
  decision_id      uuid,                                -- 指向 raw_events 中的 decision
  command          jsonb       not null,                -- { type, message, options, ... }
  status           text        not null default 'pending'
                    check (status in ('pending','delivered','executed','expired')),
  created_at       timestamptz not null default now(),
  delivered_at     timestamptz,
  executed_at      timestamptz
);

create index if not exists command_queue_pending
  on command_queue (user_id, target_device_id, status)
  where status = 'pending';

-- ---------------------------------------------------------------------------
-- RLS：MVP 单用户。生产多用户时为每表开启 RLS 并按 auth.uid() 限定。
--   alter table raw_events enable row level security;
--   create policy own_rows on raw_events using (user_id = auth.uid()::text);
-- ---------------------------------------------------------------------------
