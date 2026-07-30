/// 技能树几何的断言脚本。项目**没有前端测试框架**（也不要加），用 jiti 直接跑 TS：
///
///   node node_modules/.pnpm/jiti@*/node_modules/jiti/lib/jiti-cli.mjs scripts/skilltree-geometry-check.ts
///
/// 守三件事：
/// 1. 雷达是**换投影**不是第二套坐标：t=0 半径由依赖深度决定，t=1 由掌握度决定，中间单调。
/// 2. 弹性松弛**确定性**：同输入两次得到同一坐标（否则节点位置每次打开都变，空间记忆没了）。
/// 3. 弹簧不许把语义弄脏：角度不出扇区，半径不出目标环 ±RADIUS_SLACK。

import { createBody, relax, RADIUS_SLACK, type Body } from "../src/skilltree/physics";
import { place, radarRadius, sectorAngle, treeRadius } from "../src/skilltree/projection";
import type { SkillNode, SkillSector } from "../src/skilltree/types";

let failures = 0;
function check(label: string, condition: boolean, detail = "") {
  if (condition) {
    console.log(`  ok  ${label}`);
  } else {
    failures += 1;
    console.error(`FAIL  ${label}${detail ? ` — ${detail}` : ""}`);
  }
}
function near(a: number, b: number, tolerance = 1e-9) {
  return Math.abs(a - b) < tolerance;
}

const TAU = Math.PI * 2;

function sector(index: number, start: number, end: number, mastery = 0): SkillSector {
  return {
    index,
    area_id: `area-${index}`,
    name: `领域${index}`,
    focus: false,
    start_angle: start,
    end_angle: end,
    mastery,
    attained: 0,
    total: 0,
    slots: 3,
  };
}

function node(id: string, over: Partial<SkillNode> = {}): SkillNode {
  return {
    id,
    kind: "skill",
    title: id,
    area_id: "area-0",
    track: "undecided",
    status: "active",
    parent_id: null,
    depends_on: [],
    blocked_by: [],
    state: "available",
    progress: 0,
    done_leaves: 0,
    total_leaves: 1,
    depth: 0,
    sector: 0,
    slot: 0,
    slot_count: 3,
    created_at: 0,
    due_at_ms: null,
    success_criteria: null,
    target_value: null,
    current_value: null,
    unit: null,
    goal_count: 0,
    ...over,
  };
}

// ── 1. 投影：t 只在"半径的含义"之间插值，角度永远来自扇区 ──────────────────
{
  console.log("投影（雷达 = 换投影，不是第二个视图）");
  const s = sector(0, -Math.PI / 2, -Math.PI / 2 + TAU / 3);
  const deep = node("deep", { depth: 3, progress: 0 });
  const maxDepth = 4;

  const tree = place(deep, s, maxDepth, deep.progress, 0);
  const radar = place(deep, s, maxDepth, deep.progress, 1);
  check("t=0 的半径 = treeRadius(depth)", near(tree.radius, treeRadius(3, maxDepth)));
  check("t=1 的半径 = radarRadius(progress)", near(radar.radius, radarRadius(0)));
  check(
    "角度与 t 无关（角度 = 领域，这条图例在任何投影下都成立）",
    near(tree.angle, radar.angle) && near(tree.angle, sectorAngle(s, deep.slot, deep.slot_count)),
  );

  // 中间是单调插值：不许在中途反向或跳变。
  let monotonic = true;
  let previous = tree.radius;
  const descending = radar.radius < tree.radius;
  for (let i = 1; i <= 20; i += 1) {
    const r = place(deep, s, maxDepth, deep.progress, i / 20).radius;
    if (descending ? r > previous + 1e-12 : r < previous - 1e-12) monotonic = false;
    previous = r;
  }
  check("t 在 0→1 之间单调插值", monotonic);

  // 掌握度越高，雷达投影越靠外——"我在这个领域到哪一步"必须读得出来。
  check(
    "radarRadius 随进度递增",
    radarRadius(0) < radarRadius(0.5) && radarRadius(0.5) < radarRadius(1),
  );
  check("treeRadius 随深度递增", treeRadius(0, 4) < treeRadius(1, 4) && treeRadius(1, 4) < treeRadius(4, 4));

  // 槽位分角：两端各留半格，节点不会压在扇区分界线上。
  const first = sectorAngle(s, 0, 3);
  const last = sectorAngle(s, 2, 3);
  check("槽位角在扇区内且不贴边界", first > s.start_angle && last < s.end_angle);
}

