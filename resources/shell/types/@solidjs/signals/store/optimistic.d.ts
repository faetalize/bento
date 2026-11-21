import type { Refreshable } from "../core/index.js";
import { type StoreTarget } from "./target.js";
import { type NoArray, type NoFn, type ProjectionOptions, type Store, type StoreOptions, type StoreSetter } from "./types.js";
export declare function notifyOptimisticWrites(t: StoreTarget, pb: Record<PropertyKey, any>): Record<PropertyKey, any> | null;
export declare function sweepOverlaid(): void;
export declare function createOptimisticStore<T extends object = {}>(initialValue: NoFn<T> | Store<NoFn<T>>, options?: StoreOptions): [get: Store<T>, set: StoreSetter<T>];
export declare function createOptimisticStore<T extends object = {}>(fn: (() => T | Promise<T> | AsyncIterable<T>) & NoArray<T>, seed?: null, options?: Omit<ProjectionOptions, "seedLoadingValue">): [get: Refreshable<Store<T>>, set: StoreSetter<T>];
export declare function createOptimisticStore<T extends object = {}>(fn: (draft: T) => void | T | Promise<void | T> | AsyncIterable<void | T>, seed: NoFn<T> | Store<NoFn<T>>, options?: ProjectionOptions): [get: Refreshable<Store<T>>, set: StoreSetter<T>];
