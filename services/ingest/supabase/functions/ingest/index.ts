// Supabase Edge Function: 幂等批量事件摄取
// POST /functions/v1/ingest   body: { events: BehaviorEvent[] }
//
// 端侧 outbox 批量上传到此处。以 event_id 幂等：重复上报 ON CONFLICT DO NOTHING。
// 部署: supabase functions deploy ingest
//
// 注：也可直接用 PostgREST (POST /rest/v1/raw_events, Prefer: resolution=ignore-duplicates)
// 省掉本函数；保留它是为了将来在摄取时做轻校验 / 触发引擎。

import { createClient } from "jsr:@supabase/supabase-js@2";

const MAX_BATCH = 500;

Deno.serve(async (req) => {
  if (req.method !== "POST") {
    return new Response("Method Not Allowed", { status: 405 });
  }

  let body: { events?: unknown[] };
  try {
    body = await req.json();
  } catch {
    return json({ error: "invalid json" }, 400);
  }

  const events = body.events;
  if (!Array.isArray(events) || events.length === 0) {
    return json({ error: "events[] required" }, 400);
  }
  if (events.length > MAX_BATCH) {
    return json({ error: `batch too large (max ${MAX_BATCH})` }, 413);
  }

  const supabase = createClient(
    Deno.env.get("SUPABASE_URL")!,
    Deno.env.get("SUPABASE_SERVICE_ROLE_KEY")!,
  );

  // 幂等 upsert：event_id 冲突则忽略（append-only，不覆盖）。
  const { error } = await supabase
    .from("raw_events")
    .upsert(events as Record<string, unknown>[], {
      onConflict: "event_id",
      ignoreDuplicates: true,
    });

  if (error) {
    return json({ error: error.message }, 500);
  }
  return json({ accepted: events.length }, 200);
});

function json(obj: unknown, status: number): Response {
  return new Response(JSON.stringify(obj), {
    status,
    headers: { "content-type": "application/json" },
  });
}
