//! Dynamic query builder and streaming machine for relations of any arity.
//!
//! The builder does every allocation: level members, prefix sums and the trie
//! layouts. The machine reuses a flat cursor buffer and a flat position buffer,
//! so a step of the search never allocates.

use core::ops::Range;

use crate::trie::{
    build_layout, cursor_next, layout_child_range, layout_cursor, layout_level, leapfrog,
};
use crate::types::{
    BuildError, JoinLevel, JoinMember, JoinQuery, RelationInput, State, TrieCursor, TrieLayout,
};

/// Builds a query from relations with arbitrary arity.
///
/// Every variable in `0..variables` must be bound by at least one relation, and
/// no relation may repeat a variable. Duplicate queries with no solution are
/// valid; only malformed inputs are rejected.
pub fn build_join(
    relations: &[RelationInput<'_>],
    variables: usize,
) -> Result<JoinQuery, BuildError> {
    let validation: Result<(), BuildError> = validate(relations, variables);
    let result: Result<JoinQuery, BuildError> = match validation {
        Err(error) => Err(error),
        Ok(()) => assemble(relations, variables),
    };
    result
}

fn validate(relations: &[RelationInput<'_>], variables: usize) -> Result<(), BuildError> {
    let mut result: Result<(), BuildError> = Ok(());
    if relations.is_empty() || variables == 0 {
        result = Err(BuildError::NoRelations);
    }
    let mut index: usize = 0;
    while result.is_ok() && index < relations.len() {
        result = check_relation(index, relations[index], variables);
        index += 1;
    }
    result
}

fn check_relation(
    index: usize,
    input: RelationInput<'_>,
    variables: usize,
) -> Result<(), BuildError> {
    let width: usize = input.vars.len();
    let result: Result<(), BuildError> = if width == 0 {
        Err(BuildError::EmptyRelation { relation: index })
    } else if width > u8::MAX as usize {
        Err(BuildError::TooManyColumns { relation: index })
    } else if !input.rows.len().is_multiple_of(width) {
        Err(BuildError::RaggedRows { relation: index })
    } else {
        check_variables(index, input.vars, variables)
    };
    result
}

fn check_variables(index: usize, vars: &[u32], variables: usize) -> Result<(), BuildError> {
    let mut result: Result<(), BuildError> = Ok(());
    let mut position: usize = 0;
    while result.is_ok() && position < vars.len() {
        let variable: u32 = vars[position];
        if variable as usize >= variables {
            result = Err(BuildError::UnknownVariable { variable });
        } else if repeated_before(vars, position) {
            result = Err(BuildError::DuplicateVariable { relation: index });
        }
        position += 1;
    }
    result
}

fn repeated_before(vars: &[u32], position: usize) -> bool {
    let mut earlier: usize = 0;
    let mut repeated: bool = false;
    while !repeated && earlier < position {
        if vars[earlier] == vars[position] {
            repeated = true;
        }
        earlier += 1;
    }
    repeated
}

fn assemble(relations: &[RelationInput<'_>], variables: usize) -> Result<JoinQuery, BuildError> {
    let mut tries: Vec<TrieLayout> = Vec::with_capacity(relations.len());
    let mut tables: Vec<Vec<Option<u32>>> = Vec::with_capacity(relations.len());
    let mut widths: Vec<usize> = Vec::with_capacity(relations.len());
    let mut index: usize = 0;
    while index < relations.len() {
        let width: usize = relations[index].vars.len();
        let order: Vec<u8> = sorted_attributes(relations[index].vars);
        let trie: TrieLayout = build_layout(relations[index].rows, width, &order);
        tries.push(trie);
        tables.push(depth_table(relations[index].vars, &order, variables));
        widths.push(width);
        index += 1;
    }
    let built: Result<Vec<JoinLevel>, BuildError> = build_levels(&tables, variables);
    let result: Result<JoinQuery, BuildError> = match built {
        Err(error) => Err(error),
        Ok(levels) => {
            let cursor_base: Vec<u32> = cursor_prefix(&levels);
            let positions_base: Vec<u32> = width_prefix(&widths);
            Ok(JoinQuery {
                variables,
                levels,
                tries,
                cursor_base,
                positions_base,
            })
        }
    };
    result
}

/// Attribute indexes ordered by their global variable.
fn sorted_attributes(vars: &[u32]) -> Vec<u8> {
    let mut order: Vec<u8> = Vec::with_capacity(vars.len());
    let mut position: usize = 0;
    while position < vars.len() {
        order.push(position as u8);
        position += 1;
    }
    let mut current: usize = 1;
    while current < order.len() {
        insert_sorted(&mut order, vars, current);
        current += 1;
    }
    order
}

fn insert_sorted(order: &mut [u8], vars: &[u32], current: usize) {
    let attribute: u8 = order[current];
    let variable: u32 = vars[usize::from(attribute)];
    let mut slot: usize = current;
    while slot > 0 && vars[usize::from(order[slot - 1])] > variable {
        order[slot] = order[slot - 1];
        slot -= 1;
    }
    order[slot] = attribute;
}

/// Trie depth of every global variable, `None` when the relation lacks it.
fn depth_table(vars: &[u32], order: &[u8], variables: usize) -> Vec<Option<u32>> {
    let mut table: Vec<Option<u32>> = vec![None; variables];
    let mut depth: usize = 0;
    while depth < order.len() {
        let variable: usize = vars[usize::from(order[depth])] as usize;
        table[variable] = Some(depth as u32);
        depth += 1;
    }
    table
}

fn build_levels(
    tables: &[Vec<Option<u32>>],
    variables: usize,
) -> Result<Vec<JoinLevel>, BuildError> {
    let mut levels: Vec<JoinLevel> = Vec::with_capacity(variables);
    let mut failure: Option<BuildError> = None;
    let mut variable: usize = 0;
    while failure.is_none() && variable < variables {
        let members: Vec<JoinMember> = collect_members(tables, variable);
        if members.is_empty() {
            failure = Some(BuildError::UnusedVariable {
                variable: variable as u32,
            });
        } else {
            levels.push(JoinLevel { members });
        }
        variable += 1;
    }
    let mut result: Result<Vec<JoinLevel>, BuildError> = Ok(levels);
    if let Some(error) = failure {
        result = Err(error);
    }
    result
}

fn collect_members(tables: &[Vec<Option<u32>>], variable: usize) -> Vec<JoinMember> {
    let mut members: Vec<JoinMember> = Vec::new();
    let mut index: usize = 0;
    while index < tables.len() {
        push_member(&mut members, index, tables[index][variable]);
        index += 1;
    }
    members
}

fn push_member(members: &mut Vec<JoinMember>, relation: usize, depth: Option<u32>) {
    if let Some(value) = depth {
        members.push(JoinMember {
            relation: relation as u32,
            depth: value,
        });
    }
}

fn cursor_prefix(levels: &[JoinLevel]) -> Vec<u32> {
    let mut base: Vec<u32> = Vec::with_capacity(levels.len() + 1);
    base.push(0);
    let mut index: usize = 0;
    while index < levels.len() {
        base.push(base[index] + levels[index].members.len() as u32);
        index += 1;
    }
    base
}

fn width_prefix(widths: &[usize]) -> Vec<u32> {
    let mut base: Vec<u32> = Vec::with_capacity(widths.len() + 1);
    base.push(0);
    let mut index: usize = 0;
    while index < widths.len() {
        base.push(base[index] + widths[index] as u32);
        index += 1;
    }
    base
}

/// Streaming state machine. `next_solution` advances until one binding is
/// complete, so memory stays flat no matter how many solutions exist.
pub struct JoinMachine<'a> {
    query: &'a JoinQuery,
    /// Levels currently open, bottom first.
    stack: Vec<u32>,
    /// One slot per member of every level, filled when the level opens.
    cursors: Vec<TrieCursor<'a>>,
    /// Values bound for the levels below the top, ordered by level.
    path: Vec<u32>,
    /// Absolute index of relation `r` at depth `d`, laid out by
    /// `positions_base`, for the last matched prefix.
    positions: Vec<u32>,
    state: State,
    /// Set when the last level matched; consumed by `next_solution`.
    emit: bool,
    done: bool,
}

impl<'a> JoinMachine<'a> {
    /// Prepares the buffers from `query` and opens the root level. Nothing is
    /// allocated after this call.
    pub fn new(query: &'a JoinQuery) -> JoinMachine<'a> {
        let slots: usize = last_slot(&query.cursor_base);
        let mut cursors: Vec<TrieCursor<'a>> = Vec::with_capacity(slots);
        let mut index: usize = 0;
        while index < slots {
            cursors.push(TrieCursor {
                keys: &[],
                block_end: 0,
                position: 0,
            });
            index += 1;
        }
        let mut machine: JoinMachine<'a> = JoinMachine {
            query,
            stack: Vec::with_capacity(query.variables),
            cursors,
            path: Vec::with_capacity(query.variables),
            positions: vec![0; last_slot(&query.positions_base)],
            state: State::Search,
            emit: false,
            done: false,
        };
        open_level(&mut machine, 0);
        machine.stack.push(0);
        machine
    }

    /// Writes the next solution into `out` and returns true, or returns false
    /// when no solution is left.
    pub fn next_solution(&mut self, out: &mut Vec<u32>) -> bool {
        let mut ready: bool = false;
        let mut running: bool = !self.done;
        while running {
            let current: State = self.state;
            let next: State = match current {
                State::Search => step_search(self),
                State::Descend => step_descend(self),
                State::Advance => step_advance(self),
                State::Ascend => step_ascend(self),
                State::Done => State::Done,
            };
            self.state = next;
            if next == State::Done {
                self.done = true;
                running = false;
            }
            if self.emit {
                self.emit = false;
                ready = true;
                running = false;
            }
        }
        if ready {
            out.clear();
            out.extend_from_slice(&self.path);
        }
        ready
    }

    /// Number of variables bound by each solution.
    pub fn variable_count(&self) -> usize {
        self.query.variables
    }
}

/// Iterator adapter over the machine. It allocates one `Vec` per solution;
/// `next_solution` with a reused buffer avoids even that.
pub struct JoinIter<'a> {
    machine: JoinMachine<'a>,
}

impl<'a> JoinIter<'a> {
    /// Wraps a fresh machine, so the iterator starts at the first solution.
    pub fn new(query: &'a JoinQuery) -> JoinIter<'a> {
        JoinIter {
            machine: JoinMachine::new(query),
        }
    }

    /// Same contract as [`JoinMachine::next_solution`], for callers that want
    /// to reuse a buffer instead of taking the `Iterator` allocation.
    pub fn next_solution(&mut self, out: &mut Vec<u32>) -> bool {
        self.machine.next_solution(out)
    }
}

impl<'a> Iterator for JoinIter<'a> {
    type Item = Vec<u32>;

    fn next(&mut self) -> Option<Vec<u32>> {
        let mut out: Vec<u32> = Vec::with_capacity(self.machine.variable_count());
        let mut result: Option<Vec<u32>> = None;
        if self.machine.next_solution(&mut out) {
            result = Some(out);
        }
        result
    }
}

fn last_slot(base: &[u32]) -> usize {
    let mut last: u32 = 0;
    if let Some(value) = base.last() {
        last = *value;
    }
    last as usize
}

fn open_level<'a>(machine: &mut JoinMachine<'a>, level: usize) {
    let query: &'a JoinQuery = machine.query;
    let base: usize = query.cursor_base[level] as usize;
    let members: &[JoinMember] = &query.levels[level].members;
    let mut index: usize = 0;
    while index < members.len() {
        let member: JoinMember = members[index];
        machine.cursors[base + index] = member_cursor(query, member, &machine.positions);
        index += 1;
    }
}

/// Opens the cursor of one member. Depth 0 spans the whole root level; deeper
/// members read the parent position recorded when the previous depth matched.
fn member_cursor<'a>(
    query: &'a JoinQuery,
    member: JoinMember,
    positions: &[u32],
) -> TrieCursor<'a> {
    let relation: usize = member.relation as usize;
    let depth: usize = member.depth as usize;
    let trie: &'a TrieLayout = &query.tries[relation];
    let mut range: Range<u32> = layout_level(trie, 0);
    if depth > 0 {
        let base: usize = query.positions_base[relation] as usize;
        range = layout_child_range(trie, depth - 1, positions[base + depth - 1]);
    }
    layout_cursor(trie, range)
}

