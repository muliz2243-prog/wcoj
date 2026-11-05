//! Leapfrog Triejoin over flat tries, with no external dependencies.
//!
//! Two engines share the same trie layout:
//!
//! - [`join`] answers conjunctive queries over relations of any arity (`K`
//!   columns) and any number of variables (`N` levels), streaming solutions
//!   instead of collecting them.
//! - [`wcoj`] keeps the fixed triangle query used by the comparison report and
//!   acts as the reference implementation.
//! - [`hash_join`] is the binary-join baseline the report measures against.
//!
//! [`types::TrieCursor`] moves forward only, seeking with exponential probes
//! and a binary search, so the hot loop never allocates. [`types::TrieLayout`]
//! stores every level of a relation in one contiguous key buffer and one
//! contiguous offset buffer.
//!
//! # Example
//!
//! A ternary relation `R(a, b, c)` joined with a binary `S(c, d)`:
//!
//! ```
//! use wcoj::join::{build_join, JoinIter};
//! use wcoj::types::{BuildError, RelationInput};
//!
//! fn main() -> Result<(), BuildError> {
//!     let left: Vec<u32> = vec![0, 1, 5, 0, 2, 6];
//!     let right: Vec<u32> = vec![5, 7, 6, 8];
//!     let query = build_join(
//!         &[
//!             RelationInput {
//!                 rows: &left,
//!                 vars: &[0, 1, 2],
//!             },
//!             RelationInput {
//!                 rows: &right,
//!                 vars: &[2, 3],
//!             },
//!         ],
//!         4,
//!     )?;
//!     let solutions: Vec<Vec<u32>> = JoinIter::new(&query).collect();
//!     assert_eq!(solutions, vec![vec![0, 1, 5, 7], vec![0, 2, 6, 8]]);
//!     Ok(())
//! }
//! ```

pub mod hash_join;
pub mod input;
pub mod join;
pub mod report;
pub mod trie;
pub mod types;
pub mod wcoj;
