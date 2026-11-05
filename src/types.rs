//! Shared data types. No logic lives here.

/// Columns per relation.
pub const ARITY: usize = 2;

/// Query variables: a, b and c.
pub const VARIABLES: usize = 3;

/// Relations of the triangle query: R, S and T.
pub const RELATIONS: usize = 3;

/// Every variable of the triangle is constrained by exactly two relations.
pub const MEMBERS_PER_LEVEL: usize = 2;

/// One input tuple of a binary relation.
pub type Edge = [u32; ARITY];

/// One query result, in variable order a, b, c.
pub type Triangle = [u32; VARIABLES];

/// One trie level in compressed sparse row form.
///
/// `keys` holds every value of the level. The children of `keys[i]` are the
/// entries `child_offsets[i]..child_offsets[i + 1]` of the next level, so a
/// node is a range instead of a pointer. The last level keeps
/// `child_offsets` empty.
pub struct Level {
    pub keys: Vec<u32>,
    pub child_offsets: Vec<u32>,
}

/// Sorted trie of one relation.
///
/// Levels follow the query variable order, not the file column order, so
/// T(c, a) is stored as (a, c).
pub struct FlatTrie {
    pub levels: [Level; ARITY],
}

/// Position inside one trie block.
///
/// `position` only moves forward and `block_end` is exclusive. Both are
/// absolute indexes of `keys`, which lets the machine read a parent offset
/// without translating ranges.
#[derive(Clone, Copy)]
pub struct TrieCursor<'a> {
    pub keys: &'a [u32],
    pub block_end: u32,
    pub position: u32,
}

/// Name kept for the fixed-arity engine during the migration.
pub type LevelCursor<'a> = TrieCursor<'a>;

/// Trie of a relation with dynamic arity.
///
/// Every depth shares one `keys` buffer and one `offsets` buffer, so a lookup
/// costs one index into `level_start` and a slice, never a pointer chase.
/// `level_start[d]..level_start[d + 1]` is the absolute key range of depth `d`
/// inside `keys`, and `offset_start[d]..offset_start[d + 1]` is the offset
/// range of that level. The last level has no offsets.
pub struct TrieLayout {
    pub keys: Vec<u32>,
    pub offsets: Vec<u32>,
    pub level_start: Vec<u32>,
    pub offset_start: Vec<u32>,
    pub depth: usize,
}

/// Relation and depth of one member of a query level.
#[derive(Clone, Copy, Debug)]
pub struct Member {
    /// 0 = R, 1 = S, 2 = T.
    pub relation: u8,
    /// Depth inside that relation's trie.
    pub depth: u8,
}

/// One variable of the query and the relations that constrain it.
pub struct QueryLevel {
    pub members: [Member; MEMBERS_PER_LEVEL],
}

/// Fixed triangle query in variable order a, b, c.
///
/// Level `i` binds variable `i`, so no separate variable table is needed.
/// `tries[r]` holds the relation that `Member::relation` names.
pub struct Query {
    pub levels: [QueryLevel; VARIABLES],
    pub tries: [FlatTrie; RELATIONS],
}

/// Tree walker state.
///
/// Transitions are values, so the control flow stays in one loop and never
/// relies on the call stack.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// Look for a value shared by every cursor of the current level.
    Search,
    /// Bind the shared value and open the next level.
    Descend,
    /// Move the leader cursor past the last match.
    Advance,
    /// Close the current level and resume the parent.
    Ascend,
    /// No frame left.
    Done,
}

/// One open trie level.
pub struct Frame<'a> {
    /// 0-based level, equal to the variable index it binds.
    pub level: u8,
    pub members: [Member; MEMBERS_PER_LEVEL],
    pub cursors: [LevelCursor<'a>; MEMBERS_PER_LEVEL],
}

/// Explicit stack machine that replaces the recursive join.
pub struct Machine<'a> {
    pub query: &'a Query,
    /// Bottom is level 0 and the top is the level being processed.
    pub stack: Vec<Frame<'a>>,
    /// Values bound for the levels below the top, ordered by level. The top
    /// level gains its entry when it matches, so a search at level `i` rewrites
    /// the slot at `i` and leaves the rest untouched.
    pub path: Vec<u32>,
    /// Absolute index of relation `r` at depth `d` for the last matched prefix.
    /// The level that opens depth `d` reads `positions[r][d - 1]`, and that
    /// slot is written when depth `d - 1` matched.
    pub positions: [[u32; ARITY]; RELATIONS],
    pub output: Vec<Triangle>,
    pub state: State,
}