// ── 2 & 3. 物理：确定性 + 不越界 ──────────────────────────────────────────
{
  console.log("弹性松弛（确定性 + 不许越界）");
  const sectors = [
    sector(0, -Math.PI / 2, -Math.PI / 2 + TAU / 2),
    sector(1, -Math.PI / 2 + TAU / 2, -Math.PI / 2 + TAU),
  ];
  const nodes: SkillNode[] = [
    node("a", { depth: 0, sector: 0, slot: 0, slot_count: 3 }),
    // 故意把 b、c 放在同一槽位：不加斥力它们会完全重合。
    node("b", { depth: 1, sector: 0, slot: 1, slot_count: 3, depends_on: ["a"] }),
    node("c", { depth: 1, sector: 0, slot: 1, slot_count: 3, depends_on: ["a"] }),
    node("d", { depth: 2, sector: 1, slot: 0, slot_count: 2, depends_on: ["b"] }),
  ];
  const maxDepth = 2;

  const build = (): Body[] =>
    nodes.map((n) => {
      const s = sectors[n.sector];
      return createBody(
        n.id,
        place(n, s, maxDepth, n.progress, 0),
        { start: s.start_angle, end: s.end_angle },
        13,
        n.depends_on,
      );
    });

  const first = relax(build(), 400, 260);
  const second = relax(build(), 400, 260);
  const identical = first.every((body, index) => {
    const other = second[index];
    return near(body.angle, other.angle, 1e-12) && near(body.radius, other.radius, 1e-12);
  });
  check("同输入两次松弛得到同一坐标（无 Math.random）", identical);

  const inBounds = first.every(
    (body) =>
      body.angle >= body.minAngle - 1e-9 &&
      body.angle <= body.maxAngle + 1e-9 &&
      body.radius >= body.target.radius - RADIUS_SLACK - 1e-9 &&
      body.radius <= body.target.radius + RADIUS_SLACK + 1e-9,
  );
  check("角度不出扇区、半径不出目标环 ±slack", inBounds);

  const b = first.find((body) => body.id === "b")!;
  const c = first.find((body) => body.id === "c")!;
  check("同槽位的两个节点被推开，不再重合", Math.abs(b.angle - c.angle) + Math.abs(b.radius - c.radius) > 1e-4);

  const d = first.find((body) => body.id === "d")!;
  check("跨扇区节点留在自己的扇区里", d.angle >= sectors[1].start_angle && d.angle <= sectors[1].end_angle);

  // 相邻深度环的间距必须大于弹簧的径向余量，否则"半径 = 要先会什么"会被物理读错。
  // Core 的 MAX_DEPTH 是 8，所以最坏情况要在 maxDepth=8 时也成立。
  let ringsSeparated = true;
  for (let maxD = 0; maxD <= 8; maxD += 1) {
    const gap = treeRadius(1, maxD) - treeRadius(0, maxD);
    if (gap <= 2 * RADIUS_SLACK) ringsSeparated = false;
  }
  check("相邻深度环的间距 > 2×径向余量（含 maxDepth=8 最坏情况）", ringsSeparated);
  const a = first.find((body) => body.id === "a")!;
  check("实际落位：depth 0 仍在 depth 1 内侧", a.radius < b.radius);
}

if (failures > 0) {
  console.error(`\n${failures} 项断言失败`);
  process.exit(1);
}
console.log("\n技能树几何断言全部通过");
