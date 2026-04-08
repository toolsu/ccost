use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct TokenRecord {
    pub timestamp: DateTime<Utc>,
    pub model: String,
    pub session_id: String,
    pub project: String,
    pub agent_id: String,
    pub tool_names: String,
    pub line: u32,
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
    pub agent_id: String,
    pub tool_names: String,
    pub line: u32,
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
            agent_id: record.agent_id.clone(),
            tool_names: record.tool_names.clone(),
            line: record.line,
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
    pub cache_creation_cost_per_token: f64,
    pub cache_read_cost_per_token: f64,
    pub input_cost_per_token: f64,
    pub output_cost_per_token: f64,
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
    Subagent,
    Tool,
    Line,
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
            "subagent" => Ok(Self::Subagent),
            "tool" => Ok(Self::Tool),
            "line" => Ok(Self::Line),
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
            Self::Subagent => "subagent",
            Self::Tool => "tool",
            Self::Line => "line",
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
            Self::Subagent => "Subagent",
            Self::Tool => "Tool",
            Self::Line => "Line",
        }
    }

    pub fn all_valid() -> &'static [&'static str] {
        &[
            "day", "hour", "month", "session", "project", "model", "subagent", "tool", "line",
        ]
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

#[cfg(test)]
mod tests {
    use super::*;

    // --- GroupDimension::from_str ---

    #[test]
    fn test_group_dimension_from_str_all_valid() {
        assert_eq!(
            "day".parse::<GroupDimension>().unwrap(),
            GroupDimension::Day
        );
        assert_eq!(
            "hour".parse::<GroupDimension>().unwrap(),
            GroupDimension::Hour
        );
        assert_eq!(
            "month".parse::<GroupDimension>().unwrap(),
            GroupDimension::Month
        );
        assert_eq!(
            "session".parse::<GroupDimension>().unwrap(),
            GroupDimension::Session
        );
        assert_eq!(
            "project".parse::<GroupDimension>().unwrap(),
            GroupDimension::Project
        );
        assert_eq!(
            "model".parse::<GroupDimension>().unwrap(),
            GroupDimension::Model
        );
        assert_eq!(
            "subagent".parse::<GroupDimension>().unwrap(),
            GroupDimension::Subagent
        );
        assert_eq!(
            "tool".parse::<GroupDimension>().unwrap(),
            GroupDimension::Tool
        );
        assert_eq!(
            "line".parse::<GroupDimension>().unwrap(),
            GroupDimension::Line
        );
    }

    #[test]
    fn test_group_dimension_from_str_invalid() {
        let err = "week".parse::<GroupDimension>().unwrap_err();
        assert!(err.contains("invalid group dimension"));
        assert!(err.contains("week"));
    }

    // --- GroupDimension::as_str roundtrip ---

    #[test]
    fn test_group_dimension_as_str_roundtrip() {
        let variants = [
            GroupDimension::Day,
            GroupDimension::Hour,
            GroupDimension::Month,
            GroupDimension::Session,
            GroupDimension::Project,
            GroupDimension::Model,
            GroupDimension::Subagent,
            GroupDimension::Tool,
            GroupDimension::Line,
        ];
        for v in variants {
            let s = v.as_str();
            let parsed: GroupDimension = s.parse().unwrap();
            assert_eq!(parsed, v, "roundtrip failed for {:?}", v);
        }
    }

    // --- GroupDimension::label ---

    #[test]
    fn test_group_dimension_label_all() {
        assert_eq!(GroupDimension::Day.label(), "Date");
        assert_eq!(GroupDimension::Hour.label(), "Hour");
        assert_eq!(GroupDimension::Month.label(), "Month");
        assert_eq!(GroupDimension::Session.label(), "Session");
        assert_eq!(GroupDimension::Project.label(), "Project");
        assert_eq!(GroupDimension::Model.label(), "Model");
        assert_eq!(GroupDimension::Subagent.label(), "Subagent");
        assert_eq!(GroupDimension::Tool.label(), "Tool");
        assert_eq!(GroupDimension::Line.label(), "Line");
    }

    // --- GroupDimension::all_valid ---

    #[test]
    fn test_group_dimension_all_valid() {
        let valid = GroupDimension::all_valid();
        assert_eq!(valid.len(), 9);
        // Every entry must be parseable as a GroupDimension
        for s in valid {
            assert!(
                s.parse::<GroupDimension>().is_ok(),
                "'{}' should be a valid GroupDimension",
                s
            );
        }
        // Spot-check expected members
        assert!(valid.contains(&"day"));
        assert!(valid.contains(&"subagent"));
    }

    // --- GroupDimension Display ---

    #[test]
    fn test_group_dimension_display() {
        let variants = [
            GroupDimension::Day,
            GroupDimension::Hour,
            GroupDimension::Month,
            GroupDimension::Session,
            GroupDimension::Project,
            GroupDimension::Model,
            GroupDimension::Subagent,
            GroupDimension::Tool,
            GroupDimension::Line,
        ];
        for v in variants {
            assert_eq!(
                format!("{}", v),
                v.as_str(),
                "Display output mismatch for {:?}",
                v
            );
        }
    }

    // --- SortOrder::from_str ---

    #[test]
    fn test_sort_order_from_str_valid() {
        assert_eq!("asc".parse::<SortOrder>().unwrap(), SortOrder::Asc);
        assert_eq!("desc".parse::<SortOrder>().unwrap(), SortOrder::Desc);
    }

    #[test]
    fn test_sort_order_from_str_invalid() {
        let err = "ascending".parse::<SortOrder>().unwrap_err();
        assert!(err.contains("invalid sort order"));
        assert!(err.contains("ascending"));
    }

    // --- PricedTokenRecord::from_token_record ---

    #[test]
    fn test_priced_token_record_from_token_record() {
        use chrono::Utc;
        let now = Utc::now();
        let record = TokenRecord {
            timestamp: now,
            model: "claude-3-5-sonnet".to_string(),
            session_id: "sess-abc".to_string(),
            project: "my-project".to_string(),
            agent_id: "agent-1".to_string(),
            tool_names: "Edit, Read".to_string(),
            line: 3,
            input_tokens: 100,
            output_tokens: 50,
            cache_creation_tokens: 10,
            cache_read_tokens: 5,
        };

        let priced = PricedTokenRecord::from_token_record(&record, 0.01, 0.02, 0.003, 0.04);

        assert_eq!(priced.timestamp, now);
        assert_eq!(priced.model, "claude-3-5-sonnet");
        assert_eq!(priced.session_id, "sess-abc");
        assert_eq!(priced.project, "my-project");
        assert_eq!(priced.agent_id, "agent-1");
        assert_eq!(priced.tool_names, "Edit, Read");
        assert_eq!(priced.line, 3);
        assert_eq!(priced.input_tokens, 100);
        assert_eq!(priced.output_tokens, 50);
        assert_eq!(priced.cache_creation_tokens, 10);
        assert_eq!(priced.cache_read_tokens, 5);
        assert!((priced.input_cost - 0.01).abs() < f64::EPSILON);
        assert!((priced.cache_creation_cost - 0.02).abs() < f64::EPSILON);
        assert!((priced.cache_read_cost - 0.003).abs() < f64::EPSILON);
        assert!((priced.output_cost - 0.04).abs() < f64::EPSILON);
        // total_cost = sum of all four components
        let expected_total = 0.01 + 0.02 + 0.003 + 0.04;
        assert!((priced.total_cost - expected_total).abs() < 1e-10);
    }

    // --- GroupOptions::default ---

    #[test]
    fn test_group_options_default() {
        let opts = GroupOptions::default();
        assert_eq!(opts.order, SortOrder::Asc);
        assert!(opts.tz.is_none());
    }
}
