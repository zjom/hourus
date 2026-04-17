use chrono::TimeDelta;
use clap::ValueEnum;
use std::{
    io::{self, Write},
    sync::Arc,
};

/// Supported presentation formats for report output.
#[derive(Debug, Clone, Default, ValueEnum)]
pub enum OutputFormat {
    /// Human-readable text (default)
    #[default]
    Pretty,
    /// Newline-delimited JSON
    Json,
    /// Comma-separated values
    Csv,
    /// Tab-separated values
    Tsv,
}

impl OutputFormat {
    /// Write the total duration only (used by the default command with no subcommand).
    pub fn write_total(&self, w: &mut impl Write, total: TimeDelta) -> io::Result<()> {
        match self {
            OutputFormat::Pretty => writeln!(w, "{}", format_duration(total)),
            OutputFormat::Json => writeln!(w, r#"{{"total_minutes":{}}}"#, total.num_minutes()),
            OutputFormat::Csv => {
                writeln!(w, "total_minutes")?;
                writeln!(w, "{}", total.num_minutes())
            }
            OutputFormat::Tsv => {
                writeln!(w, "total_minutes")?;
                writeln!(w, "{}", total.num_minutes())
            }
        }
    }

    /// Write a per-task breakdown followed by the total.
    pub fn write_breakdown(
        &self,
        w: &mut impl Write,
        summary: &[(Arc<str>, TimeDelta)],
        total: TimeDelta,
    ) -> io::Result<()> {
        match self {
            OutputFormat::Pretty => {
                for (desc, dur) in summary {
                    writeln!(w, "{desc}: {}", format_duration(*dur))?;
                }
                writeln!(w, "{}", format_duration(total))
            }
            OutputFormat::Json => {
                let entries: String = summary
                    .iter()
                    .map(|(desc, dur)| {
                        format!(
                            r#"{{"task":{},"minutes":{}}}"#,
                            serde_json_escape(desc),
                            dur.num_minutes()
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(",");
                writeln!(
                    w,
                    r#"{{"entries":[{entries}],"total_minutes":{}}}"#,
                    total.num_minutes()
                )
            }
            OutputFormat::Csv => {
                writeln!(w, "task,minutes")?;
                for (desc, dur) in summary {
                    writeln!(w, "{},{}", csv_field(desc), dur.num_minutes())?;
                }
                writeln!(w, "TOTAL,{}", total.num_minutes())
            }
            OutputFormat::Tsv => {
                writeln!(w, "task\tminutes")?;
                for (desc, dur) in summary {
                    writeln!(w, "{desc}\t{}", dur.num_minutes())?;
                }
                writeln!(w, "TOTAL\t{}", total.num_minutes())
            }
        }
    }
}

/// Format a `TimeDelta` as a compact human-readable string (e.g. `"2h 30m"`, `"45m"`).
pub fn format_duration(delta: TimeDelta) -> String {
    let total_minutes = delta.num_minutes().abs();
    let hours = total_minutes / 60;
    let minutes = total_minutes % 60;
    let sign = if delta.num_seconds() < 0 { "-" } else { "" };

    match (hours, minutes) {
        (0, m) => format!("{sign}{m}m"),
        (h, 0) => format!("{sign}{h}h"),
        (h, m) => format!("{sign}{h}h {m}m"),
    }
}

/// Wrap a CSV field in quotes if it contains a comma or double-quote, escaping inner quotes.
fn csv_field(s: &str) -> String {
    if s.contains(',') || s.contains('"') {
        format!(r#""{}""#, s.replace('"', r#""""#))
    } else {
        s.to_owned()
    }
}

/// Produce a JSON string literal with proper escaping (no external dependency).
fn serde_json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str(r#"\""#),
            '\\' => out.push_str(r"\\"),
            '\n' => out.push_str(r"\n"),
            '\r' => out.push_str(r"\r"),
            '\t' => out.push_str(r"\t"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeDelta;

    fn mins(n: i64) -> TimeDelta {
        TimeDelta::minutes(n)
    }

    // --- format_duration ---

    #[test]
    fn format_duration_hours_only() {
        assert_eq!(format_duration(mins(120)), "2h");
    }

    #[test]
    fn format_duration_minutes_only() {
        assert_eq!(format_duration(mins(45)), "45m");
    }

    #[test]
    fn format_duration_hours_and_minutes() {
        assert_eq!(format_duration(mins(150)), "2h 30m");
    }

    #[test]
    fn format_duration_zero() {
        assert_eq!(format_duration(mins(0)), "0m");
    }

    #[test]
    fn format_duration_negative() {
        assert_eq!(format_duration(mins(-90)), "-1h 30m");
    }

    // --- OutputFormat::write_total ---

    fn capture_total(fmt: &OutputFormat, total: TimeDelta) -> String {
        let mut buf = Vec::new();
        fmt.write_total(&mut buf, total).unwrap();
        String::from_utf8(buf).unwrap()
    }

    #[test]
    fn write_total_pretty() {
        assert_eq!(capture_total(&OutputFormat::Pretty, mins(90)), "1h 30m\n");
    }

    #[test]
    fn write_total_json() {
        assert_eq!(
            capture_total(&OutputFormat::Json, mins(90)),
            "{\"total_minutes\":90}\n"
        );
    }

    #[test]
    fn write_total_csv() {
        assert_eq!(
            capture_total(&OutputFormat::Csv, mins(90)),
            "total_minutes\n90\n"
        );
    }

    #[test]
    fn write_total_tsv() {
        assert_eq!(
            capture_total(&OutputFormat::Tsv, mins(90)),
            "total_minutes\n90\n"
        );
    }

    // --- OutputFormat::write_breakdown ---

    fn sample_summary() -> Vec<(Arc<str>, TimeDelta)> {
        vec![("coding".into(), mins(120)), ("review".into(), mins(30))]
    }

    fn capture_breakdown(
        fmt: &OutputFormat,
        summary: &[(Arc<str>, TimeDelta)],
        total: TimeDelta,
    ) -> String {
        let mut buf = Vec::new();
        fmt.write_breakdown(&mut buf, summary, total).unwrap();
        String::from_utf8(buf).unwrap()
    }

    #[test]
    fn write_breakdown_pretty() {
        let out = capture_breakdown(&OutputFormat::Pretty, &sample_summary(), mins(150));
        assert_eq!(out, "coding: 2h\nreview: 30m\n2h 30m\n");
    }

    #[test]
    fn write_breakdown_json_structure() {
        let out = capture_breakdown(&OutputFormat::Json, &sample_summary(), mins(150));
        assert!(out.contains(r#""task":"coding""#));
        assert!(out.contains(r#""task":"review""#));
        assert!(out.contains(r#""total_minutes":150"#));
    }

    #[test]
    fn write_breakdown_csv_has_header() {
        let out = capture_breakdown(&OutputFormat::Csv, &sample_summary(), mins(150));
        assert!(out.starts_with("task,minutes\n"));
        assert!(out.contains("coding,120\n"));
        assert!(out.contains("TOTAL,150\n"));
    }

    #[test]
    fn write_breakdown_tsv_has_header() {
        let out = capture_breakdown(&OutputFormat::Tsv, &sample_summary(), mins(150));
        assert!(out.starts_with("task\tminutes\n"));
        assert!(out.contains("coding\t120\n"));
    }

    #[test]
    fn csv_field_with_comma_is_quoted() {
        assert_eq!(csv_field("a,b"), r#""a,b""#);
    }

    #[test]
    fn csv_field_without_special_chars_unchanged() {
        assert_eq!(csv_field("coding"), "coding");
    }
}
