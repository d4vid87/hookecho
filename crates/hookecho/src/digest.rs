//! Plain-language storm digest: turn the in-view alerts + storm reports into a short readable
//! briefing. Works fully offline (templated summary); if an Anthropic key is set, Claude rewrites
//! the same facts into friendlier prose.

/// One active alert to summarize.
pub struct AlertLine {
    pub event: String,
    pub area: String,
}

/// A deterministic, offline plain-language summary. Also serves as the exact fact list handed to
/// the LLM, so the two never disagree on substance.
pub fn templated(alerts: &[AlertLine], reports: [usize; 3]) -> String {
    let mut out = String::new();
    if alerts.is_empty() {
        out.push_str("No active warnings or watches in view.");
    } else {
        // Count by event type.
        let mut counts: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
        for a in alerts {
            *counts.entry(a.event.as_str()).or_default() += 1;
        }
        out.push_str("Active in view: ");
        let parts: Vec<String> = counts.iter().map(|(e, n)| format!("{n} {e}")).collect();
        out.push_str(&parts.join(", "));
        out.push('.');
        // Name a few specific areas for the highest-priority events.
        let mut areas: Vec<&str> = alerts
            .iter()
            .filter(|a| a.event.contains("Tornado") || a.event.contains("Severe"))
            .map(|a| a.area.as_str())
            .filter(|s| !s.is_empty())
            .collect();
        areas.dedup();
        if !areas.is_empty() {
            out.push_str(" Affected: ");
            out.push_str(&areas.into_iter().take(4).collect::<Vec<_>>().join("; "));
            out.push('.');
        }
    }
    let [tor, wind, hail] = reports;
    if tor + wind + hail > 0 {
        out.push_str(&format!(
            " Today's storm reports: {tor} tornado, {wind} wind, {hail} hail."
        ));
    }
    out
}

/// Rewrite the templated facts into friendly prose with Claude. `context` is the templated
/// summary (the ground truth). Returns the model's text, or an error the caller can log.
pub async fn claude(
    http: &reqwest::Client,
    key: &str,
    context: &str,
) -> anyhow::Result<String> {
    let prompt = format!(
        "You are a calm, plain-language weather briefer for the general public. In 2-4 short \
         sentences, explain what these active weather conditions mean and what people in the \
         area should do. Do not invent facts beyond what is given. Facts:\n\n{context}"
    );
    let body = serde_json::json!({
        "model": "claude-haiku-4-5",
        "max_tokens": 400,
        "messages": [{"role": "user", "content": prompt}],
    });
    let resp = http
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .body(body.to_string())
        .send()
        .await?;
    if !resp.status().is_success() {
        anyhow::bail!("Anthropic API error {}", resp.status());
    }
    let text = resp.text().await?;
    let v: serde_json::Value = serde_json::from_str(&text)?;
    let out = v["content"][0]["text"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("unexpected API response shape"))?;
    Ok(out.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templated_summarizes_and_counts() {
        let alerts = vec![
            AlertLine { event: "Tornado Warning".into(), area: "Cleveland Co.".into() },
            AlertLine { event: "Tornado Warning".into(), area: "McClain Co.".into() },
            AlertLine { event: "Severe Thunderstorm Warning".into(), area: "Grady Co.".into() },
        ];
        let s = templated(&alerts, [1, 3, 2]);
        assert!(s.contains("2 Tornado Warning"), "counts events: {s}");
        assert!(s.contains("Cleveland Co."), "names affected areas: {s}");
        assert!(s.contains("1 tornado, 3 wind, 2 hail"), "storm report tally: {s}");
    }

    #[test]
    fn templated_handles_quiet() {
        assert!(templated(&[], [0, 0, 0]).starts_with("No active"));
    }
}
