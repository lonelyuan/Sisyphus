import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  ChevronDown,
  ChevronRight,
  Eye,
  EyeOff,
  Layers3,
  LocateFixed,
  Minus,
  Plus,
  Rows3,
  X,
} from "lucide-react";
import {
  DAY,
  EMPTY,
  FOLDS,
  FOLD_LABEL,
  type Detail,
  type Fold,
  type RangeStats,
  type TimelineEvent,
  type TimelineResponse,
} from "@/timeline/types";
import { computeLayout } from "@/timeline/layout";
import { Projection } from "@/timeline/projection";
import { drawScene, type HitRegion, type Selection } from "@/timeline/render";
import {
  DEFAULT_TRACK_STATE,
  TRACKS,
  effectiveVisible,
  type TrackId,
  type TrackViewState,
} from "@/timeline/tracks";
import { cn } from "@/lib/utils";

const MIN_SPAN = 15 * 60_000;
const MAX_SPAN = 10 * 365.25 * DAY;
const FOLD_MS = 420;

export default function TimelineScreen() {
  const [center, setCenter] = useState(() => Date.now());
  const [span, setSpan] = useState(DAY);
  /// `fold` 是目标档位，`renderFold` 是当前正在渲染的档位。
  /// 切换档位时先折回线性（t→0）再展开成新档位（t→1），转场才连续。
  const [fold, setFold] = useState<Fold>("none");
  const [renderFold, setRenderFold] = useState<Fold>("none");
  const [foldT, setFoldT] = useState(0);
  const [doublePlot, setDoublePlot] = useState(false);
  const [trackStates, setTrackStates] = useState<TrackViewState[]>(DEFAULT_TRACK_STATE);
  const [focus, setFocus] = useState<TrackId>("behavior");
  const [data, setData] = useState<TimelineResponse>(EMPTY);
  const [size, setSize] = useState({ width: 900, height: 460 });
  const [selection, setSelection] = useState<Selection | null>(null);
  const [stats, setStats] = useState<RangeStats | null>(null);
  const [selected, setSelected] = useState<TimelineEvent | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");

  const canvasRef = useRef<HTMLCanvasElement>(null);
  const surfaceRef = useRef<HTMLDivElement>(null);
  const hitRegions = useRef<HitRegion[]>([]);
  const viewRef = useRef({ center, span });
  const dragRef = useRef<
    | { mode: "pan"; x: number; y: number; center: number; moved: boolean }
    | { mode: "select"; from: number; moved: boolean }
    | null
  >(null);
  const animationRef = useRef<number | null>(null);

  viewRef.current = { center, span };
  const detail = useMemo(() => detailForSpan(span), [span]);
  const start = center - span / 2;
  const end = center + span / 2;
  const compact = size.width < 560;
  const visibleTracks = useMemo(() => effectiveVisible(trackStates), [trackStates]);

  // ── 折叠转场动画 ──────────────────────────────────────────────────────────
  const foldTRef = useRef(0);
  const animateFold = useCallback((to: number, onDone?: () => void) => {
    if (animationRef.current) cancelAnimationFrame(animationRef.current);
    const from = foldTRef.current;
    if (Math.abs(to - from) < 0.001) {
      onDone?.();
      return;
    }
    const startedAt = performance.now();
    const step = (at: number) => {
      const progress = Math.min(1, (at - startedAt) / FOLD_MS);
      // ease-in-out：起步和收尾都慢，中间快，看起来才像"被压紧"而不是跳变。
      const eased = progress < 0.5
        ? 2 * progress * progress
        : 1 - (-2 * progress + 2) ** 2 / 2;
      const value = from + (to - from) * eased;
      foldTRef.current = value;
      setFoldT(value);
      if (progress < 1) {
        animationRef.current = requestAnimationFrame(step);
      } else {
        animationRef.current = null;
        onDone?.();
      }
    };
    animationRef.current = requestAnimationFrame(step);
  }, []);

  useEffect(() => {
    if (fold === renderFold) {
      animateFold(fold === "none" ? 0 : 1);
      return;
    }
    if (renderFold === "none") {
      setRenderFold(fold);
      return;
    }
    animateFold(0, () => setRenderFold(fold));
  }, [fold, renderFold, animateFold]);

  useEffect(
    () => () => {
      if (animationRef.current) cancelAnimationFrame(animationRef.current);
    },
    [],
  );

  // ── 尺寸 ─────────────────────────────────────────────────────────────────
  useEffect(() => {
    const node = surfaceRef.current;
    if (!node) return;
    const observer = new ResizeObserver(([entry]) => {
      setSize({
        width: Math.max(320, entry.contentRect.width),
        height: Math.max(240, entry.contentRect.height),
      });
    });
    observer.observe(node);
    return () => observer.disconnect();
  }, []);

  // ── 取数 ─────────────────────────────────────────────────────────────────
  useEffect(() => {
    let cancelled = false;
    const timer = window.setTimeout(async () => {
      setLoading(true);
      try {
        const response = await invoke<TimelineResponse>("query_timeline", {
          startMs: Math.round(start),
          endMs: Math.round(end),
          detail,
          // 折叠成日时格子由原始会话铺，需要更高的上限。
          maxItems: renderFold === "none" ? 1800 : 5000,
          fold: renderFold,
        });
        if (!cancelled) {
          setData(response);
          setError("");
        }
      } catch (reason) {
        if (!cancelled && "__TAURI_INTERNALS__" in window) setError(String(reason));
      } finally {
        if (!cancelled) setLoading(false);
      }
    }, 120);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [start, end, detail, renderFold]);

  const layout = useMemo(
    () => computeLayout(size.width, size.height, visibleTracks, data.grid.rows, compact),
    [size.width, size.height, visibleTracks, data.grid.rows, compact],
  );

  const projection = useMemo(
    () =>
      new Projection({
        layout,
        fold: renderFold,
        t: renderFold === "none" ? 0 : foldT,
        startMs: start,
        endMs: end,
        rows: data.grid.rows,
        cols: data.grid.cols,
        cells: data.cells,
        doublePlot,
      }),
    [layout, renderFold, foldT, start, end, data.grid.rows, data.grid.cols, data.cells, doublePlot],
  );

  // ── 绘制 ─────────────────────────────────────────────────────────────────
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const dpr = Math.min(window.devicePixelRatio || 1, 2);
    canvas.width = Math.round(size.width * dpr);
    canvas.height = Math.round(size.height * dpr);
    canvas.style.width = `${size.width}px`;
    canvas.style.height = `${size.height}px`;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    hitRegions.current = drawScene(ctx, {
      layout,
      projection,
      data,
      tracks: visibleTracks,
      focus,
      selection,
      now: Date.now(),
    });
  }, [layout, projection, data, visibleTracks, focus, selection, size]);

  // ── 选区统计 ──────────────────────────────────────────────────────────────
  const windows = useMemo<Array<[number, number]>>(() => {
    if (!selection) return [];
    if (selection.kind === "phase") {
      return projection.windowsForPhase(selection.a, selection.b);
    }
    return [[Math.min(selection.a, selection.b), Math.max(selection.a, selection.b)]];
  }, [selection, projection]);

  const windowKey = windows.map(([a, b]) => `${Math.round(a)}-${Math.round(b)}`).join(",");
  useEffect(() => {
    if (!windows.length) {
      setStats(null);
      return;
    }
    let cancelled = false;
    const timer = window.setTimeout(async () => {
      try {
        const result = await invoke<RangeStats>("query_range_stats", {
          windows: windows.map(([a, b]) => [Math.round(a), Math.round(b)]),
        });
        if (!cancelled) setStats(result);
      } catch (reason) {
        if (!cancelled && "__TAURI_INTERNALS__" in window) setError(String(reason));
      }
    }, 140);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
    // windowKey 已经概括了 windows 的内容，避免每次 render 重新请求。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [windowKey]);

  // ── 输入 ─────────────────────────────────────────────────────────────────
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const onWheel = (event: WheelEvent) => {
      event.preventDefault();
      const { center: currentCenter, span: currentSpan } = viewRef.current;
      const factor = Math.exp(event.deltaY * 0.0017);
      if (renderFold !== "none") {
        // 折叠模式：横轴是相位，缩放只改变"堆几行"。
        setSpan(clamp(currentSpan * factor, MIN_SPAN, MAX_SPAN));
        return;
      }
      if (event.shiftKey && !event.ctrlKey && !event.metaKey) {
        setCenter(currentCenter + (event.deltaY / Math.max(1, size.width)) * currentSpan);
        return;
      }
      const rect = canvas.getBoundingClientRect();
      const ratio = clamp((event.clientX - rect.left - layout.plotLeft) / layout.plotWidth, 0, 1);
      const anchor = currentCenter + (ratio - 0.5) * currentSpan;
      const nextSpan = clamp(currentSpan * factor, MIN_SPAN, MAX_SPAN);
      setSpan(nextSpan);
      setCenter(anchor - (ratio - 0.5) * nextSpan);
    };
    canvas.addEventListener("wheel", onWheel, { passive: false });
    return () => canvas.removeEventListener("wheel", onWheel);
  }, [size.width, layout.plotLeft, layout.plotWidth, renderFold]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.target instanceof HTMLInputElement) return;
      if (event.key === "Escape") {
        setSelected(null);
        setSelection(null);
        return;
      }
      if (event.key === "f") {
        setFold((current) => FOLDS[(FOLDS.indexOf(current) + 1) % FOLDS.length]);
        return;
      }
      if (event.key === "x") {
        setDoublePlot((current) => !current);
        return;
      }
      if (event.key === "t") {
        setCenter(Date.now());
        setSpan(DAY);
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);

  const onPointerDown = (event: React.PointerEvent<HTMLCanvasElement>) => {
    event.currentTarget.setPointerCapture(event.pointerId);
    const rect = event.currentTarget.getBoundingClientRect();
    const x = event.clientX - rect.left;
    const y = event.clientY - rect.top;
    // 刻度尺上拖拽 = 拉选区（DAW 的 loop region），画布里拖拽 = 平移。
    if (y <= layout.rulerH && x >= layout.plotLeft) {
      const from = projection.folded ? projection.phaseAtX(x) : projection.timeAtX(x);
      dragRef.current = { mode: "select", from, moved: false };
      setSelection({ kind: projection.folded ? "phase" : "linear", a: from, b: from });
      return;
    }
    dragRef.current = { mode: "pan", x: event.clientX, y: event.clientY, center, moved: false };
  };

  const onPointerMove = (event: React.PointerEvent<HTMLCanvasElement>) => {
    const drag = dragRef.current;
    if (!drag) return;
    const rect = event.currentTarget.getBoundingClientRect();
    if (drag.mode === "select") {
      const x = event.clientX - rect.left;
      const to = projection.folded ? projection.phaseAtX(x) : projection.timeAtX(x);
      drag.moved = true;
      setSelection({ kind: projection.folded ? "phase" : "linear", a: drag.from, b: to });
      return;
    }
    const dx = event.clientX - drag.x;
    const dy = event.clientY - drag.y;
    if (Math.abs(dx) > 3 || Math.abs(dy) > 3) drag.moved = true;
    if (projection.folded) {
      // 折叠模式纵向就是时间：上下拖动换日期。
      setCenter(drag.center - (dy / Math.max(1, layout.plotHeight)) * span);
    } else {
      setCenter(drag.center - (dx / Math.max(1, layout.plotWidth)) * span);
    }
  };

  const onPointerUp = (event: React.PointerEvent<HTMLCanvasElement>) => {
    const drag = dragRef.current;
    dragRef.current = null;
    if (!drag || drag.moved) return;
    if (drag.mode === "select") {
      setSelection(null);
      return;
    }
    const rect = event.currentTarget.getBoundingClientRect();
    const x = event.clientX - rect.left;
    const y = event.clientY - rect.top;
    const hit = hitRegions.current.find(
      (region) => x >= region.x1 && x <= region.x2 && y >= region.y1 && y <= region.y2,
    );
    if (hit?.event) {
      setSelected(hit.event);
      return;
    }
    if (hit?.cell) {
      // 点一个格子 = 把这一格当选区，直接给统计。
      setSelection({ kind: "linear", a: hit.cell.start_ms, b: hit.cell.end_ms });
      setSelected(null);
      return;
    }
    setSelected(null);
  };

  function zoom(factor: number) {
    setSpan((current) => clamp(current * factor, MIN_SPAN, MAX_SPAN));
  }

  function toggleTrack(id: TrackId, key: "visible" | "solo" | "collapsed") {
    setTrackStates((current) =>
      current.map((state) => (state.id === id ? { ...state, [key]: !state[key] } : state)),
    );
  }

  /// 焦点轨道在折叠视图里是唯一被画出来的那条，所以设焦点必须同时保证它可见，
  /// 否则会得到一张空网格。
  function focusTrack(id: TrackId) {
    setFocus(id);
    setTrackStates((current) =>
      current.map((state) => (state.id === id ? { ...state, visible: true } : state)),
    );
  }

  // 换折叠档位时清掉选区：相位选区（列/偏移）在别的档位里没有意义。
  useEffect(() => {
    setSelection(null);
  }, [fold]);

  // 焦点轨道被隐藏或被 solo 排除时，把焦点交给第一条可见轨道。
  useEffect(() => {
    if (visibleTracks.length && !visibleTracks.some((state) => state.id === focus)) {
      setFocus(visibleTracks[0].id);
    }
  }, [visibleTracks, focus]);

  const rowLabelOpacity = renderFold === "none" ? 0 : foldT;
  const trackHeaderOpacity = 1 - rowLabelOpacity;

  return (
    <section className="timeline-screen animate-in">
      <div className="timeline-toolbar">
        <div className="timeline-folds" role="group" aria-label="折叠档位">
          {FOLDS.map((option) => (
            <button
              key={option}
              className={cn("timeline-fold", fold === option && "active")}
              onClick={() => setFold(option)}
              title={foldHint(option)}
            >
              {FOLD_LABEL[option]}
            </button>
          ))}
        </div>
        {fold === "day" && (
          <button
            className={cn("timeline-chip", doublePlot && "active")}
            onClick={() => setDoublePlot((current) => !current)}
            title="双绘：每行画 48 小时（行 n 显示第 n 天与第 n+1 天），跨午夜的模式才看得出来"
          >
            <Rows3 size={13} /> 双绘
          </button>
        )}
        <span className="timeline-spacer" />
        <span className="timeline-lod" title={`本次桶粒度：${data.bucket} · 刻度：${data.tick_unit}`}>
          <Layers3 size={13} />
          {detailLabel(detail)}
          <small>{formatSpan(span)}</small>
        </span>
        <button onClick={() => zoom(0.55)} aria-label="放大时间轴">
          <Plus size={14} />
        </button>
        <input
          aria-label="时间尺度"
          type="range"
          min={Math.log10(MIN_SPAN)}
          max={Math.log10(MAX_SPAN)}
          step="0.001"
          value={Math.log10(span)}
          onChange={(event) => setSpan(10 ** Number(event.target.value))}
        />
        <button onClick={() => zoom(1.8)} aria-label="缩小时间轴">
          <Minus size={14} />
        </button>
        <button
          className="timeline-chip"
          onClick={() => {
            setCenter(Date.now());
            setSpan(DAY);
          }}
        >
          <LocateFixed size={13} /> 今天
        </button>
      </div>

      <div className="timeline-surface" ref={surfaceRef}>
        <canvas
          ref={canvasRef}
          onPointerDown={onPointerDown}
          onPointerMove={onPointerMove}
          onPointerUp={onPointerUp}
          onPointerCancel={() => {
            dragRef.current = null;
          }}
        />

        {/* 轨道头：线性视图的左列。折叠时纵轴换成日期，两者交叉淡入。 */}
        <div
          className="timeline-headers"
          style={{ width: layout.headerW, opacity: trackHeaderOpacity, pointerEvents: trackHeaderOpacity < 0.5 ? "none" : "auto" }}
        >
          {layout.tracks.map((box) => {
            const meta = TRACKS.find((track) => track.id === box.id);
            const state = trackStates.find((candidate) => candidate.id === box.id);
            if (!meta || !state) return null;
            return (
              <div
                key={box.id}
                className={cn("timeline-header", focus === box.id && "focused")}
                style={{ top: box.top, height: Math.max(16, box.height) }}
                title={meta.question}
              >
                <button
                  className="timeline-header-name"
                  onClick={() => focusTrack(box.id)}
                  title={`设为折叠焦点轨道 · ${meta.question}`}
                >
                  {compact ? meta.label.slice(0, 2) : meta.label}
                </button>
                {!compact && (
                  <div className="timeline-header-actions">
                    <button
                      onClick={() => toggleTrack(box.id, "collapsed")}
                      title={state.collapsed ? "展开" : "折叠"}
                      aria-label={state.collapsed ? "展开轨道" : "折叠轨道"}
                    >
                      {state.collapsed ? <ChevronRight size={11} /> : <ChevronDown size={11} />}
                    </button>
                    <button
                      className={cn(state.solo && "on")}
                      onClick={() => toggleTrack(box.id, "solo")}
                      title="solo：只看这一条"
                    >
                      S
                    </button>
                    <button
                      onClick={() => toggleTrack(box.id, "visible")}
                      title={state.visible ? "隐藏" : "显示"}
                      aria-label={state.visible ? "隐藏轨道" : "显示轨道"}
                    >
                      {state.visible ? <Eye size={11} /> : <EyeOff size={11} />}
                    </button>
                  </div>
                )}
              </div>
            );
          })}
          {/* 被 solo/隐藏掉的轨道仍要能找回来。 */}
          {trackStates.filter((state) => !visibleTracks.includes(state)).length > 0 && (
            <div className="timeline-hidden-tracks">
              {trackStates
                .filter((state) => !visibleTracks.includes(state))
                .map((state) => (
                  <button
                    key={state.id}
                    onClick={() => {
                      setTrackStates((current) =>
                        current.map((candidate) =>
                          candidate.id === state.id
                            ? { ...candidate, visible: true, solo: false }
                            : { ...candidate, solo: false },
                        ),
                      );
                    }}
                    title="恢复这条轨道"
                  >
                    {TRACKS.find((track) => track.id === state.id)?.label}
                  </button>
                ))}
            </div>
          )}
        </div>

        {rowLabelOpacity > 0.02 && (
          <div
            className="timeline-rows"
            style={{ width: layout.headerW, opacity: rowLabelOpacity }}
          >
            {layout.rows.map((box) => {
              const row = data.grid.rows.find((candidate) => candidate.index === box.index);
              if (!row || box.height < 9) return null;
              return (
                <div
                  key={box.index}
                  className="timeline-row-label"
                  style={{ top: box.top, height: box.height }}
                >
                  <span>{row.label}</span>
                  <small>{row.sub_label}</small>
                </div>
              );
            })}
          </div>
        )}

        {loading && <span className="timeline-loading">更新中</span>}
        {renderFold !== "none" && data.grid.truncated && (
          <span className="timeline-loading">行数已截断，缩小跨度看全部</span>
        )}
        {/* 画不下的行必须说出来：默默少画几十天，读起来和"那几天没数据"一样。 */}
        {renderFold !== "none" && !data.grid.truncated && layout.visibleRows < data.grid.rows.length && (
          <span className="timeline-loading">
            画布高度只放得下 {layout.visibleRows} / {data.grid.rows.length} 行
          </span>
        )}
        {renderFold !== "none" && focus === "state" && (
          <div className="timeline-life-empty">
            <strong>状态分不参与折叠</strong>
            <span>它是每日一个数，折叠后每行只有一格，不比线性视图多告诉你任何事。切到线性视图看趋势。</span>
          </div>
        )}
        {detail === "life" && renderFold === "none" && !data.has_long_term_source && (
          <div className="timeline-life-empty">
            <strong>长期方向保持为空</strong>
            <span>在 LifeIndex 新建或从 Notion 同步后，这里会呈现你的长期计划。</span>
          </div>
        )}
      </div>

      <footer className="timeline-footer">
        <span>{formatBoundary(start)}</span>
        <span>
          {renderFold === "none"
            ? "滚轮缩放 · 拖拽平移 · 刻度尺上拖拽选区 · F 折叠"
            : "滚轮改行数 · 上下拖拽换日期 · 刻度尺上拖拽选相位 · F 换档"}
        </span>
        <span>{formatBoundary(end)}</span>
      </footer>

      {stats && (
        <div className="timeline-stats">
          <header>
            <strong>
              {selection?.kind === "phase"
                ? `相位选区 · ${stats.windows} 个周期`
                : "选区统计"}
            </strong>
            <small>
              {selection?.kind === "phase"
                ? describePhase(projection, selection)
                : describeLinear(selection)}
            </small>
            <button onClick={() => setSelection(null)} aria-label="清除选区">
              <X size={13} />
            </button>
          </header>
          <div className="timeline-stats-grid">
            <Metric label="观测" value={formatDuration(stats.observed_ms)} hint={`选中 ${formatDuration(stats.covered_ms)}`} />
            <Metric label="专注" value={formatDuration(stats.focus_ms)} hint={share(stats.focus_ms, stats.observed_ms)} />
            <Metric label="娱乐" value={formatDuration(stats.entertainment_ms)} hint={share(stats.entertainment_ms, stats.observed_ms)} />
            <Metric label="会话" value={String(stats.session_count)} hint={`中性 ${formatDuration(stats.neutral_ms)}`} />
            <Metric
              label="干预"
              value={String(stats.intervention_count)}
              hint={stats.intervention_count ? `转移 ${stats.intervention_switched}` : "无"}
            />
            <Metric
              label="产出"
              value={String(stats.capture_count + stats.artifact_count)}
              hint={`记录 ${stats.capture_count} · artifact ${stats.artifact_count}`}
            />
          </div>
          {stats.observed_ms > 0 && (
            <div className="timeline-stats-bar" aria-hidden>
              <span style={{ flex: stats.focus_ms, background: "#69c4a4" }} />
              <span style={{ flex: stats.neutral_ms, background: "#657084" }} />
              <span style={{ flex: stats.entertainment_ms, background: "#e99d54" }} />
            </div>
          )}
          {stats.top_apps.length > 0 && (
            <ul className="timeline-stats-list">
              {stats.top_apps.map((item) => (
                <li key={item.key}>
                  <span>{item.key}</span>
                  <small>{formatDuration(item.duration_ms)}</small>
                </li>
              ))}
            </ul>
          )}
          {stats.truncated && <p className="timeline-note">选区周期过多，已截断统计。</p>}
        </div>
      )}

      {selected && (
        <button className="timeline-selection" onClick={() => setSelected(null)}>
          <span style={{ background: "#8b93ff" }} />
          <div>
            <strong>{selected.title}</strong>
            <small>
              {kindLabel(selected.kind)} · {formatEventRange(selected)}
              {selected.detail && selected.detail !== selected.kind ? ` · ${selected.detail}` : ""}
            </small>
          </div>
          <kbd>esc</kbd>
        </button>
      )}
      {data.truncated && (
        <p className="timeline-note">当前窗口事件较多，已按可见范围裁剪；继续放大可查看细节。</p>
      )}
      {error && <p className="timeline-error">时间轴暂时无法读取：{error}</p>}
    </section>
  );
}

