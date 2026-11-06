// What engines draw: a tree of nodes per workspace, see `Node` in `bento-layout.d.ts`.
//
// The tree lives here, and bento gets the changes as an ordered list of ops:
//
//     { op: "create", id, kind, props }       a node enters a tree for the first time, or again
//                                             after it was removed
//     { op: "set", id, props }                changed properties
//     { op: "append", parent, child, before } puts a node last in a parent, or before another child,
//                                             moving it from wherever it was
//     { op: "remove", id }                    bento drops the node and its subtree
//
// Ids are "n<number>" for nodes the engine creates, "w<window id>" for window nodes and
// "r<workspace id>" for the root of a workspace's tree. bento creates window nodes and roots
// itself, so the ops never create them. The field of a search is a created node, of kind "search".
// Only nodes that are in a workspace's tree are known to bento, and a node that's taken out of one
// is forgotten, and created again when it comes back.

import { BentoEvent, EventTarget, dispatch, isPropagationStopped } from "./events.js";

export function isRect(rect) {
    return (
        rect !== null &&
        typeof rect === "object" &&
        [rect.x, rect.y, rect.w, rect.h].every(Number.isFinite) &&
        rect.w >= 0 &&
        rect.h >= 0
    );
}

/** The ops of the call, and the nodes bento knows about. */
export class TreeLog {
    constructor() {
        this.ops = [];
        /** Nodes bento knows about, by id. */
        this.nodes = new Map();
        this.nextId = 1;
        /** Whether a query is running, which changes nothing. */
        this.querying = false;
    }

    record(op) {
        if (!this.querying) {
            this.ops.push(op);
        }
    }

    take() {
        const ops = this.ops;
        this.ops = [];
        return ops;
    }
}

function checkProps(props) {
    if (props === null || typeof props !== "object") {
        throw new TypeError("node properties must be an object");
    }
    if (props.rect !== undefined && !isRect(props.rect)) {
        throw new Error(`invalid rect: ${JSON.stringify(props.rect)}`);
    }
    for (const name of ["opacity", "cornerRadius"]) {
        if (props[name] !== undefined && !Number.isFinite(props[name])) {
            throw new Error(`invalid ${name}: ${JSON.stringify(props[name])}`);
        }
    }
    if (props.placeholder !== undefined && typeof props.placeholder !== "string") {
        throw new Error(`invalid placeholder: ${JSON.stringify(props.placeholder)}`);
    }
}

function copyValue(value) {
    return value === undefined ? undefined : JSON.parse(JSON.stringify(value));
}

function sameValue(a, b) {
    return JSON.stringify(a) === JSON.stringify(b);
}

/** Something drawn on a workspace: a box, text, an image, a window, or a search's field. */
export class Node extends EventTarget {
    #log;
    #id;
    #kind;
    #props;
    #parent = null;
    #children = [];
    #window = null;
    /** Whether bento knows about the node. */
    #sent;

    constructor(log, id, kind, props, sent) {
        super();
        this.#log = log;
        this.#id = id;
        this.#kind = kind;
        this.#props = copyValue(props);
        this.#sent = sent;
        if (sent) {
            log.nodes.set(id, this);
        }
    }

    get id() {
        return this.#id;
    }

    get kind() {
        return this.#kind;
    }

    get window() {
        return this.#window;
    }

    get parent() {
        return this.#parent;
    }

