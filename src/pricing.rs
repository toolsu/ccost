use std::collections::{HashMap, HashSet};
use std::fs;
use std::sync::LazyLock;

use regex::Regex;

use crate::types::{ModelPricing, PricedTokenRecord, PricingData, TokenRecord};

static DATE_SUFFIX_PRICING_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[-@]\d{8}$").unwrap());
static PREFIX_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^anthropic[/.]").unwrap());
static VERSION_SUFFIX_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"-v\d+(?::\d+)?$").unwrap());
static VALID_NAME_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^claude-\w+-\d").unwrap());

/// Load bundled pricing data embedded at compile time.
pub fn load_pricing() -> PricingData {
    let data = include_str!("pricing-data.json");
    serde_json::from_str(data).expect("Failed to parse bundled pricing-data.json")
}

/// Load pricing data from a user-provided JSON file path.
pub fn load_pricing_from_file(file_path: &str) -> Result<PricingData, Box<dyn std::error::Error>> {
    let contents = fs::read_to_string(file_path)?;
    let user: PricingData = serde_json::from_str(&contents)?;
    let mut pricing = load_pricing(); // start from bundled defaults
    // user overrides take precedence
    for (key, value) in user.models {
        pricing.models.insert(key, value);
    }
    if !user.fetched_at.is_empty() {
        pricing.fetched_at = user.fetched_at;
    }
    Ok(pricing)
}

/// Fetch live pricing data from the LiteLLM repository on GitHub.
///
/// Filters for Anthropic Claude models, normalizes keys, and maps cost fields.
pub fn fetch_live_pricing() -> Result<PricingData, Box<dyn std::error::Error>> {
    let url = "https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json";
    let resp = minreq::get(url).send()?;
    let raw: serde_json::Value = serde_json::from_str(resp.as_str()?)?;

    let obj = raw.as_object().ok_or("Expected top-level JSON object")?;

    let prefix_re = &*PREFIX_RE;
    let version_suffix_re = &*VERSION_SUFFIX_RE;
    let valid_name_re = &*VALID_NAME_RE;

    let mut models: HashMap<String, ModelPricing> = HashMap::new();

    for (key, value) in obj.iter() {
        let key: &String = key;
        let value: &serde_json::Value = value;

        // Must start with anthropic/ or anthropic.
        if !key.starts_with("anthropic/claude-") && !key.starts_with("anthropic.claude-") {
            continue;
        }

        // Must have a numeric input_cost_per_token
        let input_cost = match value
            .get("input_cost_per_token")
            .and_then(serde_json::Value::as_f64)
        {
            Some(c) => c,
            None => continue,
        };

        // Strip the anthropic/ or anthropic. prefix
        let stripped = prefix_re.replace(key.as_str(), "").to_string();

        // Strip version suffix like -v1, -v2:0
        let normalized = version_suffix_re.replace(&stripped, "").to_string();

        // Must match claude-<word>-<digit> pattern
        if !valid_name_re.is_match(&normalized) {
            continue;
        }

        // First match per normalized name wins
        if models.contains_key(&normalized) {
            continue;
        }

        let output_cost = value
            .get("output_cost_per_token")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(0.0);
        let cache_creation_cost = value
            .get("cache_creation_input_token_cost")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(0.0);
        let cache_read_cost = value
            .get("cache_read_input_token_cost")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(0.0);

        models.insert(
            normalized,
            ModelPricing {
                input_cost_per_token: input_cost,
                output_cost_per_token: output_cost,
                cache_creation_cost_per_token: cache_creation_cost,
                cache_read_cost_per_token: cache_read_cost,
            },
        );
    }

    let fetched_at = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);

    Ok(PricingData { fetched_at, models })
}

