//! Leapfrog intersection and the explicit stack machine of the triangle join.

use core::ops::Range;

use crate::trie::{child_range, cursor_new, cursor_next, leapfrog, level_range};
use crate::types::{
    ARITY, FlatTrie, Frame, LevelCursor, MEMBERS_PER_LEVEL, Machine, Member, Query, QueryLevel,
    RELATIONS, State, Triangle, VARIABLES,
};

/// Fixed triangle query over the three relation tries.
///
/// The member order inside a level is the cursor order, so index 0 is always
/// the leader that `Advance` moves.
pub fn build_query(relations: [FlatTrie; RELATIONS]) -> Query {
    Query {
        levels: [
            QueryLevel {
                members: [member(0, 0), member(2, 0)],
            },
            QueryLevel {
                members: [member(0, 1), member(1, 0)],
            },
            QueryLevel {
                members: [member(1, 1), member(2, 1)],
            },
        ],
        tries: relations,
    }
}

const fn member(relation: u8, depth: u8) -> Member {
    Member { relation, depth }
}

/// Runs the machine until the stack is empty and returns the emitted triangles.
pub fn enumerate_triangles(query: &Query) -> Vec<Triangle> {
    let mut machine: Machine<'_> = create_machine(query);
    let mut running: bool = true;
    while running {
        let next: State = match machine.state {
            State::Search => step_search(&mut machine),
            State::Descend => step_descend(&mut machine),
            State::Advance => step_advance(&mut machine),
            State::Ascend => step_ascend(&mut machine),
            State::Done => State::Done,
        };
        if next == State::Done {
            running = false;
        }
        machine.state = next;
    }
    machine.output
}

fn create_machine(query: &Query) -> Machine<'_> {
    let mut machine: Machine<'_> = Machine {
        query,
        stack: Vec::with_capacity(VARIABLES),
        path: Vec::with_capacity(VARIABLES),
        positions: [[0; ARITY]; RELATIONS],
        output: Vec::new(),
        state: State::Search,
    };
    let root: Frame<'_> = open_frame(query, 0, &machine.positions);
    machine.stack.push(root);
    machine
}

/// Opens one level. The block of each cursor comes from the parent position
/// that `positions` recorded when the previous depth matched.
fn open_frame<'a>(query: &'a Query, level: u8, positions: &[[u32; ARITY]; RELATIONS]) -> Frame<'a> {
    let config: &QueryLevel = &query.levels[usize::from(level)];
    let first: Member = config.members[0];
    let second: Member = config.members[1];
    let first_cursor: LevelCursor<'a> = member_cursor(query, first, positions);
    let second_cursor: LevelCursor<'a> = member_cursor(query, second, positions);
    Frame {
        level,
        members: [first, second],
        cursors: [first_cursor, second_cursor],
    }
}

fn member_cursor<'a>(
    query: &'a Query,
    member: Member,
    positions: &[[u32; ARITY]; RELATIONS],
) -> LevelCursor<'a> {
    let relation: usize = usize::from(member.relation);
    let depth: usize = usize::from(member.depth);
    let trie: &'a FlatTrie = &query.tries[relation];
    let keys: &'a [u32] = &trie.levels[depth].keys;
    let range: Range<u32> = if depth == 0 {
        level_range(&trie.levels[0])
    } else {
        child_range(trie, depth - 1, positions[relation][depth - 1])
    };
    cursor_new(keys, range)
}

fn step_search(machine: &mut Machine<'_>) -> State {
    let top: usize = machine.stack.len() - 1;
    let found: Option<u32> = leapfrog(&mut machine.stack[top].cursors);
    let mut next: State = State::Ascend;
    if let Some(value) = found {
        bind_positions(machine, top);
        machine.path.truncate(top);
        machine.path.push(value);
        next = State::Descend;
    }
    next
}

/// Records the absolute cursor positions of the current level, keyed by
/// relation and depth, so deeper levels can locate their parent block.
fn bind_positions(machine: &mut Machine<'_>, top: usize) {
    let mut index: usize = 0;
    while index < MEMBERS_PER_LEVEL {
        let member: Member = machine.stack[top].members[index];
        let relation: usize = usize::from(member.relation);
        let depth: usize = usize::from(member.depth);
        let position: u32 = machine.stack[top].cursors[index].position;
        machine.positions[relation][depth] = position;
        index += 1;
    }
}

fn step_descend(machine: &mut Machine<'_>) -> State {
    let top: usize = machine.stack.len() - 1;
    let level: u8 = machine.stack[top].level;
    let mut next: State = State::Advance;
    if usize::from(level) + 1 < VARIABLES {
        let frame: Frame<'_> = open_frame(machine.query, level + 1, &machine.positions);
        machine.stack.push(frame);
        next = State::Search;
    } else {
        let triangle: Triangle = [machine.path[0], machine.path[1], machine.path[2]];
        machine.output.push(triangle);
    }
    next
}

fn step_advance(machine: &mut Machine<'_>) -> State {
    let top: usize = machine.stack.len() - 1;
    cursor_next(&mut machine.stack[top].cursors[0]);
    State::Search
}

fn step_ascend(machine: &mut Machine<'_>) -> State {
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
    use crate::types::{BuildInput, Edge};

    fn triangle_query(edges: &[Edge]) -> Query {
        build_query([
            build_trie(BuildInput {
                edges,
                order: [0, 1],
            }),
            build_trie(BuildInput {
                edges,
                order: [0, 1],
            }),
            build_trie(BuildInput {
                edges,
                order: [1, 0],
            }),
        ])
    }

    #[test]
    fn finds_the_three_rotations_of_one_cycle() {
        let query: Query = triangle_query(&[[0, 1], [1, 2], [2, 0]]);

        assert_eq!(
            enumerate_triangles(&query),
            vec![[0, 1, 2], [1, 2, 0], [2, 0, 1]]
        );
    }

    #[test]
    fn returns_empty_when_the_cycle_is_open() {
        let query: Query = triangle_query(&[[0, 1], [1, 2]]);

        assert!(enumerate_triangles(&query).is_empty());
    }

    #[test]
    fn handles_empty_relations() {
        let query: Query = triangle_query(&[]);

        assert!(enumerate_triangles(&query).is_empty());
    }

    #[test]
    fn accepts_triangles_with_repeated_vertices() {
        let query: Query = triangle_query(&[[7, 7]]);

        assert_eq!(enumerate_triangles(&query), vec![[7, 7, 7]]);
    }

    #[test]
    fn leapfrog_skips_until_all_cursors_agree() {
        let left: Vec<u32> = vec![1, 4, 7, 10];
        let right: Vec<u32> = vec![2, 5, 7, 9];
        let mut cursors: [LevelCursor<'_>; 2] = [cursor_new(&left, 0..4), cursor_new(&right, 0..4)];

        assert_eq!(leapfrog(&mut cursors), Some(7));
    }

    #[test]
    fn leapfrog_reports_no_common_value() {
        let left: Vec<u32> = vec![1, 3];
        let right: Vec<u32> = vec![2, 4];
        let mut cursors: [LevelCursor<'_>; 2] = [cursor_new(&left, 0..2), cursor_new(&right, 0..2)];

        assert_eq!(leapfrog(&mut cursors), None);
    }
}
