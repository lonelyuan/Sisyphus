# KnownEngine 设计文档与开发指导

版本：0.1  
状态：架构基线草案  
更新时间：2026-07-16

## 1. 项目背景

KnownEngine 是一个面向 AI 时代的个人与小团队知识工程系统。它的目标不是简单保存网页、PDF、博客或模型生成文本，而是建立一套可持续演进、可验证、可离线使用的知识库底层范式。

当前最明确的两个落地方向是：

1. **CTF Crawl**：面向 CTF 线下赛和训练场景，自动爬取、吸收、验证、组织 CTF 安全知识。它要能从关键词开始发现资料，也能从 CTF-Wiki、优质博客、GitHub 仓库、公开靶场和题目 WP 中冷启动。
2. **Survey Agent**：面向科研人员的文献调研智能体，重点处理论文 PDF、引用关系、研究问题、实验结论、争议点和后续头脑风暴。

两者共享一套知识存储内核、证据模型、知识图谱精炼策略和离线检索能力；差异主要在采集器、解析器、验证器和领域本体上。

## 2. 核心判断

### 2.1 不把 Notion、Obsidian 或图数据库作为唯一真相源

Notion、Obsidian、Logseq、Neo4j、向量数据库和搜索引擎都适合做某种视图，但不适合作为整个系统的规范事实源。

原因：

- Notion 是优秀的人机编辑和展示界面，但它的底层是 API 对象和增强 Markdown 的混合抽象，受 SaaS 权限、限流、版本变更和网络可用性影响。
- Obsidian 的 Markdown 文件非常适合离线阅读和人工编辑，但无法天然表达证据跨度、版本补丁、验证运行、实体合并历史和多投影同步状态。
- 图数据库适合复杂图查询和图算法，但“用了图数据库”不等于“知识组织得好”。本体质量来自约束、评审、证据和持续重构策略，而不是存储引擎本身。

因此，本项目采用：

```text
SQLite + 内容寻址文件系统 = 规范知识内核

Notion / Obsidian / 图数据库 / 搜索索引 / 向量索引 = 可重建投影
```

### 2.2 SQLite 是核心物理存储，但不是唯一存储介质

SQLite 在本项目中承担以下职责：

- 规范 ID 管理。
- 元数据目录。
- 证据链索引。
- 概念、关系、声明、文档修订、验证运行的事务存储。
- FTS5 本地全文检索。
- 投影同步状态和 outbox 事件。
- 版本化本体补丁。

但大对象不直接放进 SQLite：

- PDF。
- HTML 快照。
- 截图。
- Docker 构建日志。
- PCAP。
- 源码压缩包。
- 靶场文件。
- 模型输出大文本。
- Notion/网页导出的附件。

这些对象放入内容寻址文件系统：

```text
knowledge/
├── knowledge.db
├── blobs/
│   └── sha256/
│       └── ab/
│           └── cd/
│               └── abcdef...
├── vault/
├── indexes/
└── runs/
```

SQLite 中只记录：

- `sha256`。
- `size_bytes`。
- `mime_type`。
- `storage_path`。
- `source_url`。
- `captured_at`。
- `license_hint`。
- `provenance`。

这个设计兼顾事务性、可迁移性、离线能力和大文件管理。

## 3. 预期目标

### 3.1 知识组织目标

系统必须持续维护一个高质量领域本体，而不是无限追加无组织笔记。

“高信息熵”在工程上不建议理解为越复杂越好，而应定义为：

- 信息密度高：少废话、少模板化、少重复。
- 抽象稳定：概念层级能解释多个实例，而不是每篇 WP 都新增一个孤立 topic。
- 证据覆盖充分：重要结论能追溯到来源、代码、实验或人工验证。
- 检索成本低：选手能在比赛离线环境里快速定位 payload、原理、前置条件和利用步骤。
- 结构可读：一个父节点下的兄弟概念数量、语义重叠和命名风格受控。

例如 Web 安全中：

```text
Web Security
└── Injection
    ├── SQL Injection
    ├── Command Injection
    ├── Template Injection
    ├── LDAP Injection
    ├── XPath Injection
    └── NoSQL Injection
```

这里 `Injection` 是稳定抽象。不同注入类型不是在 `Web Security` 下堆平铺列表，而是被放在共同父节点下，并通过关系描述差异：

- 注入点。
- 解释器。
- 可控输入。
- 语法边界。
- 执行上下文。
- 常见过滤绕过。
- 可验证靶场。

### 3.2 可靠性目标

系统必须区分：

- 模型知道或猜测。
- 某篇博客声称。
- 多个来源交叉支持。
- 有源码或环境能构建。
- 在沙盒中实际复现。
- 人工确认有效。

模型自身知识只能用于：

- 生成搜索关键词。
- 发现别名。
- 提出本体补丁。
- 生成验证计划。
- 总结候选解释。

模型自身知识不能直接作为事实证据。

### 3.3 离线目标

线下断网 CTF 参赛时，系统必须能提供完整离线包：

- Markdown 文档库。
- 本地图片、附件、PDF 快照和实验产物。
- SQLite FTS5 精确检索。
- 本地向量索引。
- 概念图邻域索引。
- 面向本地小参数模型的紧凑证据包。
- 可审计引用来源。

离线场景中，核心体验是：

1. 人可以快速打开 Obsidian/Vault 查阅。
2. 本地小模型可以通过混合检索获得小而准的上下文。
3. 文档中明确区分已复现、未复现、仅引用、待验证。

## 4. 总体架构

