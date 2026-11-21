import type { Computed, Owner } from "./types.js";
/**
 * Schedule one subscriber to re-run on the next flush: inserted into the
 * dirty heap with the `_min` cursor pulled down. Tracked effects ride the
 * heap too — the heap visit is their (empty) compute phase, which hands the
 * callback to the user queue once the pass has committed (see
 * GlobalQueue._update, #3291).
 */
export declare function enqueueSub(node: Computed<any>): void;
export interface Heap {
    _heap: (Computed<unknown> | undefined)[];
    _marked: boolean;
    _min: number;
    _max: number;
}
export declare function increaseHeapSize(n: number, heap: Heap): void;
export declare function insertIntoHeap(n: Computed<any>, heap: Heap): void;
export declare function insertIntoHeapHeight(n: Computed<unknown>, heap: Heap): void;
export declare function deleteFromHeap(n: Computed<unknown>, heap: Heap): void;
export declare function markHeap(heap: Heap): void;
export declare function markNode(el: Computed<unknown>, newState?: number): void;
export declare function runHeap(heap: Heap, recompute: (el: Computed<unknown>) => void): void;
/**
 * A raised node's owned computations (through roots) move above it (#4848).
 * The heap places a node above its direct owner only, at insert, so an owner
 * raised after its children's birth left a grandchild at or below it, to run
 * before the owner whose run disposes it. Paid when a height changes, never
 * per insert. A child waiting in the heap keeps its seat this pass (its next
 * insert lifts it above its owner); what it owns moves either way.
 */
export declare function liftOwned(o: Owner, h: number): void;
