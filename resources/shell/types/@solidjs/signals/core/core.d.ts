import type { Transaction } from "./scheduler.js";
import type { Computed, NodeExtension, NodeOptions, Owner, Signal } from "./types.js";
export declare const PRIMITIVE_IN_FORBIDDEN_SCOPE_MESSAGE = "[PRIMITIVE_IN_FORBIDDEN_SCOPE] Cannot create reactive primitives inside createTrackedEffect or owner-backed onSettled";
export declare const PRIMITIVE_IN_EFFECT_CALLBACK_MESSAGE: string;
export declare const PRIMITIVE_IN_REF_CALLBACK_MESSAGE: string;
/** Dev: a computed or root created ownerless in an effect callback is outside
 * every owner and hold. `runWithOwner` there is deliberate and allowed. */
export declare function assertNotInEffectCallback(): void;
export declare const REACTIVE_WRITE_IN_OWNED_SCOPE_SIGNAL_MESSAGE: string;
export declare const REACTIVE_WRITE_IN_OWNED_SCOPE_REFRESH_MESSAGE: string;
export declare let tracking: boolean;
/** @internal verdict-module glue */
export declare function setContextInternal(v: Owner | null): void;
export declare let stale: boolean;
export declare let context: Owner | null;
export declare let snapshotCaptureActive: boolean;
export declare let snapshotSources: Set<any> | null;
export declare function setSnapshotCapture(active: boolean): void;
/** Mark `owner` a snapshot scope: readers created under it serve each
 * source's value from when hydration began and replay at
 * `releaseSnapshotScope`. With `served`, `owner` is a hydrated computed and
 * `served` its server answer: it becomes the node's snapshot if it has none. */
export declare function markSnapshotScope(owner: Owner, served?: unknown): void;
export declare function releaseSnapshotScope(owner: Owner): void;
export declare function clearSnapshots(): void;
export declare function recompute(el: Computed<any>, create?: boolean): void;
export declare function computed<T>(fn: (prev?: T) => T | PromiseLike<T> | AsyncIterable<T>): Computed<T>;
export declare function computed<T>(fn: (prev: T) => T | PromiseLike<T> | AsyncIterable<T>, options?: NodeOptions<T>): Computed<T>;
/** Lazily allocate the cold extension. Hot paths read `el._x?._field` behind
 * `_config` bits; never call `ext()` just to store a default. */
export declare function ext(el: {
    _x: NodeExtension | null;
}): NodeExtension;
/** A tracked read of a held node joins the flush to its hold (D-1), unless the
 * reader is a render effect: the frame reads the committed value outside its
 * hold's flush (D-25, `frameRead`). A node born held has no committed value,
 * so every reader joins. Under a boundary that has not shown content, a first
 * pass is the boundary's (F-4, #3540). */
/** The pass whose plain read made this flush adopt a hold: a window it then
 * opens can give the hold back (#4777). */
export declare let adopter: Computed<any> | null;
export declare function setAdopter(c: Computed<any> | null): void;
export declare function joinPass(c: Computed<any>, el: Signal<any> | Computed<any>): void;
/** `joinPass` into hold `t` (a folded guess, D-34); never a render effect. */
export declare function joinPassAt(c: Computed<any>, t: Transaction): void;
/** Born into `t`: held and never committed, or a member of a frame its held
 * pass built. It reads `t`'s values in any flush and keeps staging (#4010). */
export declare function bornInto(c: Computed<any>, t: Transaction): unknown;
/** The computed a creation under `owner` belongs to, or null. */
export declare function creatorPass(owner: Owner | null): Computed<any> | null;
/** Whether the passes running around `p` read the future or a staging: a pass
 * they created has no screen (#4341). */
export declare function creatorReads(p: Computed<any> | null, born: number): number;
/** One literal with every effect field, so V8 sees the full shape at construction. */
export declare function createEffectNode<T>(fn: (prev?: T) => T, effectFn: (val: T, prev: T | undefined) => void | (() => void), errorFn: ((err: unknown, cleanup: () => void) => void | (() => void)) | undefined, type: number, options: NodeOptions<T> | undefined): any;
/** The shared effect status notifier, installed by effect.ts. */
export declare let effectStatusNotify: ((this: any) => void) | null;
export declare function setEffectStatusNotify(fn: NonNullable<typeof effectStatusNotify>): void;
/** Effect nodes get the shared notifier; presence also marks a display consumer. */
export declare function statusNotifierOf(el: any): ((this: any) => void) | null;
export declare function signal<T>(v: T, options?: NodeOptions<T>): Signal<T>;
export declare function isEqual<T>(a: T, b: T): boolean;
/** A store slot node in one literal: target and key as back-refs, the wrap
 * cache (`px`/`pxv`) and accessor verdict pre-shaped. No options, no `_x`,
 * no expandos. Observe adds label and location slots. */
export declare function slotSignal<T>(v: T, equals: (a: T, b: T) => boolean, host: object, key: PropertyKey, acc: boolean): Signal<T>;
/** Dev: when set to a component name, a reactive read outside a nested
 * tracking scope warns. Managed by `untrack(fn, strictReadLabel)`. */