/// Three-tier fuzzy matching of a JSONL model name against pricing models.
///
/// 1. Direct exact match
/// 2. Strip trailing `-YYYYMMDD` or `@YYYYMMDD` date suffix, retry exact match
/// 3. Substring containment among keys starting with `claude-`
pub fn match_model_name<'a>(
    jsonl_name: &str,
    pricing_models: &'a HashMap<String, ModelPricing>,
) -> Option<&'a ModelPricing> {
    // Tier 1: direct exact match
    if let Some(pricing) = pricing_models.get(jsonl_name) {
        return Some(pricing);
    }

    // Tier 2: strip date suffix and retry
    let stripped = DATE_SUFFIX_PRICING_RE.replace(jsonl_name, "").to_string();
    if stripped != jsonl_name {
        if let Some(pricing) = pricing_models.get(&stripped) {
            return Some(pricing);
        }
    }

    // Tier 3: substring containment
    for (key, pricing) in pricing_models {
        if !key.starts_with("claude-") {
            continue;
        }
        if jsonl_name.contains(key.as_str()) || key.contains(jsonl_name) {
            return Some(pricing);
        }
    }

    // No match
    None
}

/// Calculate costs for a slice of token records using pricing data.
///
/// If `pricing` is None, loads the bundled pricing data.
/// Warns once per unmatched model name to stderr.
pub fn calculate_cost(
    records: &[TokenRecord],
    pricing: Option<&PricingData>,
) -> Vec<PricedTokenRecord> {
    let bundled;
    let pricing = match pricing {
        Some(p) => p,
        None => {
            bundled = load_pricing();
            &bundled
        }
    };

    let mut warned: HashSet<&str> = HashSet::new();
    let mut results = Vec::with_capacity(records.len());
    let mut model_cache: HashMap<&str, Option<&ModelPricing>> = HashMap::new();

    for record in records {
        let cached = *model_cache
            .entry(record.model.as_str())
            .or_insert_with(|| match_model_name(&record.model, &pricing.models));
        match cached {
            Some(model_pricing) => {
                let input_cost = record.input_tokens as f64 * model_pricing.input_cost_per_token;
                let cache_creation_cost = record.cache_creation_tokens as f64
                    * model_pricing.cache_creation_cost_per_token;
                let cache_read_cost =
                    record.cache_read_tokens as f64 * model_pricing.cache_read_cost_per_token;
                let output_cost = record.output_tokens as f64 * model_pricing.output_cost_per_token;

                results.push(PricedTokenRecord::from_token_record(
                    record,
                    input_cost,
                    cache_creation_cost,
                    cache_read_cost,
                    output_cost,
                ));
            }
            None => {
                if warned.insert(&record.model) {
                    eprintln!(
                        "Warning: No pricing found for model '{}', costs will be 0",
                        record.model
                    );
                }
                results.push(PricedTokenRecord::from_token_record(
                    record, 0.0, 0.0, 0.0, 0.0,
                ));
            }
        }
    }

    results
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn make_pricing(models: Vec<(&str, f64, f64, f64, f64)>) -> HashMap<String, ModelPricing> {
        models
            .into_iter()
            .map(|(name, inp, out, cc, cr)| {
                (
                    name.to_string(),
                    ModelPricing {
                        input_cost_per_token: inp,
                        output_cost_per_token: out,
                        cache_creation_cost_per_token: cc,
                        cache_read_cost_per_token: cr,
                    },
                )
            })
            .collect()
    }

    // ── match_model_name tests ──────────────────────────────────────────

    #[test]
    fn test_match_exact() {
        let models = make_pricing(vec![("claude-opus-4-6", 0.01, 0.02, 0.0, 0.0)]);
        let result = match_model_name("claude-opus-4-6", &models);
        assert!(result.is_some());
        assert_eq!(result.unwrap().input_cost_per_token, 0.01);
    }

    #[test]
    fn test_match_strip_date_suffix() {
        let models = make_pricing(vec![("claude-sonnet-4", 0.003, 0.015, 0.0, 0.0)]);
        let result = match_model_name("claude-sonnet-4-20250514", &models);
        assert!(result.is_some());
        assert_eq!(result.unwrap().input_cost_per_token, 0.003);
    }

    #[test]
    fn test_match_strip_at_date_suffix() {
        let models = make_pricing(vec![("claude-sonnet-4", 0.003, 0.015, 0.0, 0.0)]);
        let result = match_model_name("claude-sonnet-4@20250514", &models);
        assert!(result.is_some());
    }

    #[test]
    fn test_match_substring_containment() {
        let models = make_pricing(vec![("claude-opus-4-6", 0.01, 0.02, 0.0, 0.0)]);
        // jsonl_name contains the pricing key
        let result = match_model_name("claude-opus-4-6-some-variant", &models);
        assert!(result.is_some());
    }

    #[test]
    fn test_match_substring_reverse() {
        let models = make_pricing(vec![("claude-3-5-haiku-20241022", 0.001, 0.005, 0.0, 0.0)]);
        // pricing key contains jsonl_name
        let result = match_model_name("claude-3-5-haiku", &models);
        assert!(result.is_some());
    }

    #[test]
    fn test_match_no_match() {
        let models = make_pricing(vec![("claude-opus-4-6", 0.01, 0.02, 0.0, 0.0)]);
        let result = match_model_name("gpt-4o", &models);
        assert!(result.is_none());
    }

    #[test]
    fn test_match_substring_only_claude_keys() {
        // Non-claude pricing key should not match via substring tier
        let models = make_pricing(vec![("some-model-x", 0.01, 0.02, 0.0, 0.0)]);
        let result = match_model_name("some-model-x-variant", &models);
        // Tier 1 and 2 fail, tier 3 skips non-claude keys
        assert!(result.is_none());
    }

    #[test]
    fn test_match_empty_models() {
        let models: HashMap<String, ModelPricing> = HashMap::new();
        assert!(match_model_name("claude-opus-4-6", &models).is_none());
    }

    // ── load_pricing tests ──────────────────────────────────────────────

    #[test]
    fn test_load_bundled_pricing() {
        let pricing = load_pricing();
        assert!(
            !pricing.models.is_empty(),
            "bundled pricing should have models"
        );
        assert!(!pricing.fetched_at.is_empty());
    }

    #[test]
    fn test_load_pricing_from_file_merges_with_bundled() {
        let bundled = load_pricing();
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("pricing.json");
        let json = serde_json::json!({
            "fetchedAt": "2026-01-01T00:00:00Z",
            "models": {
                "claude-test-1": {
                    "inputCostPerToken": 0.001,
                    "outputCostPerToken": 0.002,
                    "cacheCreationCostPerToken": 0.0,
                    "cacheReadCostPerToken": 0.0
                }
            }
        });
        std::fs::write(&path, serde_json::to_string(&json).unwrap()).unwrap();

        let result = load_pricing_from_file(path.to_str().unwrap());
        assert!(result.is_ok());
        let pricing = result.unwrap();
        // user model is present
        assert!(pricing.models.contains_key("claude-test-1"));
        // bundled models are also present
        assert!(
            pricing.models.len() > 1,
            "should include bundled models + user model"
        );
        assert_eq!(pricing.models.len(), bundled.models.len() + 1);
        // user fetchedAt overrides bundled
        assert_eq!(pricing.fetched_at, "2026-01-01T00:00:00Z");
    }

    #[test]
    fn test_load_pricing_from_file_without_fetched_at() {
        let bundled = load_pricing();
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("pricing.json");
        let json = serde_json::json!({
            "models": {
                "claude-test-1": {
                    "inputCostPerToken": 0.001,
                    "outputCostPerToken": 0.002,
                    "cacheCreationCostPerToken": 0.0,
                    "cacheReadCostPerToken": 0.0
                }
            }
        });
        std::fs::write(&path, serde_json::to_string(&json).unwrap()).unwrap();

        let pricing = load_pricing_from_file(path.to_str().unwrap()).unwrap();
        assert!(pricing.models.contains_key("claude-test-1"));
        // fetched_at falls back to bundled value
        assert_eq!(pricing.fetched_at, bundled.fetched_at);
    }

    #[test]
    fn test_load_pricing_from_file_not_found() {
        let result = load_pricing_from_file("/nonexistent/path/pricing.json");
        assert!(result.is_err());
    }

    #[test]
    fn test_load_pricing_from_file_invalid_json() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("bad.json");
        std::fs::write(&path, "not valid json!!!").unwrap();

        let result = load_pricing_from_file(path.to_str().unwrap());
        assert!(result.is_err());
    }

    // ── calculate_cost tests ────────────────────────────────────────────

    #[test]
    fn test_calculate_cost_known_model() {
        let pricing = load_pricing();
        let records = vec![TokenRecord {
            timestamp: chrono::Utc::now(),
            model: "claude-opus-4-6".to_string(),
            session_id: "s1".to_string(),
            project: "p1".to_string(),
            input_tokens: 1000,
            output_tokens: 500,
            cache_creation_tokens: 100,
            cache_read_tokens: 200,
        }];

        let priced = calculate_cost(&records, Some(&pricing));
        assert_eq!(priced.len(), 1);
        assert!(
            priced[0].input_cost > 0.0,
            "known model should have non-zero input cost"
        );
        assert!(priced[0].output_cost > 0.0);
        assert!(priced[0].total_cost > 0.0);
    }

    #[test]
    fn test_calculate_cost_unknown_model() {
        let pricing = load_pricing();
        let records = vec![TokenRecord {
            timestamp: chrono::Utc::now(),
            model: "totally-unknown-model-xyz".to_string(),
            session_id: "s1".to_string(),
            project: "p1".to_string(),
            input_tokens: 1000,
            output_tokens: 500,
            cache_creation_tokens: 0,
            cache_read_tokens: 0,
        }];

        let priced = calculate_cost(&records, Some(&pricing));
        assert_eq!(priced.len(), 1);
        assert_eq!(priced[0].input_cost, 0.0);
        assert_eq!(priced[0].output_cost, 0.0);
        assert_eq!(priced[0].total_cost, 0.0);
    }

    #[test]
    fn test_calculate_cost_empty() {
        let pricing = load_pricing();
        let priced = calculate_cost(&[], Some(&pricing));
        assert!(priced.is_empty());
    }

    #[test]
    fn test_calculate_cost_loads_bundled_when_none() {
        let records = vec![TokenRecord {
            timestamp: chrono::Utc::now(),
            model: "claude-opus-4-6".to_string(),
            session_id: "s1".to_string(),
            project: "p1".to_string(),
            input_tokens: 1000,
            output_tokens: 500,
            cache_creation_tokens: 0,
            cache_read_tokens: 0,
        }];

        // pricing = None should load bundled
        let priced = calculate_cost(&records, None);
        assert_eq!(priced.len(), 1);
        assert!(priced[0].total_cost > 0.0);
    }

    #[test]
    fn test_calculate_cost_caches_model_lookup() {
        let pricing = load_pricing();
        // Two records with same model — caching should work correctly
        let records = vec![
            TokenRecord {
                timestamp: chrono::Utc::now(),
                model: "claude-opus-4-6".to_string(),
                session_id: "s1".to_string(),
                project: "p1".to_string(),
                input_tokens: 1000,
                output_tokens: 500,
                cache_creation_tokens: 0,
                cache_read_tokens: 0,
            },
            TokenRecord {
                timestamp: chrono::Utc::now(),
                model: "claude-opus-4-6".to_string(),
                session_id: "s1".to_string(),
                project: "p1".to_string(),
                input_tokens: 2000,
                output_tokens: 1000,
                cache_creation_tokens: 0,
                cache_read_tokens: 0,
            },
        ];

        let priced = calculate_cost(&records, Some(&pricing));
        assert_eq!(priced.len(), 2);
        // Second record has 2x tokens, should have ~2x cost
        assert!(priced[1].input_cost > priced[0].input_cost);
    }
}
