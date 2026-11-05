//! Builds a query from memory, without the command line driver.
//!
//! `R(a, b, c)` is ternary and `S(c, d)` is binary, so the query has four
//! variables and mixes relation arities. `JoinMachine` reuses one buffer, so
//! the loop allocates nothing per solution.

use wcoj::join::{JoinMachine, build_join};
use wcoj::types::{BuildError, JoinQuery, RelationInput};

fn main() -> Result<(), BuildError> {
    let left: Vec<u32> = vec![0, 1, 5, 0, 2, 6, 0, 3, 7];
    let right: Vec<u32> = vec![5, 7, 6, 8, 9, 1];
    let query: JoinQuery = build_join(
        &[
            RelationInput {
                rows: &left,
                vars: &[0, 1, 2],
            },
            RelationInput {
                rows: &right,
                vars: &[2, 3],
            },
        ],
        4,
    )?;

    let mut machine: JoinMachine<'_> = JoinMachine::new(&query);
    let mut solution: Vec<u32> = Vec::with_capacity(machine.variable_count());
    while machine.next_solution(&mut solution) {
        println!("{solution:?}");
    }
    Ok(())
}
