// Searching windows, which bento owns: bento keeps the query, takes the typing, and matches the
// query against windows' titles and app ids. The engine places and draws the field, and shows the
// results.
//
// An engine starts a search with `bento.search`, which takes effect here right away and is
// recorded for bento to carry out. bento reports the query and how well each window in scope
// matches in `sync`, and the `SearchSlot` keeps the running search current. `matches` ranks those
// by how well they match, and the windows that match equally in the engine's own order: the order
// of `Group.windows`, `Workspace.windows` or `Bento.windows`. The search ends when bento says so,
// for Enter and Escape and when its scope is gone, or when the engine cancels it.

import { BentoEvent, EventTarget, dispatch } from "./events.js";
import { createSearchField } from "./nodes.js";

export class Search extends EventTarget {
    #slot;
    #d;

    constructor(slot, data) {
        super();
        this.#slot = slot;
        this.#d = data;
    }

    get scope() {
        return this.#d.scope;
    }

    get query() {
        return this.#d.query;
    }

    get matches() {
        return Object.freeze(this.#slot.matchesOf(this.#d));
    }

    get field() {
        return this.#d.field;
    }

    cancel() {
        this.#slot.cancel(this.#d);
    }
}

/** The running search, which is at most one. */
export class SearchSlot {
    #s;
    /** `{ scope, kind, id, query, ranks, field, key, facade }` of the running search, or null. */
    #d = null;

    constructor(session) {
        this.#s = session;
    }

    /** The running search, or null. */
    get facade() {
        return this.#d?.facade ?? null;
    }

    /** What `scope` is, as bento is told: `{ kind: "all" | "workspace" | "group", id }`. */
    #scopeOf(scope) {
        if (scope === null || scope === undefined) {
            return { kind: "all" };
        }
        if (this.#s.groups.facades.includes(scope)) {
            return { kind: "group", id: scope.id };
        }
        if (this.#s.workspaces.get(scope.id)?.facade === scope) {
            return { kind: "workspace", id: scope.id };
        }
        throw new TypeError("the scope of a search must be a group, a workspace or null");
    }

    /** The windows in the search's scope, in the engine's order. */
    #windowsOf(d) {
        if (d.kind === "all") {
            return [...this.#s.windows.values()].map((w) => w.facade);
        }
        return [...d.scope.windows];
    }

    /** The windows in scope that match, best first. */
    matchesOf(d) {
        const windows = this.#windowsOf(d);
        const order = new Map(windows.map((win, i) => [win.id, i]));
        return windows
            .filter((win) => d.ranks.has(win.id))
            .sort((a, b) => d.ranks.get(b.id) - d.ranks.get(a.id) || order.get(a.id) - order.get(b.id));
    }

    #keyOf(d) {
        return JSON.stringify([d.query, this.matchesOf(d).map((win) => win.id)]);
    }

    /** Starts a search, which matches every window in scope until bento reports otherwise. */
    start(scope) {
        const { kind, id } = this.#scopeOf(scope);
        if (this.#d !== null || this.#s.querying) {
            return null;
        }

        const d = { scope: scope ?? null, kind, id, query: "", ranks: new Map(), field: createSearchField(this.#s.log) };
        d.facade = new Search(this, d);
        this.#d = d;
        // With no query every window in scope matches, as bento reports it.
        d.ranks = new Map(this.#windowsOf(d).map((win) => [win.id, 0]));
        d.key = this.#keyOf(d);
        this.#s.rec.ops.push({ op: "search", scope: kind === "all" ? { kind } : { kind, id } });
        return d.facade;
    }

    /** Takes the query and the matches that bento reports. */
    apply(info) {
        const d = this.#d;
        if (d !== null) {
            d.query = info.query;
            d.ranks = new Map(info.hits);
        }
    }

    /**
     * Returns the `change` event to fire if the query or the matches changed since listeners were
     * last told, as a call to make, or null.
     */
    announce() {
        const d = this.#d;
        if (d === null) {
            return null;
        }
        const key = this.#keyOf(d);
        if (key === d.key) {
            return null;
        }
        d.key = key;
        return () => dispatch(d.facade, new BentoEvent("change", d.facade));
    }

    /**
     * Ends the search, which listeners hear. With `pick` it picks the best match. Returns the
     * picked window's id, or null.
     */
    end(pick) {
        const d = this.#d;
        if (d === null) {
            return null;
        }
        const picked = pick ? (this.matchesOf(d)[0] ?? null) : null;
        // It's over for the engine's listeners, which may start another.
        this.#d = null;
        dispatch(d.facade, new BentoEvent("end", d.facade, { picked }));
        // The field is bento's, so it doesn't outlive the search.
        d.field.remove();
        return picked?.id ?? null;
    }

    /** Ends the search for the engine, which bento carries out. */
    cancel(d) {
        if (this.#d !== d || this.#s.querying) {
            return;
        }
        this.#s.rec.ops.push({ op: "endSearch" });
        this.end(false);
    }
}
