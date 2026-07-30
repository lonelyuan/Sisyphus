import { Archive, X } from "lucide-react";
import { Card } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  horizonLabel,
  kindLabel,
  statusLabel,
  trackOptions,
  wantsCriteria,
  type Draft,
  type LifeArea,
  type LifeHorizon,
  type LifeKind,
  type LifeStatus,
  type LifeTrack,
} from "./model";

/// 看板与技能树共用的编辑弹窗。两个视图同一条写入路径，字段语义不许分叉。
export default function ItemDraftDialog({
  draft,
  areas,
  saving,
  onChange,
  onClose,
  onSave,
  onArchive,
}: {
  draft: Draft;
  areas: LifeArea[];
  saving: boolean;
  onChange: (draft: Draft) => void;
  onClose: () => void;
  onSave: () => void;
  onArchive?: () => void;
}) {
  const criteria = wantsCriteria(draft.kind);
  return (
    <div
      className="fixed inset-0 z-50 flex items-end justify-center bg-black/50 p-3 backdrop-blur-sm sm:items-center"
      onMouseDown={(event) => event.target === event.currentTarget && onClose()}
    >
      <Card className="max-h-[92vh] w-full max-w-xl overflow-y-auto p-4 shadow-2xl">
        <div className="mb-4 flex items-center justify-between">
          <div>
            <p className="text-sm font-medium text-foreground">{draft.id ? "编辑 LifeItem" : "新建 LifeItem"}</p>
            <p className="mt-0.5 text-[10px] text-muted-foreground">修改保存后先写入 SQLite，再由 Agent 投影到 Notion。</p>
          </div>
          <Button variant="ghost" size="icon" onClick={onClose}>
            <X size={16} />
          </Button>
        </div>
        <div className="flex flex-col gap-3">
          <label className="text-[11px] text-muted-foreground">
            标题
            <Input
              className="mt-1"
              autoFocus
              value={draft.title}
              onChange={(e) => onChange({ ...draft, title: e.target.value })}
              onKeyDown={(e) => e.key === "Enter" && !e.shiftKey && onSave()}
            />
          </label>
          <label className="text-[11px] text-muted-foreground">
            补充说明
            <textarea
              className="mt-1 min-h-20 w-full resize-y rounded-md border border-input bg-input px-3 py-2 text-sm text-foreground outline-none focus:ring-2 focus:ring-ring/40"
              value={draft.body}
              onChange={(e) => onChange({ ...draft, body: e.target.value })}
            />
          </label>
          <div className="grid grid-cols-2 gap-2 sm:grid-cols-4">
            <SelectField
              label="形态"
              value={draft.kind}
              onChange={(kind) => onChange({ ...draft, kind: kind as LifeKind })}
              options={Object.entries(kindLabel)}
            />
            <SelectField
              label="主次"
              value={draft.track}
              onChange={(track) => onChange({ ...draft, track: track as LifeTrack })}
              options={trackOptions}
            />
            <SelectField
              label="时间尺度"
              value={draft.horizon}
              onChange={(horizon) => onChange({ ...draft, horizon: horizon as LifeHorizon })}
              options={Object.entries(horizonLabel)}
            />
            <SelectField
              label="状态"
              value={draft.status}
              onChange={(status) => onChange({ ...draft, status: status as LifeStatus })}
              options={Object.entries(statusLabel).filter(([key]) => key !== "archived")}
            />
          </div>
          <SelectField
            label="责任领域"
            value={draft.area_id}
            onChange={(area_id) => onChange({ ...draft, area_id })}
            options={[["", "未归属"], ...areas.map((area) => [area.id, area.focus ? `${area.name} · 重点` : area.name])]}
          />
          {criteria && (
            <>
              <label className="text-[11px] text-muted-foreground">
                完成条件（一句可判定的话）
                <Input
                  className="mt-1"
                  value={draft.success_criteria}
                  onChange={(e) => onChange({ ...draft, success_criteria: e.target.value })}
                  placeholder="如：能独立写完一个 async 服务并通过 review"
                />
              </label>
              <div className="grid grid-cols-3 gap-2">
                <label className="text-[11px] text-muted-foreground">
                  当前值
                  <Input
                    className="mt-1 text-xs"
                    value={draft.current_value}
                    onChange={(e) => onChange({ ...draft, current_value: e.target.value })}
                    placeholder="5.25"
                  />
                </label>
                <label className="text-[11px] text-muted-foreground">
                  目标值
                  <Input
                    className="mt-1 text-xs"
                    value={draft.target_value}
                    onChange={(e) => onChange({ ...draft, target_value: e.target.value })}
                    placeholder="7"
                  />
                </label>
                <label className="text-[11px] text-muted-foreground">
                  单位
                  <Input
                    className="mt-1 text-xs"
                    value={draft.unit}
                    onChange={(e) => onChange({ ...draft, unit: e.target.value })}
                    placeholder="分 / 次 / 页"
                  />
                </label>
              </div>
              <p className="text-[10px] text-muted-foreground">
                填了当前/目标值，进度就由 Core 按 current/target 算；否则只看是否完成。进度永远不是估的。
              </p>
            </>
          )}
          <div className="grid grid-cols-1 gap-2 sm:grid-cols-3">
            <DateField label="开始" value={draft.start_date} onChange={(start_date) => onChange({ ...draft, start_date })} />
            <DateField label="截止" value={draft.due_date} onChange={(due_date) => onChange({ ...draft, due_date })} />
            <DateField label="复查" value={draft.review_date} onChange={(review_date) => onChange({ ...draft, review_date })} />
          </div>
          <label className="text-[11px] text-muted-foreground">
            循环规则
            <Input
              className="mt-1"
              value={draft.recurrence}
              onChange={(e) => onChange({ ...draft, recurrence: e.target.value })}
              placeholder="如：每天 / 每周三 / RRULE:FREQ=WEEKLY"
            />
          </label>
        </div>
        <div className="mt-5 flex items-center justify-between gap-2">
          <div>
            {draft.id && onArchive && (
              <Button variant="ghost" size="sm" className="text-danger hover:text-danger" disabled={saving} onClick={onArchive}>
                <Archive size={13} /> 归档
              </Button>
            )}
          </div>
          <div className="flex gap-2">
            <Button variant="secondary" size="sm" onClick={onClose}>
              取消
            </Button>
            <Button size="sm" disabled={saving || !draft.title.trim()} onClick={onSave}>
              {saving ? "保存中…" : "保存"}
            </Button>
          </div>
        </div>
      </Card>
    </div>
  );
}

function SelectField({
  label,
  value,
  options,
  onChange,
}: {
  label: string;
  value: string;
  options: string[][];
  onChange: (value: string) => void;
}) {
  return (
    <label className="text-[11px] text-muted-foreground">
      {label}
      <select
        className="mt-1 h-9 w-full rounded-md border border-input bg-input px-2 text-xs text-foreground outline-none focus:ring-2 focus:ring-ring/40"
        value={value}
        onChange={(e) => onChange(e.target.value)}
      >
        {options.map(([key, text]) => (
          <option key={key} value={key}>
            {text}
          </option>
        ))}
      </select>
    </label>
  );
}

function DateField({ label, value, onChange }: { label: string; value: string; onChange: (value: string) => void }) {
  return (
    <label className="text-[11px] text-muted-foreground">
      {label}
      <Input className="mt-1 text-xs" type="date" value={value} onChange={(e) => onChange(e.target.value)} />
    </label>
  );
}
