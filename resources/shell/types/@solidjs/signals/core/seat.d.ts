import { type Transaction } from "./scheduler.js";
import type { Computed, Signal } from "./types.js";
type AnyNode = Signal<any> | Computed<any>;
export declare const WORLD_S = 0;
export declare const WORLD_FL = 1;
export declare const WORLD_H = 2;
export declare const POSTURE_U = 0;
export declare const POSTURE_D = 1;
export declare const POSTURE_L = 2;
export declare const POSTURE_V = 3;
export declare const POSTURE_VL = 4;
export declare const POSTURE_VP = 5;
export declare const POSTURE_A = 6;
export declare const POSTURE_F = 7;
export declare const SLOT_NONE = 0;
export declare const SLOT_C = 1;
export declare const SLOT_P = 2;
export declare const SLOT_LV = 3;
export declare const SLOT_DV = 4;
export declare const SLOT_T = 5;
export declare const SLOT_NR = 6;
export declare const SLOT_E = 7;
/** A read served from a hydration snapshot — outside the table. */
export declare const SLOT_SNAPSHOT = 8;
/** Side effects a row predicts, as a bit set. */
export declare const RAISE_FL = 1;
export declare const RAISE_H = 2;
export declare const RAISE_LANE = 4;
/** Stale reader of a hold: re-derived at its landing (`staleReader`). */
export declare const MARK_ST = 8;
/** Watches the seam (`stagedReaders`). */
export declare const MARK_W = 16;
/** Display-ahead: the pass is the hold's verdict lane's. */
export declare const MARK_AH = 32;
/** The tick joins the hold (`joinPassTx`) without the reader's world moving
 * — a leaf reading a born-held derivation (D-4, L4). */
export declare const MARK_TICK = 64;
/** A leaf's first pass read the hold's staging: born held (B2's leaf clause). */
export declare const MARK_HOLD = 128;
/** A leaf read a staging of this flush (L1): under a display-ahead mount
 * its first pass is born held (D-25, #4166 — B2's leaf clause, §6). */
export declare const MARK_SR = 256;
/** The pass enters the hold's verdict seat unconditionally (today
 * `enterLane` on a verdict lane's node — `route`'s "unless already seated"
 * test does not apply). */
export declare const RAISE_VSEAT = 512;
/** D-25 (#4275, #4284): a leaf seated as a hold's verdict reader reading a
 * lane that is not that hold's, or a flight never shown, leaves the verdict
 * seat first and reads from the frame's. */
