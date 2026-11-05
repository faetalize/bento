// DOM-style events for the objects that engines see.
//
// Listeners run in the order they were added. A listener that throws doesn't stop the others:
// its error goes to the call's result, where bento logs it.

const states = new WeakMap();
const listeners = new WeakMap();

let errorSink = (e) => {
    throw e;
};

/** Sets where errors thrown by listeners go. */
export function setErrorSink(sink) {
    errorSink = sink;
}

export class BentoEvent {
    constructor(type, target, init = {}) {
        states.set(this, { prevented: false, stopped: false, responded: false, response: null });
        Object.defineProperty(this, "type", { value: type, enumerable: true });
        Object.defineProperty(this, "target", { value: target, enumerable: true });
        for (const [name, value] of Object.entries(init)) {
            Object.defineProperty(this, name, { value, enumerable: true });
        }
    }

    get defaultPrevented() {
        return states.get(this).prevented;
    }

    preventDefault() {
        states.get(this).prevented = true;
    }

    stopPropagation() {
        states.get(this).stopped = true;
    }
}

/** An event that asks the engine for an answer; the first one given is used. */
export class QueryEvent extends BentoEvent {
    respondWith(value) {
        const state = states.get(this);
        if (!state.responded) {
            state.responded = true;
            state.response = value;
        }
    }
}

export class UnloadEvent extends BentoEvent {
    saveState(state) {
        const saved = states.get(this);
        saved.responded = true;
        // Check that it survives the trip through JSON.
        saved.response = state === undefined ? null : JSON.parse(JSON.stringify(state));
    }
}

/** The answer a query or `saveState` was given: `{ value }`, or `null` if there wasn't one. */
export function response(event) {
    const state = states.get(event);
    return state.responded ? { value: state.response } : null;
}

export function isPropagationStopped(event) {
    return states.get(event).stopped;
}

export class EventTarget {
    constructor() {
        listeners.set(this, new Map());
    }

    addEventListener(type, listener) {
        if (typeof listener !== "function") {
            throw new TypeError("an event listener must be a function");
        }
        const map = listeners.get(this);
        const list = map.get(type) ?? [];
        if (!list.includes(listener)) {
            list.push(listener);
        }
        map.set(type, list);
    }

    removeEventListener(type, listener) {
        const map = listeners.get(this);
        const list = map.get(type);
        if (list === undefined) {
            return;
        }
        const idx = list.indexOf(listener);
        if (idx !== -1) {
            list.splice(idx, 1);
        }
        if (list.length === 0) {
            map.delete(type);
        }
    }
}

/** Number of listeners `target` has for events of `type`. */
export function listenerCount(target, type) {
    return listeners.get(target)?.get(type)?.length ?? 0;
}

/** Runs the listeners of `target` for `event`, and returns the event. */
export function dispatch(target, event) {
    const list = listeners.get(target)?.get(event.type);
    for (const listener of list ? [...list] : []) {
        try {
            listener.call(target, event);
        } catch (e) {
            errorSink(e);
        }
    }
    return event;
}
