// Host side of bento's layout engine API, running next to the engine in the same JS runtime.
//
// bento talks to the engine with JSON calls and gets JSON results back. A call is
// `{ sync, event }`: `sync` brings the objects that the engine sees up to date with what bento
// knows, and `event` is what happened, which this module turns into events on those objects. The
// result says what the engine changed, and what bento should do about the event: whether a
// cancelable one was prevented, or the answer to a query. The node test harness in `test/host.js`
// uses it the same way, so engines behave the same in tests and in bento.
//
// See `bento-layout.d.ts` for the API that engines see, and `v1.js` for API 1 engines.

import { BentoEvent, QueryEvent, UnloadEvent, dispatch, listenerCount, response, setErrorSink } from "./events.js";
import { setGroupIdSource } from "./groups.js";
import { dispatchPointer } from "./nodes.js";
import { Session, heldModifiers } from "./objects.js";
import { ARG_TYPES, ARG_VALUE_TYPES, VALUE_TYPES, checkActions, checkSettings } from "./declare.js";
import { V1Adapter } from "./v1.js";

export { setGroupIdSource };

const API_VERSIONS = [1, 2];

let layoutModule = null;
let meta = null;
let session = null;
let adapter = null;

const V1_VALUE_TYPES = ["boolean", "number", "string"];

/** Sets the engine's module, with its `meta` and default export. Returns the engine's metadata. */
export function load(module) {
    const given = module.meta;
    if (given === null || typeof given !== "object") {
        throw new Error("the layout engine must export `meta`");
    }
    if (!API_VERSIONS.includes(given.api)) {
        throw new Error(
            `the layout engine is for API version ${given.api}, bento has versions ${API_VERSIONS.join(" and ")}`,
        );
    }
    if (typeof given.name !== "string") {
        throw new Error("`meta.name` must be a string");
    }
    if (typeof module.default !== "function") {
        throw new Error("the layout engine must export a default function that creates the engine");
    }
    const api = given.api;
    const valueTypes = api === 1 ? V1_VALUE_TYPES : VALUE_TYPES;
    const argTypes = api === 1 ? ARG_VALUE_TYPES : ARG_TYPES;

    if (api === 2 && given.options !== undefined) {
        throw new Error("`meta.options` is `meta.settings`, as a shell's are");
    }
    const actions = checkActions(given.actions, argTypes);
    // API 1 engines call their settings options.
    const settings = checkSettings("setting", api === 1 ? given.options : given.settings, valueTypes);
    const windowRules = api === 1 ? [] : checkSettings("window rule", given.windowRules, valueTypes);
    if (given.floating !== undefined && typeof given.floating !== "boolean") {
        throw new Error("`meta.floating` must be a boolean");
    }
    if (given.minimize !== undefined && typeof given.minimize !== "boolean") {
        throw new Error("`meta.minimize` must be a boolean");
    }
    if (given.freeDrag !== undefined && typeof given.freeDrag !== "boolean") {
        throw new Error("`meta.freeDrag` must be a boolean");
    }

    layoutModule = module;
    // Action arguments by name, as declared, for the actions as they come.
    meta = { ...given, actions: Object.fromEntries(actions.map((action) => [action.name, action])) };
    session = null;
    adapter = null;

    return JSON.stringify({
        name: given.name,
        api,
        description: typeof given.description === "string" ? given.description : null,
        actions,
        settings,
        windowRules,
        floating: given.floating !== false,
        minimize: given.minimize === true,
        freeDrag: given.freeDrag === true,
    });
}

function describe(e) {
    return e instanceof Error ? `${e.message}\n${e.stack ?? ""}` : String(e);
}

