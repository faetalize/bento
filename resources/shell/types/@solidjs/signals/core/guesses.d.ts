import { type Transaction } from "./scheduler.js";
import type { Computed, Signal } from "./types.js";
/** What an optimistic updater composes on (F-10: the writer sees its own
 * unflushed writes). */
export declare function guessedValueOf<T>(n: Signal<T> | Computed<T>): T;
export declare function pendingGuessOf(n: Signal<any> | Computed<any>): unknown;
/** `d`: taken without being written (a store target's other slots). */
export declare function take(n: Signal<any>, l: Transaction, d?: boolean): void;
export declare function optimisticWrite<T>(n: Signal<T> | Computed<T>, v: T | ((prev: T) => T)): T;
/** A guess's truth arrived: a different one corrects, an equal one confirms
 * (F-11, D-34). The truth is held by the guess's hold, not by the tick that
 * carried it, so one landing answering two actions' guesses does not
 * entangle them (D-33). */
export declare function supersede(n: Signal<any> | Computed<any>, value: unknown, changed: boolean): void;
