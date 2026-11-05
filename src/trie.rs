//! Flat trie construction and level cursor operations.

use core::cmp::Ordering;
use core::ops::Range;

use crate::types::{BuildInput, Edge, FlatTrie, Level, LevelCursor, TrieCursor, TrieLayout};

/// Builds the two-level trie of one binary relation.
///
/// Rows are permuted to the query order, then sorted and deduplicated. Level 0
/// keeps one entry per distinct first value and points into level 1, which
/// keeps the second value of every distinct pair.
pub fn build_trie(input: BuildInput<'_>) -> FlatTrie {
    let rows: Vec<Edge> = permuted_sorted_rows(input);
    let mut level0_keys: Vec<u32> = Vec::new();
    let mut level0_offsets: Vec<u32> = Vec::new();
    let mut level1_keys: Vec<u32> = Vec::with_capacity(rows.len());

    for row in &rows {
        if level0_keys.last() != Some(&row[0]) {
            level0_keys.push(row[0]);
            level0_offsets.push(level1_keys.len() as u32);
        }
        level1_keys.push(row[1]);
    }
    level0_offsets.push(level1_keys.len() as u32);

    FlatTrie {
        levels: [
            Level {
                keys: level0_keys,
                child_offsets: level0_offsets,
            },
            Level {
                keys: level1_keys,
                child_offsets: Vec::new(),
            },
        ],
    }
}

/// Copies, permutes, sorts and deduplicates the input edges.
fn permuted_sorted_rows(input: BuildInput<'_>) -> Vec<Edge> {
    let mut rows: Vec<Edge> = Vec::with_capacity(input.edges.len());
    for edge in input.edges {
        let first: usize = usize::from(input.order[0]);
        let second: usize = usize::from(input.order[1]);
        rows.push([edge[first], edge[second]]);
    }
    rows.sort_unstable();
    rows.dedup();
    rows
}

/// Whole level as an absolute range, used for the root level.
pub fn level_range(level: &Level) -> Range<u32> {
    0..level.keys.len() as u32
}

/// Child block of `parent` at `level`, as absolute indexes of the next level.
pub fn child_range(trie: &FlatTrie, level: usize, parent: u32) -> Range<u32> {
    let offsets = &trie.levels[level].child_offsets;
    let first: usize = parent as usize;
    offsets[first]..offsets[first + 1]
}

/// Cursor over `keys[range]`, positioned at the first entry of the block.
#[inline]
pub fn cursor_new(keys: &[u32], range: Range<u32>) -> LevelCursor<'_> {
    TrieCursor {
        keys,
        block_end: range.end,
        position: range.start,
    }
}

/// Value under the cursor, or `None` at the end of the block.
#[inline]
pub fn cursor_current(cursor: &LevelCursor<'_>) -> Option<u32> {
    if cursor.position < cursor.block_end {
        Some(cursor.keys[cursor.position as usize])
    } else {
        None
    }
}

/// Moves the cursor to the first value greater than or equal to `target`.
///
/// The cursor never moves backwards, so the search starts at `position` and a
/// target already behind it leaves the cursor unchanged.
#[inline]
pub fn cursor_seek(cursor: &mut LevelCursor<'_>, target: u32) {
    if cursor.position < cursor.block_end {
        let start: usize = cursor.position as usize;
        let end: usize = cursor.block_end as usize;
        cursor.position += gallop(&cursor.keys[start..end], target);
    }
}

/// Moves the cursor one entry forward inside its block.
#[inline]
pub fn cursor_next(cursor: &mut LevelCursor<'_>) {
    if cursor.position < cursor.block_end {
        cursor.position += 1;
    }
}

/// Number of leading entries of `keys` below `target`.
///
/// Exponential probes locate a bracket, then a binary search finds the first
/// entry at or above `target`. Returns `keys.len()` when every entry is below.
pub fn gallop(keys: &[u32], target: u32) -> u32 {
    let mut found: u32 = 0;
    if !keys.is_empty() && keys[0] < target {
        let mut low: usize = 0;
        let mut step: usize = 1;
        let mut probing: bool = true;
        while probing {
            if step >= keys.len() || keys[step] >= target {
                probing = false;
            } else {
                low = step;
                step *= 2;
            }
        }
        let high: usize = step.min(keys.len());
        found = low as u32 + 1 + lower_bound(&keys[low + 1..high], target);
    }
    found
}

/// Index of the first entry at or above `target` in a sorted slice.
fn lower_bound(keys: &[u32], target: u32) -> u32 {
    let mut low: usize = 0;
    let mut high: usize = keys.len();
    while low < high {
        let middle: usize = low + (high - low) / 2;
        if keys[middle] < target {
            low = middle + 1;
        } else {
            high = middle;
        }
    }
    low as u32
}