```text
                 ┌─────────────────────────────┐
                 │ Crawlers / Importers         │
                 │ Web, PDF, GitHub, Notion     │
                 └──────────────┬──────────────┘
                                │
                                v
                 ┌─────────────────────────────┐
                 │ Capture & Snapshot Layer     │
                 │ raw html/pdf/git/blob        │
                 └──────────────┬──────────────┘
                                │
                                v
                 ┌─────────────────────────────┐
                 │ Parsing & Normalization      │
                 │ text, blocks, metadata       │
                 └──────────────┬──────────────┘
                                │
                                v
                 ┌─────────────────────────────┐
                 │ Distillation Agents          │
                 │ claims, concepts, relations  │
                 └──────────────┬──────────────┘
                                │
                                v
                 ┌─────────────────────────────┐
                 │ Ontology Patch Queue         │
                 │ add/merge/move/split/reuse   │
                 └──────────────┬──────────────┘
                                │
                                v
┌──────────────────────────────────────────────────────────────┐
│ Canonical Kernel                                              │
│ SQLite: source/evidence/claim/concept/relation/version/outbox │
│ Filesystem: content-addressed blobs and run artifacts          │
└──────────────┬──────────────────┬──────────────────┬─────────┘
               │                  │                  │
               v                  v                  v
┌─────────────────────┐ ┌────────────────────┐ ┌─────────────────────┐
│ Markdown Projection │ │ Notion Projection   │ │ Graph Projection     │
│ Obsidian Vault      │ │ Workspace Pages     │ │ Neo4j/Memgraph later │
└─────────────────────┘ └────────────────────┘ └─────────────────────┘
               │                  │                  │
               v                  v                  v
┌─────────────────────┐ ┌────────────────────┐ ┌─────────────────────┐
│ Offline Retrieval   │ │ Human Editing       │ │ Graph Algorithms     │
│ FTS/vector/local LLM│ │ review/refine       │ │ optional             │
└─────────────────────┘ └────────────────────┘ └─────────────────────┘
```

## 5. 分层设计

### 5.1 Capture Layer

职责：

- 抓取网页、PDF、Git 仓库、题目附件、Dockerfile、WP、Notion 页面。
- 保存原始快照。
- 计算 hash。
- 记录来源 URL、抓取时间、HTTP 头、重定向链、许可证线索。
- 避免重复抓取。

不得做：

- 不得直接改概念图谱。
- 不得把模型总结当成事实写入。
- 不得只保存抽取后的正文而丢弃原始快照。

### 5.2 Parsing Layer

职责：

- 从 HTML/PDF/Markdown/Notebook/代码仓库中提取正文和结构。
- 保留源定位信息。
- 切分 evidence span。
- 识别代码块、命令、payload、CVE、CWE、文件路径、函数名、协议字段。

输出应包括：

```json
{
  "snapshot_id": "uuid",
  "title": "SQL Injection Cheat Sheet",
  "authors": ["..."],
  "published_at": "2025-01-01",
  "language": "en",
  "blocks": [
    {
      "block_id": "uuid",
      "type": "paragraph",
      "text": "...",
      "source_locator": {
        "kind": "html_css_path",
        "value": "article > section:nth-child(3) > p:nth-child(2)"
      }
    }
  ]
}
```

### 5.3 Distillation Layer

职责：

- 从材料中提取声明、概念、关系、示例、前置条件和验证线索。
- 生成本体补丁提案。
- 生成文档草稿。
- 生成验证计划。

约束：

- LLM 输出必须是提案，不得直接写入最终知识图谱。
- 每个 claim 必须绑定至少一个 evidence span。
- 每个新概念必须解释为什么不能复用已有概念。
- 每个 `MOVE`、`MERGE`、`SPLIT` 必须提供结构理由和影响范围。

### 5.4 Ontology Engine

职责：

- 维护概念主树和关系图。
- 检查 `IS_A` DAG 无环。
- 检查同级概念数量和语义重叠。
- 计算本体质量指标。
- 版本化每次变更。
- 支持回滚。

### 5.5 Verification Layer

职责：

- 判断 CTF WP 或靶场是否可复现。
- 准备构建上下文。
- 在隔离环境中执行验证。
- 保存日志、截图、命令、exit code、服务状态、flag 或关键证据。
- 输出结构化验证结果。

重要边界：

- Docker 不是充分安全边界。
- 对不可信 CTF 附件，应优先使用 disposable VM、microVM 或受限虚拟机，再在其中运行容器。
- 默认关闭外网，除非验证计划明确要求。
- 所有验证产物必须按 hash 存储。

### 5.6 Projection Layer

职责：

- 从规范内核生成 Markdown/Obsidian Vault。
- 从规范内核同步到 Notion。
- 从规范内核同步到图数据库。
- 从规范内核生成 FTS、向量索引和离线包。

投影必须满足：

- 幂等。
- 可重建。
- 可部分失败重试。
- 不反向覆盖规范事实，除非通过 import/review 流程转成补丁。

## 6. SQLite 与关系型数据库是否能映射文件系统和图数据库

可以，但要理解“映射”的方式。

### 6.1 映射到文件系统

文件系统负责存大对象和人类可读导出物。SQLite 负责声明这些文件是什么、从哪里来、被哪些 claim 使用、当前投影状态如何。

示例：

```text
blob 表
  sha256 = abc...
  mime_type = application/pdf
  storage_path = blobs/sha256/ab/cd/abc...

source_snapshot 表
  snapshot_id = uuid
  blob_id = abc...
  source_url = https://example.com/writeup

document_revision 表
  doc_id = uuid
  canonical_markdown = ...

projection_state 表
  projection = obsidian
  external_id = vault/Web Security/Injection/SQL Injection.md
```

因此，Markdown 文件路径不是规范 ID。路径可以变化，UUID 不变。

### 6.2 映射到图数据库

关系型数据库天然可以表达图：

```sql
concept(id, canonical_name, primary_parent_id, ...)
relation(id, subject_id, predicate, object_id, ...)
```

其中 `relation` 就是边表。常见图操作可以先在 SQLite 中完成：

- 一跳/两跳邻居查询。
- 递归父子树查询。
- 检查孤儿节点。
- 检查 `IS_A` 环。
- 查询某个概念的所有证据。
- 查某个 WP 支持了哪些概念。

SQLite 的 recursive CTE 可以支撑早期本体树和中等规模关系查询。外部图数据库只有在出现以下需求时才值得引入：

- 多跳路径查询非常频繁。
- 需要 PageRank、社区发现、中心性分析等图算法。
- 节点和边规模明显超过 SQLite 查询舒适区。
- 需要面向多人或服务化的复杂图查询接口。

即使引入 Neo4j 或 Memgraph，它也应是投影：

```text
SQLite concept/relation/relation_evidence
  -> graph projector
  -> Neo4j nodes/edges
```