export declare const MARK_LEAVE = 1024;
export interface Prediction {
    row: string;
    slot: number;
    raise: number;
    /** The hold the side effects name (ST(t), AH(t)). */
    tx: Transaction | null;
    /** The lane a RAISE_LANE moves the pass into. */
    lane: Transaction | null;
}
export interface Seat {
    node: Computed<any> | null;
    create: boolean;
    /** The world this pass reads from: its own reads' (`ownWorld`), the hold
     * it was born into (`bornTx`), or its creator's own at birth. */
    world: number;
    /** The world this pass's OWN reads raised it to (the old deciders' JOINED /
     * STAGED_READ flags). A child inherits this at birth, one level, so a
     * grandchild of a joined pass that read only committed values publishes
     * directly (D-25, design doc §7). */
    ownWorld: number;
    lane: Transaction | null;
    /** The hold whose verdict reader this pass sits as (today: `passLane` is
     * that hold's verdict lane). Set by a verdict read of the hold's node
     * (`route`), by a pass whose staging sits in the verdict lane, or by a
     * boundary's display-ahead swap; cleared by a lane read (the pass is the
     * lane's) and by D-25's leave. Verdict lanes have no node set at the swap:
     * this is the seat's memory of "which hold's verdict reader". */
    vhold: Transaction | null;
    ahead: boolean;
    posture: number;
    /** The hold this pass was born into (`bornInto`): held, uninitialized or
     * a held frame member. Its world is that hold's (H) from the start. */
    bornTx: Transaction | null;
    /** The creator is display-ahead itself (in a lane, or seated as a hold's
     * verdict reader), not merely under a display-ahead mount: B2's
     * nested-derivation exclusion (#4203, #4340) is for the latter. */
    creatorDirect: boolean;
    creatorRunning: boolean;
    /** The world the running creators read (today `creatorReads`: JOINED |
     * STAGED_READ up the running chain, transitive): a pass created under a
     * branch that read the future or a staging has no screen — when it asks
     * (`latest`/`isPending`) it is not a verdict reader but takes that world
     * (#4341), and a verdict lane's value does not break it out
     * (`enterLane`). */
    creatorsWorld: number;
    /** The running creator's own world at birth (today
     * `creatorPass(oldcontext)._flags & JOINED`): B2 inherits it one level. */
    creatorWorld: number;
    /** A leaf's first pass read the hold's staging (L2/L4): its first run is
     * the hold's — born held without its world moving (design doc §6, B2). */
    bornLeaf: boolean;
    /** A leaf read a staging of the flush (L1, MARK_SR). */
    sawStaging: boolean;
    /** The rows this pass hit, for the report. */
    trail: string[];
    /** The nodes this pass read ahead (`latest`/`isPending` windows): a plain
     * read of one of them is the screen's (D-25, the same-node clause).
     * Seeded with the previous pass's (today's sticky CONFIG_VERDICT is how a
     * plain-first memo is order-independent across runs); the close reports
     * where the seed and the rule part. */
    probed: Set<AnyNode>;
    /** The nodes this pass actually read ahead. */
    probedNow: Set<AnyNode>;
    /** The nodes this pass read plainly (tracked), first read first. */
    plainReads: AnyNode[];
    /** A known difference (filed) happened in this pass: the old seat and
     * the new have parted by construction; the pass's end is not compared. */
    known: boolean;
}
/** The seat of the running pass, or null outside every pass. */
export declare function currentSeat(): Seat | null;
/** The posture of a node from the node alone (INV-seat-2). `window`: 0
 * none, 1 `isPending`, 2 `latest` (2 wins — `latest` inside `isPending`
 * is `latest`'s value). */
export declare function postureOf(c: Computed<any> | null, window: number): number;
/** B1 — open the seat of a pass at `recompute`'s head. The creator of a
 * first pass is the RUNNING pass — the top of the seat stack (F-mount,
 * design doc §7: a `runWithOwner` into another owner still creates under
 * the pass that runs). `ownLane` is the node's own lane (LANE_DIRTY, or
 * OVERRIDE without GUESS, on a lane's transaction — a verdict lane's
 * included), decided by the caller from the node. */
export declare function openSeat(el: Computed<any>, create: boolean, creator: Computed<any> | null, ownLane: Transaction | null): Seat;
/** B2 — the born-held decision at a first pass's end: the tick has a hold
 * and the pass derived from it (world H — its own reads', or inherited
 * from its creator's own, one level), or it is a display-ahead mount's
 * first pass that read a staging (ahead, world ≥ Fl) — unless a nested
 * derivation left pending (#4203, #4340): no render effect reaches it yet. */
export declare function bornHeld(seat: Seat, isEffect: boolean, pending: boolean): boolean;
/** Close the seat opened by `openSeat`. */
export declare function closeSeat(seat: Seat): void;
/**
 * The slot a read of `el` by `c` is served from, with the side effects the
 * row predicts. Pure: nothing on `c`, `el` or the seat is touched —
 * `shadowRead` applies the raise to the shadow seat after the comparison.
 *
 * @param c the reader (null: no pass)
 * @param el the node read
 * @param tracked the read is a tracked dependency read (`tracking`)
 * @param window 0 none · 1 `isPending` · 2 `latest`
 * @param spectate a bookkeeping read (`spectating`)
 * @param snapshot the read is served from a hydration snapshot (outside the table)
 */