/// One relation with `width` columns, sorted and flattened.
///
/// `rows` holds `width` values per tuple and `order[query position]` names the
/// tuple column bound at that position. Rows are sorted by the permuted
/// columns, duplicates collapse, and everything is written in one pass into the
/// shared level buffers. All allocations happen here, never in the cursor loop.
pub fn build_layout(rows: &[u32], width: usize, order: &[u8]) -> TrieLayout {
    if width == 0 || order.len() < width || rows.len() < width {
        return empty_layout(width);
    }
    let count: usize = rows.len() / width;
    let sorted: Vec<u32> = sorted_row_order(rows, width, order, count);
    let lengths: Vec<u32> = count_level_keys(rows, width, order, count, &sorted);
    let level_start: Vec<u32> = prefix_sums(&lengths);
    let offset_start: Vec<u32> = offset_prefix_sums(&lengths);
    let mut builder: LevelBuilder = builder_new(&level_start, &offset_start, width);
    fill_levels(&mut builder, rows, order, &sorted);
    write_sentinels(&mut builder, &level_start);
    TrieLayout {
        keys: builder.keys,
        offsets: builder.offsets,
        level_start,
        offset_start,
        depth: width,
    }
}

fn empty_layout(width: usize) -> TrieLayout {
    TrieLayout {
        keys: Vec::new(),
        offsets: Vec::new(),
        level_start: vec![0; width + 1],
        offset_start: vec![0; width + 1],
        depth: width,
    }
}

/// Absolute key range of depth `depth`.
#[inline]
pub fn layout_level(layout: &TrieLayout, depth: usize) -> Range<u32> {
    let mut range: Range<u32> = 0..0;
    if depth < layout.depth {
        range = layout.level_start[depth]..layout.level_start[depth + 1];
    }
    range
}

/// Child range of the key at absolute position `parent` of depth `depth`.
///
/// Cursors store absolute positions in `keys`, so `parent` is absolute too and
/// the level start is subtracted here.
#[inline]
pub fn layout_child_range(layout: &TrieLayout, depth: usize, parent: u32) -> Range<u32> {
    let mut range: Range<u32> = 0..0;
    if depth + 1 < layout.depth {
        let level_start: u32 = layout.level_start[depth];
        if parent >= level_start {
            let relative: usize = (parent - level_start) as usize;
            let start: usize = layout.offset_start[depth] as usize;
            let limit: usize = layout.offset_start[depth + 1] as usize;
            let index: usize = start + relative;
            if index + 1 < limit {
                range = layout.offsets[index]..layout.offsets[index + 1];
            }
        }
    }
    range
}

/// Cursor over an absolute range of the shared key buffer.
#[inline]
pub fn layout_cursor(layout: &TrieLayout, range: Range<u32>) -> TrieCursor<'_> {
    TrieCursor {
        keys: &layout.keys,
        block_end: range.end,
        position: range.start,
    }
}

/// Build-time buffers of the dynamic layout. Dropped after `build_layout`.
struct LevelBuilder {
    keys: Vec<u32>,
    offsets: Vec<u32>,
    key_write: Vec<u32>,
    offset_write: Vec<u32>,
    width: usize,
}

/// Read-only view of the flat rows used by the sort comparator.
struct RowView<'a> {
    rows: &'a [u32],
    width: usize,
    order: &'a [u8],
}

fn builder_new(level_start: &[u32], offset_start: &[u32], width: usize) -> LevelBuilder {
    let mut key_write: Vec<u32> = Vec::with_capacity(width);
    let mut offset_write: Vec<u32> = Vec::with_capacity(width);
    let mut depth: usize = 0;
    while depth < width {
        key_write.push(level_start[depth]);
        offset_write.push(offset_start[depth]);
        depth += 1;
    }
    LevelBuilder {
        keys: vec![0; level_start[width] as usize],
        offsets: vec![0; offset_start[width] as usize],
        key_write,
        offset_write,
        width,
    }
}

/// Stable bottom-up merge sort of row indexes, with no closures.
fn sorted_row_order(rows: &[u32], width: usize, order: &[u8], count: usize) -> Vec<u32> {
    let mut source: Vec<u32> = Vec::with_capacity(count);
    let mut index: usize = 0;
    while index < count {
        source.push(index as u32);
        index += 1;
    }
    let mut target: Vec<u32> = vec![0; count];
    let view: RowView<'_> = RowView { rows, width, order };
    let mut run: usize = 1;
    while run < count {
        merge_pass(&view, &source, &mut target, run);
        core::mem::swap(&mut source, &mut target);
        run *= 2;
    }
    source
}

