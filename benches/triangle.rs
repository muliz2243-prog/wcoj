//! Heavy benchmark for the generic engine against the hash join baseline.
//!
//! Run with `cargo bench` for the default graph, or pass options:
//!
//! ```text
//! cargo bench --bench triangle -- --nodes 4000 --edges 40000 --runs 3
//! cargo bench --bench triangle -- --shape fan --nodes 5000
//! cargo bench --bench triangle -- --sweep --runs 1
//! ```
//!
//! The `fan` shape is the worst case for a binary join: one hub with `N`
//! incoming and `N` outgoing edges makes the intermediate `R join S` hold
//! `N * N` rows, while no triangle exists. The `random` shape is
//! Erdos-Renyi-like and grows the triangle count instead.
//!
//! Criterion would pull a dependency tree and this crate has none, so the
//! harness times a fixed number of runs and keeps the fastest.

use core::time::Duration;
use std::process::ExitCode;
use std::time::Instant;

use wcoj::hash_join::run_hash_join;
use wcoj::join::{JoinMachine, build_join};
use wcoj::types::{BuildError, Edge, HashInput, HashOutput, JoinQuery, MemoryError, RelationInput};

const DEFAULT_NODES: u32 = 400;
const DEFAULT_EDGES: usize = 12_000;
const DEFAULT_RUNS: usize = 5;
const DEFAULT_SEED: u64 = 7;
const DEFAULT_LIMIT_MB: u64 = 512;
const BYTES_PER_MB: u64 = 1024 * 1024;
const SWEEP_SIZES: [u32; 4] = [500, 1_000, 2_000, 4_000];
const USAGE: &str = "usage: triangle [--nodes N] [--edges M] [--runs R] [--seed S] \
[--limit-mb L] [--shape random|fan] [--sweep]
  random  draws M distinct edges over N nodes
  fan     hub with N sources and N sinks: N^2 intermediate rows, no triangle
  sweep   runs the built-in curve over random and fan shapes";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Shape {
    Random,
    Fan,
}

#[derive(Clone, Copy)]
struct Config {
    nodes: u32,
    edges: usize,
    runs: usize,
    seed: u64,
    limit_mb: u64,
    shape: Shape,
    sweep: bool,
}

#[derive(Clone, Copy)]
struct Measurement {
    count: u64,
    time: Duration,
    materialized: u64,
    memory_bytes: u64,
    over_limit: bool,
}

struct Report {
    shape: Shape,
    nodes: u32,
    edges: usize,
    wcoj: Measurement,
    hash: Measurement,
    matches: bool,
}

struct HashSample {
    count: u64,
    time: Duration,
    materialized: u64,
    memory_bytes: u64,
}

struct HashSummary {
    count: u64,
    time: Duration,
    materialized: u64,
    memory_bytes: u64,
    over_limit: bool,
}

struct Findings {
    mismatched: bool,
    degradation: Option<String>,
    limit: Option<String>,
}

fn main() -> ExitCode {
    // Cargo invokes the bench target with `--bench`; libtest options never
    // reach this binary, so the only extra flag to ignore is that one.
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let mut args: Vec<String> = Vec::with_capacity(raw.len());
    let mut index: usize = 0;
    while index < raw.len() {
        if raw[index] != "--bench" {
            args.push(raw[index].clone());
        }
        index += 1;
    }
    let mut code: ExitCode = ExitCode::SUCCESS;
    if has_flag(&args, "--help") {
        println!("{USAGE}");
    } else {
        code = exit_code(execute(parse_args(&args)));
    }
    code
}

fn has_flag(args: &[String], flag: &str) -> bool {
    let mut found: bool = false;
    let mut index: usize = 0;
    while !found && index < args.len() {
        if args[index] == flag {
            found = true;
        }
        index += 1;
    }
    found
}

fn exit_code(result: Result<bool, String>) -> ExitCode {
    match result {
        Err(message) => fail(&message),
        Ok(true) => ExitCode::FAILURE,
        Ok(false) => ExitCode::SUCCESS,
    }
}