/** The verdict windows' read, set while a window is open. */
export declare let verdict: ((el: Signal<any> | Computed<any>, c: Computed<any> | null) => unknown) | null;
export declare function setVerdict(fn: typeof verdict): void;
export declare let strictRead: string | false;
export declare let untrackDepth: number;
/** Dev: > 0 while user code that is imperative by construction runs (effect
 * callback, action slice), so the post-await check does not blame the caller. */
export declare let callbackDepth: number;
export declare function enterCallback(): void;
export declare function exitCallback(): void;
/** Dev: > 0 while teardown runs cleanups: their writes are imperative too, so
 * the owned-scope guard stands down (#4165). No try/finally here (prod), so
 * dev.ts resets it on the next microtask. */
export declare let disposalDepth: number;
export declare function enterDisposal(): void;
export declare function exitDisposal(): void;
export declare function resetDisposalDepth(): void;
/** Dev: the event current when the strict-read window opened, or `ANY_EVENT`.
 * A handler dispatched synchronously from an effect callback is a snapshot,
 * so a read under a different event is outside the window (#4235). Read
 * once per queue: the getter is costly under jsdom. */
export declare let strictReadEvent: unknown;
export declare const dispatchingEvent: () => unknown;
export declare function setStrictRead(v: string | false, event?: unknown): string | false;
/** Dev-only: runs `fn` inside the strict-read window labelled `label`, tracking
 * unchanged. Compiled dev DOM output wraps event handler expressions in it
 * (#1742). Production builds just call `fn`. */
export declare function strictReadTracked<T>(fn: () => T, label?: string): T;
export declare function strictReadActive(): boolean;
/**
 * Runs `fn` outside of any reactive tracking — reads inside `fn` will not
 * subscribe the current scope. Returns whatever `fn` returns.
 *
 * Use `untrack` inside a memo or effect when you need to read a signal once
 * without making the surrounding computation depend on its future changes.
 *
 * In a component body, `untrack` marks a read as a deliberate one-time
 * snapshot and silences `STRICT_READ_UNTRACKED`. Use it only when the value
 * must never update and is never downstream of async: an async-derived value
 * can still be pending (the read throws) or mid-transition (the snapshot is
 * the old committed value). When it should follow its source, move the read
 * to where the value is used (JSX, a memo, an effect's compute, the event
 * handler, or an accessor passed to the hook that needs it) instead.
 *
 * `untrack` stops dependency tracking, not async settlement. Reading an
 * uninitialized async source still suspends the owning computation; it runs
 * again when that source first resolves. Afterward, the untracked read is a
 * snapshot and does not subscribe the computation to later changes.
 *
 * If the first read must return immediately, give the async source a
 * `loadingValue` before reading it with `untrack`. `isPending` and `latest`
 * inspect updates to an existing value; neither supplies a missing initial
 * value.
 *
 * Pass a `strictReadLabel` string to enable a dev-mode warning: any reactive
 * read inside `fn` that isn't inside a nested tracking scope will log a
 * warning naming the label.
 *
 * @example
 * ```ts
 * createEffect(
 *   () => trigger(),                 // tracks `trigger` only
 *   () => {
 *     const snapshot = untrack(() => state); // read once, untracked
 *     log(snapshot);
 *   }
 * );
 *
 * const color = createMemo<string>(() => fetchColor(), { loadingValue: "gray" });
 * const initialColor = untrack(color); // "gray" while the fetch is unresolved
 * ```
 */
export declare function untrack<T>(fn: () => T, strictReadLabel?: string | false): T;
/** A write, refresh, action call or `onFlush` here is an effect callback's
 * imperative work, not a computation's. */
export declare function inRefCallback(): boolean;
/** Runs a compiled `ref()` callback untracked in the creating pass's context
 * (#4761); in dev, creating a primitive there is PRIMITIVE_IN_EFFECT_CALLBACK.
 *
 * @internal */
export declare function runRefCallback(fn: () => void): void;
/** A bookkeeping read on behalf of no node: it records no re-run link (#3528)
 * and no transaction entry (#3540) on `context`. */
export declare let spectating: boolean;
/** Evaluates `fn` untracked as a bookkeeping read (`spectating`). */
export declare function spectate<T>(fn: () => T): T;
export declare function prepareComputed(comp: Computed<unknown>, refresh: boolean): void;
/** Does reader `c` see a staged node's committed value? Read's fast block
 * carries the same test inline. An untracked read and a frame-only reader
 * (createTrackedEffect / onSettled, D-29) see the committed frame. */
export declare function readerSeesCommitted(el: Signal<any> | Computed<any>, c: Computed<any> | null): boolean;
/** Set when a node is staged outside a flush, cleared when a flush begins:
 * one module boolean keeps the F-10 arm free inside a flush. */
export declare let unflushedStaged: boolean;
export declare function markUnflushedStaged(): void;
/** A write is visible from the flush that carries it (F-10): outside a flush
 * a staged node serves its committed value. Promoted writes are exempt. */
