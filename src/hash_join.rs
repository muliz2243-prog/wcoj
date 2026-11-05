//! Hash join baseline for the triangle query.
//!
//! The baseline materializes the intermediate R join S and then probes T, which
//! is the cost the leapfrog avoids. Both indexes are probed only, never
//! iterated, so the output order comes from the edge list.

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::BuildHasherDefault;

use crate::types::{Edge, HashInput, HashOutput, MemoryError, Triangle};

/// Fixed hasher so the same input probes the same buckets on every run.
type Index = HashMap<u32, Vec<u32>, BuildHasherDefault<DefaultHasher>>;

const TRIANGLE_BYTES: u64 = core::mem::size_of::<Triangle>() as u64;

/// Plans the intermediate result, refuses it when it does not fit, and runs it.
///
/// Edges are assumed validated: sorted, deduplicated and free of malformed
/// tuples.
pub fn run_hash_join(input: HashInput<'_>) -> Result<HashOutput, MemoryError> {
    let plan: HashPlan = plan_hash_join(input.edges);
    if plan.memory_bytes > input.memory_limit_bytes {
        return Err(MemoryError::LimitExceeded {
            estimated_bytes: plan.memory_bytes,
            limit_bytes: input.memory_limit_bytes,
        });
    }
    Ok(execute_plan(input.edges, plan))
}

struct HashPlan {
    r_by_second: Index,
    t_by_second: Index,
    materialized: u64,
    memory_bytes: u64,
}

fn plan_hash_join(edges: &[Edge]) -> HashPlan {
    let r_by_second: Index = index_by_first(edges);
    let t_by_second: Index = index_by_second(edges);
    let materialized: u64 = estimate_materialized(edges, &r_by_second);
    let memory_bytes: u64 = materialized.saturating_mul(TRIANGLE_BYTES);
    HashPlan {
        r_by_second,
        t_by_second,
        materialized,
        memory_bytes,
    }
}

/// Indexes a relation by its first column, so `index[b]` lists every `c` of
/// the tuples `(b, c)`.
fn index_by_first(edges: &[Edge]) -> Index {
    let mut index: Index = Index::default();
    for edge in edges {
        index.entry(edge[0]).or_default().push(edge[1]);
    }
    sort_buckets(&mut index);
    index
}

/// Indexes a relation by its second column, so `index[a]` lists every `c` of
/// the tuples `(c, a)`.
fn index_by_second(edges: &[Edge]) -> Index {
    let mut index: Index = Index::default();
    for edge in edges {
        index.entry(edge[1]).or_default().push(edge[0]);
    }
    sort_buckets(&mut index);
    index
}

/// Sorts every bucket in place. Iteration order does not affect the result
/// because buckets are independent of each other.
fn sort_buckets(index: &mut Index) {
    for values in index.values_mut() {
        values.sort_unstable();
        values.dedup();
    }
}

/// Number of tuples R join S produces, computed before allocating them.
fn estimate_materialized(edges: &[Edge], r_by_second: &Index) -> u64 {
    let mut total: u64 = 0;
    for edge in edges {
        let degree: u64 = match r_by_second.get(&edge[1]) {
            Some(values) => values.len() as u64,
            None => 0,
        };
        total = total.saturating_add(degree);
    }
    total
}

fn execute_plan(edges: &[Edge], plan: HashPlan) -> HashOutput {
    let pairs: Vec<Triangle> = materialize(edges, &plan.r_by_second, plan.materialized);
    let triangles: Vec<Triangle> = filter_by_third(pairs, &plan.t_by_second);
    HashOutput {
        triangles,
        materialized: plan.materialized,
        memory_bytes: plan.memory_bytes,
    }
}

fn materialize(edges: &[Edge], r_by_second: &Index, count: u64) -> Vec<Triangle> {
    let mut pairs: Vec<Triangle> = Vec::with_capacity(count as usize);
    for edge in edges {
        if let Some(values) = r_by_second.get(&edge[1]) {
            extend_with_matches(&mut pairs, edge[0], edge[1], values);
        }
    }
    pairs
}

fn extend_with_matches(pairs: &mut Vec<Triangle>, a: u32, b: u32, values: &[u32]) {
    for c in values {
        pairs.push([a, b, *c]);
    }
}

fn filter_by_third(pairs: Vec<Triangle>, t_by_second: &Index) -> Vec<Triangle> {
    let mut triangles: Vec<Triangle> = Vec::new();
    for pair in pairs {
        if bucket_contains(t_by_second, pair[0], pair[2]) {
            triangles.push(pair);
        }
    }
    triangles
}

fn bucket_contains(index: &Index, key: u32, value: u32) -> bool {
    let mut present: bool = false;
    if let Some(values) = index.get(&key) {
        present = values.binary_search(&value).is_ok();
    }
    present
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(edges: &[Edge], limit: u64) -> Result<HashOutput, MemoryError> {
        run_hash_join(HashInput {
            edges,
            memory_limit_bytes: limit,
        })
    }

    #[test]
    fn finds_the_three_rotations_of_one_cycle() {
        let edges: Vec<Edge> = vec![[0, 1], [1, 2], [2, 0]];
        let output: HashOutput = run(&edges, 1_000_000).unwrap();

        assert_eq!(output.materialized, 3);
        assert_eq!(output.triangles, vec![[0, 1, 2], [1, 2, 0], [2, 0, 1]]);
    }

    #[test]
    fn returns_empty_when_the_cycle_is_open() {
        let edges: Vec<Edge> = vec![[0, 1], [1, 2]];
        let output: HashOutput = run(&edges, 1_000_000).unwrap();

        assert!(output.triangles.is_empty());
    }

    #[test]
    fn returns_empty_for_an_empty_graph() {
        let edges: Vec<Edge> = vec![];
        let output: HashOutput = run(&edges, 1_000_000).unwrap();

        assert!(output.triangles.is_empty());
        assert_eq!(output.materialized, 0);
    }

    #[test]
    fn accepts_triangles_with_repeated_vertices() {
        let edges: Vec<Edge> = vec![[7, 7]];
        let output: HashOutput = run(&edges, 1_000_000).unwrap();

        assert_eq!(output.triangles, vec![[7, 7, 7]]);
    }

    #[test]
    fn refuses_to_materialize_above_the_limit() {
        let edges: Vec<Edge> = vec![[0, 0], [0, 1], [1, 0], [1, 1]];

        match run(&edges, 10) {
            Ok(output) => panic!(
                "expected memory error, got {} triangles",
                output.triangles.len()
            ),
            Err(MemoryError::LimitExceeded {
                estimated_bytes,
                limit_bytes,
            }) => {
                assert_eq!(limit_bytes, 10);
                assert!(estimated_bytes > limit_bytes);
            }
        }
    }
}