不能让图数据库成为唯一真相源，否则会出现 Markdown、Notion、检索索引和图谱之间互相覆盖、难以回滚的问题。

### 6.3 关系型数据库的边界

SQLite 不适合：

- 多机器同时写。
- 很多用户高并发写。
- 复杂权限模型。
- 长期在线服务级高可用。
- 大规模向量检索主存储。

因此，约束是：

- MVP 阶段所有 agent 通过单 Writer Service 写 SQLite。
- agent 不直接写数据库。
- 开启 WAL。
- 事务内写规范表和 outbox。
- 投影异步消费 outbox。

迁移 PostgreSQL 的触发条件：

- 多台机器同时采集和写入。
- 多用户协作需要权限隔离。
- SQLite writer 队列成为瓶颈。
- 需要可靠远程服务部署。

## 7. 规范数据模型

### 7.1 实体分组

核心实体：

- `source`：来源，如网站、Git 仓库、论文、Notion 页面。
- `source_snapshot`：某次抓取的不可变快照。
- `blob`：内容寻址大对象。
- `evidence_span`：可定位证据片段。
- `claim`：知识声明。
- `claim_evidence`：声明和证据的绑定。
- `concept`：概念节点。
- `concept_alias`：别名。
- `relation`：概念、文档、题目、技术之间的关系。
- `relation_evidence`：关系证据。
- `ontology_patch`：本体变更提案。
- `ontology_version`：本体版本。
- `document`：规范文档。
- `document_revision`：文档修订。
- `challenge`：CTF 题目或靶场。
- `verification_run`：验证运行。
- `artifact`：验证产物。
- `chunk`：检索切片。
- `embedding`：向量记录或向量索引引用。
- `outbox_event`：投影事件。
- `projection_state`：外部系统同步状态。

### 7.2 初始 SQL 草案

这是 MVP 初始 schema，不要求一次性覆盖所有未来能力，但必须保留演进空间。

```sql
PRAGMA foreign_keys = ON;

CREATE TABLE blob (
  sha256 TEXT PRIMARY KEY,
  size_bytes INTEGER NOT NULL,
  mime_type TEXT,
  storage_path TEXT NOT NULL,
  original_filename TEXT,
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE source (
  id TEXT PRIMARY KEY,
  kind TEXT NOT NULL,
  canonical_url TEXT,
  title TEXT,
  author TEXT,
  license_hint TEXT,
  trust_tier TEXT NOT NULL DEFAULT 'unknown',
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE source_snapshot (
  id TEXT PRIMARY KEY,
  source_id TEXT NOT NULL REFERENCES source(id),
  blob_sha256 TEXT REFERENCES blob(sha256),
  fetched_url TEXT,
  fetched_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  http_status INTEGER,
  content_hash TEXT NOT NULL,
  parser_version TEXT,
  metadata_json TEXT NOT NULL DEFAULT '{}',
  UNIQUE(source_id, content_hash)
);

CREATE TABLE evidence_span (
  id TEXT PRIMARY KEY,
  snapshot_id TEXT NOT NULL REFERENCES source_snapshot(id),
  locator_kind TEXT NOT NULL,
  locator_value TEXT NOT NULL,
  text TEXT NOT NULL,
  text_sha256 TEXT NOT NULL,
  start_offset INTEGER,
  end_offset INTEGER,
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE claim (
  id TEXT PRIMARY KEY,
  statement TEXT NOT NULL,
  normalized_statement TEXT NOT NULL,
  claim_type TEXT NOT NULL,
  reliability_state TEXT NOT NULL DEFAULT 'captured',
  created_by TEXT NOT NULL,
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE claim_evidence (
  claim_id TEXT NOT NULL REFERENCES claim(id),
  evidence_span_id TEXT NOT NULL REFERENCES evidence_span(id),
  support_type TEXT NOT NULL,
  note TEXT,
  PRIMARY KEY (claim_id, evidence_span_id)
);

CREATE TABLE concept (
  id TEXT PRIMARY KEY,
  canonical_name TEXT NOT NULL,
  concept_type TEXT NOT NULL,
  primary_parent_id TEXT REFERENCES concept(id),
  definition TEXT NOT NULL,
  status TEXT NOT NULL DEFAULT 'active',
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  UNIQUE(canonical_name, concept_type)
);

CREATE TABLE concept_alias (
  id TEXT PRIMARY KEY,
  concept_id TEXT NOT NULL REFERENCES concept(id),
  alias TEXT NOT NULL,
  language TEXT,
  source TEXT NOT NULL DEFAULT 'agent',
  UNIQUE(concept_id, alias)
);

CREATE TABLE relation (
  id TEXT PRIMARY KEY,
  subject_kind TEXT NOT NULL,
  subject_id TEXT NOT NULL,
  predicate TEXT NOT NULL,
  object_kind TEXT NOT NULL,
  object_id TEXT NOT NULL,
  confidence_state TEXT NOT NULL DEFAULT 'captured',
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  UNIQUE(subject_kind, subject_id, predicate, object_kind, object_id)
);

CREATE TABLE relation_evidence (
  relation_id TEXT NOT NULL REFERENCES relation(id),
  evidence_span_id TEXT NOT NULL REFERENCES evidence_span(id),
  support_type TEXT NOT NULL,
  PRIMARY KEY (relation_id, evidence_span_id)
);

CREATE TABLE ontology_version (
  id TEXT PRIMARY KEY,
  parent_version_id TEXT REFERENCES ontology_version(id),
  version_number INTEGER NOT NULL UNIQUE,
  description TEXT,
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE ontology_patch (
  id TEXT PRIMARY KEY,
  base_version_id TEXT REFERENCES ontology_version(id),
  patch_type TEXT NOT NULL,
  status TEXT NOT NULL DEFAULT 'proposed',
  proposer TEXT NOT NULL,
  rationale TEXT NOT NULL,
  patch_json TEXT NOT NULL,
  metrics_before_json TEXT NOT NULL DEFAULT '{}',
  metrics_after_json TEXT NOT NULL DEFAULT '{}',
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  reviewed_at TEXT,
  reviewer TEXT
);

CREATE TABLE document (
  id TEXT PRIMARY KEY,
  document_type TEXT NOT NULL,
  title TEXT NOT NULL,
  primary_concept_id TEXT REFERENCES concept(id),
  status TEXT NOT NULL DEFAULT 'draft',
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE document_revision (
  id TEXT PRIMARY KEY,
  document_id TEXT NOT NULL REFERENCES document(id),
  revision_number INTEGER NOT NULL,
  canonical_markdown TEXT NOT NULL,
  frontmatter_json TEXT NOT NULL DEFAULT '{}',
  created_by TEXT NOT NULL,
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  UNIQUE(document_id, revision_number)
);

CREATE TABLE challenge (
  id TEXT PRIMARY KEY,
  title TEXT NOT NULL,
  category TEXT,
  source_id TEXT REFERENCES source(id),
  files_blob_sha256 TEXT REFERENCES blob(sha256),
  environment_state TEXT NOT NULL DEFAULT 'unknown',
  metadata_json TEXT NOT NULL DEFAULT '{}',
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE verification_run (
  id TEXT PRIMARY KEY,
  challenge_id TEXT REFERENCES challenge(id),
  document_id TEXT REFERENCES document(id),
  status TEXT NOT NULL,
  runner_kind TEXT NOT NULL,
  sandbox_profile TEXT NOT NULL,
  started_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  finished_at TEXT,
  command_log TEXT,
  result_summary TEXT,
  error_summary TEXT,
  metadata_json TEXT NOT NULL DEFAULT '{}'
);

CREATE TABLE artifact (
  id TEXT PRIMARY KEY,
  run_id TEXT REFERENCES verification_run(id),
  blob_sha256 TEXT NOT NULL REFERENCES blob(sha256),
  artifact_type TEXT NOT NULL,
  description TEXT,
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE chunk (
  id TEXT PRIMARY KEY,
  document_revision_id TEXT REFERENCES document_revision(id),
  evidence_span_id TEXT REFERENCES evidence_span(id),
  chunk_text TEXT NOT NULL,
  chunk_sha256 TEXT NOT NULL,
  token_count INTEGER,
  metadata_json TEXT NOT NULL DEFAULT '{}'
);

CREATE VIRTUAL TABLE chunk_fts USING fts5(
  chunk_text,
  content='chunk',
  content_rowid='rowid'
);

CREATE TABLE embedding (
  id TEXT PRIMARY KEY,
  chunk_id TEXT NOT NULL REFERENCES chunk(id),
  model_name TEXT NOT NULL,
  model_version TEXT,
  vector_ref TEXT NOT NULL,
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  UNIQUE(chunk_id, model_name, model_version)
);

CREATE TABLE outbox_event (
  id TEXT PRIMARY KEY,
  event_type TEXT NOT NULL,
  aggregate_kind TEXT NOT NULL,
  aggregate_id TEXT NOT NULL,
  payload_json TEXT NOT NULL,
  status TEXT NOT NULL DEFAULT 'pending',
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  processed_at TEXT,
  error TEXT
);

CREATE TABLE projection_state (
  id TEXT PRIMARY KEY,
  projection_name TEXT NOT NULL,
  aggregate_kind TEXT NOT NULL,
  aggregate_id TEXT NOT NULL,
  external_id TEXT,
  external_version TEXT,
  status TEXT NOT NULL DEFAULT 'pending',
  last_synced_at TEXT,
  error TEXT,
  UNIQUE(projection_name, aggregate_kind, aggregate_id)
);
```