export declare function unflushedValue(el: Signal<any> | Computed<any>): unknown;
/** The value a signal had when this tick began writing it (F-10). */
export declare function tickBase(el: Signal<any>): unknown;
/** A held node rewritten outside a flush keeps the last flushed staging until
 * the flush that carries the rewrite (F-10); the clock stamps the stash. */
export declare function stashFlushed(el: Signal<any> | Computed<any>): void;
/** A derivation served the committed value because of an unflushed write runs
 * again in the flush that carries it: it linked after the write walked (F-10, #3337). */
export declare function markLateLinker(c: Computed<any>): true;
export declare function resyncUnflushedCompanions(): void;
export declare function pullComputed(owner: Computed<any>, c: Computed<any>): void;
/** Dev: every read's diagnostics. `nodeValue`'s first landing serves a store
 * leaf without `read`, so it calls this too. */
export declare function devRead(el: Signal<any> | Computed<any>): void;
/** @internal Compiler output (`@solidjs/web`): truthy while the running pass is
 * a stale reader of an open hold (D-9). A grouped stateful prop binding then
 * skips an unchanged write, so a sibling's re-run does not put the committed
 * value back over the field (#4872). */
export declare function heldRead(): number;
export declare function read<T>(el: Signal<T> | Computed<T>): T;
/** The value reader `c` (null: untracked) is served from `el`; read's fast
 * block keeps its trivial ternary for speed. */
export declare function serve(el: Signal<any> | Computed<any>, c: Computed<any> | null): unknown;
/** A pass read a staging of this flush: the seam decides (D-25). Lane work
 * too: if the flush parks, the pass re-derives on the committed world and
 * its lane waits that round, so the held write never shows through it. A
 * folded lane only before the frame has a transaction (D-34). Not a node
 * born staged: a re-run would read the same staging every round (#3969). */
export declare function stagedRead(c: Computed<any>, el?: Signal<any> | Computed<any>): void;
/** A verdict reader's plain read of a staged node sees the screen once the
 * frame has a transaction, and re-derives at its landing (D-25, #3322).
 * Before that the seam answers. A node born into the future (F-7), a reader
 * born into the transaction (#4010) or into the hold (D-25) has no screen. */
export declare function stagedScreen(c: Computed<any>, el?: Signal<any> | Computed<any>): boolean;
export declare function ownedScopeWriteMessage(owner: Owner): string;
/** A write inside a recompute outside a flush is promoted at that pass's end. */
export declare function notePromotedWrite(el: Signal<any> | Computed<any>): void;
/** The first write during snapshot capture to a plain user signal records the
 * pre-write value. Async landings reveal, and projection leaves are their
 * compute's, so neither qualifies. */
export declare function captureWriteSnapshot<T>(el: Signal<T> | Computed<T>, current: T): void;
/** Called before every signal or store write while set. The server-components
 * client mounts still-deferred slots before a write applies, then clears it. */
export declare let writeHook: ((node?: object) => void) | null;
/** @internal */
export declare function setWriteHook(hook: ((node?: object) => void) | null): void;
/** The one guarded comparator call. A comparator that read unresolved async
 * reports a change (#4111); any other throw is `el`'s error, routed to its
 * boundary and answered "same" (D-38, #2837).
 * @internal */
export declare function isSame(el: Signal<any>, a: unknown, b: unknown): unknown;
/** Observe: the landing reports this write as `asyncLanded`, not `write`. */
export declare function landThroughSetSignal(el: Computed<any>, value: unknown): boolean;
export declare function setSignal<T>(el: Signal<T> | Computed<T>, v: T | ((prev: T) => T)): T;
/** User-facing setter for the memo form of `createSignal(fn)`. The write
 * stages like any other, and a later source change re-derives with it as
 * `prev` (D-8, CS-R31). REACTIVE_MANUAL_WRITE marks it a user proposal until
 * a pass re-derives it or the commit lands it. */
export declare function setMemo<T>(el: Computed<T>, v: T | ((prev: T) => T)): T;
/**
 * Executes `fn` with the given `owner` set as the current owner. Any reactive
 * primitives (`createSignal`, `createMemo`, `createEffect`, `onCleanup`,
 * `cleanup`, etc.) created inside `fn` are attached to that owner, so they
 * are disposed when the owner is disposed.
 *
 * The classic pattern: capture the current owner with `getOwner()` inside a
 * component, then re-enter it from a callback (event handler, async resolve,
 * setTimeout) so disposables created in the callback get cleaned up with the
 * component.
 *
 * @example
 * ```ts
 * function delayed<T>(ms: number, fn: () => T) {
 *   const owner = getOwner();
 *   setTimeout(() => runWithOwner(owner, fn), ms);
 * }
 * ```
 */
export declare function runWithOwner<T>(owner: Owner | null, fn: () => T): T;
export declare function staleValues<T>(fn: () => T, set?: boolean): T;
/** Core half of `refresh()` (signals.ts): marks the next recompute a quiet
 * re-ask and schedules it. */
export declare function markRefresh(node: Computed<any>): void;
