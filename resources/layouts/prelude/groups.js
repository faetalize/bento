// Groups of windows, which bento owns: a group is a set of windows and which one is active.
//
// bento reports every group in `sync`, and the `GroupSet` keeps them current. Engines change them
// with method calls, which take effect here right away and are recorded for bento to carry out in
// order with the engine's other calls. Both sides follow the same rules, in `src/layout/groups.rs`
// for bento: a window is in one group at most, a group with fewer than two windows is gone, and
// when the active window leaves, the one that takes its place in the order is active, or the last
// one if it was last. Engines hear about groups through events, whoever changed them.

import { BentoEvent, EventTarget, dispatch } from "./events.js";

let nextId = () => {
    throw new Error("bento gave the prelude no way to make group ids");
};

/** Sets where the ids of groups come from, since bento makes groups too. */
export function setGroupIdSource(source) {
    nextId = source;
}

export class Group extends EventTarget {
    #s;
    #d;

    constructor(session, data) {
        super();
        this.#s = session;
        this.#d = data;
    }

    get id() {
        return this.#d.id;
    }

    get windows() {
        return Object.freeze(this.#d.windows.map((id) => this.#s.windows.get(id)?.facade).filter(Boolean));
    }

    get active() {
        return this.#s.windows.get(this.#d.active)?.facade ?? null;
    }

    add(win) {
        const window = this.#s.groups.windowOf(win);
        if (!this.#s.groups.has(this.#d)) {
            return false;
        }
        if (this.#s.querying || this.#d.windows.includes(window.id)) {
            return true;
        }
        this.#s.groups.add(this.#d.id, window.id);
        this.#s.rec.ops.push({ op: "groupAdd", group: this.#d.id, window: window.id });
        return true;
    }

    remove(win) {
        const window = this.#s.groups.windowOf(win);
        if (!this.#s.groups.has(this.#d) || !this.#d.windows.includes(window.id)) {
            return false;
        }
        if (this.#s.querying) {
            return true;
        }
        this.#s.groups.leave(window.id);
        this.#s.rec.ops.push({ op: "groupRemove", group: this.#d.id, window: window.id });
        return true;
    }

    activate(win) {
        const window = this.#s.groups.windowOf(win);
        if (!this.#s.groups.has(this.#d) || !this.#d.windows.includes(window.id)) {
            return false;
        }
        if (this.#s.querying) {
            return true;
        }
        if (this.#d.active !== window.id) {
            this.#d.active = window.id;
            this.#s.rec.ops.push({ op: "groupActivate", group: this.#d.id, window: window.id });
        }
        return true;
    }

    dissolve() {
        if (!this.#s.groups.has(this.#d) || this.#s.querying) {
            return;
        }
        this.#s.groups.dissolve(this.#d.id);
        this.#s.rec.ops.push({ op: "dissolveGroup", group: this.#d.id });
    }
}

/** Every group that bento reports, and the groups that listeners were told about. */
export class GroupSet {
    #s;
    /** Group id to its data: `{ id, windows, active, facade }`, windows by id. */
    #groups = new Map();
    /** Group id to the data and the key of what listeners were last told. */
    #announced = new Map();

    constructor(session) {
        this.#s = session;
    }

    get facades() {
        return [...this.#groups.values()].map((d) => d.facade);
    }

    /** Whether the group is one that exists. */
    has(d) {
        return this.#groups.get(d.id) === d;
    }

    /** The group that a window is in, or null. */
    of(windowId) {
        for (const d of this.#groups.values()) {
            if (d.windows.includes(windowId)) {
                return d.facade;
            }
        }
        return null;
    }

    /** The data of a window that's in the engine; throws for anything else. */
    windowOf(win) {
        const d = this.#s.windows.get(win?.id);
        if (d === undefined || d.facade !== win) {
            throw new Error("not a window of this session");
        }
        return d;
    }

    /** Makes the groups what bento says they are. */
    apply(infos) {
        const ids = new Set(infos.map((info) => info.id));
        for (const id of [...this.#groups.keys()]) {
            if (!ids.has(id)) {
                this.#groups.delete(id);
            }
        }
        for (const info of infos) {
            for (const window of info.windows) {
                if (!this.#s.windows.has(window)) {
                    throw new Error(`group ${info.id} has window ${window}, which is not in the layout engine`);
                }
            }
            let d = this.#groups.get(info.id);
            if (d === undefined) {
                d = { id: info.id };
                d.facade = new Group(this.#s, d);
                this.#groups.set(info.id, d);
            }
            d.windows = [...info.windows];
            d.active = info.active;
        }
    }

    /**
     * Makes the listeners' view of groups current: returns the events to fire for groups that came,
     * changed or went since they were last told, as calls to make.
     */
    announce() {
        const bento = this.#s.bento;
        const event = (target, type, init) => () => dispatch(target, new BentoEvent(type, target, init));
        const fire = [];
        for (const d of this.#groups.values()) {
            const key = JSON.stringify([d.windows, d.active]);
            const known = this.#announced.get(d.id);
            if (known === undefined) {
                fire.push(event(bento, "groupadd", { group: d.facade }));
            } else if (known.key !== key) {
                fire.push(event(d.facade, "change"));
            }
            this.#announced.set(d.id, { d, key });
        }
        for (const [id, known] of [...this.#announced]) {
            if (!this.#groups.has(id)) {
                fire.push(event(known.d.facade, "remove"));
                this.#announced.delete(id);
            }
        }
        return fire;
    }

    /** Groups windows, for `Bento.createGroup`. */
    create(windows, active) {
        if (!Array.isArray(windows)) {
            throw new TypeError("the windows of a group must be an array");
        }
        const members = [];
        for (const win of windows) {
            const id = this.windowOf(win).id;
            if (!members.includes(id)) {
                members.push(id);
            }
        }
        const first = active === undefined || active === null ? undefined : this.windowOf(active).id;
        if (first !== undefined && !members.includes(first)) {
            throw new Error("the active window of a group must be one of its windows");
        }
        // A query changes nothing, so it can't make a group.
        if (members.length < 2 || this.#s.querying) {
            return null;
        }

        const id = nextId();
        const activeId = first ?? members[0];
        for (const window of members) {
            this.leave(window);
        }
        const d = { id, windows: members, active: activeId };
        d.facade = new Group(this.#s, d);
        this.#groups.set(id, d);
        this.#s.rec.ops.push({ op: "createGroup", id, windows: [...members], active: activeId });
        return d.facade;
    }

    /** Adds a window to a group, taking it out of the group it's in. */
    add(id, window) {
        this.leave(window);
        this.#groups.get(id).windows.push(window);
    }

    /** Takes a window out of the group it's in, which is gone when one window is left. */
    leave(window) {
        const d = [...this.#groups.values()].find((d) => d.windows.includes(window));
        if (d === undefined) {
            return;
        }
        const pos = d.windows.indexOf(window);
        d.windows.splice(pos, 1);
        if (d.windows.length < 2) {
            this.#groups.delete(d.id);
        } else if (d.active === window) {
            d.active = d.windows[Math.min(pos, d.windows.length - 1)];
        }
    }

    dissolve(id) {
        this.#groups.delete(id);
    }
}