/// Edges as read from the file, before validation.
pub struct RawEdges {
    pub edges: Vec<Edge>,
}

/// Validated directed graph.
pub struct Graph {
    pub edges: Vec<Edge>,
    pub node_count: u64,
}

/// Input of the trie builder.
pub struct BuildInput<'a> {
    pub edges: &'a [Edge],
    /// Column permutation that sorts this relation by the query order.
    pub order: [u8; ARITY],
}

/// Input of the hash join baseline.
///
/// Edges must be distinct: a repeated edge multiplies the materialized rows.
/// The file path deduplicates during validation, and memory-mapped callers
/// must do the same.
pub struct HashInput<'a> {
    pub edges: &'a [Edge],
    pub memory_limit_bytes: u64,
}

/// Result of the hash join baseline.
pub struct HashOutput {
    pub triangles: Vec<Triangle>,
    /// Tuples produced by the intermediate R join S before filtering.
    pub materialized: u64,
    pub memory_bytes: u64,
}

/// Failures of the input side.
///
/// Each variant carries the field values needed to print a message without a
/// second lookup.
#[derive(Debug)]
pub enum InputError {
    Io {
        path: String,
        reason: String,
    },
    Parse {
        line: u32,
        text: String,
        reason: String,
    },
}

/// Raised before materializing the hash join when the estimate does not fit.
#[derive(Debug)]
pub enum MemoryError {
    LimitExceeded {
        estimated_bytes: u64,
        limit_bytes: u64,
    },
}

/// Metrics of both engines for one run.
pub struct Comparison {
    pub edge_count: u64,
    pub node_count: u64,
    pub wcoj_triangles: u64,
    pub hash_triangles: u64,
    pub wcoj_nanos: u128,
    pub hash_nanos: u128,
    pub hash_materialized: u64,
    pub hash_memory_bytes: u64,
    /// False when the engines disagree, which the report prints first.
    pub results_match: bool,
    pub sample: Vec<Triangle>,
}

/// Input of the comparison stage.
pub struct CompareInput<'a> {
    pub wcoj: &'a [Triangle],
    pub hash: &'a HashOutput,
    pub wcoj_nanos: u128,
    pub hash_nanos: u128,
    pub edge_count: u64,
    pub node_count: u64,
    pub sample_size: usize,
}

/// Presentation limits of the report.
pub struct ReportOptions {
    pub sample_size: usize,
    pub bar_width: usize,
}

/// One attribute list as the caller provides it.
///
/// `rows` holds the tuples flattened with `vars.len()` values per tuple, and
/// `vars[j]` is the global variable bound to attribute `j`.
#[derive(Clone, Copy)]
pub struct RelationInput<'a> {
    pub rows: &'a [u32],
    pub vars: &'a [u32],
}

/// Variable and depth that constrain one trie cursor.
#[derive(Clone, Copy, Debug)]
pub struct JoinMember {
    pub relation: u32,
    /// Rank of the variable inside the relation, not the attribute position.
    pub depth: u32,
}

/// One variable of the query and every cursor that constrains it.
pub struct JoinLevel {
    pub members: Vec<JoinMember>,
}

/// Query over an arbitrary number of variables.
///
/// `cursor_base[i]..cursor_base[i + 1]` is the slice of the machine cursor
/// buffer owned by level `i`, and `positions_base[r]` is the first slot of
/// relation `r` inside the machine position buffer. Both are prefix sums, so
/// opening a level never allocates.
pub struct JoinQuery {
    pub variables: usize,
    pub levels: Vec<JoinLevel>,
    pub tries: Vec<TrieLayout>,
    pub cursor_base: Vec<u32>,
    pub positions_base: Vec<u32>,
}

/// Failures of the dynamic query builder.
#[derive(Debug)]
pub enum BuildError {
    NoRelations,
    EmptyRelation { relation: usize },
    RaggedRows { relation: usize },
    TooManyColumns { relation: usize },
    UnknownVariable { variable: u32 },
    DuplicateVariable { relation: usize },
    UnusedVariable { variable: u32 },
}

/// Command line values already parsed.
pub struct CliOptions {
    pub input_path: String,
    pub memory_limit_bytes: u64,
    pub report: ReportOptions,
}
