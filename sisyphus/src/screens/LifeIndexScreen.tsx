import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { LayoutGrid, ExternalLink } from "lucide-react";
import { Card, CardLabel } from "@/components/ui/card";

interface LifeIndexCard {
  id: string;
  section: string;
  title: string;
  body: string;
  source_ref: string | null;
  source_updated_at: number | null;
  observed_at: number;
}

export default function LifeIndexScreen() {
  const [cards, setCards] = useState<LifeIndexCard[]>([]);
  const [loaded, setLoaded] = useState(false);

  useEffect(() => {
    invoke<LifeIndexCard[]>("list_lifeindex")
      .then(setCards)
      .catch(() => {})
      .finally(() => setLoaded(true));
  }, []);

  // 按分区分组，保持后端已排好的顺序。
  const sections: { name: string; cards: LifeIndexCard[] }[] = [];
  for (const c of cards) {
    let s = sections.find((x) => x.name === c.section);
    if (!s) {
      s = { name: c.section, cards: [] };
      sections.push(s);
    }
    s.cards.push(c);
  }

  return (
    <div className="animate-in mx-auto flex max-w-3xl flex-col gap-3 p-5 md:p-8">
      <div className="flex items-center gap-2">
        <LayoutGrid size={16} strokeWidth={1.75} className="text-accent" />
        <h2 className="text-sm font-medium text-foreground">人生看板</h2>
      </div>

      {loaded && cards.length === 0 && (
        <Card className="flex flex-col gap-2 p-5 text-sm text-muted-foreground">
          <p>看板还是空的。</p>
          <p className="text-xs leading-relaxed">
            看板内容看齐你的 Notion：每天 8:30 由智能体只读参考你的 Notion（长期目标 / 短期 Todo /
            研究问题 / 个人发展）与本地行为，提炼成卡片写到这里。也可以在 Agent 对话里说“刷新一下我的看板”。
            Notion 始终只读，不会被改写。
          </p>
        </Card>
      )}

      {sections.map((s) => (
        <div key={s.name} className="flex flex-col gap-2">
          <CardLabel>{s.name}</CardLabel>
          <div className="grid gap-2 sm:grid-cols-2">
            {s.cards.map((c) => (
              <Card key={c.id} className="flex flex-col gap-1.5 p-3.5">
                <div className="flex items-start justify-between gap-2">
                  <span className="text-sm font-medium text-foreground">{c.title}</span>
                  {c.source_ref && (
                    <a
                      href={c.source_ref}
                      target="_blank"
                      rel="noreferrer"
                      className="shrink-0 text-muted-foreground hover:text-accent"
                      title="来源"
                    >
                      <ExternalLink size={13} />
                    </a>
                  )}
                </div>
                {c.body && (
                  <p className="whitespace-pre-wrap text-xs leading-relaxed text-muted-foreground">{c.body}</p>
                )}
              </Card>
            ))}
          </div>
        </div>
      ))}
    </div>
  );
}