export declare function slot(c: Computed<any> | null, el: AnyNode, tracked: boolean, window: number, spectate: boolean, snapshot: boolean): Prediction;
export interface ShadowMismatch {
    kind: string;
    row: string;
    node: string;
    reader: string;
    expected: string;
    actual: string;
    count: number;
    detail: string;
    /** The generator case first seen in (`__SEAT_SHADOW_CASE__`), for a replay. */
    case: string | null;
}
/** The shadow's record, for a reporter (`tests/seat-shadow-report.ts`,
 * the generator's campaign) and `globalThis.__SEAT_SHADOW__`. */
export declare function shadowReport(): {
    reads: number;
    passes: number;
    orphanReads: number;
    rows: Record<string, number>;
    mismatches: ShadowMismatch[];
    /** Known differences hit, by issue number (`shadowKnown`). */
    known: Record<string, number>;
};
export declare function shadowReset(): void;
/** True while the body of `read` runs for the frame on top (consumed by the
 * re-entry; nested reads are shadowed on their own). */
export declare let shadowBypass: boolean;
export declare function shadowEnter(): void;
export declare function shadowConsumeBypass(): boolean;
/** The decision point of `read`/`verdictValue`: where today's deciders
 * begin. Records the prediction for the frame on top. */
export declare function shadowExpect(c: Computed<any> | null, el: AnyNode, tracked: boolean, window: number, spectate: boolean, snapshot: boolean): void;
/** A raise issued outside `read` — the store's read path calls the kept
 * shims (`stagedRead`, `joinPass`, `joinPassAt`) directly (design doc §1.2).
 * Applied to the shadow seat when no read frame of `c` is open, so the
 * world comparison does not blame a row for the store's join. */
export declare function shadowExternalRaise(c: Computed<any>, raise: number, lane?: Transaction | null, tx?: Transaction | null): void;
/** The pass entered a verdict window (`markVerdictReader`): a verdict reader
 * from here (CONFIG_VERDICT, set by the caller) — unless its running
 * creators read the future or a staging (#4341): then it is in their world
 * and stays the frame's pass (today: their JOINED | STAGED_READ bits are
 * copied onto it). */
export declare function shadowWindow(c: Computed<any>): void;
/** A boundary's display-ahead swap (`fallback`, #3540 #4337): the output's
 * pass is seated where the re-arm's `on` pass was — a lane's work, or a
 * verdict reader's (the `isPending`/`latest` the `on` read). */
export declare function shadowSwapSeat(out: Computed<any>, l: Transaction): void;
/** The swap is structure (#3540, #4337): what the fallback's build reads
 * ahead — a verdict (`latest()`), a lane — is the fallback's display-ahead,
 * not the output's. The output's seat around `fallback`'s `read(b._fb)`:
 * saved before, restored after (today: `passLane` and the PROBED /
 * LANE_READ flags, bracketed). */
export declare function shadowSeatSave(out: Computed<any>): [Transaction | null, Transaction | null] | null;
export declare function shadowSeatRestore(out: Computed<any>, saved: [Transaction | null, Transaction | null] | null): void;
/** A return site of the old deciders: the slot it serves. */
export declare function shadowActual(s: number): void;
/** The body returned (or threw: `thrown` is the slot of the throw). Compare,
 * apply the predicted raise to the shadow seat, pop. */
/** Trace aid: the last error a shadowed read threw. */
export declare let lastThrown: unknown;
export declare function shadowThrown(e: unknown): void;
export declare function shadowKnownHits(): ReadonlyMap<string, number>;
export declare function shadowLeave(thrown: number): void;
/** `recompute`'s head, after the old seating: the new seat beside it. */
export declare function shadowSeatOpen(el: Computed<any>, create: boolean, creator: Computed<any> | null, ownLane: Transaction | null, mount: unknown, creatorJoined: boolean): Seat;
/** `recompute`'s finally: the born-held decision beside the old one. */
export declare function shadowSeatClose(el: Computed<any>, isEffect: boolean, oldJoined: boolean, oldCreator: Computed<any> | null, oldMount: unknown): void;
export {};
