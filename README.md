# wcoj

[![CI](https://github.com/muliz2243-prog/wcoj/actions/workflows/ci.yml/badge.svg)](https://github.com/muliz2243-prog/wcoj/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

A Worst-Case Optimal Join engine in Rust: Leapfrog Triejoin over flat tries for
relations of any arity and queries with any number of variables. No external
dependencies, no allocation in the join loop.

On the worst case for a binary join, a hub with 4,000 sources and 4,000 sinks,
the hash baseline materializes 16,004,000 intermediate rows (183 MB) and takes
332 ms. The Leapfrog engine returns the same answer in 0.65 ms.

```
shape      nodes     edges   triangles   wcoj ms   hash ms    ratio hash MB materialized
fan         4000      8000           0     0.654   332.459  508.03x   183.1    16004000
```

## Requirements

Rust 1.85 or newer (the crate uses edition 2024). No system libraries, no
build scripts, no dependencies.

## Install

1. `git clone https://github.com/muliz2243-prog/wcoj`
2. `cd wcoj`
3. `cargo test`

## Quickstart

```rust
use wcoj::join::{build_join, JoinIter};
use wcoj::types::{BuildError, RelationInput};

fn main() -> Result<(), BuildError> {
    // R(a, b, c) is ternary; S(c, d) is binary.
    let left: Vec<u32> = vec![0, 1, 5, 0, 2, 6];
    let right: Vec<u32> = vec![5, 7, 6, 8];
    let query = build_join(
        &[
            RelationInput { rows: &left, vars: &[0, 1, 2] },
            RelationInput { rows: &right, vars: &[2, 3] },
        ],
        4,
    )?;
    let solutions: Vec<Vec<u32>> = JoinIter::new(&query).collect();
    assert_eq!(solutions, vec![vec![0, 1, 5, 7], vec![0, 2, 6, 8]]);
    Ok(())
}
```

`JoinMachine::next_solution` writes into a buffer you own, so the hot loop makes
no allocations. `JoinIter` is the `Iterator` wrapper for the common case.

## What it does

- Builds a flat compressed sparse row trie per relation, one `keys` buffer and
  one `offsets` buffer for all levels.
- Runs Leapfrog intersection with exponential seek plus binary search.
- Streams solutions; nothing accumulates in memory.
- Ships the triangle query used by the comparison report and a hash join
  baseline with a memory estimate.

## Command line

```
cargo run --release -- edges.txt
cargo run --release -- data/dense.txt --sample 3
cargo run --release -- data/dense.txt --limit-mb 1
```

The input is one `u v` edge per line. `#` starts a comment, blank lines are
ignored, repeated edges collapse, and errors report the 1-based line number.

## Performance

`cargo bench --bench triangle` runs a dependency-free harness. The table was
measured on Windows 11 with rustc 1.99.1 in release mode, single run per
configuration:

| shape | nodes | edges | solutions | WCOJ | hash join | ratio | hash memory |
|---|---:|---:|---:|---:|---:|---:|---:|
| random | 400 | 12,000 | 27,048 | 7.9 ms | 14.1 ms | 1.77x | 4.1 MB |
| fan | 2,000 | 4,000 | 0 | 0.32 ms | 72.6 ms | 228x | 45.7 MB |
| fan | 4,000 | 8,000 | 0 | 0.65 ms | 332.5 ms | 508x | 183.1 MB |

The fan shape is the case that breaks a hash join: N sources and N sinks make
the intermediate R join S hold N squared rows, and no triangle exists. At
10,000 nodes the hash join refuses to start (1,144.5 MB estimated) while the
Leapfrog engine answers in 1.6 ms.

Only one machine was measured, and only the triangle query. Other cyclic
queries are supported by the generic engine but are not benchmarked.

## Design

The trie layout stores every level of a relation in two flat vectors, so a
cursor is a slice plus two integers and descending a level never translates
indices. The machine is an explicit stack with preallocated cursor and position
buffers, which keeps the code free of recursion and of allocations per step.

The core does not call `unwrap`, `expect` or `panic!`. The hash baseline
requires distinct edges; the file path deduplicates during validation, and
in-memory callers do the same.

## Status

Experimental. The API may change before 1.0. Unit tests, doctests, the example
and the benchmark run on every push.

## License

MIT. See [LICENSE](LICENSE).
