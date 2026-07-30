/// 布局几何。线性视图与折叠视图共用一套盒子计算，
/// 好处是转场动画只需要在两组 y 之间插值，而不是切换两套渲染路径。

import type { FoldRow } from "./types";
import { TRACKS, type TrackId, type TrackViewState } from "./tracks";

export const RULER_H = 30;
export const HEADER_W = 128;
export const TRACK_GAP = 5;
export const COLLAPSED_H = 16;
export const PAD_BOTTOM = 6;
/// 行高下限：折叠到几百行时（≈ 一年 actogram）每行只有 1–2px，仍然是可读的密度图。
/// 低于它就宁可画不下并明说，而不是悄悄少画几十行。
export const MIN_ROW_H = 1;
export const MAX_ROW_H = 64;

export interface TrackBox {
  id: TrackId;
  top: number;
  height: number;
  collapsed: boolean;
}

export interface RowBox {
  index: number;
  top: number;
  height: number;
}

export interface Layout {
  width: number;
  height: number;
  headerW: number;
  rulerH: number;
  plotLeft: number;
  plotWidth: number;
  plotTop: number;
  plotHeight: number;
  /// 线性视图的轨道盒子（折叠时仍算出来，转场要用它做起点）。
  tracks: TrackBox[];
  /// 折叠视图的行盒子。
  rows: RowBox[];
  rowHeight: number;
  /// 实际能画下的行数（超出的部分不画，由调用方提示）。
  visibleRows: number;
}

export function computeLayout(
  width: number,
  height: number,
  states: TrackViewState[],
  rows: FoldRow[],
  compact: boolean,
): Layout {
  const headerW = compact ? 54 : HEADER_W;
  const plotLeft = headerW;
  const plotWidth = Math.max(40, width - headerW);
  const plotTop = RULER_H;
  const plotHeight = Math.max(40, height - RULER_H - PAD_BOTTOM);

  const expanded = states.filter((state) => !state.collapsed);
  const weightTotal = expanded.reduce((sum, state) => sum + weightOf(state.id), 0) || 1;
  const collapsedTotal = states.length - expanded.length;
  const gaps = TRACK_GAP * Math.max(0, states.length - 1);
  const flexible = Math.max(24, plotHeight - gaps - collapsedTotal * COLLAPSED_H);

  const tracks: TrackBox[] = [];
  let cursor = plotTop;
  for (const state of states) {
    const boxHeight = state.collapsed
      ? COLLAPSED_H
      : (weightOf(state.id) / weightTotal) * flexible;
    tracks.push({ id: state.id, top: cursor, height: boxHeight, collapsed: state.collapsed });
    cursor += boxHeight + TRACK_GAP;
  }

  const rowHeight = rows.length
    ? clamp(plotHeight / rows.length, MIN_ROW_H, MAX_ROW_H)
    : 0;
  const visibleRows = rowHeight ? Math.min(rows.length, Math.floor(plotHeight / rowHeight)) : 0;
  const rowBoxes: RowBox[] = rows.slice(0, visibleRows).map((row, index) => ({
    index: row.index,
    top: plotTop + index * rowHeight,
    height: rowHeight,
  }));

  return {
    width,
    height,
    headerW,
    rulerH: RULER_H,
    plotLeft,
    plotWidth,
    plotTop,
    plotHeight,
    tracks,
    rows: rowBoxes,
    rowHeight,
    visibleRows,
  };
}

function weightOf(id: TrackId): number {
  return TRACKS.find((track) => track.id === id)?.weight ?? 1;
}

export function trackBox(layout: Layout, id: TrackId): TrackBox | undefined {
  return layout.tracks.find((box) => box.id === id);
}

export function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

export function lerp(a: number, b: number, t: number): number {
  return a + (b - a) * t;
}

/// 平滑过渡（用于 LOD 交叉淡入与折叠转场）。
export function smoothstep(a: number, b: number, value: number): number {
  const x = clamp((value - a) / (b - a || 1), 0, 1);
  return x * x * (3 - 2 * x);
}
