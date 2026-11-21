import { latestOf } from "./truth.js";
import { type Transaction } from "./scheduler.js";
import type { Computed, Owner, Signal } from "./types.js";
export { latestOf };
/** Live lanes (parented). Scanned at every seam while non-empty. */
export declare const lanes: Transaction[];
export declare function newLane(parent: Transaction): Transaction;
/** `v` is lane `l`'s value of `n` from now — a guess written or re-asked, a
 * store's composed row, a guess coming back: on screen at once once the
 * lane has revealed (what it covers kept for the lane's end, `cover`),
 * staged for the reveal before. */
export declare function setLaneValue(n: Signal<any>, l: Transaction, v: unknown): void;
/** What the screen shows of a node: NOT_PENDING when nothing (uninitialized). */
export declare function screenOf(el: Signal<any> | Computed<any>): unknown;
/** The guess layer's screen of a node (guesses.ts): a shown guess re-asked
 * keeps the shown one until its lane's next reveal (#4775); `revealed`
 * forgets it. */
export declare let guessScreen: ((el: Signal<any>, revealed?: true) => unknown) | null;
/** A truth landed beneath a guess: the parent's staging, held for its
 * landing to commit (CONFIG_HELD) — an older question's answer (A18), or the
 * guess's confirmation (A24). The guess stays the value. */
export declare function landBeneath(n: Signal<any> | Computed<any>, truth: unknown): void;
/** A guess was written (guesses.ts `optimisticWrite`). */
export declare function armGuessed(): void;
export declare let takeHook: ((n: Signal<any>, l: Transaction, d?: boolean) => void) | null;
export declare let guessWriteHook: ((el: Signal<any> | Computed<any>, v: unknown) => boolean) | null;
export declare let revertLandingHook: ((u: Transaction) => void) | null;
export declare function installGuessLayer(take: NonNullable<typeof takeHook>, write: NonNullable<typeof guessWriteHook>, revert: NonNullable<typeof revertLandingHook>, screen: NonNullable<typeof guessScreen>): void;
/** Two kinds of member, one set (a lane is never a node). Lanes a revert
 * dissolved: the guesses they took come back at their parent's landing
 * (`endLanes`). Shown guesses a correction re-homed into their hold
 * (guesses.ts `supersede`, `dissolveLane`): the slot's value for the screen and
 * untracked reads until that hold lands (D-34), each keeps it open on its
 * own flight as it did while a guess (`lanesBlocked`, #4690); the hold's
 * landing clears them (`endLanes`). */
export declare const marked: WeakSet<Transaction | Signal<any>>;
/** A shown guess a correction re-homed into its hold (guesses.ts `supersede`,
 * `dissolveLane`): the screen's and the untracked reads' value until the
 * commit, and `latest`'s (D-34, decision 3 of 2026-11-05, #4689) — the
 * displayed slot's newest value is the guess, the truth its staging. */
export declare function keptGuess(n: Signal<any> | Computed<any>): boolean;
/** `n`'s readers are stale readers of `t` (re-derived at its landing). */
export declare function rerun(n: Signal<any>, t: Transaction): void;
/** A guess's own truth is in flight: the node's own refetch — or, a store
 * leaf's, its family's derive (the slot's flight; store/optimistic.ts
 * installs the resolver). A17: the guess stands in for the flight. */
export declare function inFlight(n: Signal<any>): unknown;
/** The running pass read lane `l`'s work as a derivation: it is `l`'s — or,
 * already another lane's, the frame it computes derives from both guesses
 * and cannot show one without the other (#3335, A15 for lanes): the two
 * lanes are linked — one reveal unit, each blocked while any is — while
 * their lifetimes stay their own (#2912: a guess reverts with its own
 * action, not with the one it shares a reader with). (Lanes never nest: a
 * lane is never the flush's transaction, D-37, so no guess is parented to
 * one.) */
export declare function enterLane(l: Transaction, c: Computed<any>): void;
/** Is `l` a verdict lane (its parent's `_verdict`)? False for none. */
export declare function isVerdict(l: Transaction | null | undefined): boolean;
/** Verdicts (maintainer, 2026-10-01): `isPending` and `latest` are
 * optimistic state the system supplies — a verdict is a guess ("pending,
 * until this lands"; "the proposal is the value") decided at the seam like
 * a user's, and its readers break out of the hold the same way: work of a
 * lane under the holder, shown now, re-derived at the landing, ending with
 * it. Judged at the seam like any lane: blocked while a verdict reader's
 * own derivation is in flight (lane rule 3), shown otherwise. */