function Metric({ label, value, hint }: { label: string; value: string; hint?: string }) {
  return (
    <div className="timeline-metric">
      <small>{label}</small>
      <strong>{value}</strong>
      {hint && <span>{hint}</span>}
    </div>
  );
}

function detailForSpan(span: number): Detail {
  if (span <= 12 * 60 * 60_000) return "minute";
  if (span <= 8 * DAY) return "day";
  if (span <= 180 * DAY) return "week";
  return "life";
}

function detailLabel(detail: Detail) {
  return { minute: "事件细节", day: "行为区间", week: "每日状态", life: "长期方向" }[detail];
}

function foldHint(fold: Fold) {
  switch (fold) {
    case "none":
      return "线性：一条轴从左到右";
    case "day":
      return "按日折叠（actogram）：每行一天，横轴是日内时刻 —— 看作息漂移与深夜时段";
    case "week":
      return "按周折叠：7 列即传统日历，每格一天";
    case "month":
      return "按月折叠：每行一个月，横轴是日期";
    case "year":
      return "按年折叠：每行一年，横轴是一年里的第几天";
  }
}

function formatSpan(span: number) {
  if (span < 60 * 60_000) return `${Math.round(span / 60_000)} 分钟`;
  if (span < 2 * DAY) return `${(span / 3_600_000).toFixed(span < 12 * 3_600_000 ? 1 : 0)} 小时`;
  if (span < 120 * DAY) return `${Math.round(span / DAY)} 天`;
  if (span < 730 * DAY) return `${(span / (30 * DAY)).toFixed(1)} 个月`;
  return `${(span / (365 * DAY)).toFixed(1)} 年`;
}

