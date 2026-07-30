/// 投影层：线性轴与折叠轴之间的**唯一**坐标映射。
///
/// 折叠不是另一个视图，而是同一条轴换了投影：把线性轴按周期取模，一个周期一行。
/// 因此转场动画就是把 `t` 从 0 推到 1，在两组坐标之间插值 ——
/// 视觉上每一天的线段被逐渐拉直并堆叠起来（"弹簧被压紧"），
/// 而不需要任何 3D。3D 螺旋在截图里好看，但远端遮挡近端、透视让长度不可比，
/// 恰好毁掉折叠唯一的价值：**不同日期的同一时刻落在同一条竖线上**。

import type { AxisCell, Fold, FoldRow } from "./types";
import { clamp, lerp, type Layout } from "./layout";

export interface Rect {
  x1: number;
  x2: number;
  y: number;
  h: number;
  /// 折叠行号；线性模式为 -1。
  row: number;
}

interface CellIndex {
  start_ms: number;
  end_ms: number;
  row: number;
  col: number;
}

export interface ProjectionInput {
  layout: Layout;
  fold: Fold;
  /// 折叠进度：0 = 纯线性，1 = 完全折叠。转场期间取中间值。
  t: number;
  startMs: number;
  endMs: number;
  rows: FoldRow[];
  cols: number;
  cells: AxisCell[];
  doublePlot: boolean;
}

export class Projection {
  readonly layout: Layout;
  readonly fold: Fold;
  readonly t: number;
  readonly startMs: number;
  readonly endMs: number;
  readonly rows: FoldRow[];
  readonly cols: number;
  readonly doublePlot: boolean;
  /// 一行代表的名义时长（取首行，DST 长短日只影响那一行的填充比例）。
  readonly rowSpan: number;
  /// 折叠后横轴的可用宽度：双绘时一行画 48 小时，所以单日只占一半。
  readonly foldWidth: number;
  private readonly cellIndex: CellIndex[];
  private readonly cellByRowCol: Map<string, CellIndex>;

  constructor(input: ProjectionInput) {
    this.layout = input.layout;
    this.fold = input.fold;
    this.t = clamp(input.t, 0, 1);
    this.startMs = input.startMs;
    this.endMs = input.endMs;
    this.rows = input.rows;
    this.cols = Math.max(1, input.cols);
    this.doublePlot = input.doublePlot && input.fold === "day";
    this.rowSpan = input.rows.length
      ? input.rows[0].end_ms - input.rows[0].start_ms
      : 86_400_000;
    this.foldWidth = this.doublePlot ? input.layout.plotWidth / 2 : input.layout.plotWidth;
    this.cellIndex = input.cells
      .map((cell) => ({
        start_ms: cell.start_ms,
        end_ms: cell.end_ms,
        row: cell.row,
        col: cell.col,
      }))
      .sort((a, b) => a.start_ms - b.start_ms);
    this.cellByRowCol = new Map(
      this.cellIndex.map((cell) => [`${cell.row}:${cell.col}`, cell]),
    );
  }

  get folded(): boolean {
    return this.t > 0.001 && this.rows.length > 0;
  }

  get span(): number {
    return Math.max(1, this.endMs - this.startMs);
  }

  /// 线性投影：时间 → x。
  linearX(ms: number): number {
    return this.layout.plotLeft + ((ms - this.startMs) / this.span) * this.layout.plotWidth;
  }

  /// 线性反投影：x → 时间。
  timeAtX(x: number): number {
    return (
      this.startMs + ((x - this.layout.plotLeft) / this.layout.plotWidth) * this.span
    );
  }

  rowContaining(ms: number): FoldRow | undefined {
    let lo = 0;
    let hi = this.rows.length - 1;
    while (lo <= hi) {
      const mid = (lo + hi) >> 1;
      const row = this.rows[mid];
      if (ms < row.start_ms) hi = mid - 1;
      else if (ms >= row.end_ms) lo = mid + 1;
      else return row;
    }
    return undefined;
  }

  private cellContaining(ms: number): CellIndex | undefined {
    let lo = 0;
    let hi = this.cellIndex.length - 1;
    while (lo <= hi) {
      const mid = (lo + hi) >> 1;
      const cell = this.cellIndex[mid];
      if (ms < cell.start_ms) hi = mid - 1;
      else if (ms >= cell.end_ms) lo = mid + 1;
      else return cell;
    }
    return undefined;
  }

  /// 折叠投影：时间 → 行内 x。
  ///
  /// 按日折叠时用行内偏移（连续，会话能画成条）；
  /// 周/月/年折叠时用后端给的列号（离散，短月右侧留空，列在行之间才可比）。
  foldX(ms: number, row: FoldRow): number {
    if (this.fold === "day") {
      const span = Math.max(1, row.end_ms - row.start_ms);
      return this.layout.plotLeft + ((ms - row.start_ms) / span) * this.foldWidth;
    }
    const cell = this.cellContaining(ms);
    if (!cell) {
      const span = Math.max(1, row.end_ms - row.start_ms);
      return this.layout.plotLeft + ((ms - row.start_ms) / span) * this.foldWidth;
    }
    const frac = clamp((ms - cell.start_ms) / Math.max(1, cell.end_ms - cell.start_ms), 0, 1);
    return this.layout.plotLeft + ((cell.col + frac) / this.cols) * this.layout.plotWidth;
  }