fn fail(message: &str) -> ExitCode {
    eprintln!("{message}");
    ExitCode::FAILURE
}

fn execute(parsed: Result<Config, String>) -> Result<bool, String> {
    match parsed {
        Err(message) => Err(message),
        Ok(config) => run(&config),
    }
}

fn run(config: &Config) -> Result<bool, String> {
    if config.sweep {
        run_sweep(config)
    } else {
        run_single(config)
    }
}

fn run_single(config: &Config) -> Result<bool, String> {
    let edges: Vec<Edge> = generate(config);
    let report: Report = measure(config, &edges)?;
    let mut mismatched: bool = false;
    print_header();
    print_row(&report);
    if report.hash.over_limit {
        println!("{:<9} memory limit reached before measuring", "hash");
    } else {
        println!("{:<9} {}", "match", yes_no(report.matches));
        mismatched = !report.matches;
    }
    Ok(mismatched)
}

fn run_sweep(config: &Config) -> Result<bool, String> {
    let configs: Vec<Config> = sweep_configs(config);
    let mut findings: Findings = Findings {
        mismatched: false,
        degradation: None,
        limit: None,
    };
    let mut failure: Option<String> = None;
    let mut index: usize = 0;
    print_header();
    while failure.is_none() && index < configs.len() {
        let edges: Vec<Edge> = generate(&configs[index]);
        match measure(&configs[index], &edges) {
            Err(message) => failure = Some(message),
            Ok(report) => {
                print_row(&report);
                absorb_report(&mut findings, &report);
            }
        }
        index += 1;
    }
    print_summary(&findings);
    let mut result: Result<bool, String> = Ok(findings.mismatched);
    if let Some(message) = failure {
        result = Err(message);
    }
    result
}

fn sweep_configs(base: &Config) -> Vec<Config> {
    let mut configs: Vec<Config> = Vec::with_capacity(SWEEP_SIZES.len() * 2);
    let mut index: usize = 0;
    while index < SWEEP_SIZES.len() {
        let nodes: u32 = SWEEP_SIZES[index];
        let dense: usize = nodes as usize * 10;
        let fan: usize = nodes as usize * 2;
        configs.push(config_for(base, Shape::Random, nodes, dense));
        configs.push(config_for(base, Shape::Fan, nodes, fan));
        index += 1;
    }
    configs
}

fn config_for(base: &Config, shape: Shape, nodes: u32, edges: usize) -> Config {
    Config {
        nodes,
        edges,
        runs: base.runs,
        seed: base.seed,
        limit_mb: base.limit_mb,
        shape,
        sweep: true,
    }
}

fn absorb_report(findings: &mut Findings, report: &Report) {
    if !report.hash.over_limit && !report.matches {
        findings.mismatched = true;
    }
    if findings.degradation.is_none()
        && !report.hash.over_limit
        && slow_enough(report.wcoj.time, report.hash.time)
    {
        findings.degradation = Some(describe(report));
    }
    if findings.limit.is_none() && report.hash.over_limit {
        findings.limit = Some(describe(report));
    }
}

fn slow_enough(wcoj: Duration, hash: Duration) -> bool {
    let mut slow: bool = false;
    if wcoj.as_nanos() > 0 {
        slow = hash.as_nanos() >= wcoj.as_nanos().saturating_mul(2);
    }
    slow
}

fn describe(report: &Report) -> String {
    format!(
        "shape={} nodes={} edges={}",
        shape_name(report.shape),
        report.nodes,
        report.edges
    )
}

fn print_summary(findings: &Findings) {
    match &findings.limit {
        Some(text) => println!("{:<9} {text}", "memory"),
        None => println!("{:<9} none within the sweep", "memory"),
    }
    match &findings.degradation {
        Some(text) => println!("{:<9} {text}", "degrade"),
        None => println!("{:<9} none within the sweep", "degrade"),
    }
}