    get children() {
        return Object.freeze(this.#children.slice());
    }

    get props() {
        return Object.freeze({ ...this.#props });
    }

    /** Changes some properties. Those with a transition animate to their new value. */
    set(props) {
        checkProps(props);
        const changed = {};
        for (const [name, value] of Object.entries(props)) {
            if (value !== undefined && !sameValue(this.#props[name], value)) {
                changed[name] = copyValue(value);
            }
        }
        if (Object.keys(changed).length === 0 || this.#log.querying) {
            return;
        }
        Object.assign(this.#props, changed);
        if (!this.#sent) {
            return;
        }
        const last = this.#log.ops.at(-1);
        if (last?.op === "set" && last.id === this.#id) {
            Object.assign(last.props, changed);
        } else {
            this.#log.record({ op: "set", id: this.#id, props: changed });
        }
    }

    /** Adds a child last, or before `before`, moving it from wherever it was. */
    append(child, before = null) {
        if (!(child instanceof Node)) {
            throw new TypeError("only a node can be added to a node");
        }
        if (child.#kind === "root") {
            throw new Error("a workspace's root can't be added to a node");
        }
        if (before !== null && before !== undefined) {
            if (!(before instanceof Node) || before.#parent !== this) {
                throw new Error("`before` must be a child of the node");
            }
        } else {
            before = null;
        }
        for (let node = this; node !== null; node = node.#parent) {
            if (node === child) {
                throw new Error("a node can't be added to itself or to its own subtree");
            }
        }
        const siblings = this.#children;
        const at = siblings.indexOf(child);
        const inPlace = at !== -1 && (before === null ? at === siblings.length - 1 : siblings[at + 1] === before);
        if (child === before || inPlace || this.#log.querying) {
            return;
        }

        const live = this.#isLive();
        if (!live && child.#hasWindows()) {
            throw new Error("a window's node can only be in a workspace's tree");
        }
        if (child.#hasWindows() && this.#root() !== child.#root()) {
            throw new Error("a window's node stays in its workspace's tree; move the window with moveTo");
        }

        const old = child.#parent;
        if (old !== null) {
            old.#children.splice(old.#children.indexOf(child), 1);
        }
        child.#parent = this;
        this.#children.splice(
            before === null ? this.#children.length : this.#children.indexOf(before),
            0,
            child,
        );

        if (live) {
            child.#publish(this, before);
        } else if (child.#sent) {
            // It leaves the tree, so bento drops it.
            this.#log.record({ op: "remove", id: child.#id });
            child.#forget();
        }
    }

    /** Takes it out of the tree. A window's node can't be removed, only moved. */
    remove() {
        if (this.#kind === "window" || this.#kind === "root") {
            throw new Error(`a ${this.#kind}'s node can't be removed, only moved`);
        }
        if (this.#hasWindows()) {
            throw new Error("a node with windows in it can't be removed, move the windows first");
        }
        const parent = this.#parent;
        if (parent === null || this.#log.querying) {
            return;
        }
        parent.#children.splice(parent.#children.indexOf(this), 1);
        this.#parent = null;
        if (this.#sent) {
            this.#log.record({ op: "remove", id: this.#id });
            this.#forget();
        }
    }

    /** Whether the node is in a workspace's tree. */
    #isLive() {
        return this.#root().#kind === "root";
    }

    #root() {
        let node = this;
        while (node.#parent !== null) {
            node = node.#parent;
        }
        return node;
    }

    #hasWindows() {
        return this.#kind === "window" || this.#children.some((child) => child.#hasWindows());
    }

    /** Tells bento about the node and the part of its subtree that bento doesn't know. */
    #publish(parent, before) {
        if (!this.#sent) {
            this.#sent = true;
            this.#log.nodes.set(this.#id, this);
            this.#log.record({
                op: "create",
                id: this.#id,
                kind: this.#kind,
                props: copyValue(this.#props),
            });
        }
        this.#log.record({
            op: "append",
            parent: parent.#id,
            child: this.#id,
            before: before === null ? null : before.#id,
        });
        for (const child of this.#children) {
            if (!child.#sent) {
                child.#publish(this, null);
            }
        }
    }

    /** Marks the node and its subtree as unknown to bento, after bento dropped them. */
    #forget() {
        this.#sent = false;
        this.#log.nodes.delete(this.#id);
        for (const child of this.#children) {
            child.#forget();
        }
    }

    /** Takes the node out of its parent without telling bento, for a window that's gone. */
    static detach(node) {
        const parent = node.#parent;
        if (parent !== null) {
            parent.#children.splice(parent.#children.indexOf(node), 1);
            node.#parent = null;
        }
        node.#forget();
    }

    /** Puts a window's node last in `parent`, without telling bento. */
    static attach(parent, node) {
        const old = node.#parent;
        if (old !== null) {
            old.#children.splice(old.#children.indexOf(node), 1);
        }
        node.#parent = parent;
        parent.#children.push(node);
    }

    /** Sets the node's window, for a window's node. */
    static bindWindow(node, window) {
        node.#window = window;
    }
}

/** A node that engines draw. It's in no tree until it's added to one. */
export function createNode(log, kind, props = {}) {
    if (!["box", "text", "image"].includes(kind)) {
        throw new Error(`unknown node kind: ${kind}`);
    }
    checkProps(props);
    return new Node(log, `n${log.nextId++}`, kind, props, false);
}

/** The field of a search, which bento creates with the search. The engine puts it in a tree. */
export function createSearchField(log) {
    return new Node(log, `n${log.nextId++}`, "search", {}, false);
}

/** The root of a workspace's tree, which bento creates with the workspace. */
export function createRoot(log, workspaceId) {
    return new Node(log, `r${workspaceId}`, "root", {}, true);
}

/** A window's node, which bento creates with the window. */
export function createWindowNode(log, window) {
    const node = new Node(log, `w${window.id}`, "window", {}, true);
    Node.bindWindow(node, window);
    return node;
}

/** Puts a window's node last in `root`, without telling bento: bento does the same. */
export function attachWindowNode(root, node) {
    Node.attach(root, node);
}

/** Takes a window's node out of its tree without telling bento, when its window is gone. */
export function detachNode(node) {
    Node.detach(node);
}

/** Windows of a tree, in tree order, which is from the bottom to the top. */
export function windowsOf(root) {
    const windows = [];
    const visit = (node) => {
        if (node.window !== null) {
            windows.push(node.window);
        }
        node.children.forEach(visit);
    };
    visit(root);
    return windows;
}

/**
 * Delivers a pointer event from bento to the node it hit. It bubbles to the node's parents until a
 * listener stops it, except `pointerenter` and `pointerleave`: bento sends those to every node
 * the pointer enters or leaves.
 */
export function dispatchPointer(target, event) {
    const { type: _call, node: _node, event: type, ...init } = event;
    const pointerEvent = new BentoEvent(type, target, init);
    const bubbles = type !== "pointerenter" && type !== "pointerleave";
    for (let node = target; node !== null; node = bubbles ? node.parent : null) {
        dispatch(node, pointerEvent);
        if (isPropagationStopped(pointerEvent)) {
            break;
        }
    }
    return pointerEvent;
}