  /// 列 → x 区间（折叠格子用）。
  colRange(col: number): { x1: number; x2: number } {
    const width = this.layout.plotWidth / this.cols;
    const x1 = this.layout.plotLeft + col * width;
    return { x1, x2: x1 + width };
  }

  /// 小时格子（按日折叠的长跨度档）的 x 区间。
  hourRange(col: number): { x1: number; x2: number } {
    const width = this.foldWidth / 24;
    const x1 = this.layout.plotLeft + col * width;
    return { x1, x2: x1 + width };
  }

  /// 把一个时间区间放到画布上。折叠时按行切段，跨日的会话会自然分成两块。
  ///
  /// `linearTop` / `linearHeight` 是该轨道在线性布局里的位置，
  /// 折叠时它们只是插值起点。
  place(startMs: number, endMs: number, linearTop: number, linearHeight: number): Rect[] {
    const end = Math.max(endMs, startMs);
    if (!this.folded) {
      return [
        {
          x1: this.linearX(startMs),
          x2: this.linearX(end),
          y: linearTop,
          h: linearHeight,
          row: -1,
        },
      ];
    }
    const out: Rect[] = [];
    for (const row of this.rows) {
      if (row.end_ms <= startMs || row.start_ms >= end) continue;
      const from = Math.max(startMs, row.start_ms);
      const to = Math.min(end, row.end_ms);
      if (to < from) continue;
      const box = this.layout.rows.find((candidate) => candidate.index === row.index);
      if (!box) continue;
      out.push({
        x1: lerp(this.linearX(from), this.foldX(from, row), this.t),
        x2: lerp(this.linearX(to), this.foldX(to, row), this.t),
        y: lerp(linearTop, box.top, this.t),
        h: lerp(linearHeight, box.height, this.t),
        row: row.index,
      });
      // 双绘：行 n 同时显示第 n 天与第 n+1 天，跨午夜的模式才看得出来。
      if (this.doublePlot) {
        const previous = this.layout.rows.find(
          (candidate) => candidate.index === row.index - 1,
        );
        if (previous) {
          out.push({
            x1: lerp(this.linearX(from), this.foldX(from, row) + this.foldWidth, this.t),
            x2: lerp(this.linearX(to), this.foldX(to, row) + this.foldWidth, this.t),
            y: lerp(linearTop, previous.top, this.t),
            h: lerp(linearHeight, previous.height, this.t),
            row: row.index - 1,
          });
        }
      }
    }
    return out;
  }

  /// 点事件（capture / 干预 / 里程碑）的位置。
  places(ms: number, linearTop: number, linearHeight: number): Rect[] {
    return this.place(ms, ms, linearTop, linearHeight);
  }

  // ── 相位轴（折叠时刻度尺的坐标）──────────────────────────────────────────

  /// 相位轴的总长度：按日折叠是 24h（双绘 48h）的毫秒数，日历折叠是列数。
  get phaseSpan(): number {
    if (this.fold === "day") return this.rowSpan * (this.doublePlot ? 2 : 1);
    return this.cols;
  }

  phaseX(phase: number): number {
    return this.layout.plotLeft + (phase / this.phaseSpan) * this.layout.plotWidth;
  }

  phaseAtX(x: number): number {
    const ratio = clamp((x - this.layout.plotLeft) / this.layout.plotWidth, 0, 1);
    return ratio * this.phaseSpan;
  }

  /// 相位选区 → 绝对时间窗口列表（每行一段）。
  ///
  /// 这是折叠视图存在的最强理由：框住"每天 22:00–02:00"这一竖条，
  /// 就能问出线性视图问不出的问题——而统计口径和线性选区完全一样。
  windowsForPhase(a: number, b: number): Array<[number, number]> {
    const from = Math.min(a, b);
    const to = Math.max(a, b);
    const out: Array<[number, number]> = [];
    if (this.fold === "day") {
      for (const row of this.rows) {
        out.push([row.start_ms + from, row.start_ms + to]);
      }
      return out;
    }
    const colFrom = Math.floor(from);
    const colTo = Math.max(colFrom, Math.ceil(to) - 1);
    for (const row of this.rows) {
      let start: number | null = null;
      let end: number | null = null;
      for (let col = colFrom; col <= colTo; col += 1) {
        const cell = this.cellByRowCol.get(`${row.index}:${col}`);
        if (!cell) continue;
        if (start === null) start = cell.start_ms;
        end = cell.end_ms;
      }
      if (start !== null && end !== null) out.push([start, end]);
    }
    return out;
  }

  /// 相位区间在某一行里对应的绝对时间（用于绘制选区高亮）。
  phaseWindowInRow(row: FoldRow, a: number, b: number): [number, number] | null {
    if (this.fold === "day") {
      return [row.start_ms + Math.min(a, b), row.start_ms + Math.max(a, b)];
    }
    const colFrom = Math.floor(Math.min(a, b));
    const colTo = Math.max(colFrom, Math.ceil(Math.max(a, b)) - 1);
    let start: number | null = null;
    let end: number | null = null;
    for (let col = colFrom; col <= colTo; col += 1) {
      const cell = this.cellByRowCol.get(`${row.index}:${col}`);
      if (!cell) continue;
      if (start === null) start = cell.start_ms;
      end = cell.end_ms;
    }
    return start !== null && end !== null ? [start, end] : null;
  }
}
