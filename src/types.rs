use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct TokenRecord {
    pub timestamp: DateTime<Utc>,
    pub model: String,
    pub session_id: String,
    pub project: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_tokens: u64,
    pub cache_read_tokens: u64,
}

#[derive(Debug, Clone)]
pub struct PricedTokenRecord {
    pub timestamp: DateTime<Utc>,
    pub model: String,
    pub session_id: String,
    pub project: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_tokens: u64,
    pub cache_read_tokens: u64,
    pub input_cost: f64,
    pub cache_creation_cost: f64,
    pub cache_read_cost: f64,
    pub output_cost: f64,
    pub total_cost: f64,
}

impl PricedTokenRecord {
    pub fn from_token_record(
        record: &TokenRecord,
        input_cost: f64,
        cache_creation_cost: f64,
        cache_read_cost: f64,
        output_cost: f64,
    ) -> Self {
        Self {
            timestamp: record.timestamp,
            model: record.model.clone(),
            session_id: record.session_id.clone(),
            project: record.project.clone(),
            input_tokens: record.input_tokens,
            output_tokens: record.output_tokens,
            cache_creation_tokens: record.cache_creation_tokens,
            cache_read_tokens: record.cache_read_tokens,
            input_cost,
            cache_creation_cost,
            cache_read_cost,
            output_cost,
            total_cost: input_cost + cache_creation_cost + cache_read_cost + output_cost,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupedData {
    pub label: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_tokens: u64,
    pub cache_read_tokens: u64,
    pub input_cost: f64,
    pub cache_creation_cost: f64,
    pub cache_read_cost: f64,
    pub output_cost: f64,
    pub total_cost: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<GroupedData>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelPricing {
    pub input_cost_per_token: f64,
    pub output_cost_per_token: f64,
    pub cache_creation_cost_per_token: f64,
    pub cache_read_cost_per_token: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PricingData {
    #[serde(default)]
    pub fetched_at: String,
    pub models: HashMap<String, ModelPricing>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GroupDimension {
    Day,
    Hour,
    Month,
    Session,
    Project,
    Model,
}

impl std::str::FromStr for GroupDimension {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "day" => Ok(Self::Day),
            "hour" => Ok(Self::Hour),
            "month" => Ok(Self::Month),
            "session" => Ok(Self::Session),
            "project" => Ok(Self::Project),
            "model" => Ok(Self::Model),
            _ => Err(format!("invalid group dimension: '{}'", s)),
        }
    }
}

impl GroupDimension {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Day => "day",
            Self::Hour => "hour",
            Self::Month => "month",
            Self::Session => "session",
            Self::Project => "project",
            Self::Model => "model",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Day => "Date",
            Self::Hour => "Hour",
            Self::Month => "Month",
            Self::Session => "Session",
            Self::Project => "Project",
            Self::Model => "Model",
        }
    }

    pub fn all_valid() -> &'static [&'static str] {
        &["day", "hour", "month", "session", "project", "model"]
    }
}

impl std::fmt::Display for GroupDimension {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortOrder {
    Asc,
    Desc,
}

impl std::str::FromStr for SortOrder {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "asc" => Ok(Self::Asc),
            "desc" => Ok(Self::Desc),
            _ => Err(format!("invalid sort order: '{}'", s)),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct LoadOptions {
    pub claude_dir: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub tz: Option<String>,
    pub project: Option<String>,
    pub model: Option<String>,
    pub session: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DedupStats {
    pub before: usize,
    pub after: usize,
}

#[derive(Debug, Clone)]
pub struct RecordsMeta {
    pub earliest: Option<DateTime<Utc>>,
    pub latest: Option<DateTime<Utc>>,
    pub projects: Vec<String>,
    pub models: Vec<String>,
    pub sessions: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PriceMode {
    Off,
    Integer,
    Decimal,
}

pub struct LoadResult {
    pub records: Vec<TokenRecord>,
    pub dedup: DedupStats,
    pub meta: RecordsMeta,
}

pub struct GroupResult {
    pub data: Vec<GroupedData>,
    pub totals: GroupedData,
}

pub struct GroupOptions {
    pub order: SortOrder,
    pub tz: Option<String>,
}

impl Default for GroupOptions {
    fn default() -> Self {
        Self {
            order: SortOrder::Asc,
            tz: None,
        }
    }
}
