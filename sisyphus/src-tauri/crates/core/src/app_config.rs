// LLM / Notion 配置的唯一 schema 定义（Tauri commands 与 agent_runtime 两处共用，
// 此前各自定义过一份，字段一改就容易失配）。

use serde::{Deserialize, Serialize};
use std::path::Path;

pub const LLM_CONFIG_FILE: &str = "llm_config.json";
pub const NOTION_CONFIG_FILE: &str = "notion_config.json";

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct LlmConfig {
    /// pi-ai provider id；openai 默认使用 Responses API。
    #[serde(default)]
    pub format: String,
    /// 自定义 API base URL（留空=用该 provider 默认）
    #[serde(default)]
    pub base_url: String,
    /// 模型名（pi-ai provider 目录内的模型 id）
    #[serde(default)]
    pub model: String,
    /// API key（仅后端保存，不随 get_llm_config 返回）
    #[serde(default)]
    pub api_key: String,
}

pub fn read_llm_config(data_dir: &Path) -> LlmConfig {
    std::fs::read_to_string(data_dir.join(LLM_CONFIG_FILE))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct NotionConfig {
    #[serde(default)]
    pub token: String,
    #[serde(default)]
    pub page_id: String,
    #[serde(default)]
    pub sync_enabled: bool,
}

pub fn read_notion_config(data_dir: &Path) -> NotionConfig {
    std::fs::read_to_string(data_dir.join(NOTION_CONFIG_FILE))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}
