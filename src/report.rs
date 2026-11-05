//! Comparison of both engines and text report. Calculation lives in
//! `compare`; formatting never changes a metric.

use crate::types::{CompareInput, Comparison, ReportOptions, Triangle};

const LABEL_EDGES: &str = "edges";
const LABEL_NODES: &str = "nodes";
const LABEL_WCOJ: &str = "wcoj";
const LABEL_HASH: &str = "hash";
const LABEL_MATERIALIZED: &str = "materialized";
const LABEL_MATCH: &str = "match";
const LABEL_SPEEDUP: &str = "speedup";
const LABEL_SAMPLE: &str = "sample";
const LABEL_WCOJ_BAR: &str = "wcoj";
const LABEL_HASH_BAR: &str = "hash";
const TEXT_YES: &str = "yes";
const TEXT_NO: &str = "no";
const TEXT_NONE: &str = "(none)";
const TEXT_UNDEFINED: &str = "n/a";

/// Collects the metrics of both engines. Outputs are compared element by
/// element, not only by count, so a different order also fails the match.
pub fn compare(input: CompareInput<'_>) -> Comparison {
    let wcoj_triangles: u64 = input.wcoj.len() as u64;
    let hash_triangles: u64 = input.hash.triangles.len() as u64;
    let mut sample: Vec<Triangle> = Vec::new();
    let limit: usize = input.sample_size.min(input.wcoj.len());
    let mut index: usize = 0;
    while index < limit {
        sample.push(input.wcoj[index]);
        index += 1;
    }
    Comparison {
        edge_count: input.edge_count,
        node_count: input.node_count,
        wcoj_triangles,
        hash_triangles,
        wcoj_nanos: input.wcoj_nanos,
        hash_nanos: input.hash_nanos,
        hash_materialized: input.hash.materialized,
        hash_memory_bytes: input.hash.memory_bytes,
        results_match: input.wcoj == input.hash.triangles.as_slice(),
        sample,
    }
}

/// Renders the full report. It only formats the values it receives.
pub fn format_report(comparison: &Comparison, options: &ReportOptions) -> String {
    let mut text: String = String::new();
    text.push_str(&format_metrics(comparison));
    text.push_str(&format_sample(comparison));
    text.push_str(&format_bars(comparison, options));
    text
}

fn format_metrics(comparison: &Comparison) -> String {
    let mut text: String = String::new();
    push_line(&mut text, LABEL_EDGES, &comparison.edge_count.to_string());
    push_line(&mut text, LABEL_NODES, &comparison.node_count.to_string());
    push_line(
        &mut text,
        LABEL_WCOJ,
        &format!(
            "{} triangles  {} ns  {}",
            comparison.wcoj_triangles,
            comparison.wcoj_nanos,
            format_millis(comparison.wcoj_nanos)
        ),
    );
    push_line(
        &mut text,
        LABEL_HASH,
        &format!(
            "{} triangles  {} ns  {}",
            comparison.hash_triangles,
            comparison.hash_nanos,
            format_millis(comparison.hash_nanos)
        ),
    );
    push_line(
        &mut text,
        LABEL_MATERIALIZED,
        &format!(
            "{} tuples  {} bytes",
            comparison.hash_materialized, comparison.hash_memory_bytes
        ),
    );
    let match_text: &str = if comparison.results_match {
        TEXT_YES
    } else {
        TEXT_NO
    };
    push_line(&mut text, LABEL_MATCH, match_text);
    push_line(
        &mut text,
        LABEL_SPEEDUP,
        &format_speedup(comparison.wcoj_nanos, comparison.hash_nanos),
    );
    text
}

fn format_sample(comparison: &Comparison) -> String {
    let mut text: String = String::from(LABEL_SAMPLE);
    let mut any: bool = false;
    for triangle in &comparison.sample {
        text.push_str(&format!(
            " ({}, {}, {})",
            triangle[0], triangle[1], triangle[2]
        ));
        any = true;
    }
    if !any {
        text.push_str(TEXT_NONE);
    }
    text.push('\n');
    text
}

fn format_bars(comparison: &Comparison, options: &ReportOptions) -> String {
    let maximum: u128 = comparison.wcoj_nanos.max(comparison.hash_nanos);
    let mut text: String = String::new();
    let wcoj_bar: String = format_bar(comparison.wcoj_nanos, maximum, options.bar_width);
    let hash_bar: String = format_bar(comparison.hash_nanos, maximum, options.bar_width);
    text.push_str(&format!("{:<6} |{}|\n", LABEL_WCOJ_BAR, wcoj_bar));
    text.push_str(&format!("{:<6} |{}|\n", LABEL_HASH_BAR, hash_bar));
    text
}