fn merge_pass(view: &RowView<'_>, source: &[u32], target: &mut [u32], run: usize) {
    let mut start: usize = 0;
    while start < source.len() {
        let middle: usize = (start + run).min(source.len());
        let end: usize = (start + 2 * run).min(source.len());
        merge_run(view, source, target, start, middle, end);
        start = end;
    }
}

fn merge_run(
    view: &RowView<'_>,
    source: &[u32],
    target: &mut [u32],
    start: usize,
    middle: usize,
    end: usize,
) {
    let mut left: usize = start;
    let mut right: usize = middle;
    let mut write: usize = start;
    while left < middle || right < end {
        let take_left: bool = right >= end
            || (left < middle
                && compare_rows(view, source[left], source[right]) != Ordering::Greater);
        if take_left {
            target[write] = source[left];
            left += 1;
        } else {
            target[write] = source[right];
            right += 1;
        }
        write += 1;
    }
}

/// Lexicographic comparison on the permuted columns.
fn compare_rows(view: &RowView<'_>, left: u32, right: u32) -> Ordering {
    let left_base: usize = left as usize * view.width;
    let right_base: usize = right as usize * view.width;
    let mut depth: usize = 0;
    let mut result: Ordering = Ordering::Equal;
    while depth < view.width && result == Ordering::Equal {
        let column: usize = usize::from(view.order[depth]);
        result = view.rows[left_base + column].cmp(&view.rows[right_base + column]);
        depth += 1;
    }
    result
}

/// Number of leading permuted columns shared by two sorted rows.
fn common_prefix(rows: &[u32], width: usize, order: &[u8], previous: u32, current: u32) -> usize {
    let previous_base: usize = previous as usize * width;
    let current_base: usize = current as usize * width;
    let mut depth: usize = 0;
    while depth < width {
        let column: usize = usize::from(order[depth]);
        if rows[previous_base + column] != rows[current_base + column] {
            return depth;
        }
        depth += 1;
    }
    depth
}

/// Number of distinct keys per level, equal to the distinct prefixes.
fn count_level_keys(
    rows: &[u32],
    width: usize,
    order: &[u8],
    count: usize,
    row_order: &[u32],
) -> Vec<u32> {
    let mut lengths: Vec<u32> = vec![0; width];
    let mut index: usize = 0;
    let mut previous: u32 = 0;
    while index < count {
        let current: u32 = row_order[index];
        let common: usize = if index == 0 {
            0
        } else {
            common_prefix(rows, width, order, previous, current)
        };
        add_levels(&mut lengths, common);
        previous = current;
        index += 1;
    }
    lengths
}

fn add_levels(lengths: &mut [u32], first: usize) {
    let mut depth: usize = first;
    while depth < lengths.len() {
        lengths[depth] += 1;
        depth += 1;
    }
}

fn prefix_sums(lengths: &[u32]) -> Vec<u32> {
    let mut starts: Vec<u32> = Vec::with_capacity(lengths.len() + 1);
    starts.push(0);
    let mut depth: usize = 0;
    while depth < lengths.len() {
        starts.push(starts[depth] + lengths[depth]);
        depth += 1;
    }
    starts
}

/// Offset slots per level: one start per key plus one end sentinel, except the
/// last level, which has no children.
fn offset_prefix_sums(lengths: &[u32]) -> Vec<u32> {
    let mut starts: Vec<u32> = Vec::with_capacity(lengths.len() + 1);
    starts.push(0);
    let mut depth: usize = 0;
    while depth < lengths.len() {
        let mut extra: u32 = 0;
        if depth + 1 < lengths.len() {
            extra = lengths[depth] + 1;
        }
        starts.push(starts[depth] + extra);
        depth += 1;
    }
    starts
}

fn fill_levels(builder: &mut LevelBuilder, rows: &[u32], order: &[u8], row_order: &[u32]) {
    let mut index: usize = 0;
    let mut previous: u32 = 0;
    while index < row_order.len() {
        let current: u32 = row_order[index];
        let common: usize = if index == 0 {
            0
        } else {
            common_prefix(rows, builder.width, order, previous, current)
        };
        open_row(builder, rows, order, current, common);
        previous = current;
        index += 1;
    }
}

