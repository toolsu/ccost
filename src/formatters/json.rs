use crate::types::{DedupStats, GroupedData};
use serde_json;

pub struct JsonMeta {
    pub dimensions: Vec<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub tz: Option<String>,
    pub project: Option<String>,
    pub model: Option<String>,
    pub session: Option<String>,
    pub order: String,
    pub earliest: Option<String>,
    pub latest: Option<String>,
    pub projects: Vec<String>,
    pub models: Vec<String>,
    pub sessions: Vec<String>,
    pub generated_at: String,
    pub pricing_date: String,
}

/// Format data as pretty-printed JSON with meta, data, totals, and dedup sections.
pub fn format_json(
    data: &[GroupedData],
    totals: &GroupedData,
    meta: &JsonMeta,
    dedup: &DedupStats,
) -> String {
    let meta_value = serde_json::json!({
        "dimensions": meta.dimensions,
        "from": meta.from,
        "to": meta.to,
        "tz": meta.tz,
        "project": meta.project,
        "model": meta.model,
        "session": meta.session,
        "order": meta.order,
        "earliest": meta.earliest,
        "latest": meta.latest,
        "projects": meta.projects,
        "models": meta.models,
        "sessions": meta.sessions,
        "generatedAt": meta.generated_at,
        "pricingDate": meta.pricing_date,
    });

    let output = serde_json::json!({
        "meta": meta_value,
        "data": data,
        "totals": totals,
        "dedup": dedup,
    });

    serde_json::to_string_pretty(&output).unwrap_or_else(|_| "{}".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_json_basic() {
        let data = vec![GroupedData {
            label: "2025-01".to_string(),
            input_tokens: 1000,
            output_tokens: 500,
            cache_creation_tokens: 200,
            cache_read_tokens: 300,
            input_cost: 0.10,
            cache_creation_cost: 0.02,
            cache_read_cost: 0.03,
            output_cost: 0.05,
            total_cost: 0.20,
            children: None,
        }];
        let totals = data[0].clone();
        let meta = JsonMeta {
            dimensions: vec!["day".to_string()],
            from: Some("2025-01-01".to_string()),
            to: Some("2025-01-31".to_string()),
            tz: Some("UTC".to_string()),
            project: None,
            model: None,
            session: None,
            order: "asc".to_string(),
            earliest: Some("2025-01-01T00:00:00+00:00".to_string()),
            latest: Some("2025-01-31T23:59:59+00:00".to_string()),
            projects: vec!["myproject".to_string()],
            models: vec!["claude-3".to_string()],
            sessions: vec!["s1".to_string()],
            generated_at: "2025-02-01T00:00:00Z".to_string(),
            pricing_date: "2025-01-15".to_string(),
        };
        let dedup = DedupStats {
            before: 100,
            after: 90,
        };

        let result = format_json(&data, &totals, &meta, &dedup);
        assert!(result.contains("\"dimensions\""));
        assert!(result.contains("\"from\""));
        assert!(result.contains("inputTokens"));
        assert!(result.contains("2025-01"));
    }

    #[test]
    fn test_format_json_null_from_to() {
        let data = vec![];
        let totals = GroupedData {
            label: "TOTAL".to_string(),
            input_tokens: 0,
            output_tokens: 0,
            cache_creation_tokens: 0,
            cache_read_tokens: 0,
            input_cost: 0.0,
            cache_creation_cost: 0.0,
            cache_read_cost: 0.0,
            output_cost: 0.0,
            total_cost: 0.0,
            children: None,
        };
        let meta = JsonMeta {
            dimensions: vec![],
            from: None,
            to: None,
            tz: None,
            project: None,
            model: None,
            session: None,
            order: "asc".to_string(),
            earliest: None,
            latest: None,
            projects: vec![],
            models: vec![],
            sessions: vec![],
            generated_at: "2025-02-01T00:00:00Z".to_string(),
            pricing_date: "2025-01-15".to_string(),
        };
        let dedup = DedupStats {
            before: 0,
            after: 0,
        };

        let result = format_json(&data, &totals, &meta, &dedup);
        assert!(result.contains("\"from\": null"));
        assert!(result.contains("\"to\": null"));
    }
}
