# P0-01: DeleteNode undo corrupts the map when the deleted node was the highest-numbered root with children

**Severity:** P0 (data corruption) · **Area:** mandala/document · **Verified:** yes (traced end-to-end)

## Context

`MindMapDocument::delete_node` orphans the deleted node's children by minting fresh root-level Dewey IDs and cascade-renaming each orphaned subtree. `UndoAction::DeleteNode` must reverse all of it (CODE_CONVENTIONS.md §3: every user-facing mutation has a fully-reversing undo branch). CONCEPTS.md ("Dewey-decimal IDs") claims fresh IDs are minted "without reusing deleted gaps". Neither holds in the case below.

## Problem

`src/application/document/topology.rs:21-42` removes the node **before** minting orphan IDs:

```rust
pub fn delete_node(&mut self, node_id: &str) -> Option<UndoAction> {
    let node = self.mindmap.nodes.remove(node_id)?;   // removed BEFORE minting
    ...
    for cid in &child_ids {
        let new_root_id = self.fresh_child_id(None);   // scans the SHRUNK map
```

`fresh_child_id(None)` (`topology.rs:243-263`) returns max-remaining-root-segment + 1. With roots `"0".."3"` and `"3"` (child `"3.0"`) deleted, remaining max is `2`, so the orphaned child is cascade-renamed to **`"3"` — the just-deleted node's own ID**.

On undo (`src/application/document/undo.rs`, `DeleteNode` arm):

```rust
let restored_id = node.id.clone();                    // "3"
self.mindmap.nodes.insert(restored_id.clone(), node); // OVERWRITES the renamed child — data destroyed
for (old_id, root_id) in orphaned_children {          // ("3.0", "3")
    self.cascade_rename(&root_id, &old_id);           // renames the RESTORED PARENT "3" -> "3.0"
    if let Some(child) = self.mindmap.nodes.get_mut(&old_id) {
        child.parent_id = Some(restored_id.clone());  // dangling: no node "3" remains
    }
}
```

Net effect of delete + Ctrl-Z: child node data destroyed, parent keyed `"3.0"` with `parent_id` pointing at nonexistent `"3"`; the edge-rename pass can additionally produce self-referential edges.

Trigger is realistic: newly created orphan nodes always take `max+1` root IDs, so "the most recently created root" is exactly the highest-numbered one.

Amplifier: `cascade_rename` (`topology.rs:68-114`) silently overwrites on key collision (`nodes.insert(new, node)`), so nothing surfaces the corruption. It is also O(renames²) + O(edges·renames) via inner linear scans.

## Why no test caught it

`tests_delete.rs` never undoes a root delete; its fixture helper picks non-root nodes. Zero coverage of ID collision on delete+undo.

## Fix plan

1. Mint orphan IDs against an ID space that still contains the deleted node: compute all fresh IDs (accounting for ones just minted) **before** `nodes.remove(node_id)`, or reserve `node_id` during the scan.
2. Defense in depth: `cascade_rename` and the `DeleteNode` undo arm refuse to overwrite an existing key — `log::error!` + skip rather than silent replacement (§9: degrade, don't corrupt).
3. Perf drive-by (§5): replace `cascade_rename`'s inner `for (ro, rn) in &renames` linear scan with a `HashMap<old,new>` lookup.
4. Regression tests (same commit, TEST_CONVENTIONS §T7): delete the max-numbered root with child + grandchild → undo → assert full restoration (node set, parent_ids, edges, payloads byte-equal).

## Acceptance criteria

- New tests fail on current main, pass after fix.
- `./test.sh` green.
- Delete+undo of any root restores the exact pre-delete model.

## Pointers

`src/application/document/topology.rs` (delete_node, fresh_child_id, cascade_rename); `src/application/document/undo.rs` (DeleteNode arm); `src/application/document/tests_delete.rs`; CODE_CONVENTIONS §3/§9; TEST_CONVENTIONS §T1 (undo round-trips are the #1 fundamental).
