//! Output format dispatch for clinically-meaningful tools.
//!
//! Researchers using this server in three different roles want three
//! different shapes from the same Garmin endpoint:
//!
//!   * LLM-facing chat   → pretty-printed JSON (current default)
//!   * Statistical batch → CSV that concatenates row-wise across days
//!   * Biosignal analysis → EDF (deferred — see note below)
//!
//! `ClinicalExport` lets a tool produce its data once as a typed payload
//! and pick the serialisation at the boundary, instead of every tool
//! hard-coding `serde_json::to_string_pretty`.
//!
//! EDF is intentionally NOT in this trait yet:
//!
//!   * It is a binary format; MCP returns text, so EDF needs a temp-file
//!     path or base64 wrapper — both are policy decisions worth their
//!     own PR.
//!   * Only timeseries-shaped metrics (HRV readings, raw HR samples,
//!     SpO2, respiration, stress) fit the EDF data model; daily summary
//!     tools must NOT emit EDF.
//!   * The choice of EDF crate (`edfio` vs hand-rolled writer) deserves
//!     its own evaluation.

use rmcp::schemars;
use serde::Deserialize;
use serde_json::{Map, Value};

#[derive(Debug, Default, Deserialize, schemars::JsonSchema, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    #[default]
    Json,
    Csv,
}

pub trait ClinicalExport {
    fn to_json(&self) -> String;
    fn to_csv(&self) -> String;

    fn render(&self, fmt: OutputFormat) -> String {
        match fmt {
            OutputFormat::Json => self.to_json(),
            OutputFormat::Csv => self.to_csv(),
        }
    }
}

/// One-row summary (sleep, stats, training_readiness, daily HR …).
///
/// JSON: pretty-printed object — same shape as the legacy `pluck` output.
/// CSV: header row + value row, suitable for `cat day1.csv day2.csv | …`
/// after stripping later headers.
pub struct FlatSummary {
    pub fields: Map<String, Value>,
}

impl ClinicalExport for FlatSummary {
    fn to_json(&self) -> String {
        serde_json::to_string_pretty(&Value::Object(self.fields.clone()))
            .unwrap_or_else(|e| format!("Error: {e}"))
    }

    fn to_csv(&self) -> String {
        if self.fields.is_empty() {
            return String::new();
        }
        let keys: Vec<&str> = self.fields.keys().map(|k| k.as_str()).collect();
        let values: Vec<String> = self.fields.values().map(value_to_csv_cell).collect();
        format!("{}\n{}\n", keys.join(","), values.join(","))
    }
}

/// HRV — summary block + per-reading time series (5-minute samples).
///
/// JSON: pretty-printed { date, …summary fields…, readings_count,
/// first_reading_gmt, last_reading_gmt }.  Full readings array is omitted
/// to keep LLM tool output compact; researchers who need the raw samples
/// pass `format: csv`.
///
/// CSV: `reading_time_gmt,hrv_value` — one row per 5-minute reading.
pub struct HrvPayload {
    pub date: String,
    pub summary: Map<String, Value>,
    pub readings: Vec<Map<String, Value>>,
}

impl ClinicalExport for HrvPayload {
    fn to_json(&self) -> String {
        let mut out = Map::new();
        out.insert("date".to_string(), Value::String(self.date.clone()));
        for (k, v) in &self.summary {
            out.insert(k.clone(), v.clone());
        }
        out.insert(
            "readings_count".to_string(),
            Value::Number(self.readings.len().into()),
        );
        if let Some(first) = self.readings.first().and_then(|r| r.get("readingTimeGMT")) {
            out.insert("first_reading_gmt".to_string(), first.clone());
        }
        if let Some(last) = self.readings.last().and_then(|r| r.get("readingTimeGMT")) {
            out.insert("last_reading_gmt".to_string(), last.clone());
        }
        serde_json::to_string_pretty(&Value::Object(out))
            .unwrap_or_else(|e| format!("Error: {e}"))
    }