fn push_line(text: &mut String, label: &str, value: &str) {
    text.push_str(&format!("{:<12} {}\n", label, value));
}

/// Integer milliseconds with three decimals. No float ever touches a metric.
fn format_millis(nanos: u128) -> String {
    let millis: u128 = nanos / 1_000_000;
    let micros: u128 = (nanos % 1_000_000) / 1_000;
    format!("{}.{:03} ms", millis, micros)
}

fn format_speedup(wcoj_nanos: u128, hash_nanos: u128) -> String {
    let mut text: String = String::from(TEXT_UNDEFINED);
    let scaled: Option<u128> = hash_nanos.saturating_mul(100).checked_div(wcoj_nanos);
    if let Some(hundredths) = scaled {
        text = format!("{}.{:02}x", hundredths / 100, hundredths % 100);
    }
    text
}

fn format_bar(value: u128, maximum: u128, width: usize) -> String {
    let mut filled: usize = 0;
    if maximum > 0 && width > 0 {
        let scaled: u128 = value.saturating_mul(width as u128) / maximum;
        filled = scaled.min(width as u128) as usize;
    }
    let mut bar: String = String::with_capacity(width);
    let mut index: usize = 0;
    while index < width {
        if index < filled {
            bar.push('#');
        } else {
            bar.push('.');
        }
        index += 1;
    }
    bar
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{HashOutput, Triangle};

    fn hash_output(triangles: Vec<Triangle>) -> HashOutput {
        HashOutput {
            triangles,
            materialized: 10,
            memory_bytes: 120,
        }
    }

    #[test]
    fn compare_reports_counts_and_sample() {
        let wcoj: Vec<Triangle> = vec![[0, 1, 2], [1, 2, 0]];
        let hash: HashOutput = hash_output(vec![[0, 1, 2], [1, 2, 0]]);
        let comparison: Comparison = compare(CompareInput {
            wcoj: &wcoj,
            hash: &hash,
            wcoj_nanos: 1_000,
            hash_nanos: 5_000,
            edge_count: 3,
            node_count: 3,
            sample_size: 1,
        });

        assert_eq!(comparison.wcoj_triangles, 2);
        assert_eq!(comparison.hash_triangles, 2);
        assert!(comparison.results_match);
        assert_eq!(comparison.sample, vec![[0, 1, 2]]);
        assert_eq!(comparison.hash_materialized, 10);
    }

    #[test]
    fn compare_detects_different_outputs() {
        let wcoj: Vec<Triangle> = vec![[0, 1, 2]];
        let hash: HashOutput = hash_output(vec![[1, 2, 0]]);
        let comparison: Comparison = compare(CompareInput {
            wcoj: &wcoj,
            hash: &hash,
            wcoj_nanos: 1,
            hash_nanos: 1,
            edge_count: 3,
            node_count: 3,
            sample_size: 5,
        });

        assert!(!comparison.results_match);
    }

    #[test]
    fn formats_milliseconds_without_floats() {
        assert_eq!(format_millis(0), "0.000 ms");
        assert_eq!(format_millis(1_234_567), "1.234 ms");
        assert_eq!(format_millis(999_999), "0.999 ms");
    }

    #[test]
    fn formats_speedup_in_hundredths() {
        assert_eq!(format_speedup(1_000_000, 3_710_000), "3.71x");
        assert_eq!(format_speedup(0, 10), TEXT_UNDEFINED);
    }

    #[test]
    fn scales_the_bar_to_the_slowest_engine() {
        assert_eq!(format_bar(50, 100, 10), "#####.....");
        assert_eq!(format_bar(100, 100, 10), "##########");
        assert_eq!(format_bar(0, 0, 4), "....");
        assert_eq!(format_bar(7, 100, 0), "");
    }

    #[test]
    fn report_contains_one_line_per_metric() {
        let comparison: Comparison = Comparison {
            edge_count: 3,
            node_count: 3,
            wcoj_triangles: 1,
            hash_triangles: 1,
            wcoj_nanos: 1_000,
            hash_nanos: 2_000,
            hash_materialized: 5,
            hash_memory_bytes: 60,
            results_match: true,
            sample: vec![[0, 1, 2]],
        };
        let options: ReportOptions = ReportOptions {
            sample_size: 5,
            bar_width: 4,
        };
        let text: String = format_report(&comparison, &options);

        assert!(text.contains("1000 ns"));
        assert!(text.contains("0.001 ms"));
        assert!(text.contains("(0, 1, 2)"));
        assert!(text.contains("2.00x"));
    }
}