fn step_search(machine: &mut JoinMachine<'_>) -> State {
    let top: usize = machine.stack.len() - 1;
    let level: usize = machine.stack[top] as usize;
    let query: &JoinQuery = machine.query;
    let span: Range<usize> =
        (query.cursor_base[level] as usize)..(query.cursor_base[level + 1] as usize);
    let found: Option<u32> = leapfrog(&mut machine.cursors[span]);
    let mut next: State = State::Ascend;
    if let Some(value) = found {
        bind_positions(machine, level);
        machine.path.truncate(top);
        machine.path.push(value);
        next = State::Descend;
    }
    next
}

/// Records the absolute cursor positions of the current level, keyed by
/// relation and depth, so deeper levels can locate their parent block.
fn bind_positions(machine: &mut JoinMachine<'_>, level: usize) {
    let query: &JoinQuery = machine.query;
    let base: usize = query.cursor_base[level] as usize;
    let members: &[JoinMember] = &query.levels[level].members;
    let mut index: usize = 0;
    while index < members.len() {
        let member: JoinMember = members[index];
        let relation: usize = member.relation as usize;
        let depth: usize = member.depth as usize;
        let position: u32 = machine.cursors[base + index].position;
        machine.positions[query.positions_base[relation] as usize + depth] = position;
        index += 1;
    }
}