/** Handles one call from bento, given and returned as JSON. */
export function handle(callJson) {
    const call = JSON.parse(callJson);
    const errors = [];
    setErrorSink((e) => errors.push(describe(e)));

    let result = null;
    let isQuery = false;
    let startFailed = false;
    try {
        if (call.event.type === "create") {
            create(call);
        } else {
            if (session === null) {
                throw new Error("no layout engine instance");
            }
            // A window leaving takes it out of its group, so the engine hears that the window
            // left before it hears what that did to the group.
            const { groups, ...sync } = call.sync ?? {};
            const groupsAfter = call.event.type === "windowremove" ? groups : undefined;
            if (call.sync) {
                const first = groupsAfter === undefined ? call.sync : sync;
                for (const fire of session.applySync(first)) {
                    fire();
                }
            }
            isQuery = call.event.type === "newwindowsize";
            const focus = session.focusKey();
            // The engine can take the keyboard only for what the person does.
            session.personStarted = isPersonStarted(call.event);
            try {
                result = deliver(call.event);
            } finally {
                session.personStarted = false;
            }
            if (groupsAfter !== undefined) {
                session.groups.apply(groupsAfter);
            }
            if (!isQuery) {
                session.fireFocusChanges(focus);
                session.fireGroupChanges();
                session.fireKeyboardRelease();
                session.fireSearchChanges();
            }
        }
    } catch (e) {
        errors.push(describe(e));
        // An engine that throws while it starts doesn't run.
        startFailed = call.event.type === "create";
    }

    let out;
    if (session === null) {
        out = { logs: [] };
    } else if (isQuery) {
        // Queries change nothing, whatever the engine did.
        out = { logs: session.rec.logs };
        session.discardOutput();
    } else {
        out = session.takeOutput();
    }
    out.result = result;
    out.error = errors.length > 0 ? errors.join("\n") : null;
    out.startFailed = startFailed;
    return JSON.stringify(out);
}

/** Creates the engine for the session, and adds the windows that are already there. */
function create(call) {
    const event = call.event;
    session = new Session(meta, event.settings, event.savedState);
    session.setModifiers(event.modifiers);
    adapter = null;
    // The engine learns about what's there when it looks, not from events. Groups need the windows.
    const { groups, ...sync } = call.sync;
    session.applySync(sync);

    const windows = event.windows.map((info) => {
        const d = session.addWindow(info);
        if (meta.api === 1) {
            d.rect = info.rect ? Object.freeze({ ...info.rect }) : null;
        }
        return { d, info };
    });

    session.groups.apply(groups ?? []);

    if (meta.api === 1) {
        adapter = new V1Adapter(session, layoutModule);
        adapter.start();
        adapter.adopt(windows.map(({ d }) => d.facade));
        return;
    }

    layoutModule.default(session.bento);
    const focus = session.focusKey();
    for (const { d, info } of windows) {
        dispatch(
            session.bento,
            new BentoEvent("windowadd", session.bento, {
                window: d.facade,
                reason: "adopt",
                activate: d.id === event.focused,
                rect: info.rect ?? null,
                pointer: null,
                nextTo: null,
            }),
        );
    }
    session.fireFocusChanges(focus);
    // Groups that are there are announced after the windows in them.
    session.fireGroupChanges();
    session.fireKeyboardRelease();
}

/**
 * Saves the engine's state, before bento loads another version of the engine. Returns it as JSON,
 * which is `null` if the engine has none.
 */
export function saveAll() {
    if (session === null) {
        return "null";
    }
    const event = new UnloadEvent("unload", session.bento);
    setErrorSink(() => {});
    dispatch(session.bento, event);
    return JSON.stringify(response(event)?.value ?? null);
}

/** The state the engine exposes, as JSON, which is `null` if it exposes none. */
export function exposedState() {
    if (session === null || session.stateProvider === null) {
        return "null";
    }
    const state = session.stateProvider();
    return JSON.stringify(state === undefined ? null : state);
}

function mustWindow(id) {
    const d = session.windows.get(id);
    if (d === undefined) {
        throw new Error(`window ${id} is not in the layout engine`);
    }
    return d.facade;
}