### 7.3 必须保持的不变量

1. `claim` 没有证据时只能处于 `captured` 或 `hypothesis`，不能进入 `grounded`。
2. `relation` 如果是领域事实关系，必须有 `relation_evidence`，除非显式标记为 `curated_seed`。
3. `concept.primary_parent_id` 形成主导航树，不能有环。
4. `IS_A`、`PART_OF` 等层级关系必须是 DAG。
5. `MERGE`、`SPLIT`、`MOVE` 必须通过 `ontology_patch`，不得直接改表。
6. 同一 URL 的重复抓取必须通过内容 hash 去重。
7. 所有投影输出必须能从 SQLite 和 blobs 重新生成。
8. agent 不直接写核心表，只提交结构化命令到 Writer Service。
9. 本地模型生成内容必须记录 `created_by`、模型名、prompt 版本和输入证据范围。
10. 外部系统 ID 只能存在 `projection_state` 或 adapter 私有表中，不能成为规范 ID。

## 8. 本体与知识图谱策略

### 8.1 主树 + 辅助图

为兼顾人类可读和机器推理，概念结构分两层：

1. **主树**：每个概念最多一个 `primary_parent_id`，用于 Obsidian 文件夹、Notion 页面层级、离线导航。
2. **辅助图**：通过 `relation` 表表达多父关系、前置知识、攻击链、绕过技术、实例、工具、靶场、论文引用。

示例：

```text
主树：
Web Security
└── Injection
    ├── SQL Injection
    └── Command Injection

辅助图：
SQL Injection --EXPLOITS--> SQL Parser Boundary
SQL Injection --HAS_EXAMPLE--> sqli-labs Less-1
Command Injection --REQUIRES--> OS Command Execution Context
SQL Injection --SIBLING_OF--> NoSQL Injection
Template Injection --CAN_LEAD_TO--> RCE
```

### 8.2 补丁类型

每轮新增材料后，agent 必须输出本体补丁提案：

```text
REUSE        复用已有概念，不新增节点。
ADD          新增概念。
MERGE        合并重复概念。
MOVE         调整主树父节点。
SPLIT        拆分过宽或语义混杂概念。
DEPRECATE    标记概念废弃但保留历史。
ADD_RELATION 新增辅助关系。
UPDATE_DEF   更新定义、边界或别名。
```

### 8.3 补丁 JSON 示例

```json
{
  "patch_type": "MOVE",
  "target": {
    "kind": "concept",
    "id": "concept-ssti"
  },
  "before": {
    "primary_parent_id": "concept-web-security"
  },
  "after": {
    "primary_parent_id": "concept-injection"
  },
  "rationale": "SSTI 的共同机制是用户输入进入模板解释器并改变模板语义，和 SQL/Command/LDAP Injection 共享注入抽象。放在 Web Security 根节点下会造成漏洞类型平铺。",
  "evidence": [
    {
      "evidence_span_id": "span-...",
      "support_type": "definition"
    }
  ],
  "impact": {
    "affected_documents": ["doc-..."],
    "affected_children": [],
    "risk": "low"
  }
}
```

