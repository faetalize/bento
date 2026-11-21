import { NotReadyError } from "./error.js";
import type { Computed, Link } from "./types.js";
export declare function addPendingSource(el: Computed<any>, source: Computed<any>): boolean;
export declare function removePendingSource(el: Computed<any>, source: Computed<any>): boolean;
export declare function clearInFlight(el: Computed<any>): void;
export declare function retryReaches(el: Computed<any>, source: any): boolean;
/** Park a loading-window node on an unready source: registered for the
 * source's settle, with no visible pending and no propagation. */
export declare function parkLoadingWindow(el: Computed<any>, e: NotReadyError): void;
export declare function setPendingError(el: Computed<any>, source?: Computed<any>, error?: any): void;
export declare function forEachDependent(el: Computed<any>, fn: (node: Computed<any>, link: Link) => void): void;
export declare function releaseIfSettledUnobserved(node: Computed<any>): void;
export declare function releaseSettledDependents(el: Computed<any>): void;
/** An error a held node's pass found is an answer of that frame, staged like
 * a value and published at the commit (D-41). Not for a lane pass, nor a
 * node born held. Returns whether it was staged. */
export declare function stageError(el: Computed<any>, error: any, inPass?: boolean): boolean;
export declare function settleErroredDependents(el: Computed<any>, error: any): void;
export declare function settlePendingSource(el: Computed<any>, source?: Computed<any>): void;
export declare function isThenable<T>(value: T | PromiseLike<T>): value is PromiseLike<T>;
export declare function releaseFlightTeardown(el: Computed<any>): void;
/** Internal: run `fn` when the current pass's flight is superseded, with the
 * iterator close, rather than when the replacing pass commits. A no-op
 * outside a computed's own pass. */
export declare function onSupersede(fn: () => void): void;
/** A flight's landing through a setter (bound by store/projection.ts). A throw
 * from `commit` is the flight's error (`fail`, false returned). */
export declare function landBySetter<T>(el: Computed<T>, value: T, commit: (value: T) => void, wasUninitialized: number, fail: (error: unknown) => void): boolean;
export declare function handleAsync<T>(el: Computed<T>, result: T | PromiseLike<T> | AsyncIterable<T>, setter?: (value: T, wasUninitialized: number, fail: (error: unknown) => void) => boolean, onError?: (error: unknown, pending: boolean) => void): T;
export declare function clearStatus(el: Computed<any>, clearUninitialized?: boolean): void;
export declare function notifyStatus(el: Computed<any>, status: number, error?: any): void;
/** The dependents' half of `notifyStatus`. Also used alone for a store
 * family's leaves, which are not the derive's dependents. */
export declare function propagateStatus(el: Computed<any>, status: number, error: any): void;