fn print_header() {
    println!(
        "{:<8} {:>7} {:>9} {:>11} {:>9} {:>9} {:>8} {:>7} {:>11}",
        "shape",
        "nodes",
        "edges",
        "triangles",
        "wcoj ms",
        "hash ms",
        "ratio",
        "hash MB",
        "materialized"
    );
}

fn print_row(report: &Report) {
    let hash_time: String = if report.hash.over_limit {
        String::from("limit")
    } else {
        format_duration(report.hash.time)
    };
    let ratio: String = if report.hash.over_limit {
        String::from("-")
    } else {
        format_ratio(report.wcoj.time, report.hash.time)
    };
    println!(
        "{:<8} {:>7} {:>9} {:>11} {:>9} {:>9} {:>8} {:>7} {:>11}",
        shape_name(report.shape),
        report.nodes,
        report.edges,
        report.wcoj.count,
        format_duration(report.wcoj.time),
        hash_time,
        ratio,
        format_mb(report.hash.memory_bytes),
        report.hash.materialized
    );
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

fn shape_name(shape: Shape) -> &'static str {
    match shape {
        Shape::Random => "random",
        Shape::Fan => "fan",
    }
}

fn measure(config: &Config, edges: &[Edge]) -> Result<Report, String> {
    let built: Result<JoinQuery, BuildError> = query_from(edges);
    let mut result: Result<Report, String> = Err(String::from("invalid query"));
    if let Ok(query) = built {
        let (wcoj_count, wcoj_time): (u64, Duration) = measure_wcoj(&query, config.runs);
        let hash: Measurement = measure_hash(edges, config);
        let wcoj: Measurement = Measurement {
            count: wcoj_count,
            time: wcoj_time,
            materialized: 0,
            memory_bytes: 0,
            over_limit: false,
        };
        let matches: bool = !hash.over_limit && wcoj.count == hash.count;
        result = Ok(Report {
            shape: config.shape,
            nodes: config.nodes,
            edges: edges.len(),
            wcoj,
            hash,
            matches,
        });
    }
    result
}

/// Deterministic 64-bit linear congruential generator, no dependencies.
struct Lcg {
    state: u64,
}

impl Lcg {
    fn new(seed: u64) -> Lcg {
        Lcg { state: seed }
    }

    fn next(&mut self) -> u32 {
        self.state = self
            .state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.state >> 33) as u32
    }
}

fn generate(config: &Config) -> Vec<Edge> {
    let mut edges: Vec<Edge> = match config.shape {
        Shape::Random => random_edges(config.nodes, config.edges, config.seed),
        Shape::Fan => fan_edges(config.nodes),
    };
    edges.sort_unstable();
    edges.dedup();
    edges
}

/// Draws distinct edges until `target` is reached or the attempt budget ends.
///
/// Duplicates are rejected by sorting and deduplicating after each batch, so
/// the result is deterministic for a given seed and never biased by truncation.
fn random_edges(nodes: u32, target: usize, seed: u64) -> Vec<Edge> {
    let mut generator: Lcg = Lcg::new(seed);
    let mut edges: Vec<Edge> = Vec::with_capacity(target);
    let budget: usize = target.saturating_mul(6);
    let mut attempts: usize = 0;
    while edges.len() < target && attempts < budget {
        let need: usize = target - edges.len();
        let mut drawn: usize = 0;
        while drawn < need && attempts < budget {
            let first: u32 = generator.next() % nodes;
            let second: u32 = generator.next() % nodes;
            if first != second {
                edges.push([first, second]);
            }
            drawn += 1;
            attempts += 1;
        }
        edges.sort_unstable();
        edges.dedup();
    }
    edges
}

/// Hub with `nodes` sources and `nodes` sinks: `nodes^2` intermediate rows.
fn fan_edges(nodes: u32) -> Vec<Edge> {
    let mut edges: Vec<Edge> = Vec::with_capacity(nodes as usize * 2);
    let mut node: u32 = 1;
    while node <= nodes {
        edges.push([node, 0]);
        edges.push([0, node]);
        node += 1;
    }
    edges
}

