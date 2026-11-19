import { type Transaction } from "./scheduler.js";
import type { Computed, Signal } from "./types.js";
/** A lane's record of a node beside the node's two slots: its guess until
 * the lane reveals it (its staging is a truth's), and after the reveal the
 * value the screen showed before (a guess reverts to it at the lane's end;
 * once shown (CONFIG_SHOWN). Dropped when the node leaves its lane. */
export declare const covers: WeakMap<Signal<any>, unknown>;
/** A lane node's latest value. A guess: the guess — the lane's record of it
 * until the reveal (`under`), the screen's value after. A derivation: its
 * staging under the lane (a lane pass's answer, until the reveal), else
 * what the screen shows — NOT_PENDING for a node with neither (born in the
 * lane, uninitialized: its reads throw NotReady, D-22; #4363). */
export declare function latestOf(el: Signal<any> | Computed<any>): unknown;
/** The value a guess covered: the committed value until the lane reveals
 * the guess, the lane's record of it after (`under`) */
export declare function covered(n: Signal<any> | Computed<any>): unknown;
/** The truth beneath a guess: a landing staged there — the parent's (lanes.ts
 * `landBeneath`), or the same tick's — else the value it covered. */
export declare function underOf(n: Signal<any> | Computed<any>): unknown;
/** Truth readers (`until`, `refresh`; CONFIG_AUTHORITATIVE) — pay-for-use:
 * the first one installs these on GlobalQueue (signals.ts `watch`).
 *
 * A truth reader's read of a lane node: the lane's value when it is the
 * truth (#3943), else the truth beneath a guess. */
export declare function truthRead(el: Signal<any> | Computed<any>): unknown;
/** A guess confirmed: its lane's truth readers re-read — of the lane's
 * derivations too, which may be the truth now (#3943). */
export declare function truthConfirmed(l: Transaction): void;
