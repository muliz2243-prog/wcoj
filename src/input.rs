//! File, line and graph parsing. This module is the input border: it is the
//! only place allowed to touch the filesystem and to propagate errors with `?`.

use crate::types::{ARITY, Edge, Graph, InputError, RawEdges};

/// Reads the whole input file as text.
pub fn read_file(path: &str) -> Result<String, InputError> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(error) => Err(InputError::Io {
            path: String::from(path),
            reason: error.to_string(),
        }),
    }
}

/// Parses one edge per non-empty line. `#` starts a comment and blank lines are
/// ignored. The number of column checks is fixed because v1 only handles binary
/// relations.
pub fn parse_edges(text: &str) -> Result<RawEdges, InputError> {
    let mut edges: Vec<Edge> = Vec::new();
    for (index, raw_line) in text.lines().enumerate() {
        let line: &str = strip_comment(raw_line);
        if !line.is_empty() {
            let number: u32 = index as u32 + 1;
            edges.push(parse_line(line, number)?);
        }
    }
    Ok(RawEdges { edges })
}

/// Removes the comment tail and the surrounding whitespace.
fn strip_comment(line: &str) -> &str {
    let mut end: usize = line.len();
    if let Some(position) = line.find('#') {
        end = position;
    }
    line[..end].trim()
}

fn parse_line(line: &str, number: u32) -> Result<Edge, InputError> {
    let mut fields = line.split_whitespace();
    let first: &str = match fields.next() {
        Some(text) => text,
        None => {
            return Err(InputError::Parse {
                line: number,
                text: String::from(line),
                reason: String::from("expected two columns"),
            });
        }
    };
    let second: &str = match fields.next() {
        Some(text) => text,
        None => {
            return Err(InputError::Parse {
                line: number,
                text: String::from(line),
                reason: String::from("expected two columns"),
            });
        }
    };
    if fields.next().is_some() {
        return Err(InputError::Parse {
            line: number,
            text: String::from(line),
            reason: String::from("expected exactly two columns"),
        });
    }
    let a: u32 = parse_number(first, line, number)?;
    let b: u32 = parse_number(second, line, number)?;
    Ok([a, b])
}

fn parse_number(field: &str, line: &str, number: u32) -> Result<u32, InputError> {
    match field.parse::<u32>() {
        Ok(value) => Ok(value),
        Err(_) => Err(InputError::Parse {
            line: number,
            text: String::from(line),
            reason: format!("column '{}' is not a u32", field),
        }),
    }
}

/// Sorts and deduplicates the edges, then counts the distinct node ids.
pub fn validate_graph(raw: RawEdges) -> Graph {
    let mut edges: Vec<Edge> = raw.edges;
    edges.sort_unstable();
    edges.dedup();
    let node_count: u64 = count_distinct_nodes(&edges);
    Graph { edges, node_count }
}

fn count_distinct_nodes(edges: &[Edge]) -> u64 {
    let mut nodes: Vec<u32> = Vec::with_capacity(edges.len() * ARITY);
    for edge in edges {
        nodes.push(edge[0]);
        nodes.push(edge[1]);
    }
    nodes.sort_unstable();
    nodes.dedup();
    nodes.len() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ignores_comments_and_blank_lines() {
        let text: &str = "# header\n\n0 1   # first edge\n\t1 2\n";
        let raw: RawEdges = parse_edges(text).unwrap();

        assert_eq!(raw.edges, vec![[0, 1], [1, 2]]);
    }

    #[test]
    fn accepts_a_comment_only_file() {
        let raw: RawEdges = parse_edges("# nothing here\n").unwrap();

        assert!(raw.edges.is_empty());
    }

    #[test]
    fn reports_the_line_of_a_missing_column() {
        let result: Result<RawEdges, InputError> = parse_edges("0 1\n7\n");

        match result {
            Ok(raw) => panic!("expected a parse error, got {} edges", raw.edges.len()),
            Err(InputError::Parse { line, text, reason }) => {
                assert_eq!(line, 2);
                assert_eq!(text, "7");
                assert_eq!(reason, "expected two columns");
            }
            Err(InputError::Io { .. }) => panic!("expected a parse error"),
        }
    }

    #[test]
    fn rejects_a_third_column() {
        let result: Result<RawEdges, InputError> = parse_edges("0 1 2\n");

        assert!(matches!(result, Err(InputError::Parse { .. })));
    }

    #[test]
    fn rejects_a_negative_number() {
        let result: Result<RawEdges, InputError> = parse_edges("0 -1\n");

        assert!(matches!(result, Err(InputError::Parse { .. })));
    }

    #[test]
    fn sorts_deduplicates_and_counts_nodes() {
        let raw: RawEdges = RawEdges {
            edges: vec![[2, 3], [0, 1], [0, 1]],
        };
        let graph: Graph = validate_graph(raw);

        assert_eq!(graph.edges, vec![[0, 1], [2, 3]]);
        assert_eq!(graph.node_count, 4);
    }

    #[test]
    fn reports_a_missing_file() {
        let result: Result<String, InputError> = read_file("wcoj_missing_input_file.txt");

        match result {
            Ok(text) => panic!("expected an io error, got {} bytes", text.len()),
            Err(InputError::Io { path, reason }) => {
                assert_eq!(path, "wcoj_missing_input_file.txt");
                assert!(!reason.is_empty());
            }
            Err(InputError::Parse { .. }) => panic!("expected an io error"),
        }
    }
}
