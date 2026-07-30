/// 技能树的前端类型：与 `core::skillmap` 的序列化一一对应。
///
/// **不要在前端补字段或补算术**——扇区角度、依赖深度、四态、领域掌握度都由 Core 给。
/// 前端只做两件事：投影（极坐标 → 屏幕，见 `projection.ts`）与绘制（`draw.ts`）。

export type NodeState = "attained" | "in_progress" | "available" | "locked";

export interface SkillSector {
  index: number;
  area_id: string | null;
  name: string;
  focus: boolean;
  /** 扇区角区间（弧度）。前端不算角度。 */
  start_angle: number;
  end_angle: number;
  /** 该领域技能进度的等权平均 = 雷达顶点半径。 */
  mastery: number;
  attained: number;
  total: number;
  /** 扇区环上的槽位总数。 */
  slots: number;
}

export interface SkillNode {
  id: string;
  kind: "skill" | "milestone";
  title: string;
  area_id: string | null;
  track: string;
  status: string;
  /** 里程碑的父技能。 */
  parent_id: string | null;
  depends_on: string[];
  /** 其中还没达成的前置——UI 要说得出"需先完成 X"。 */
  blocked_by: string[];
  state: NodeState;
  progress: number;
  done_leaves: number;
  total_leaves: number;
  depth: number;
  sector: number;
  slot: number;
  slot_count: number;
  created_at: number;
  due_at_ms: number | null;
  success_criteria: string | null;
  target_value: number | null;
  current_value: number | null;
  unit: string | null;
  goal_count: number;
}

export interface SkillEdge {
  from: string;
  to: string;
  satisfied: boolean;
}

export interface IdeaMote {
  id: string;
  title: string;
  area_id: string | null;
  sector: number;
  created_at: number;
  review_at_ms: number | null;
  due_review: boolean;
}

export interface SkillMap {
  at_ms: number;
  sectors: SkillSector[];
  nodes: SkillNode[];
  edges: SkillEdge[];
  ideas: IdeaMote[];
  max_depth: number;
  attained: number;
  total: number;
}

export interface ProgressChange {
  item_id: string;
  at_ms: number;
  progress: number;
  done_leaves: number;
  total_leaves: number;
  state: NodeState;
}

export interface SectorChange {
  sector: number;
  at_ms: number;
  mastery: number;
}

export interface Growth {
  from_ms: number;
  to_ms: number;
  instants: number[];
  changes: ProgressChange[];
  sectors: SectorChange[];
}

export const EMPTY_MAP: SkillMap = {
  at_ms: 0,
  sectors: [],
  nodes: [],
  edges: [],
  ideas: [],
  max_depth: 0,
  attained: 0,
  total: 0,
};
