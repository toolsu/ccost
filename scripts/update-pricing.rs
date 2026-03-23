/// Fetch latest pricing from LiteLLM and overwrite src/pricing-data.json.
///
/// Usage: cargo run --bin update-pricing
fn main() {
    match ccost::fetch_live_pricing() {
        Ok(pricing) => {
            let json = serde_json::to_string_pretty(&pricing).unwrap();
            std::fs::write("src/pricing-data.json", format!("{json}\n")).unwrap();
            eprintln!(
                "Updated src/pricing-data.json ({} models, fetched at {})",
                pricing.models.len(),
                pricing.fetched_at
            );
        }
        Err(e) => {
            eprintln!("Error fetching pricing: {e}");
            std::process::exit(1);
        }
    }
}