### 8.4 本体质量指标

每次 patch 应计算以下指标：

- `orphan_rate`：孤立概念比例。
- `average_branch_factor`：平均分支数。
- `max_branch_factor`：最大单父节点子节点数。
- `max_depth`：主树最大深度。
- `sibling_overlap_score`：兄弟节点语义重叠。
- `duplicate_candidate_count`：疑似重复概念数。
- `evidence_coverage`：有证据支持的关系比例。
- `retrieval_hit_rate_delta`：离线检索回归集命中变化。
- `structural_churn_cost`：移动、合并对已有文档链接的扰动成本。

不要追求机械的“红黑树式平衡”。知识图谱不是搜索树。更合适的是保持：

- 父节点抽象一致。
- 同级节点粒度接近。
- 分支不过宽。
- 深度不过深。
- 高频查询路径短。
- 大变更可解释、可回滚。

### 8.5 人工审批边界

可以自动应用：

- 新增别名。
- 新增低风险 `HAS_EXAMPLE` 关系。
- 将 WP 绑定到已有 challenge。
- 新增 evidence span。
- 新增 draft 文档。

必须人工审批：

- `MERGE`。
- `SPLIT`。
- 大范围 `MOVE`。
- 修改核心概念定义。
- 删除或废弃概念。
- 将未验证结论提升到高可靠状态。

## 9. 可靠性模型

### 9.1 状态定义

不要只用一个 LLM 生成的 0-1 confidence。使用可解释状态：

```text
Captured          已捕获，来自某个来源或模型提案。
Grounded          已绑定明确证据片段。
Corroborated      多个独立来源支持。
Built             环境或代码能构建。
Reproduced        在隔离环境中复现成功。
Human-validated   人工确认有效。
Deprecated        已过时或不建议使用。
Contradicted      存在明确反证。
```

### 9.2 来源可信分层

建议初始 trust tier：

```text
T0 官方标准、论文原文、项目源码、靶场源码、CVE/NVD/厂商公告。
T1 CTF-Wiki、知名安全团队博客、比赛官方 WP、主流会议材料。
T2 个人博客、GitHub issue、论坛长帖、课程笔记。
T3 聚合站、转载站、无法确认作者的文章。
T4 模型生成、无来源摘要、二手转述。
```

T4 不能作为事实证据，只能作为候选输入。

### 9.3 Claim 写入规则

一个 claim 的最小结构：

```json
{
  "statement": "SQL Injection arises when untrusted input changes the intended structure of a SQL query.",
  "claim_type": "definition",
  "concept_ids": ["concept-sql-injection", "concept-injection"],
  "evidence_span_ids": ["span-1", "span-2"],
  "reliability_state": "grounded"
}
```

不得写入：

```json
{
  "statement": "SQL 注入很危险。",
  "evidence_span_ids": []
}
```

原因：

- 太泛。
- 无证据。
- 不能指导检索、验证或练习。

## 10. CTF Crawl 详细设计

### 10.1 数据来源

初始来源：

- CTF-Wiki。
- 比赛官方 WP。
- 知名安全团队博客。
- GitHub challenge 仓库。
- Docker 化靶场。
- sqli-labs、DVWA、WebGoat 等基础靶场。
- CVE PoC 仓库，但默认低信任且高隔离。

采集器必须记录：

- URL。
- 访问时间。
- HTML/PDF/Git commit hash。
- 作者。
- 许可证线索。
- 语言。
- 是否转载。
- 是否包含可运行环境。
- 是否包含附件。

### 10.2 CTF 领域核心实体

```text
Challenge
  title
  category
  competition
  year
  files
  docker_context
  expected_flag_pattern
  source

Writeup
  source
  target_challenge
  steps
  payloads
  assumptions
  environment

ExploitTechnique
  concept
  preconditions
  primitives
  payload_templates
  bypasses

VerificationRun
  challenge
  writeup
  sandbox
  commands
  result
  artifacts
```

### 10.3 WP 处理流程

```text
1. 抓取 WP 原文和附件。
2. 解析题目名称、比赛、分类、技术点、payload、命令和环境线索。
3. 判断是否存在可复现环境：
   - 有 Dockerfile/docker-compose。
   - 有源码和依赖说明。
   - 有 challenge 附件。
   - 有公开靶场链接。
4. 如果可复现，生成 VerificationPlan。
5. 在隔离环境中构建和运行。
6. 执行 WP 步骤或让 coding agent 补全利用脚本。
7. 保存证据。
8. 更新 challenge、claim、relation、document。
9. 如果不可复现，标记为 Grounded 或 Corroborated，但不能标记 Reproduced。
```

### 10.4 VerificationPlan 示例

```json
{
  "challenge_id": "challenge-...",
  "goal": "Verify command injection in the provided web challenge.",
  "sandbox_profile": "vm-no-egress-docker",
  "inputs": [
    {
      "kind": "blob",
      "sha256": "..."
    }
  ],
  "steps": [
    {
      "kind": "build",
      "command": "docker compose build",
      "timeout_seconds": 300
    },
    {
      "kind": "run",
      "command": "docker compose up -d",
      "timeout_seconds": 120
    },
    {
      "kind": "probe",
      "command": "curl -i http://127.0.0.1:8080/",
      "timeout_seconds": 20
    },
    {
      "kind": "exploit",
      "command": "python3 exploit.py",
      "timeout_seconds": 60
    }
  ],
  "success_criteria": [
    "response contains flag pattern",
    "command execution evidence captured"
  ],
  "network": {
    "egress": "deny",
    "allowed_hosts": ["127.0.0.1"]
  }
}
```

### 10.5 验证结果

```json
{
  "status": "reproduced",
  "summary": "The exploit obtains command execution through the vulnerable ping parameter.",
  "artifacts": [
    {
      "type": "build_log",
      "sha256": "..."
    },
    {
      "type": "screenshot",
      "sha256": "..."
    },
    {
      "type": "exploit_script",
      "sha256": "..."
    }
  ],
  "claims_supported": ["claim-..."],
  "claims_contradicted": []
}
```