    fn to_csv(&self) -> String {
        let mut out = String::from("reading_time_gmt,hrv_value\n");
        for r in &self.readings {
            let t = r
                .get("readingTimeGMT")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let v = r
                .get("value")
                .map(value_to_csv_cell)
                .unwrap_or_default();
            out.push_str(&csv_quote(t));
            out.push(',');
            out.push_str(&v);
            out.push('\n');
        }
        out
    }
}

/// `[[timestamp_ms, value], …]` — Garmin's universal pattern for stress,
/// body battery, and respiration samples (and possibly more).
///
/// JSON: summary fields + `sample_count` + first/last timestamps for context.
/// CSV: `timestamp_ms,<value_column>` — one row per sample, sentinel values
/// (-1, -2 in stress for "gap" / "activity") preserved so the downstream
/// statistical tool can decide how to handle them.
pub struct TimeseriesArray {
    pub date: String,
    /// Header for the value column; e.g. "body_battery", "stress",
    /// "respiration_bpm".
    pub value_column: &'static str,
    /// Each entry is the raw `[ts, value]` pair from Garmin.  Stored as
    /// (i64, Value) so non-integer values (rare) survive as JSON.
    pub samples: Vec<(i64, Value)>,
    /// Summary fields included in JSON output alongside sample_count.
    pub summary: Map<String, Value>,
}

impl ClinicalExport for TimeseriesArray {
    fn to_json(&self) -> String {
        let mut out = self.summary.clone();
        out.insert("date".to_string(), Value::String(self.date.clone()));
        out.insert(
            "sample_count".to_string(),
            Value::Number(self.samples.len().into()),
        );
        if let Some((ts, _)) = self.samples.first() {
            out.insert("first_sample_ts_ms".to_string(), Value::Number((*ts).into()));
        }
        if let Some((ts, _)) = self.samples.last() {
            out.insert("last_sample_ts_ms".to_string(), Value::Number((*ts).into()));
        }
        serde_json::to_string_pretty(&Value::Object(out))
            .unwrap_or_else(|e| format!("Error: {e}"))
    }

    fn to_csv(&self) -> String {
        let mut out = format!("timestamp_ms,{}\n", self.value_column);
        for (ts, v) in &self.samples {
            out.push_str(&ts.to_string());
            out.push(',');
            out.push_str(&value_to_csv_cell(v));
            out.push('\n');
        }
        out
    }
}

/// Tabular event log over a date range — blood pressure readings,
/// weigh-ins, and similar.  Each row is one measurement event; columns
/// are explicit so CSVs concatenate cleanly across calls.
///
/// JSON: array of objects (legacy compatibility).
/// CSV: header line + one row per event, in `columns` order.
pub struct EventTable {
    /// CSV column order; also serves as the JSON-mode field projection
    /// (any row keys not in `columns` are dropped from CSV but kept in
    /// JSON for fidelity).
    pub columns: Vec<&'static str>,
    pub rows: Vec<Map<String, Value>>,
}

impl ClinicalExport for EventTable {
    fn to_json(&self) -> String {
        let array: Vec<Value> = self.rows.iter().cloned().map(Value::Object).collect();
        serde_json::to_string_pretty(&Value::Array(array))
            .unwrap_or_else(|e| format!("Error: {e}"))
    }

    fn to_csv(&self) -> String {
        let mut out = self.columns.join(",");
        out.push('\n');
        for row in &self.rows {
            let cells: Vec<String> = self
                .columns
                .iter()
                .map(|col| row.get(*col).map(value_to_csv_cell).unwrap_or_default())
                .collect();
            out.push_str(&cells.join(","));
            out.push('\n');
        }
        out
    }
}

fn value_to_csv_cell(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => csv_quote(s),
        other => csv_quote(&other.to_string()),
    }
}

/// RFC 4180 minimal quoting: only quote when the cell contains
/// comma, quote, or newline.
fn csv_quote(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}