export declare function verdictLane(t: Transaction): Transaction;
/** A lane pass's node (core.ts `recompute`, before its staging; a pending
 * propagation under a lane pass, async.ts): listed, carrying the lane's
 * value from now (a derivation of a guess — its readers become lane work).
 * Entering the lane supersedes a frame staging the node carried: this
 * pass's answer replaces it, and the frame's hold on it is over. An
 * effect's value slot is private (no lane value). A pass in a lane's seat
 * that read none of the lane's world has left it (false): its result is the
 * frame's — a derivation whose branch no longer reaches the guess — and no
 * hold the lane took over from a transaction stays on it (#3698: lane work
 * never makes its node transaction work; a born-held effect's hold went to
 * the lane). A pass interrupted before it got there (pending, errored) did
 * not leave (A30: it never got there). A first pass under a lane is the
 * lane's (ruling A); a guess is written, not derived — it never leaves this
 * way. `errored`: the pass has no answer — or, a lane pass's pending
 * propagation (async.ts `propagateStatus`), the node gone pending. */
export declare function laneStage(el: Computed<any>, l: Transaction, create: boolean, errored: boolean | Computed<any>): boolean;
/** A lane's end, two ways. `into === null`: the parent landed — the lane's
 * values are the truth now: a guess lands the truth staged beneath it or
 * reverts to the one it covered (`_value`, untouched by the guess); a
 * derivation's latest value commits (its frame with it); what the screen
 * showed and changes notifies. Otherwise the lane dissolves into `into`
 * (the parent, on a correction): its values are void — what it showed stays
 * the screen (`_value` takes it, so the parent's re-derivation compares
 * against what is seen), a truth staged beneath a guess is the parent's to
 * land, and the runs it held (showing the void guess) are dropped: the
 * parent's re-derivation makes the runs that show. */
export declare function dissolveLane(l: Transaction, into: Transaction | null, except?: Signal<any>): void;
/** Lanes. A lane's node is read by its reader's kind: an authoritative
 * reader (`until`) sees the truth — staged beneath a guess, else committed
 * — and is no lane's; an untracked read sees the lane's latest value (A17:
 * "direct read shows optimistic"; A28); a children-forbidden reader sees
 * the frame (A32). A render effect in the frame's seat reading a lane the
 * seam found blocked, or a member whose own flight is up (held or not — the
 * lane-membership ruling, 2026-10-05), sees the screen, re-derived at the
 * reveal (#3460's stale reader) — or, the node a
 * flight that has never shown (nothing to show), waits on it: its own frame
 * holds (A15, #3334), not the lane's parent. Any other tracked reader is
 * the lane's work from this read — a derivation; a leaf reading a lane that
 * has shown, or one the seam has not judged yet (its run waits on the
 * verdict: released if the lane shows, held if it blocks) — and sees the
 * lane's latest. NOT_PENDING falls through: lane work's read of a pending
 * member throws like any. A verdict reader entering a lane the seam has
 * not judged reads ahead of it like a staging read: if the seam holds the
 * lane, the pass re-derives on the screen (`entered`, #4284) — `latest()`
 * beside the lane's nodes tears ahead of the hold however the reads are
 * grouped into effects. */
/** #4788: the lanes whose guesses an action read imperatively (its body, a
 * setter: `laneRead`, a store setter's draft), each with the question the
 * reading action asks (`question`): a guess that action writes next derives
 * from them (guesses.ts `optimisticWrite`); another action's reads are not
 * its own. Kept while the lane read lives (guesses.ts `applyGuessesAt`). */
export declare const readGuesses: unknown[];
export declare function laneRead(c: Computed<any> | null, el: Signal<any> | Computed<any>): unknown;
/** A guess in `l` whose own truth is in flight (its refetch — authoritative:
 * the landing supersedes, not the body's end). */
export declare function guessFlights(l: Transaction): boolean;
/** A collected mount still held, uninitialized, on screen and waiting —
 * on its own flight, an action open in its frame, or a flight its frame
 * holds — under `o`, or under one of lane `l`'s nodes (by its owner
 * chain). */
export declare function heldMountUnder(o: Owner | null, l: Transaction | null): boolean;
/** The guess layer's seam (guesses.ts `applyGuessesAt`), before its
 * guesses open their lanes. */
export declare function markJudged(): void;
/** The effects that derive plainly from a landing this round that
 * re-derives a stale reader (A15) reading it as committed: its held runs
 * wait for the re-derivation (`land`), and these runs are that frame too —
 * they wait with them (verdict.ts `attachWaits`; F-1, F-7, #4677). The walk
 * stops at verdict readers: what derives from a display-ahead read shows
 * now (F-6, D-25, #4398). Dropped at the end of the round's effect phase. */
export declare let waiting: Set<unknown> | null;
/** This flush's verdict readers that read a flight ahead of the frame
 * (#4661, the seam). */
export declare const ahead: Computed<any>[];