## 11. Survey Agent 详细设计

Survey Agent 和 CTF Crawl 共用内核，但 source/parser/claim 类型不同。

### 11.1 PDF 处理

流程：

```text
1. 保存原始 PDF blob。
2. 提取元数据：标题、作者、机构、年份、DOI、arXiv ID。
3. 提取正文、章节、图表标题、引用。
4. 按页码和段落建立 evidence span。
5. 提取研究问题、方法、数据集、实验设置、主要结论、局限。
6. 生成文献卡片和主题图谱补丁。
```

### 11.2 论文 claim 类型

```text
problem_statement
method_definition
experiment_result
dataset_usage
limitation
future_work
contradiction
related_work_relation
```

### 11.3 文献图谱关系

```text
Paper --PROPOSES--> Method
Paper --EVALUATES_ON--> Dataset
Method --IMPROVES--> Baseline
Paper --CONTRADICTS--> Paper
Paper --EXTENDS--> Method
Method --REQUIRES--> Assumption
```

## 12. 文档生成与 Markdown 规范

### 12.1 规范 Markdown 不是 Notion Markdown

系统内部使用 portable Markdown，兼容 Obsidian 和普通编辑器。Notion-flavored Markdown 只在 Notion adapter 中使用。

原因：

- Notion 增强 Markdown 使用 XML-like tags、属性和 Notion 特定 block 表达。
- 这些语法不完全等价于标准 Markdown。
- 如果把 Notion-flavored Markdown 当规范格式，会污染 Obsidian 和本地检索。

### 12.2 文档 frontmatter

```yaml
---
id: doc-sql-injection
title: SQL Injection
type: concept_note
primary_concept: concept-sql-injection
reliability: grounded
updated_at: 2026-07-16T00:00:00Z
aliases:
  - SQLi
parents:
  - Injection
tags:
  - web
  - injection
  - ctf
---
```

### 12.3 概念笔记模板

````markdown
# SQL Injection

## Definition

SQL Injection is ...

## Mechanism

- User-controlled input crosses a SQL syntax boundary.
- The database parser treats part of the input as executable query structure.

## Preconditions

- ...

## Exploitation Patterns

### Error-based

...

### Boolean-based

...

## Common Payloads

```sql
' OR '1'='1
```

## Bypasses

...

## Examples

- [[sqli-labs Less-1]]

## Evidence

- [source title, captured date, span id]

## Verification Status

| Target | Status | Run |
| --- | --- | --- |
| sqli-labs Less-1 | Reproduced | run-... |

## Related

- Parent: [[Injection]]
- Siblings: [[Command Injection]], [[NoSQL Injection]]
- Leads to: [[Authentication Bypass]], [[Data Exfiltration]]
````

### 12.4 文档生成规则

- 每个概念至少有一篇 concept note。
- 每个 challenge 至少有一篇 challenge note。
- 每篇 reproduced WP 可以有一篇 lab note。
- 文档中必须标记可靠性状态。
- payload 和命令必须保留代码块。
- 引用证据必须能追溯到 `evidence_span.id`。
- 不在文档里堆未处理长摘录，原文在 snapshot 中保存。

## 13. Notion 与 Obsidian 同步

### 13.1 Notion 角色

Notion 是在线精炼中心和人工审阅界面，不是唯一真相源。

Notion adapter 负责：

- 创建/更新页面。
- 同步概念页、challenge 页、review queue。
- 把人工编辑导回为 `document_revision` 或 `ontology_patch`。
- 记录 Notion page id 到 `projection_state`。

### 13.2 Notion 当前 API 注意事项

截至 2026-07-16 核对的官方文档：

- Notion 支持通过增强 Markdown 创建、读取和更新页面内容。
- 增强 Markdown 也叫 Notion-flavored Markdown，不是纯标准 Markdown。
- Notion API 仍有请求限流和 payload 限制，例如连接级平均约 3 requests/s，payload 最大 1000 block elements 和 500KB。
- 2026-03-11 API 版本引入破坏性变化，包括 append block children 的 `after` 改为 `position`，`archived` 改为 `in_trash`，`transcription` 改为 `meeting_notes`。

参考：

- https://developers.notion.com/guides/data-apis/working-with-markdown-content
- https://developers.notion.com/guides/data-apis/enhanced-markdown
- https://developers.notion.com/reference/request-limits
- https://developers.notion.com/guides/get-started/upgrade-guide-2026-03-11

### 13.3 Notion 同步策略

```text
document_revision created
  -> outbox_event(document.updated)
  -> notion_projector reads canonical markdown
  -> convert portable markdown to Notion-flavored markdown
  -> PATCH /v1/pages/:page_id/markdown or POST /v1/pages
  -> update projection_state
```

失败处理：

- 429/529：尊重 `Retry-After`，指数退避。
- payload 过大：拆分章节或使用子页面。
- unsupported block：回退 block API 或保留纯文本。
- 附件：本地 blob 需要先上传到可访问对象存储或选择只同步链接占位。

### 13.4 Obsidian 同步策略

Obsidian vault 是纯文件投影：

```text
vault/
├── Web Security/
│   └── Injection/
│       ├── Injection.md
│       ├── SQL Injection.md
│       └── Command Injection.md
├── Challenges/
│   └── ...
└── _assets/
```

规则：

- 文件路径由 `primary_parent` 和标题生成。
- frontmatter 中保存规范 ID。
- wikilink 由概念关系生成。
- 文件名冲突用短 ID 后缀解决。
- 人工直接改 Obsidian 文件时，不自动覆盖内核，必须通过 import diff 生成 `document_revision` 提案。

## 14. 离线检索与本地模型

### 14.1 混合检索

离线检索使用三路召回：

1. FTS5：关键词、payload、函数名、CVE、错误信息、符号。
2. 向量检索：语义问题、相似 WP、概念解释。
3. 图邻域：父子概念、前置知识、相关靶场、已复现实例。

最终 rerank 输入：