function mustWorkspace(id) {
    const d = session.workspaces.get(id);
    if (d === undefined) {
        throw new Error(`workspace ${id} doesn't exist`);
    }
    return d.facade;
}

const edgesOf = (edges) => ({ left: false, right: false, top: false, bottom: false, ...edges });

/** Turns an action's arguments into what the engine declared, see `ArgSpec`. */
function actionArgs(name, args) {
    const declared = meta.actions[name]?.args ?? [];
    return args.map((arg, i) => {
        switch (declared[i]?.type) {
            case "workspace":
                return session.workspaces.get(arg)?.facade ?? null;
            case "monitor":
                return session.monitors.get(arg)?.facade ?? null;
            default:
                return arg;
        }
    });
}

/** Turns an event from bento into events on the objects, and returns what bento needs to know. */
/** Whether an event is something the person did: an action, a key, or a press on a node. */
function isPersonStarted(event) {
    switch (event.type) {
        case "action":
        case "keydown":
            return true;
        case "pointer":
            return event.event === "pointerdown" || event.event === "click";
        default:
            return false;
    }
}

function deliver(event) {
    const bento = session.bento;
    switch (event.type) {
        case "sync":
            return null;
        case "windowadd": {
            const d = session.addWindow(event.window);
            const how = event.how;
            dispatch(
                bento,
                new BentoEvent("windowadd", bento, {
                    window: d.facade,
                    reason: how.reason,
                    activate: how.activate,
                    rect: how.rect ?? null,
                    pointer: how.pointer ?? null,
                    nextTo: how.nextTo === null || how.nextTo === undefined ? null : (bento.window(how.nextTo) ?? null),
                }),
            );
            return null;
        }
        case "windowremove": {
            const win = mustWindow(event.id);
            dispatch(win, new BentoEvent("remove", win, { reason: event.reason }));
            session.deleteWindow(session.windows.get(event.id));
            return null;
        }
        case "windowmove": {
            const win = mustWindow(event.id);
            const to = session.workspaces.get(event.to);
            session.relocateWindow(session.windows.get(event.id), to);
            dispatch(
                win,
                new BentoEvent("move", win, {
                    from: mustWorkspace(event.from),
                    to: to.facade,
                    activate: event.activate,
                }),
            );
            return null;
        }
        case "moderequest": {
            const win = mustWindow(event.id);
            const e = dispatch(
                win,
                new BentoEvent("moderequest", win, { mode: event.mode, request: { ...event.request } }),
            );
            if (!e.defaultPrevented) {
                win.setMode(event.mode);
            }
            return { prevented: e.defaultPrevented };
        }
        case "activaterequest": {
            const win = mustWindow(event.id);
            const e = dispatch(win, new BentoEvent("activaterequest", win));
            return { prevented: e.defaultPrevented };
        }
        case "floatrequest": {
            const win = mustWindow(event.id);
            dispatch(
                win,
                new BentoEvent("floatrequest", win, {
                    reason: event.reason,
                    parent: event.parent === null ? null : (bento.window(event.parent) ?? null),
                    size: event.size ?? null,
                }),
            );
            return null;
        }
        case "minimizerequest": {
            const win = mustWindow(event.id);
            dispatch(win, new BentoEvent("minimizerequest", win));
            return null;
        }
        case "resizebegin": {
            const win = mustWindow(event.id);
            const handled = listenerCount(win, "resizebegin") > 0;
            const e = dispatch(
                win,
                new BentoEvent("resizebegin", win, { edges: edgesOf(event.edges), delta: { ...event.delta } }),
            );
            return { handled, prevented: e.defaultPrevented };
        }
        case "resizeupdate": {
            const win = mustWindow(event.id);
            dispatch(
                win,
                new BentoEvent("resizeupdate", win, { edges: edgesOf(event.edges), delta: { ...event.delta } }),
            );
            return null;
        }
        case "resizeend": {
            const win = mustWindow(event.id);
            dispatch(win, new BentoEvent("resizeend", win));
            return null;
        }
        case "edgedoubleclick": {
            const win = mustWindow(event.id);
            dispatch(win, new BentoEvent("edgedoubleclick", win, { edges: edgesOf(event.edges) }));
            return null;
        }
        case "camerachange": {
            const ws = mustWorkspace(event.workspace);
            dispatch(ws, new BentoEvent("camerachange", ws));
            return null;
        }
        case "dragenter":
        case "dragmove":
        case "dragleave": {
            const ws = mustWorkspace(event.workspace);
            const type = event.type;
            if (type === "dragleave") {
                dispatch(ws, new BentoEvent(type, ws));
            } else {
                dispatch(
                    ws,
                    new BentoEvent(type, ws, {
                        window: session.detachedWindow(event.window),
                        pointer: { ...event.pointer },
                        rect: { ...event.rect },
                    }),
                );
            }
            return null;
        }
        case "swipebegin":
        case "swipeupdate":
        case "swipeend": {
            const ws = mustWorkspace(event.workspace);
            dispatch(
                ws,
                new BentoEvent(event.type, ws, {
                    delta: event.delta,
                    velocity: event.velocity,
                    touchpad: event.touchpad,
                }),
            );
            return null;
        }
        case "searchend":
            return { picked: session.search.end(event.pick) };
        case "action": {
            const window = event.window === null || event.window === undefined ? null : mustWindow(event.window);
            dispatch(
                bento,
                new BentoEvent("action", bento, {
                    name: event.name,
                    args: Object.freeze(actionArgs(event.name, event.args)),
                    window,
                }),
            );
            return null;
        }
        case "newwindowsize": {
            return session.query(() => {
                const ws = mustWorkspace(event.workspace);
                const e = dispatch(
                    bento,
                    new QueryEvent("newwindowsize", bento, {
                        workspace: ws,
                        rules: Object.freeze({ ...event.rules }),
                        minSize: Object.freeze({ ...event.minSize }),
                        maxSize: Object.freeze({ ...event.maxSize }),
                        borders: Object.freeze({ ...event.borders }),
                    }),
                );
                const answer = response(e)?.value;
                if (answer === undefined) {
                    return { w: 0, h: 0 };
                }
                if (!answer || !Number.isFinite(answer.w) || !Number.isFinite(answer.h)) {
                    throw new Error(`invalid new window size: ${JSON.stringify(answer)}`);
                }
                return { w: Math.max(0, answer.w), h: Math.max(0, answer.h) };
            });
        }
        case "timer":
            session.fireTimer(event.id);
            return null;
        case "keydown":
        case "keyup": {
            if (!session.keyboardGrabbed) {
                return { prevented: false };
            }
            const e = dispatch(
                bento,
                new BentoEvent(event.type, bento, {
                    key: event.key,
                    text: event.text,
                    modifiers: heldModifiers(event.modifiers),
                    repeat: event.repeat,
                }),
            );
            return { prevented: e.defaultPrevented };
        }
        case "keyboardrelease":
            if (session.endGrab()) {
                dispatch(bento, new BentoEvent("keyboardrelease", bento, { reason: event.reason }));
            }
            return null;
        case "modifierschange":
            session.setModifiers(event.modifiers);
            dispatch(bento, new BentoEvent("modifierschange", bento, { modifiers: session.modifiers }));
            return null;
        case "pointer": {
            const node = session.log.nodes.get(event.node);
            if (node !== undefined) {
                // A press is when the engine can begin a move or resize with the pointer.
                session.pressing = event.event === "pointerdown";
                try {
                    dispatchPointer(node, event);
                } finally {
                    session.pressing = false;
                }
            }
            return null;
        }
        default:
            throw new Error(`unknown event: ${event.type}`);
    }
}
