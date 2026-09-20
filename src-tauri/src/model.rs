use serde::{Deserialize, Serialize};
use serde_json::Value;

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageWindow {
    pub id: String,
    pub label: String,
    pub used_percent: Option<f64>,
    pub resets_at: Option<i64>,
    pub duration_mins: Option<i64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub provider: String,
    pub state: String,
    pub account_id: Option<String>,
    pub account_label: Option<String>,
    pub windows: Vec<UsageWindow>,
    pub observed_at: Option<i64>,
    pub source: String,
    pub message: Option<String>,
}
impl Snapshot {
    pub fn empty(provider: &str, state: &str, message: &str) -> Self {
        Self {
            provider: provider.into(),
            state: state.into(),
            account_id: None,
            account_label: None,
            windows: vec![],
            observed_at: None,
            source: if provider == "codex" {
                "Codex app server"
            } else {
                "Claude Code status line"
            }
            .into(),
            message: Some(message.into()),
        }
    }
}
pub fn percentage(v: &Value) -> Option<f64> {
    v.as_f64()
        .filter(|x| x.is_finite())
        .map(|x| x.clamp(0.0, 100.0))
}
pub fn remaining(w: &UsageWindow) -> Option<f64> {
    w.used_percent.map(|n| (100.0 - n).clamp(0.0, 100.0))
}
pub fn fresh(s: &Snapshot, t: i64) -> bool {
    let max_age = if s.source == "Claude Desktop usage cache" {
        // Claude Desktop's own probe normally records a sample every 12–15 minutes.
        1200
    } else {
        600
    };
    s.state == "ready" && s.observed_at.is_some_and(|at| t >= at && t - at < max_age)
}
pub fn alert_threshold(w: &UsageWindow, t: i64) -> Option<i64> {
    if w.resets_at.is_none_or(|reset| reset <= t) {
        return None;
    }
    match remaining(w) {
        Some(n) if n <= 10.0 => Some(10),
        Some(n) if n <= 20.0 => Some(20),
        _ => None,
    }
}
pub fn codex_windows(v: &Value) -> Vec<UsageWindow> {
    let mut out = vec![];
    let buckets: Vec<(String, &Value)> =
        match v.get("rateLimitsByLimitId").and_then(Value::as_object) {
            Some(map) if !map.is_empty() => map.iter().map(|(k, v)| (k.clone(), v)).collect(),
            _ => v
                .get("rateLimits")
                .filter(|v| v.is_object())
                .map(|b| vec![(b["limitId"].as_str().unwrap_or("codex").into(), b)])
                .unwrap_or_default(),
        };
    for (id, b) in buckets {
        for name in ["primary", "secondary"] {
            if let Some(w) = b.get(name).filter(|v| v.is_object()) {
                let mins = w["windowDurationMins"].as_i64();
                let duration = match mins {
                    Some(300) => "5-hour window".into(),
                    Some(10080) => "Weekly window".into(),
                    Some(m) if m % 60 == 0 => format!("{}-hour window", m / 60),
                    Some(m) => format!("{m}-minute window"),
                    _ => format!("{} window", name),
                };
                let prefix = b["limitName"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .or_else(|| {
                        if id != "codex" {
                            Some(id.as_str())
                        } else {
                            None
                        }
                    });
                out.push(UsageWindow {
                    id: format!("{id}:{name}"),
                    label: prefix
                        .map(|p| format!("{p} · {duration}"))
                        .unwrap_or(duration),
                    used_percent: percentage(&w["usedPercent"]),
                    resets_at: w["resetsAt"].as_i64(),
                    duration_mins: mins,
                });
            }
        }
    }
    out
}
pub fn claude_windows(v: &Value) -> Vec<UsageWindow> {
    [
        ("five_hour", "5-hour window", 300),
        ("seven_day", "Weekly window", 10080),
    ]
    .into_iter()
    .filter_map(|(id, label, mins)| {
        let w = v.get(id)?.as_object()?;
        Some(UsageWindow {
            id: id.into(),
            label: label.into(),
            used_percent: w.get("used_percentage").and_then(percentage),
            resets_at: w.get("resets_at").and_then(Value::as_i64),
            duration_mins: Some(mins),
        })
    })
    .collect()
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub theme: String,
    pub show_used: bool,
    pub alerts: bool,
    pub codex_enabled: bool,
    pub claude_enabled: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: "system".into(),
            show_used: false,
            alerts: false,
            codex_enabled: true,
            claude_enabled: false,
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn missing_is_not_zero() {
        assert_eq!(percentage(&Value::Null), None);
        assert_eq!(percentage(&json!(140)), Some(100.));
        assert_eq!(
            codex_windows(&json!({"rateLimits":{"primary":null}})).len(),
            0
        );
    }
    #[test]
    fn windows_are_dynamic() {
        let w = codex_windows(
            &json!({"rateLimitsByLimitId":{"custom":{"primary":{"usedPercent":25,"windowDurationMins":60}}}}),
        );
        assert_eq!(w[0].id, "custom:primary");
        assert_eq!(remaining(&w[0]), Some(75.));
    }
    #[test]
    fn absent_claude_window() {
        assert_eq!(
            claude_windows(&json!({"five_hour":{"used_percentage":20}})).len(),
            1
        );
    }
    #[test]
    fn reset_and_stale_boundaries() {
        let mut s = Snapshot::empty("codex", "ready", "");
        s.observed_at = Some(1000);
        assert!(fresh(&s, 1599));
        assert!(!fresh(&s, 1600));
        assert!(!fresh(&s, 999));
        s.source = "Claude Desktop usage cache".into();
        assert!(fresh(&s, 2199));
        assert!(!fresh(&s, 2200));
        let w = UsageWindow {
            id: "x".into(),
            label: "x".into(),
            used_percent: Some(90.),
            resets_at: Some(2000),
            duration_mins: None,
        };
        assert_eq!(alert_threshold(&w, 1999), Some(10));
        assert_eq!(alert_threshold(&w, 2000), None);
    }
}