```json
{
  "query": "blind sql injection time based payload mysql",
  "fts_hits": ["chunk-1", "chunk-2"],
  "vector_hits": ["chunk-3", "chunk-4"],
  "graph_context": {
    "concepts": ["SQL Injection", "Blind SQL Injection", "MySQL"],
    "examples": ["sqli-labs Less-9"]
  }
}
```

### 14.2 给本地小模型的上下文包

本地小模型不应拿整篇长文，而应拿结构化证据包：

```json
{
  "question": "...",
  "concept_context": [
    {
      "name": "Blind SQL Injection",
      "definition": "...",
      "parent": "SQL Injection",
      "reliability": "grounded"
    }
  ],
  "evidence": [
    {
      "span_id": "span-...",
      "source_title": "...",
      "text": "...",
      "reliability": "grounded"
    }
  ],
  "verified_examples": [
    {
      "challenge": "sqli-labs Less-9",
      "status": "reproduced",
      "doc_id": "doc-..."
    }
  ],
  "constraints": [
    "Only answer from provided evidence.",
    "If evidence is insufficient, say so."
  ]
}
```

### 14.3 离线包 manifest

```json
{
  "pack_id": "ctf-offline-2026-07",
  "generated_at": "2026-07-16T00:00:00Z",
  "ontology_version": 12,
  "document_revision_count": 500,
  "blob_count": 1200,
  "fts_index_version": "fts5-v1",
  "embedding_model": "bge-small-zh-en",
  "embedding_index": "faiss-flat-v1",
  "local_llm_profile": "qwen2.5-coder-7b-instruct",
  "hash": "..."
}
```

## 15. Writer Service 设计

### 15.1 为什么需要 Writer Service

SQLite 可以很好地支撑本地单机知识库，但不能让多个 agent 直接并发写数据库。Writer Service 负责：

- 串行化写入。
- 校验 schema。
- 运行不变量检查。
- 生成 outbox。
- 控制事务边界。
- 记录审计日志。

### 15.2 命令类型

```text
CaptureSourceCommand
CreateEvidenceSpansCommand
ProposeOntologyPatchCommand
ApplyOntologyPatchCommand
CreateDocumentRevisionCommand
RecordVerificationRunCommand
CreateProjectionStateCommand
MarkProjectionSyncedCommand
```

### 15.3 写入事务模式

```text
BEGIN IMMEDIATE;
  validate command
  insert/update canonical tables
  insert outbox_event
  insert audit log
COMMIT;
```

### 15.4 SQLite 配置

```sql
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
PRAGMA foreign_keys = ON;
PRAGMA busy_timeout = 5000;
```

## 16. Agent 开发协议

### 16.1 Agent 输出必须结构化

禁止只输出自然语言总结后直接入库。每个 agent 应输出明确 schema。

DistillationResult：

```json
{
  "source_snapshot_id": "snapshot-...",
  "claims": [],
  "concept_candidates": [],
  "relation_candidates": [],
  "ontology_patches": [],
  "document_draft": {
    "title": "...",
    "markdown": "..."
  },
  "verification_candidates": []
}
```

### 16.2 Agent 不能越权

agent 可以：

- 读已有概念摘要。
- 读 evidence spans。
- 提出 patch。
- 写草稿。
- 请求验证。

agent 不可以：

- 直接 merge 概念。
- 删除概念。
- 提升可靠性到 `Reproduced`。
- 覆盖人工文档。
- 把无来源模型结论标记为事实。

### 16.3 Prompt 版本化

每次模型加工必须记录：

- model name。
- model version。
- prompt template version。
- input snapshot ids。
- output hash。
- created_at。

否则无法复现实验、排查污染或比较不同 agent 质量。

## 17. 开发仓库建议结构

```text
KnownEngine/
├── docs/
│   └── knowledge-engine-design.md
├── knownengine/
│   ├── __init__.py
│   ├── kernel/
│   │   ├── db.py
│   │   ├── migrations/
│   │   ├── models.py
│   │   └── writer.py
│   ├── blobs/
│   │   └── store.py
│   ├── capture/
│   │   ├── web.py
│   │   ├── github.py
│   │   ├── pdf.py
│   │   └── notion.py
│   ├── parse/
│   │   ├── html.py
│   │   ├── markdown.py
│   │   └── pdf.py
│   ├── distill/
│   │   ├── prompts/
│   │   ├── schemas.py
│   │   └── agents.py
│   ├── ontology/
│   │   ├── patch.py
│   │   ├── metrics.py
│   │   └── validation.py
│   ├── verify/
│   │   ├── planner.py
│   │   ├── runner.py
│   │   └── sandbox.py
│   ├── projectors/
│   │   ├── obsidian.py
│   │   ├── notion.py
│   │   ├── graph.py
│   │   └── offline_pack.py
│   ├── retrieval/
│   │   ├── fts.py
│   │   ├── vectors.py
│   │   └── hybrid.py
│   └── cli.py
├── tests/
├── pyproject.toml
└── README.md
```

推荐 MVP 使用 Python：

- `sqlite3` 或 SQLAlchemy Core。
- Pydantic 做命令 schema。
- Alembic 或自研轻量 migration。
- httpx 做抓取。
- BeautifulSoup/readability-lxml/trafilatura 做 HTML 正文抽取。
- pypdf/pdfplumber 做 PDF 初版解析。
- FastAPI 可选，用于本地服务。
- pytest 做测试。

## 18. CLI 设计

MVP CLI 示例：

```bash
known init ./knowledge
known capture url https://ctf-wiki.org/web/sqli/
known distill snapshot <snapshot-id>
known ontology review
known ontology apply <patch-id>
known project obsidian
known project notion
known index rebuild
known search "time based blind sql injection mysql"
known verify challenge <challenge-id>
known pack offline --profile ctf
```

## 19. 测试策略

### 19.1 单元测试

必须覆盖：

- blob hash 和路径生成。
- source 去重。
- evidence span 定位。
- claim 必须绑定 evidence 的校验。
- ontology DAG 无环校验。
- patch JSON schema。
- Markdown frontmatter 生成。
- projection 幂等性。

### 19.2 集成测试

最小集成用例：

1. 导入一篇 SQL Injection Markdown。
2. 提取 evidence span。
3. 创建 `Injection` 和 `SQL Injection` 概念。
4. 生成 concept note。
5. 生成 Obsidian 文件。
6. FTS 搜索 `UNION SELECT` 能命中。
7. 导出离线包 manifest。