/// Triangle query over the same edge list: R(a, b), S(b, c), T(c, a).
fn query_from(edges: &[Edge]) -> Result<JoinQuery, BuildError> {
    let mut rows: Vec<u32> = Vec::with_capacity(edges.len() * 2);
    let mut index: usize = 0;
    while index < edges.len() {
        rows.push(edges[index][0]);
        rows.push(edges[index][1]);
        index += 1;
    }
    let query: Result<JoinQuery, BuildError> = build_join(
        &[
            RelationInput {
                rows: &rows,
                vars: &[0, 1],
            },
            RelationInput {
                rows: &rows,
                vars: &[1, 2],
            },
            RelationInput {
                rows: &rows,
                vars: &[2, 0],
            },
        ],
        3,
    );
    query
}

fn measure_wcoj(query: &JoinQuery, runs: usize) -> (u64, Duration) {
    let total: usize = runs.max(1);
    let mut best: Duration = Duration::MAX;
    let mut count: u64 = 0;
    let mut run: usize = 0;
    while run < total {
        let (produced, elapsed): (u64, Duration) = wcoj_once(query);
        count = produced;
        if elapsed < best {
            best = elapsed;
        }
        run += 1;
    }
    (count, best)
}

fn wcoj_once(query: &JoinQuery) -> (u64, Duration) {
    let start: Instant = Instant::now();
    let mut machine: JoinMachine<'_> = JoinMachine::new(query);
    let mut buffer: Vec<u32> = Vec::with_capacity(machine.variable_count());
    let mut count: u64 = 0;
    while machine.next_solution(&mut buffer) {
        count += 1;
    }
    (count, start.elapsed())
}

fn measure_hash(edges: &[Edge], config: &Config) -> Measurement {
    let total: usize = config.runs.max(1);
    let mut summary: HashSummary = HashSummary {
        count: 0,
        time: Duration::MAX,
        materialized: 0,
        memory_bytes: 0,
        over_limit: false,
    };
    let mut run: usize = 0;
    while run < total && !summary.over_limit {
        absorb_hash(&mut summary, hash_once(edges, config.limit_mb));
        run += 1;
    }
    Measurement {
        count: summary.count,
        time: summary.time,
        materialized: summary.materialized,
        memory_bytes: summary.memory_bytes,
        over_limit: summary.over_limit,
    }
}

fn hash_once(edges: &[Edge], limit_mb: u64) -> Result<HashSample, MemoryError> {
    let start: Instant = Instant::now();
    let output: HashOutput = run_hash_join(HashInput {
        edges,
        memory_limit_bytes: limit_mb.saturating_mul(BYTES_PER_MB),
    })?;
    let sample: HashSample = HashSample {
        count: output.triangles.len() as u64,
        time: start.elapsed(),
        materialized: output.materialized,
        memory_bytes: output.memory_bytes,
    };
    Ok(sample)
}

/// Returns true when the memory limit stopped the measurement.
fn absorb_hash(summary: &mut HashSummary, sample: Result<HashSample, MemoryError>) -> bool {
    let mut stop: bool = false;
    match sample {
        Ok(value) => record_hash(summary, &value),
        Err(error) => {
            report_memory(summary, &error);
            stop = true;
        }
    }
    stop
}

fn record_hash(summary: &mut HashSummary, value: &HashSample) {
    summary.count = value.count;
    summary.materialized = value.materialized;
    summary.memory_bytes = value.memory_bytes;
    if value.time < summary.time {
        summary.time = value.time;
    }
}

fn report_memory(summary: &mut HashSummary, error: &MemoryError) {
    match error {
        MemoryError::LimitExceeded {
            estimated_bytes, ..
        } => {
            summary.memory_bytes = *estimated_bytes;
        }
    }
    summary.over_limit = true;
}

