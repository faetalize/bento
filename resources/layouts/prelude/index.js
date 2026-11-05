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
import { dispatchPointer } from "./nodes.js";
import { Session, specTypeName } from "./objects.js";
import { V1Adapter } from "./v1.js";

const API_VERSIONS = [1, 2];

let layoutModule = null;
let meta = null;
let session = null;
let adapter = null;

const VALUE_TYPES = ["boolean", "number", "string", "choice", "size"];
const V1_VALUE_TYPES = ["boolean", "number", "string"];
const ARG_TYPES = [...VALUE_TYPES, "workspace", "monitor"];

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
    const argTypes = api === 1 ? VALUE_TYPES : ARG_TYPES;

    const actions = given.actions ?? {};
    const options = given.options ?? {};
    const windowRules = api === 1 ? {} : (given.windowRules ?? {});
    const actionArgs = new Map(
        Object.entries(actions).map(([name, spec]) => [name, checkArgs(name, spec?.args, argTypes)]),
    );
    checkSpecs("option", options, valueTypes);
    checkSpecs("window rule", windowRules, valueTypes);
    if (given.floating !== undefined && typeof given.floating !== "boolean") {
        throw new Error("`meta.floating` must be a boolean");
    }

    layoutModule = module;
    meta = { ...given, actions, options, windowRules };
    session = null;
    adapter = null;

    // Descriptions are only documentation, so a malformed one is dropped instead of failing the
    // load. Lists keep the order the engine declares things in, for settings screens.
    const text = (value) => (typeof value === "string" ? value : null);
    const specJson = ([name, spec]) => ({
        name,
        type: spec.type,
        list: spec.list === true,
        default: spec.default ?? null,
        choices: spec.type === "choice" ? spec.choices : [],
        description: text(spec.description),
    });
    return JSON.stringify({
        name: given.name,
        api,
        description: text(given.description),
        actions: Object.entries(actions).map(([name, spec]) => ({
            name,
            description: text(spec?.description),
            args: actionArgs.get(name),
        })),
        options: Object.entries(options).map(specJson),
        windowRules: Object.entries(windowRules).map(specJson),
        floating: given.floating !== false,
    });
}

/** Checks the declarations of options or window rules, which have the same shape. */
function checkSpecs(what, specs, types) {
    for (const [name, spec] of Object.entries(specs)) {
        if (spec === null || typeof spec !== "object" || !types.includes(spec.type)) {
            throw new Error(`${what} ${name} has an unknown type: ${spec?.type}`);
        }
        if (spec.type === "choice") {
            const choices = spec.choices;
            if (!Array.isArray(choices) || choices.length === 0 || !choices.every((c) => typeof c === "string")) {
                throw new Error(`${what} ${name} is a choice, so it needs a list of choices`);
            }
        }
        if (spec.list !== undefined && typeof spec.list !== "boolean") {
            throw new Error(`\`list\` of ${what} ${name} must be a boolean`);
        }
        if (!defaultFits(spec)) {
            throw new Error(`the default of ${what} ${name} must be a ${specTypeName(spec)}`);
        }
    }
}

function defaultFits(spec) {
    const one = (v) => {
        switch (spec.type) {
            case "boolean":
            case "number":
            case "string":
                return typeof v === spec.type;
            case "choice":
                return spec.choices.includes(v);
            case "size":
                return v !== null && typeof v === "object" && (Number.isFinite(v.proportion) || Number.isFinite(v.fixed));
            default:
                return false;
        }
    };
    if (spec.list) {
        return Array.isArray(spec.default) && spec.default.every(one);
    }
    return one(spec.default) || (spec.type === "size" && spec.default === null);
}

/**
 * Checks the arguments that action `name` declares, and returns them as bento reads them. An
 * action that declares none takes none. A plain string, the older way to declare an argument, is
 * an argument of any type that may be left out.
 */
function checkArgs(name, args, types) {
    if (args === undefined) {
        return [];
    }
    if (!Array.isArray(args)) {
        throw new Error(`the arguments of action ${name} must be an array`);
    }
    let optional = false;
    return args.map((arg, i) => {
        if (typeof arg === "string") {
            arg = { name: arg };
        }
        const label = `argument ${i + 1} of action ${name}`;
        if (arg === null || typeof arg !== "object" || typeof arg.name !== "string") {
            throw new Error(`${label} must have a name`);
        }
        const type = arg.type ?? null;
        if (type !== null && !types.includes(type)) {
            throw new Error(`${label} has an unknown type: ${type}`);
        }
        const choices = arg.choices ?? [];
        if (type === "choice") {
            if (!Array.isArray(choices) || choices.length === 0) {
                throw new Error(`${label} is a choice, so it needs a list of choices`);
            }
            if (!choices.every((choice) => typeof choice === "string")) {
                throw new Error(`the choices of ${label} must be strings`);
            }
        }
        const value = arg.default ?? null;
        if (type === null) {
            // Untyped arguments aren't checked, so they never have to be given.
        } else if (value !== null) {
            const valid = {
                boolean: typeof value === "boolean",
                number: typeof value === "number",
                string: typeof value === "string",
                choice: choices.includes(value),
                size: typeof value === "string",
                workspace: typeof value === "number" || typeof value === "string",
                monitor: typeof value === "string",
            }[type];
            if (!valid) {
                throw new Error(`the default of ${label} doesn't fit its type`);
            }
            optional = true;
        } else if (optional) {
            // Arguments are given in order, so one that must be given can't follow one that can
            // be left out.
            throw new Error(`${label} has no default, but an argument before it has one`);
        }
        return {
            name: arg.name,
            type,
            default: value,
            choices: type === "choice" ? choices : [],
            description: typeof arg.description === "string" ? arg.description : null,
        };
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
    try {
        if (call.event.type === "create") {
            create(call);
        } else {
            if (session === null) {
                throw new Error("no layout engine instance");
            }
            if (call.sync) {
                for (const fire of session.applySync(call.sync)) {
                    fire();
                }
            }
            isQuery = call.event.type === "newwindowsize";
            result = deliver(call.event);
        }
    } catch (e) {
        errors.push(describe(e));
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
    return JSON.stringify(out);
}

/** Creates the engine for the session, and adds the windows that are already there. */
function create(call) {
    const event = call.event;
    session = new Session(meta, event.options, event.savedState);
    adapter = null;
    // The engine learns about what's there when it looks, not from events.
    session.applySync(call.sync);

    const windows = event.windows.map((info) => {
        const d = session.addWindow(info);
        if (meta.api === 1) {
            d.rect = info.rect ? Object.freeze({ ...info.rect }) : null;
        }
        return { d, info };
    });

    if (meta.api === 1) {
        adapter = new V1Adapter(session, layoutModule);
        adapter.start();
        adapter.adopt(
            windows.map(({ d }) => d.facade),
            event.focused,
        );
        return;
    }

    layoutModule.default(session.bento);
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
        case "pointer": {
            const node = session.log.nodes.get(event.node);
            if (node !== undefined) {
                dispatchPointer(node, event);
            }
            return null;
        }
        default:
            throw new Error(`unknown event: ${event.type}`);
    }
}