/// Writes one key per opened depth plus the start offset of its children.
fn open_row(
    builder: &mut LevelBuilder,
    rows: &[u32],
    order: &[u8],
    row_index: u32,
    first_depth: usize,
) {
    let base: usize = row_index as usize * builder.width;
    let mut depth: usize = first_depth;
    while depth < builder.width {
        let slot: usize = builder.key_write[depth] as usize;
        builder.keys[slot] = rows[base + usize::from(order[depth])];
        builder.key_write[depth] += 1;
        if depth + 1 < builder.width {
            let offset_slot: usize = builder.offset_write[depth] as usize;
            builder.offsets[offset_slot] = builder.key_write[depth + 1];
            builder.offset_write[depth] += 1;
        }
        depth += 1;
    }
}

/// Closes every level with the end boundary of its child level.
fn write_sentinels(builder: &mut LevelBuilder, level_start: &[u32]) {
    let mut depth: usize = 0;
    while depth + 1 < builder.width {
        let slot: usize = builder.offset_write[depth] as usize;
        builder.offsets[slot] = level_start[depth + 2];
        depth += 1;
    }
}

/// Intersection of every cursor: returns the first value that all of them hold.
///
/// Each pass computes the maximum of the current values and seeks every cursor
/// below it. A cursor that lands past the maximum changes the maximum, so the
/// pass repeats. Cursors only move forward, which bounds the loop.
pub fn leapfrog(cursors: &mut [TrieCursor<'_>]) -> Option<u32> {
    let mut result: Option<u32> = None;
    let mut searching: bool = true;
    while searching {
        let maximum: Option<u32> = maximum_current(cursors);
        let aligned: bool = align_at(cursors, maximum);
        if aligned || maximum.is_none() {
            result = maximum;
            searching = false;
        }
    }
    result
}

fn maximum_current(cursors: &[TrieCursor<'_>]) -> Option<u32> {
    let mut maximum: u32 = 0;
    let mut index: usize = 0;
    let mut complete: bool = true;
    while complete && index < cursors.len() {
        let current: Option<u32> = cursor_current(&cursors[index]);
        if let Some(value) = current {
            maximum = maximum.max(value);
        } else {
            complete = false;
        }
        index += 1;
    }
    let mut result: Option<u32> = None;
    if complete && !cursors.is_empty() {
        result = Some(maximum);
    }
    result
}

fn align_at(cursors: &mut [TrieCursor<'_>], maximum: Option<u32>) -> bool {
    let mut aligned: bool = false;
    if let Some(value) = maximum {
        aligned = align_to(cursors, value);
    }
    aligned
}

fn align_to(cursors: &mut [TrieCursor<'_>], maximum: u32) -> bool {
    let mut aligned: bool = true;
    let mut index: usize = 0;
    while aligned && index < cursors.len() {
        let current: Option<u32> = cursor_current(&cursors[index]);
        if current != Some(maximum) {
            cursor_seek(&mut cursors[index], maximum);
            let reached: Option<u32> = cursor_current(&cursors[index]);
            if reached != Some(maximum) {
                aligned = false;
            }
        }
        index += 1;
    }
    aligned
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_two_levels_for_binary_edges() {
        let edges: Vec<Edge> = vec![[1, 2], [1, 3], [2, 3]];
        let trie: FlatTrie = build_trie(BuildInput {
            edges: &edges,
            order: [0, 1],
        });

        assert_eq!(trie.levels[0].keys, vec![1, 2]);
        assert_eq!(trie.levels[0].child_offsets, vec![0, 2, 3]);
        assert_eq!(trie.levels[1].keys, vec![2, 3, 3]);
        assert!(trie.levels[1].child_offsets.is_empty());
    }

    #[test]
    fn drops_duplicate_edges() {
        let edges: Vec<Edge> = vec![[1, 2], [1, 2], [2, 1]];
        let trie: FlatTrie = build_trie(BuildInput {
            edges: &edges,
            order: [0, 1],
        });

        assert_eq!(trie.levels[0].keys, vec![1, 2]);
        assert_eq!(trie.levels[1].keys, vec![2, 1]);
        assert_eq!(trie.levels[0].child_offsets, vec![0, 1, 2]);
    }

    #[test]
    fn permutes_columns_before_sorting() {
        let edges: Vec<Edge> = vec![[3, 1], [3, 2]];
        let trie: FlatTrie = build_trie(BuildInput {
            edges: &edges,
            order: [1, 0],
        });

        assert_eq!(trie.levels[0].keys, vec![1, 2]);
        assert_eq!(trie.levels[1].keys, vec![3, 3]);
    }

    #[test]
    fn child_range_matches_offsets() {
        let edges: Vec<Edge> = vec![[1, 2], [1, 3], [2, 3]];
        let trie: FlatTrie = build_trie(BuildInput {
            edges: &edges,
            order: [0, 1],
        });

        assert_eq!(child_range(&trie, 0, 0), 0..2);
        assert_eq!(child_range(&trie, 0, 1), 2..3);
    }

    #[test]
    fn gallop_returns_first_index_at_or_above_target() {
        let keys: Vec<u32> = vec![10, 20, 30, 40, 50];

        assert_eq!(gallop(&keys, 0), 0);
        assert_eq!(gallop(&keys, 10), 0);
        assert_eq!(gallop(&keys, 25), 2);
        assert_eq!(gallop(&keys, 50), 4);
        assert_eq!(gallop(&keys, 51), 5);
        assert_eq!(gallop(&[], 7), 0);
    }

    #[test]
    fn cursor_seeks_inside_its_block_only() {
        let keys: Vec<u32> = vec![10, 20, 30, 40];
        let mut cursor: LevelCursor<'_> = cursor_new(&keys, 1..3);

        assert_eq!(cursor_current(&cursor), Some(20));

        cursor_seek(&mut cursor, 15);
        assert_eq!(cursor_current(&cursor), Some(20));

        cursor_seek(&mut cursor, 25);
        assert_eq!(cursor_current(&cursor), Some(30));

        cursor_seek(&mut cursor, 99);
        assert_eq!(cursor_current(&cursor), None);

        cursor_next(&mut cursor);
        assert_eq!(cursor_current(&cursor), None);
    }

    #[test]
    fn builds_three_levels_for_ternary_rows() {
        let rows: Vec<u32> = vec![1, 2, 3, 1, 2, 4, 2, 5, 6];
        let layout: TrieLayout = build_layout(&rows, 3, &[0, 1, 2]);

        assert_eq!(layout.level_start, vec![0, 2, 4, 7]);
        assert_eq!(layout.offset_start, vec![0, 3, 6, 6]);
        assert_eq!(layout.keys, vec![1, 2, 2, 5, 3, 4, 6]);
        assert_eq!(layout.offsets, vec![2, 3, 4, 4, 6, 7]);
        assert_eq!(layout_child_range(&layout, 0, 0), 2..3);
        assert_eq!(layout_child_range(&layout, 0, 1), 3..4);
        assert_eq!(layout_child_range(&layout, 1, 2), 4..6);
        assert_eq!(layout_child_range(&layout, 1, 3), 6..7);
    }

    #[test]
    fn permutes_columns_of_wide_rows() {
        let rows: Vec<u32> = vec![3, 1, 3, 2];
        let layout: TrieLayout = build_layout(&rows, 2, &[1, 0]);

        assert_eq!(layout.keys, vec![1, 2, 3, 3]);
        assert_eq!(layout.level_start, vec![0, 2, 4]);
    }

    #[test]
    fn collapses_identical_rows() {
        let rows: Vec<u32> = vec![1, 2, 3, 1, 2, 3, 1, 2, 3];
        let layout: TrieLayout = build_layout(&rows, 3, &[0, 1, 2]);

        assert_eq!(layout.level_start, vec![0, 1, 2, 3]);
        assert_eq!(layout.keys, vec![1, 2, 3]);
        assert_eq!(layout.offsets, vec![1, 2, 2, 3]);
    }

    #[test]
    fn supports_a_single_column_relation() {
        let rows: Vec<u32> = vec![5, 3, 5, 1];
        let layout: TrieLayout = build_layout(&rows, 1, &[0]);

        assert_eq!(layout.keys, vec![1, 3, 5]);
        assert!(layout.offsets.is_empty());
        assert_eq!(layout_child_range(&layout, 0, 0), 0..0);
    }

    #[test]
    fn rejects_an_incomplete_permutation() {
        let rows: Vec<u32> = vec![1, 2];
        let layout: TrieLayout = build_layout(&rows, 2, &[0]);

        assert!(layout.keys.is_empty());
        assert_eq!(layout.level_start, vec![0, 0, 0]);
    }

    #[test]
    fn seeks_inside_a_dynamic_level() {
        let rows: Vec<u32> = vec![10, 1, 10, 2, 20, 1];
        let layout: TrieLayout = build_layout(&rows, 2, &[0, 1]);
        let mut cursor: TrieCursor<'_> = layout_cursor(&layout, layout_level(&layout, 0));

        assert_eq!(cursor_current(&cursor), Some(10));

        cursor_seek(&mut cursor, 15);
        assert_eq!(cursor_current(&cursor), Some(20));

        cursor_next(&mut cursor);
        assert_eq!(cursor_current(&cursor), None);
    }
}