fn step_descend(machine: &mut JoinMachine<'_>) -> State {
    let top: usize = machine.stack.len() - 1;
    let level: usize = machine.stack[top] as usize;
    let mut next: State = State::Advance;
    if level + 1 < machine.query.variables {
        open_level(machine, level + 1);
        machine.stack.push((level + 1) as u32);
        next = State::Search;
    } else {
        machine.emit = true;
    }
    next
}

fn step_advance(machine: &mut JoinMachine<'_>) -> State {
    let top: usize = machine.stack.len() - 1;
    let level: usize = machine.stack[top] as usize;
    let base: usize = machine.query.cursor_base[level] as usize;
    cursor_next(&mut machine.cursors[base]);
    State::Search
}

fn step_ascend(machine: &mut JoinMachine<'_>) -> State {
    machine.stack.pop();
    machine.path.pop();
    let mut next: State = State::Advance;
    if machine.stack.is_empty() {
        next = State::Done;
    }
    next
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trie::build_trie;
    use crate::types::{BuildInput, Edge, Query, Triangle};
    use crate::wcoj::{build_query, enumerate_triangles};

    fn triangle_edges() -> Vec<u32> {
        vec![0, 1, 1, 2, 2, 0]
    }

    fn triangle_query(rows: &[u32]) -> Result<JoinQuery, BuildError> {
        build_join(
            &[
                RelationInput {
                    rows,
                    vars: &[0, 1],
                },
                RelationInput {
                    rows,
                    vars: &[0, 1],
                },
                RelationInput {
                    rows,
                    vars: &[2, 0],
                },
            ],
            3,
        )
    }

    fn fixed_triangle() -> Query {
        let edges: Vec<Edge> = vec![[0, 1], [1, 2], [2, 0]];
        build_query([
            build_trie(BuildInput {
                edges: &edges,
                order: [0, 1],
            }),
            build_trie(BuildInput {
                edges: &edges,
                order: [0, 1],
            }),
            build_trie(BuildInput {
                edges: &edges,
                order: [1, 0],
            }),
        ])
    }

    #[test]
    fn matches_the_fixed_triangle_engine() {
        let rows: Vec<u32> = triangle_edges();
        let built: Result<JoinQuery, BuildError> = triangle_query(&rows);
        let query: JoinQuery = match built {
            Ok(value) => value,
            Err(_) => panic!("expected a valid query"),
        };
        let fixed: Query = fixed_triangle();
        let expected: Vec<Triangle> = enumerate_triangles(&fixed);
        let mut expected_rows: Vec<Vec<u32>> = Vec::new();
        for triangle in &expected {
            expected_rows.push(triangle.to_vec());
        }
        let produced: Vec<Vec<u32>> = JoinIter::new(&query).collect();

        assert_eq!(produced, expected_rows);
        assert_eq!(produced, vec![vec![0, 1, 2], vec![1, 2, 0], vec![2, 0, 1]]);
    }

    #[test]
    fn enumerates_every_tuple_of_a_single_ternary_relation() {
        let rows: Vec<u32> = vec![1, 2, 3, 1, 2, 4, 2, 5, 6];
        let built: Result<JoinQuery, BuildError> = build_join(
            &[RelationInput {
                rows: &rows,
                vars: &[0, 1, 2],
            }],
            3,
        );
        let query: JoinQuery = match built {
            Ok(value) => value,
            Err(_) => panic!("expected a valid query"),
        };
        let produced: Vec<Vec<u32>> = JoinIter::new(&query).collect();

        assert_eq!(produced, vec![vec![1, 2, 3], vec![1, 2, 4], vec![2, 5, 6]]);
    }

    #[test]
    fn joins_four_variables_with_four_relations() {
        let first: Vec<u32> = vec![0, 1];
        let second: Vec<u32> = vec![1, 2];
        let third: Vec<u32> = vec![2, 3];
        let fourth: Vec<u32> = vec![3, 0];
        let built: Result<JoinQuery, BuildError> = build_join(
            &[
                RelationInput {
                    rows: &first,
                    vars: &[0, 1],
                },
                RelationInput {
                    rows: &second,
                    vars: &[1, 2],
                },
                RelationInput {
                    rows: &third,
                    vars: &[2, 3],
                },
                RelationInput {
                    rows: &fourth,
                    vars: &[3, 0],
                },
            ],
            4,
        );
        let query: JoinQuery = match built {
            Ok(value) => value,
            Err(_) => panic!("expected a valid query"),
        };
        let produced: Vec<Vec<u32>> = JoinIter::new(&query).collect();

        assert_eq!(produced, vec![vec![0, 1, 2, 3]]);
    }

    #[test]
    fn streams_with_a_reused_buffer() {
        let rows: Vec<u32> = triangle_edges();
        let built: Result<JoinQuery, BuildError> = triangle_query(&rows);
        let query: JoinQuery = match built {
            Ok(value) => value,
            Err(_) => panic!("expected a valid query"),
        };
        let mut machine: JoinMachine<'_> = JoinMachine::new(&query);
        let mut buffer: Vec<u32> = Vec::new();

        assert!(machine.next_solution(&mut buffer));
        assert_eq!(buffer, vec![0, 1, 2]);
        assert!(machine.next_solution(&mut buffer));
        assert_eq!(buffer, vec![1, 2, 0]);
        assert!(machine.next_solution(&mut buffer));
        assert_eq!(buffer, vec![2, 0, 1]);
        assert!(!machine.next_solution(&mut buffer));
    }

    #[test]
    fn rejects_an_empty_query() {
        assert!(matches!(build_join(&[], 3), Err(BuildError::NoRelations)));
    }

    #[test]
    fn rejects_a_variable_that_no_relation_binds() {
        let rows: Vec<u32> = vec![1, 2];
        let built: Result<JoinQuery, BuildError> = build_join(
            &[RelationInput {
                rows: &rows,
                vars: &[0, 1],
            }],
            3,
        );

        assert!(matches!(
            built,
            Err(BuildError::UnusedVariable { variable: 2 })
        ));
    }

    #[test]
    fn rejects_ragged_rows() {
        let rows: Vec<u32> = vec![1, 2, 3];
        let built: Result<JoinQuery, BuildError> = build_join(
            &[RelationInput {
                rows: &rows,
                vars: &[0, 1],
            }],
            2,
        );

        assert!(matches!(built, Err(BuildError::RaggedRows { .. })));
    }

    #[test]
    fn rejects_an_unknown_variable() {
        let rows: Vec<u32> = vec![1, 2];
        let built: Result<JoinQuery, BuildError> = build_join(
            &[RelationInput {
                rows: &rows,
                vars: &[0, 3],
            }],
            3,
        );

        assert!(matches!(
            built,
            Err(BuildError::UnknownVariable { variable: 3 })
        ));
    }

    #[test]
    fn rejects_a_repeated_variable() {
        let rows: Vec<u32> = vec![1, 2];
        let built: Result<JoinQuery, BuildError> = build_join(
            &[RelationInput {
                rows: &rows,
                vars: &[0, 0],
            }],
            2,
        );

        assert!(matches!(built, Err(BuildError::DuplicateVariable { .. })));
    }

    #[test]
    fn rejects_a_relation_without_attributes() {
        let rows: Vec<u32> = Vec::new();
        let built: Result<JoinQuery, BuildError> = build_join(
            &[RelationInput {
                rows: &rows,
                vars: &[],
            }],
            2,
        );

        assert!(matches!(built, Err(BuildError::EmptyRelation { .. })));
    }
}