function formatDuration(ms: number) {
  if (ms <= 0) return "0";
  const minutes = Math.round(ms / 60_000);
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;
  if (hours < 24) return rest ? `${hours}h ${rest}m` : `${hours}h`;
  const days = Math.floor(hours / 24);
  return `${days}d ${hours % 24}h`;
}

function share(part: number, total: number) {
  if (total <= 0) return "—";
  return `${Math.round((part / total) * 100)}%`;
}

function formatBoundary(time: number) {
  return new Date(time).toLocaleDateString("zh-CN", {
    year: "numeric",
    month: "short",
    day: "numeric",
  });
}

function formatClock(time: number) {
  return new Date(time).toLocaleTimeString("zh-CN", { hour: "2-digit", minute: "2-digit" });
}

function describeLinear(selection: Selection | null) {
  if (!selection) return "";
  const from = Math.min(selection.a, selection.b);
  const to = Math.max(selection.a, selection.b);
  return `${formatBoundary(from)} ${formatClock(from)} → ${formatClock(to)}`;
}

function describePhase(projection: Projection, selection: Selection) {
  const from = Math.min(selection.a, selection.b);
  const to = Math.max(selection.a, selection.b);
  if (projection.fold === "day") {
    const anchor = projection.rows[0]?.start_ms ?? Date.now();
    return `每天 ${formatClock(anchor + from)} → ${formatClock(anchor + to)}`;
  }
  return `第 ${Math.floor(from) + 1} – ${Math.max(Math.floor(from) + 1, Math.ceil(to))} 列`;
}

function formatEventRange(event: TimelineEvent) {
  const from = new Date(event.start_ms);
  const left = from.toLocaleString("zh-CN", {
    month: "numeric",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
  if (event.end_ms <= event.start_ms) return left;
  return `${left}–${formatClock(event.end_ms)}`;
}

function kindLabel(kind: TimelineEvent["kind"]) {
  switch (kind) {
    case "intervention":
      return "干预";
    case "capture":
      return "记录";
    case "goal":
      return "目标";
    case "task":
      return "任务";
    case "reminder":
      return "提醒";
    case "knowledge":
      return "知识";
    case "rule":
      return "规则";
    case "system":
      return "系统";
    default:
      return "行为";
  }
}

function clamp(value: number, min: number, max: number) {
  return Math.min(max, Math.max(min, value));
}