### 19.3 本体回归测试

维护一组固定查询：

```text
"SQL injection and command injection common parent"
"SSTI belongs to injection or template engine"
"blind SQL injection mysql sleep payload"
"CTF web command injection reproduced examples"
```

每次本体 patch 后检查：

- 关键概念是否仍可被检索。
- 父节点是否符合预期。
- 同义词是否保留。
- 文档链接是否断裂。

### 19.4 验证沙盒测试

必须覆盖：

- 无外网运行。
- 超时终止。
- 日志保存。
- artifact hash 保存。
- 构建失败状态。
- 复现失败状态。
- 成功复现状态。

## 20. 安全约束

CTF Crawl 会处理不可信代码、压缩包、镜像和 exploit，必须默认高风险。

约束：

- 不在宿主机直接运行未知脚本。
- 不把宿主目录挂进容器可写路径。
- 默认禁用外网。
- 限制 CPU、内存、磁盘、进程数。
- 限制运行时长。
- 保存所有命令日志。
- 验证环境一次性销毁。
- 不自动执行需要特权容器的样本。
- 不自动运行明显 destructive 的命令。
- 对 PoC 仓库默认只读分析，执行需显式验证计划。

## 21. Notion/外部系统适配开发约束

### 21.1 Adapter 隔离

所有外部系统适配器必须在 `projectors/` 或 `capture/` 内，不得污染核心模型。

禁止在核心表中出现：

- `notion_page_id`。
- `obsidian_path`。
- `neo4j_node_id`。

应统一存在：

```text
projection_state(projection_name, aggregate_kind, aggregate_id, external_id)
```

### 21.2 同步方向

默认方向：

```text
Canonical Kernel -> Projection
```

反向导入必须变成：

```text
Projection diff -> Review -> Command -> Canonical Kernel
```

不能让 Notion 或 Obsidian 静默覆盖 SQLite。

## 22. MVP 切片

建议第一个 MVP 不要覆盖所有 CTF 知识，而是选择一个窄主题：

```text
Web Security / Injection
```

范围：

- CTF-Wiki 中 SQL Injection、Command Injection、SSTI 相关页面。
- 20 篇高质量 WP。
- 2-3 个可 Docker 复现靶场。
- 30-50 个核心概念。
- 100-200 个 claim。
- 5-10 篇 concept notes。
- 5 篇 challenge notes。

必须验证：

1. SQL Injection、Command Injection、SSTI 被放到 `Injection` 下。
2. `Injection` 的定义能解释这些子类共同机制。
3. 每个关键 claim 有 evidence span。
4. 至少一个 challenge 达到 `Reproduced`。
5. Obsidian vault 可离线浏览。
6. FTS 能搜 payload 和错误信息。
7. 本地小模型能基于证据包回答问题。

## 23. 里程碑

### M0：内核骨架

- 初始化项目。
- 建立 SQLite schema。
- 建立 blob store。
- 建立 Writer Service。
- 建立 migration 和测试。

### M1：采集与证据

- URL 抓取。
- Markdown/HTML 解析。
- evidence span 保存。
- source 去重。

### M2：本体补丁

- 概念表。
- 关系表。
- patch schema。
- DAG 校验。
- review CLI。

### M3：文档投影

- concept note 生成。
- Obsidian vault 投影。
- FTS5 索引。
- 搜索 CLI。

### M4：CTF 验证

- challenge 实体。
- VerificationPlan。
- 初版 sandbox runner。
- artifact 保存。

### M5：Notion 投影

- Notion page mapping。
- Markdown 转 Notion-flavored Markdown。
- 限流队列。
- 失败重试。

### M6：离线包

- manifest。
- vault + blobs + indexes 打包。
- 本地模型证据包接口。

## 24. 开发质量要求

### 24.1 数据优先

先把 schema、不变量、迁移、测试做好，再扩展 agent。知识工程系统最怕先堆 agent，最后没有可审计事实层。

### 24.2 所有生成内容可追溯

任何 AI 生成的：

- 概念定义。
- 文档段落。
- claim。
- relation。
- patch rationale。

都必须记录输入证据、模型、prompt 版本和输出 hash。

### 24.3 投影可删除重建

以下目录应可删除后重建：

- `vault/`
- `indexes/`
- `graph export`
- `notion projection state` 的外部页面内容，除 page id mapping 外。

不能删除后丢失规范知识。

### 24.4 避免过早引入复杂基础设施

MVP 不建议一开始引入：

- PostgreSQL。
- Neo4j/Memgraph。
- Qdrant/Milvus。
- 分布式任务队列。
- 多 agent 编排平台。

除非已有真实瓶颈。先用 SQLite、文件系统、单 Writer、简单队列把知识闭环跑通。

## 25. 需要尽早明确的开放问题

1. CTF Crawl 的初始许可策略：哪些来源允许全文保存，哪些只保存摘要和链接。
2. 沙盒运行方案：本地 VM、远程隔离机、Firecracker、Lima/Colima、Docker-only 的边界。
3. Notion 反向导入策略：人工在 Notion 改文档后，是整篇 diff，还是只允许 review 字段回写。
4. 本地小模型选择：中文/英文/代码混合检索场景下的 embedding 和 reranker。
5. 人工审批界面：先 CLI，还是早期做一个本地 Web UI。
6. 文档粒度：一个概念一页，还是大类总览 + 子概念页。
7. 版权和敏感材料：线下包是否包含原文 PDF、完整 WP、源码镜像。

## 26. 推荐下一步

第一阶段应把工程闭环压到最小：

```text
一个主题：Injection
两个来源：CTF-Wiki + 1 个高质量 WP
一个验证目标：sqli-labs 或等价 Docker 靶场
一个输出：Obsidian vault + SQLite FTS
```

完成后再加 Notion、更多爬虫和图数据库投影。这样能尽早验证最关键假设：

- agent 是否真的能维护抽象层级。
- evidence 是否足够细。
- 文档是否人类可读。
- 离线检索是否比普通文件夹更有用。
- CTF 验证产物是否能显著提高知识可靠性。
