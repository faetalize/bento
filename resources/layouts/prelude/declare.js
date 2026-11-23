// What engines and shells declare in their `meta`: their settings and actions, by name. One shape
// for both, checked here the same way, and handed to bento as lists in the order they're declared,
// for settings screens. bento's shell runtime imports this file too.

// Colours, gradients and folders are for settings and window rules: binds write an action's
// arguments as plain values.
export const ARG_VALUE_TYPES = ["boolean", "number", "string", "choice", "size"];
export const VALUE_TYPES = [...ARG_VALUE_TYPES, "color", "gradient", "folder"];
export const ARG_TYPES = [...ARG_VALUE_TYPES, "workspace", "monitor"];

const text = (value) => (typeof value === "string" ? value : null);

/**
 * Checks settings, or window rules, which have the same shape, declared by name, and returns them
 * as bento reads them. Descriptions are only documentation, so a malformed one is dropped instead
 * of failing.
 */
export function checkSettings(what, specs, types = VALUE_TYPES) {
    if (specs === undefined || specs === null) {
        return [];
    }
    if (typeof specs !== "object" || Array.isArray(specs)) {
        throw new Error(`${what}s are declared as an object, by name`);
    }
    return Object.entries(specs).map(([name, spec]) => {
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
        if (spec.type === "gradient" && spec.list === true) {
            // A gradient is written with the properties of its node, so a node holds one.
            throw new Error(`${what} ${name} is a gradient, which can't be a list`);
        }
        for (const where of ["page", "group"]) {
            if (spec[where] !== undefined && typeof spec[where] !== "string") {
                throw new Error(`\`${where}\` of ${what} ${name} must be a string`);
            }
        }
        return {
            name,
            type: spec.type,
            list: spec.list === true,
            default: spec.default ?? null,
            choices: spec.type === "choice" ? spec.choices : [],
            description: text(spec.description),
            page: text(spec.page),
            group: text(spec.group),
        };
    });
}

/**
 * Checks actions declared by name, and returns them as bento reads them, with the keys each is
 * bound to by default as `binds`: a list of `{ key, args }`, from `bind`, which is a key, or a
 * list of keys and of keys with the arguments they run the action with.
 */
export function checkActions(actions, types = ARG_TYPES) {
    if (actions === undefined || actions === null) {
        return [];
    }
    if (typeof actions !== "object" || Array.isArray(actions)) {
        throw new Error("actions are declared as an object, by name");
    }
    return Object.entries(actions).map(([name, spec]) => ({
        name,
        description: text(spec?.description),
        args: checkArgs(name, spec?.args, types),
        binds: checkBinds(name, spec?.bind),
    }));
}

/** Checks the keys action `name` is bound to by default, as `bind` gives them. */
function checkBinds(name, bind) {
    if (bind === undefined || bind === null) {
        return [];
    }
    const list = Array.isArray(bind) ? bind : [bind];
    return list.map((one, i) => {
        const label = `bind ${i + 1} of action ${name}`;
        if (typeof one === "string") {
            return { key: one, args: [] };
        }
        if (one === null || typeof one !== "object" || typeof one.key !== "string") {
            throw new Error(`${label} must be a key, like "Mod+V", or { key, args }`);
        }
        const args = one.args ?? [];
        if (!Array.isArray(args) || args.some((arg) => !["string", "number", "boolean"].includes(typeof arg))) {
            throw new Error(`the args of ${label} must be a list of strings, numbers and booleans`);
        }
        return { key: one.key, args };
    });
}

/**
 * Checks the arguments that action `name` declares, and returns them as bento reads them. An
 * action that declares none takes none. A plain string, the older way to declare an argument, is
 * an argument of any type that may be left out.
 */
export function checkArgs(name, args, types = ARG_TYPES) {
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
