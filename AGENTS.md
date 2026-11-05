# AGENTS.md — wcoj

Leapfrog Triejoin (WCOJ) vs Hash Join comparison over directed triangles
`R(a,b) ⋈ S(b,c) ⋈ T(c,a)`. Crate `wcoj`, edition 2024, no external dependencies.

## Module map

| File | Responsibility |
|---|---|
| `src/types.rs` | Contracts only: `Edge`, `Triangle`, `FlatTrie`, `LevelCursor`, `Query`, `Machine`, `Graph`, `HashInput/Output`, `InputError`, `MemoryError`, `Comparison`, `CliOptions`, `ReportOptions`. No logic. |
| `src/trie.rs` | Two-level CSR construction (`keys` + `child_offsets`), order/dedup per permutation, monotone cursor, `gallop` + binary search. Includes `build_layout` for K-ary: one `keys` and one `offsets` flat buffer shared by all levels, plus `level_start`/`offset_start`; the merge sort is iterative and closure-free. `leapfrog` and its realignments live here because they operate on `TrieCursor` and serve any K/N. `layout_child_range` takes the parent's absolute position. |
| `src/wcoj.rs` | Fixed query (R=[0,1], S=[0,1], T=[1,0]) and state machine with an explicit stack. Reference engine for the triangle CLI. No recursion. |
| `src/join.rs` | Generic K/N engine: `build_join` (validates, orders attributes by variable, builds one `TrieLayout` per relation) and a machine with preallocated flat buffers. `JoinMachine::next_solution` writes into a reused `Vec`; `JoinIter` implements `Iterator<Item = Vec<u32>>`. |
| `src/hash_join.rs` | Baseline: per-column indexes, memory estimate before materializing, `R⋈S` and filter against T, fixed hasher for reproducibility. |
| `src/input.rs` | Reading, parsing (`u v`, `#` comments, blank lines), validation, ordering/dedup, node counting. |
| `src/report.rs` | `compare` (compares full vectors) and `format_report` (integer times in ns and ms, bars, sample). Does not alter metrics. |
| `src/main.rs` | Driver and argument parsing; the only place that reads the clock (`Instant`) and writes errors to stderr. |

## Validation commands

```
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test
cargo run --release -- edges.txt
cargo run --release -- data/dense.txt --sample 3
cargo run --release -- data/dense.txt --limit-mb 1   # must fail with exit 1
cargo run --example custom_ternary
cargo bench --bench triangle
cargo bench --bench triangle -- --shape fan --nodes 2000
cargo bench --bench triangle -- --sweep
cargo doc --no-deps
```

`cargo test` runs 46 unit tests in `lib`. Tests live at the end of the file they
test and are the only place where `unwrap`/`panic!` are allowed.

## Code rules (LOTA-F, mandatory)

- No recursion, no trait objects, no callbacks/closures in the core, no mutable
  global state, no overloaded operators.
- Core without `unwrap()`, `expect()`, `panic!()` or `?`: errors as values.
  `?` only in edge modules (`main.rs`).
- Explicit types, at most 3 indentation levels, at most 60 lines per function
  and 1000 per file. One visible exit point per function.
- Names and comments in English; technical comments only.

## Fixed decisions (v1)

- Arity 2 only; no generalization to N.
- Directed triangles counted as they appear; no reorienting or deduplicating.
- Timings measure only each engine's execution, not trie construction.
- `hash_memory_bytes` counts only materialized tuples (12 bytes/tuple): it is a
  lower bound on the real index footprint.
- The hash join aborts before reserving if the estimate exceeds `--limit-mb`
  (512 MB by default).
- `HashInput` requires distinct edges: a repeated one multiplies the
  materialized tuples. `validate_graph` deduplicates on the file path; any
  in-memory caller (for example the bench) must deduplicate beforehand.

## Fixtures

- `edges.txt`: two coupled triangles, loose edges, comments, tabs and one
  duplicate edge. Expected: 8 edges, 8 nodes, 6 triangles.
- `data/triangle.txt`: a minimal cycle (3 edges, 3 triangles by rotation).
- `data/graph.txt`: 120 nodes, ~2.7k deterministic random edges.
- `data/dense.txt`: 400 nodes, ~11.5k edges; the `R⋈S` intermediate is around
  333k tuples and is the case where the WCOJ advantage is measured.

## Publishable artifacts

- Public repository: https://github.com/muliz2243-prog/wcoj
- Published crate: https://crates.io/crates/wcoj (0.1.0); docs build on
  https://docs.rs/wcoj. Publish with `CARGO_REGISTRY_TOKEN` read from
  `token_crates.txt` (gitignored); GitHub pushes use `token.txt`.
- `README.md` and `Cargo.toml` metadata (`description`, `readme`, `license`,
  `authors`, `repository`, `homepage`, `documentation`, `keywords`, `categories`).
- CI in `.github/workflows/ci.yml`: fmt, clippy `-D warnings`, tests, docs and the
  default bench on every push and PR.
- `.gitignore` excludes `target/`, `token.txt` and `datos criticos.../`.
- `examples/custom_ternary.rs`: K=3 / N=4 query from memory, no CLI.
- `benches/triangle.rs`: custom harness using `Instant` instead of criterion to
  keep zero dependencies. Accepts `--nodes`, `--edges`, `--runs`, `--seed`,
  `--limit-mb`, `--shape random|fan` and `--sweep`. The `fan` shape is the hash
  join's worst case: `N` sources and `N` sinks produce `N^2` intermediate rows
  and zero triangles. It deduplicates edges before comparing. Cargo passes
  `--bench` to the executable: the parser ignores it.
- `src/lib.rs` documents the API and ships an executable doctest; `cargo doc
  --no-deps` with `RUSTDOCFLAGS='-D warnings'` must stay clean.