fn format_duration(elapsed: Duration) -> String {
    let millis: u128 = elapsed.as_millis();
    let fraction: u32 = elapsed.subsec_micros() % 1000;
    format!("{millis}.{fraction:03}")
}

fn format_ratio(wcoj: Duration, hash: Duration) -> String {
    let mut text: String = String::from("n/a");
    let scaled: Option<u128> = hash
        .as_nanos()
        .saturating_mul(100)
        .checked_div(wcoj.as_nanos());
    if let Some(hundredths) = scaled {
        text = format!("{}.{:02}x", hundredths / 100, hundredths % 100);
    }
    text
}

fn format_mb(bytes: u64) -> String {
    let tenths: u128 = (bytes as u128 * 10) / (BYTES_PER_MB as u128);
    format!("{}.{}", tenths / 10, tenths % 10)
}

fn parse_args(args: &[String]) -> Result<Config, String> {
    let mut config: Config = Config {
        nodes: DEFAULT_NODES,
        edges: DEFAULT_EDGES,
        runs: DEFAULT_RUNS,
        seed: DEFAULT_SEED,
        limit_mb: DEFAULT_LIMIT_MB,
        shape: Shape::Random,
        sweep: false,
    };
    let mut failure: Option<String> = None;
    let mut index: usize = 0;
    while failure.is_none() && index < args.len() {
        if args[index] == "--sweep" {
            config.sweep = true;
            index += 1;
        } else {
            failure = read_option(&mut config, args, index);
            index += 2;
        }
    }
    let mut result: Result<Config, String> = Ok(config);
    if let Some(message) = failure {
        result = Err(message);
    }
    result
}

fn read_option(config: &mut Config, args: &[String], index: usize) -> Option<String> {
    let argument: &str = args[index].as_str();
    match args.get(index + 1) {
        None => Some(format!("option '{argument}' needs a value\n{USAGE}")),
        Some(text) => assign_option(config, argument, text.as_str()),
    }
}

fn assign_option(config: &mut Config, argument: &str, raw: &str) -> Option<String> {
    if argument == "--nodes" {
        assign_u32(&mut config.nodes, raw, argument)
    } else if argument == "--edges" {
        assign_usize(&mut config.edges, raw, argument)
    } else if argument == "--runs" {
        assign_usize(&mut config.runs, raw, argument)
    } else if argument == "--seed" {
        assign_u64(&mut config.seed, raw, argument)
    } else if argument == "--limit-mb" {
        assign_u64(&mut config.limit_mb, raw, argument)
    } else if argument == "--shape" {
        assign_shape(&mut config.shape, raw)
    } else {
        Some(format!("unknown option '{argument}'\n{USAGE}"))
    }
}

fn assign_shape(target: &mut Shape, raw: &str) -> Option<String> {
    let mut failure: Option<String> = None;
    if raw == "random" {
        *target = Shape::Random;
    } else if raw == "fan" {
        *target = Shape::Fan;
    } else {
        failure = Some(format!("unknown shape '{raw}'; expected random or fan"));
    }
    failure
}

fn assign_u32(target: &mut u32, raw: &str, option: &str) -> Option<String> {
    match raw.parse::<u32>() {
        Ok(value) => {
            *target = value;
            None
        }
        Err(_) => Some(invalid(option, raw)),
    }
}

fn assign_usize(target: &mut usize, raw: &str, option: &str) -> Option<String> {
    match raw.parse::<usize>() {
        Ok(value) => {
            *target = value;
            None
        }
        Err(_) => Some(invalid(option, raw)),
    }
}

fn assign_u64(target: &mut u64, raw: &str, option: &str) -> Option<String> {
    match raw.parse::<u64>() {
        Ok(value) => {
            *target = value;
            None
        }
        Err(_) => Some(invalid(option, raw)),
    }
}

fn invalid(option: &str, raw: &str) -> String {
    format!("option '{option}' expects a non-negative integer, got '{raw}'")
}
