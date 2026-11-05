//! Command line driver. Reads the input, runs both engines and prints the
//! report. Timing lives here because the core never reads the clock.

use std::process::ExitCode;
use std::time::Instant;

use wcoj::hash_join::run_hash_join;
use wcoj::input::{parse_edges, read_file, validate_graph};
use wcoj::report::{compare, format_report};
use wcoj::trie::build_trie;
use wcoj::types::{
    BuildInput, CliOptions, CompareInput, Comparison, FlatTrie, Graph, HashInput, HashOutput,
    InputError, MemoryError, Query, RawEdges, ReportOptions, Triangle,
};
use wcoj::wcoj::{build_query, enumerate_triangles};

const USAGE: &str = "usage: wcoj <edges-file> [--limit-mb N] [--sample N]";
const DEFAULT_LIMIT_MB: u64 = 512;
const DEFAULT_SAMPLE_SIZE: usize = 5;
const DEFAULT_BAR_WIDTH: usize = 40;
const BYTES_PER_MB: u64 = 1024 * 1024;

enum Failure {
    Arguments(String),
    Input(InputError),
    Memory(MemoryError),
}

impl From<InputError> for Failure {
    fn from(error: InputError) -> Self {
        Failure::Input(error)
    }
}

impl From<MemoryError> for Failure {
    fn from(error: MemoryError) -> Self {
        Failure::Memory(error)
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut code: ExitCode = ExitCode::SUCCESS;
    if let Err(failure) = run_cli(&args) {
        eprintln!("{}", describe(&failure));
        code = ExitCode::FAILURE;
    }
    code
}

fn run_cli(args: &[String]) -> Result<(), Failure> {
    let options: CliOptions = parse_args(args).map_err(Failure::Arguments)?;
    run(options)
}

fn run(options: CliOptions) -> Result<(), Failure> {
    let comparison: Comparison = run_pipeline(&options)?;
    let text: String = format_report(&comparison, &options.report);
    print!("{text}");
    Ok(())
}

fn run_pipeline(options: &CliOptions) -> Result<Comparison, Failure> {
    let text: String = read_file(&options.input_path)?;
    let raw: RawEdges = parse_edges(&text)?;
    let graph: Graph = validate_graph(raw);

    let r: FlatTrie = build_trie(BuildInput {
        edges: &graph.edges,
        order: [0, 1],
    });
    let s: FlatTrie = build_trie(BuildInput {
        edges: &graph.edges,
        order: [0, 1],
    });
    let t: FlatTrie = build_trie(BuildInput {
        edges: &graph.edges,
        order: [1, 0],
    });
    let query: Query = build_query([r, s, t]);

    let wcoj_start: Instant = Instant::now();
    let wcoj: Vec<Triangle> = enumerate_triangles(&query);
    let wcoj_nanos: u128 = wcoj_start.elapsed().as_nanos();

    let hash_start: Instant = Instant::now();
    let hash: HashOutput = run_hash_join(HashInput {
        edges: &graph.edges,
        memory_limit_bytes: options.memory_limit_bytes,
    })?;
    let hash_nanos: u128 = hash_start.elapsed().as_nanos();

    Ok(compare(CompareInput {
        wcoj: &wcoj,
        hash: &hash,
        wcoj_nanos,
        hash_nanos,
        edge_count: graph.edges.len() as u64,
        node_count: graph.node_count,
        sample_size: options.report.sample_size,
    }))
}

fn parse_args(args: &[String]) -> Result<CliOptions, String> {
    let mut path: Option<String> = None;
    let mut limit_mb: u64 = DEFAULT_LIMIT_MB;
    let mut sample_size: usize = DEFAULT_SAMPLE_SIZE;
    let mut index: usize = 0;

    while index < args.len() {
        let argument: &str = args[index].as_str();
        if argument == "--limit-mb" {
            let raw: &str = next_value(args, index, "--limit-mb")?;
            limit_mb = parse_count(raw, "--limit-mb")?;
            index += 2;
        } else if argument == "--sample" {
            let raw: &str = next_value(args, index, "--sample")?;
            sample_size = parse_count(raw, "--sample")? as usize;
            index += 2;
        } else if argument.starts_with("--") {
            return Err(format!("unknown option '{argument}'\n{USAGE}"));
        } else if path.is_none() {
            path = Some(String::from(argument));
            index += 1;
        } else {
            return Err(format!("unexpected argument '{argument}'\n{USAGE}"));
        }
    }

    let input_path: String = match path {
        Some(value) => value,
        None => return Err(format!("missing input file\n{USAGE}")),
    };
    Ok(CliOptions {
        input_path,
        memory_limit_bytes: limit_mb.saturating_mul(BYTES_PER_MB),
        report: ReportOptions {
            sample_size,
            bar_width: DEFAULT_BAR_WIDTH,
        },
    })
}

fn next_value<'a>(args: &'a [String], index: usize, option: &str) -> Result<&'a str, String> {
    match args.get(index + 1) {
        Some(value) => Ok(value.as_str()),
        None => Err(format!("option '{option}' needs a value\n{USAGE}")),
    }
}

fn parse_count(raw: &str, option: &str) -> Result<u64, String> {
    match raw.parse::<u64>() {
        Ok(value) => Ok(value),
        Err(_) => Err(format!(
            "option '{option}' expects a non-negative integer, got '{raw}'"
        )),
    }
}

fn describe(failure: &Failure) -> String {
    match failure {
        Failure::Arguments(message) => String::from(message.as_str()),
        Failure::Input(error) => describe_input(error),
        Failure::Memory(error) => describe_memory(error),
    }
}

fn describe_input(error: &InputError) -> String {
    match error {
        InputError::Io { path, reason } => format!("cannot read '{path}': {reason}"),
        InputError::Parse { line, text, reason } => format!("line {line}: {reason} in '{text}'"),
    }
}

fn describe_memory(error: &MemoryError) -> String {
    match error {
        MemoryError::LimitExceeded {
            estimated_bytes,
            limit_bytes,
        } => format!(
            "hash join needs {estimated_bytes} bytes, limit is {limit_bytes}; raise --limit-mb"
        ),
    }
}
