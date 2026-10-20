// bento's tiling layout: windows on a grid of cells.
//
// The workspace is a strip of near-square cells, 12 rows tall, that scrolls horizontally without
// end. Every tiled window covers a rectangle of whole cells: a starting column and row plus a
// span. Windows are independent of each other: they can sit in different rows, stack on top of
// each other and leave holes. Moving or resizing one pushes the others out of the way, in order,
// and with left gravity (the `gravity` option, on by default) windows slide left within their
// rows until they touch another window or column 0, which is the fixed left edge of the strip.
//
// This file has two parts:
//
// 1. The cell grid: pure arithmetic on rectangles of cells, with no windows, pixels or camera.
//    `Grid`, `Area` and `CellMetrics` are exported so that tests can check them on
//    their own.
// 2. The layout: it keeps one item on the grid per window, or per tabbed area of several windows,
//    and maps the grid onto window rects, focus, the camera and actions.
//
// Coordinates: column 0 of the grid starts at x = 0 on the canvas, and the camera's x is where
// the left edge of the view is, so a window in column 0 shows at the left edge of the view when
// the camera is at x = -gap. The camera never moves vertically: tiles sit at their y in the view.
//
// The camera only moves when the focused window would otherwise leave the view, and for actions
// about the view itself: focusing another window, centering, fullscreen, swiping.
// Layout changes like closing windows, pushes and gravity never move it; the windows that move
// slide into place on screen while the rest stay still.

export const meta = {
    api: 1,
    name: "bento",
    description: "Windows on a grid of near-square cells that scrolls horizontally.",
    actions: {},
    options: {
        gravity: {
            type: "boolean",
            default: true,
            description:
                "Windows slide left within their rows until they touch another window or the " +
                "left edge of the grid. Without gravity, windows stay where they're put and the " +
                "grid goes on to the left too.",
        },
    },
};

// =================================================================================================
// Part 1: the cell grid
// =================================================================================================
//
// A port of bento's `grid.rs`. Positions never depend on where the windows are: column 0 stays
// put, and nothing gets renumbered to start at the leftmost window.

/** Default number of rows in the grid. 12 divides evenly into halves, thirds, quarters, sixths. */
export const DEFAULT_ROWS = 12;

/** Largest width of a window in columns, which keeps cell arithmetic far from overflowing. */
export const MAX_COLS = 10000;

/** Direction for a grid operation. */
export const Direction = Object.freeze({ Left: "left", Right: "right", Up: "up", Down: "down" });
const { Left, Right, Up, Down } = Direction;

function opposite(dir) {
    return { [Left]: Right, [Right]: Left, [Up]: Down, [Down]: Up }[dir];
}

export function isHorizontal(dir) {
    return dir === Left || dir === Right;
}

/** Truncates to an integer, like Rust's `as i32`, without ever giving -0. */
function toInt(x) {
    return Math.trunc(x) + 0 || 0;
}

/** Rounds halves away from zero, like Rust's `f64::round`, to an integer. */
export function round(x) {
    return toInt(Math.sign(x) * Math.round(Math.abs(x)));
}

function clamp(value, min, max) {
    return Math.min(Math.max(value, min), max);
}

/** Compares two arrays of numbers element by element, like Rust tuples. */
function compareKeys(a, b) {
    for (let i = 0; i < a.length; i++) {
        if (a[i] !== b[i]) {
            return a[i] < b[i] ? -1 : 1;
        }
    }
    return 0;
}

/** Sorts `array` in place by `key`, which returns an array of numbers. The sort is stable. */
function sortBy(array, key) {
    return array.sort((a, b) => compareKeys(key(a), key(b)));
}

/** The element with the smallest key, the first one on ties, like Rust's `min_by_key`. */
function minBy(array, key) {
    let best = null;
    let bestKey = null;
    for (const value of array) {
        const k = key(value);
        if (bestKey === null || compareKeys(k, bestKey) < 0) {
            best = value;
            bestKey = k;
        }
    }
    return best;
}

/** The element with the largest key, the last one on ties, like Rust's `max_by_key`. */
function maxBy(array, key) {
    let best = null;
    let bestKey = null;
    for (const value of array) {
        const k = key(value);
        if (bestKey === null || compareKeys(k, bestKey) >= 0) {
            best = value;
            bestKey = k;
        }
    }
    return best;
}

/** Rectangle of whole cells. Areas never change; methods return new ones. */
export class Area {
    constructor(col, row, cols, rows) {
        /** Leftmost column. */
        this.col = col;
        /** Topmost row. */
        this.row = row;
        /** Width in columns. */
        this.cols = cols;
        /** Height in rows. */
        this.rows = rows;
        Object.freeze(this);
    }

    static from({ col, row, cols, rows }) {
        return new Area(col, row, cols, rows);
    }

    /** One past the rightmost column. */
    get right() {
        return this.col + this.cols;
    }

    /** One past the bottom row. */
    get bottom() {
        return this.row + this.rows;
    }

    /** This area with some of its fields changed. */
    with({ col = this.col, row = this.row, cols = this.cols, rows = this.rows }) {
        return new Area(col, row, cols, rows);
    }

    equals(other) {
        return (
            other !== null &&
            this.col === other.col &&
            this.row === other.row &&
            this.cols === other.cols &&
            this.rows === other.rows
        );
    }

    overlaps(other) {
        return (
            this.col < other.right &&
            other.col < this.right &&
            this.row < other.bottom &&
            other.row < this.bottom
        );
    }

    shifted(dx, dy) {
        return this.with({ col: this.col + dx, row: this.row + dy });
    }

    /** Span perpendicular to `dir`: rows for horizontal directions, columns for vertical ones. */
    crossSpan(dir) {
        return isHorizontal(dir) ? [this.row, this.bottom] : [this.col, this.right];
    }

    toJSON() {
        return { col: this.col, row: this.row, cols: this.cols, rows: this.rows };
    }
}

/**
 * Windows placed on the cell grid, by key. Keys can be anything; they're compared with `===`.
 *
 * Every change pushes overlapping windows out of the way, then collapses empty columns, or with
 * left gravity, slides windows left. Changes that would push a window past column 0 with gravity
 * don't happen at all.
 */
export class Grid {
    constructor(rows = DEFAULT_ROWS) {
        if (!(rows > 0)) {
            throw new Error("a grid needs at least one row");
        }
        /** Number of rows. */
        this.rows = rows;
        /** Whether windows slide left within their rows until they touch another window. */
        this.gravity = false;
        /** Window held in place by the pointer, which gravity doesn't move until it's released. */
        this.pinned = null;
        /** Windows and their areas, in insertion order, as `{ key, area }`. */
        this.items = [];
    }

    clone() {
        const grid = new Grid(this.rows);
        grid.gravity = this.gravity;
        grid.pinned = this.pinned;
        grid.items = this.snapshot();
        return grid;
    }

    equals(other) {
        return (
            this.rows === other.rows &&
            this.gravity === other.gravity &&
            this.pinned === other.pinned &&
            sameItems(this.items, other.items)
        );
    }

    /** Copy of the items, to restore if a change doesn't go through. */
    snapshot() {
        return this.items.map(({ key, area }) => ({ key, area }));
    }

    /**
     * Pins a window in place against gravity, like holding it with the pointer, or releases the
     * pin with `null` and lets the window fall.
     */
    setPinned(key) {
        this.pinned = key;
        this.normalize();
    }

    /** Turns left gravity on or off. Turning it on slides windows left right away. */
    setGravity(gravity) {
        this.gravity = gravity;
        this.normalize();
    }

    get length() {
        return this.items.length;
    }

    /** Width of the strip in columns: one past the rightmost column that a window covers. */
    width() {
        if (this.items.length === 0) {
            return 0;
        }
        return Math.max(...this.items.map(({ area }) => area.right));
    }

    area(key) {
        const item = this.items.find((item) => item.key === key);
        return item ? item.area : null;
    }

    keys() {
        return this.items.map(({ key }) => key);
    }

    idx(key) {
        return this.items.findIndex((item) => item.key === key);
    }

    /** Clamps an area to a valid size and to the rows of the grid. */
    clampArea(area) {
        const col = this.gravity ? Math.max(area.col, 0) : area.col;
        const cols = clamp(area.cols, 1, MAX_COLS);
        const rows = clamp(area.rows, 1, this.rows);
        const row = clamp(area.row, 0, this.rows - rows);
        return new Area(col, row, cols, rows);
    }

    /** Inserts a window at `area`, pushing overlapping windows to the right. */
    insert(key, area) {
        this.items.push({ key, area: this.clampArea(area) });
        this.settle([this.items.length - 1], Right);
        this.normalize();
    }

    /**
     * Inserts a window of `cols` × `rows` cells right of `after`, or at the end of the strip.
     *
     * The window lines up with the top of `after` where it fits.
     */
    insertAfter(key, after, cols, rows) {
        const a = after === null || after === undefined ? null : this.area(after);
        this.insert(
            key,
            a ? new Area(a.right, a.row, cols, rows) : new Area(this.width(), 0, cols, rows),
        );
    }

    /**
     * Resizes every window to its span from `span(key)`, which returns `[cols, rows]`, and scales
     * column positions by `ratio`, for a grid with different cells.
     *
     * Windows keep their order and rows; ones that would overlap the windows before them move
     * right.
     */
    rescale(ratio, span) {
        for (const item of this.items) {
            const [cols, rows] = span(item.key);
            const col = round(item.area.col * ratio);
            item.area = this.clampArea(new Area(col, item.area.row, cols, rows));
        }

        // Going left to right, every window before the current one is already in its final place.
        const order = sortBy([...this.items.keys()], (i) => [
            this.items[i].area.col,
            this.items[i].area.row,
        ]);
        order.forEach((i, n) => {
            const area = this.items[i].area;
            let col = area.col;
            for (const j of order.slice(0, n)) {
                const b = this.items[j].area;
                if (b.row < area.bottom && area.row < b.bottom) {
                    col = Math.max(col, b.right);
                }
            }
            this.items[i].area = area.with({ col });
        });

        this.normalize();
    }

    /** Removes a window, leaving its cells empty. Returns its area, or `null`. */
    remove(key) {
        const idx = this.idx(key);
        if (idx === -1) {
            return null;
        }
        const [{ area }] = this.items.splice(idx, 1);
        if (this.pinned === key) {
            this.pinned = null;
        }
        this.normalize();
        return area;
    }

    /**
     * Sets a window's area, pushing overlapping windows in `dir`.
     *
     * Returns whether any window changed position or size.
     */
    setArea(key, area, dir) {
        const idx = this.idx(key);
        if (idx === -1) {
            return false;
        }

        const before = this.snapshot();
        this.items[idx].area = this.clampArea(area);
        this.settle([idx], dir);
        return this.finish(before);
    }

    /**
     * Sets the areas of several windows at once, given as `[key, area]` pairs, adding the ones
     * not in the grid yet, and pushes other overlapping windows to the right.
     *
     * The areas must not overlap each other. Returns whether any window changed position or size.
     */
    place(areas) {
        const before = this.snapshot();

        const pinned = [];
        for (const [key, given] of areas) {
            const area = this.clampArea(given);
            let idx = this.idx(key);
            if (idx === -1) {
                this.items.push({ key, area });
                idx = this.items.length - 1;
            } else {
                this.items[idx].area = area;
            }
            pinned.push(idx);
        }

        this.settle(pinned, Right);
        return this.finish(before);
    }

    /**
     * Resizes a window to `cols` × `rows`, keeping its top-left corner.
     *
     * Growing down pushes windows below it down; growing right pushes windows right.
     */
    resize(key, cols, rows) {
        const idx = this.idx(key);
        if (idx === -1) {
            return false;
        }

        const before = this.snapshot();

        this.items[idx].area = this.clampArea(this.items[idx].area.with({ rows }));
        this.settle([idx], Down);

        this.items[idx].area = this.clampArea(this.items[idx].area.with({ cols }));
        this.settle([idx], Right);

        return this.finish(before);
    }

    /**
     * Moves a window by one cell in `dir`.
     *
     * If the cells it moves into are occupied, it trades places with the windows there instead:
     * it jumps past them and they move into the space it left. Returns whether any window changed
     * position.
     */
    moveWindow(key, dir) {
        const idx = this.idx(key);
        if (idx === -1) {
            return false;
        }

        const area = this.items[idx].area;
        const [dx, dy] = { [Left]: [-1, 0], [Right]: [1, 0], [Up]: [0, -1], [Down]: [0, 1] }[dir];
        const target = area.shifted(dx, dy);
        if (target.row < 0 || target.bottom > this.rows) {
            return false;
        }

        const before = this.snapshot();

        // Blockers all touch the window's edge in `dir`, since they can't overlap the window.
        const blockers = [...this.items.keys()].filter(
            (i) => i !== idx && this.items[i].area.overlaps(target),
        );

        if (blockers.length === 0) {
            this.items[idx].area = target;
        } else {
            const blockerAreas = blockers.map((i) => this.items[i].area);
            let windowArea;
            let shift;
            switch (dir) {
                case Right: {
                    const end = Math.max(...blockerAreas.map((a) => a.right));
                    windowArea = area.with({ col: end - area.cols });
                    shift = [-area.cols, 0];
                    break;
                }
                case Left: {
                    const start = Math.min(...blockerAreas.map((a) => a.col));
                    windowArea = area.with({ col: start });
                    shift = [area.cols, 0];
                    break;
                }
                case Down: {
                    const end = Math.max(...blockerAreas.map((a) => a.bottom));
                    windowArea = area.with({ row: end - area.rows });
                    shift = [0, -area.rows];
                    break;
                }
                case Up: {
                    const start = Math.min(...blockerAreas.map((a) => a.row));
                    windowArea = area.with({ row: start });
                    shift = [0, area.rows];
                    break;
                }
            }

            this.items[idx].area = windowArea;
            for (const i of blockers) {
                this.items[i].area = this.items[i].area.shifted(shift[0], shift[1]);
            }

            // The blockers moved against `dir`, so anything they now overlap gets pushed that way
            // too.
            this.settle([...blockers, idx], opposite(dir));
        }

        return this.finish(before);
    }

    /**
     * Finishes a change that started from `before`, returning whether anything changed.
     *
     * With left gravity, column 0 is the left edge of the grid that windows pack against, so a
     * change that pushed a window past it doesn't happen at all. Without gravity, the grid goes on
     * to the left too.
     */
    finish(before) {
        if (this.gravity && this.items.some(({ area }) => area.col < 0)) {
            this.items = before;
            return false;
        }

        this.normalize();

        // Shifting every window by the same number of columns leaves the layout as it was, only
        // somewhere else on the grid, so it isn't a change. Like moving the leftmost window left
        // without gravity: collapsing the empty column it leaves pulls everything else along.
        const shiftedBy = (dx) =>
            this.items.length === before.length &&
            this.items.every(
                ({ key, area }, n) =>
                    key === before[n].key &&
                    area.equals(before[n].area.with({ col: before[n].area.col + dx })),
            );
        const dx =
            this.items.length > 0 && before.length > 0
                ? this.items[0].area.col - before[0].area.col
                : 0;
        if (dx !== 0 && shiftedBy(dx)) {
            this.items = before;
            return false;
        }

        return !sameItems(this.items, before);
    }

    /**
     * Columns of cells where windows start, left to right.
     *
     * These number the grid's columns for actions and IPC, like the columns of a strip.
     */
    columnStarts() {
        return [...new Set(this.items.map(({ area }) => area.col))].sort((a, b) => a - b);
    }

    /** Windows covering column of cells `col`, top to bottom, like the tiles of a column. */
    stack(col) {
        const stack = this.items.filter(({ area }) => area.col <= col && col < area.right);
        return sortBy(stack, ({ area }) => [area.row]).map(({ key }) => key);
    }

    /**
     * Finds the window to focus when moving from `key` in `dir`.
     *
     * `anchor` is the row (for left and right) or column (for up and down) that the user is
     * travelling along. It's clamped into the window's own span; `null` uses the window's top row
     * or left column.
     *
     * Among the windows past the edge in `dir`, the nearest one wins, so repeatedly moving in one
     * direction never skips a window. Windows overlapping the anchor break ties, then windows
     * overlapping the current window's span, then the distance from the anchor.
     */
    neighbor(key, dir, anchor = null) {
        return this.findNeighbor(key, dir, anchor, false);
    }

    /**
     * Like `neighbor`, but only finds windows that share rows (for left and right) or columns
     * (for up and down) with `key`, the way columns in a strip or tiles in a column are next to
     * each other.
     */
    adjacent(key, dir, anchor = null) {
        return this.findNeighbor(key, dir, anchor, true);
    }

    findNeighbor(key, dir, anchor, strict) {
        const area = this.area(key);
        if (area === null) {
            return null;
        }
        const [start, end] = area.crossSpan(dir);
        const along = clamp(anchor ?? start, start, end - 1);

        const candidates = [];
        for (const { key: other, area: a } of this.items) {
            if (other === key) {
                continue;
            }
            const gap = {
                [Right]: a.col - area.right,
                [Left]: area.col - a.right,
                [Down]: a.row - area.bottom,
                [Up]: area.row - a.bottom,
            }[dir];
            if (gap < 0) {
                continue;
            }

            const [s, e] = a.crossSpan(dir);
            let tier;
            if (s <= along && along < e) {
                tier = 0;
            } else if (s < end && start < e) {
                tier = 1;
            } else if (strict) {
                continue;
            } else {
                tier = 2;
            }
            const dist = along < s ? s - along : along >= e ? along - e + 1 : 0;

            candidates.push({ key: other, sortKey: [tier === 2 ? 1 : 0, gap, tier, dist, s] });
        }

        const best = minBy(candidates, (c) => c.sortKey);
        return best ? best.key : null;
    }

    /**
     * Resolves overlaps by pushing windows in `dir`.
     *
     * Windows at the indices in `pinned` never move, and must not overlap each other. A window
     * pushed off the top or bottom goes to the right of the window that pushed it instead,
     * keeping its rows.
     */
    settle(pinned, dir) {
        if (this.sweep(pinned, dir)) {
            // Make room for the windows that went to the right.
            this.sweep(pinned, Right);
        }
    }

    /**
     * Pushes windows in `dir` so that none overlap, keeping their order along `dir`.
     *
     * Going through the windows in order along `dir`, each one moves only as far as it has to to
     * clear the windows before it that share rows (or columns, going up or down) with it, and any
     * pinned window. So a window pushes the next one, which pushes the one after it.
     *
     * Returns whether a window was pushed off the top or bottom and went right instead.
     */
    sweep(pinned, dir) {
        const order = sortBy(
            [...this.items.keys()].filter((i) => !pinned.includes(i)),
            (i) => [this.along(i, dir), this.items[i].area.crossSpan(dir)[0], i],
        );

        const sharesLines = (a, b) => {
            const [s1, e1] = a.crossSpan(dir);
            const [s2, e2] = b.crossSpan(dir);
            return s1 < e2 && s2 < e1;
        };
        // Whether `a` reaches past where `b` ends along `dir`, and so has to move.
        const behind = (a, b) =>
            ({
                [Right]: a.col < b.right,
                [Left]: a.right > b.col,
                [Down]: a.row < b.bottom,
                [Up]: a.bottom > b.row,
            })[dir];
        // `a` moved just past `b` along `dir`.
        const past = (a, b) =>
            ({
                [Right]: () => a.with({ col: b.right }),
                [Left]: () => a.with({ col: b.col - a.cols }),
                [Down]: () => a.with({ row: b.bottom }),
                [Up]: () => a.with({ row: b.row - a.rows }),
            })[dir]();

        const placed = [];
        let wentRight = false;
        for (const i of order) {
            const orig = this.items[i].area;
            let area = orig;
            let pusher = null;

            // Stay after the windows that came before in these rows or columns. They only ever
            // moved further along `dir`, so this keeps the order.
            for (const j of placed) {
                const b = this.items[j].area;
                if (sharesLines(area, b) && behind(area, b)) {
                    area = past(area, b);
                    pusher = j;
                }
            }

            // Clear the pinned windows, which can be anywhere.
            for (;;) {
                const j = pinned.find((j) => area.overlaps(this.items[j].area));
                if (j === undefined) {
                    break;
                }
                area = past(area, this.items[j].area);
                pusher = j;
            }

            if (area.row < 0 || area.bottom > this.rows) {
                // Pushed off the top or bottom: go right of the pusher instead, in the same rows.
                this.items[i].area = orig.with({ col: this.items[pusher].area.right });
                wentRight = true;
                continue;
            }

            this.items[i].area = area;
            placed.push(i);
        }
        return wentRight;
    }

    /** Position of a window along `dir`; larger means further in that direction. */
    along(idx, dir) {
        const a = this.items[idx].area;
        return { [Right]: a.col, [Left]: -a.right, [Down]: a.row, [Up]: -a.bottom }[dir];
    }

    /**
     * Collapses columns that no window covers between windows, moving the windows right of them
     * left. With left gravity, slides windows left within their rows instead.
     */
    normalize() {
        if (this.gravity) {
            this.slideLeft();
            return;
        }

        // A window held by the pointer stays where it is until it's released.
        if (this.pinned !== null) {
            return;
        }

        // Positions don't depend on the windows: the leftmost window stays where it is, and so
        // does any space before it.
        if (this.items.length === 0) {
            return;
        }
        const min = Math.min(...this.items.map(({ area }) => area.col));
        const width = this.width() - min;

        const covered = new Array(width).fill(false);
        for (const { area } of this.items) {
            for (let c = area.col; c < area.right; c++) {
                covered[c - min] = true;
            }
        }

        // Number of uncovered columns before each column.
        const emptyBefore = [];
        let count = 0;
        for (const isCovered of covered) {
            emptyBefore.push(count);
            if (!isCovered) {
                count += 1;
            }
        }

        // A window covers contiguous columns, so none of its own columns collapse.
        for (const item of this.items) {
            item.area = item.area.with({ col: item.area.col - emptyBefore[item.area.col - min] });
        }
    }

    /**
     * Slides every window left within its rows until it touches another window or column 0.
     *
     * This leaves no fully empty columns either, since every window touches something on its
     * left all the way to column 0.
     *
     * A pinned window doesn't slide, but windows to its left can still push it right, the same
     * as they would push a window held by the pointer.
     */
    slideLeft() {
        const pinned = this.pinned === null ? -1 : this.idx(this.pinned);

        // Going left to right, every window to the left of the current one is already in its
        // final place. Windows only move left, except the pinned one, which only moves right, but
        // only as far as the windows before it in the order, so the order holds.
        const order = sortBy([...this.items.keys()], (i) => [
            this.items[i].area.col,
            this.items[i].area.row,
        ]);

        order.forEach((i, n) => {
            const area = this.items[i].area;
            let col = 0;
            for (const j of order.slice(0, n)) {
                const b = this.items[j].area;
                if (b.row < area.bottom && area.row < b.bottom) {
                    col = Math.max(col, b.right);
                }
            }
            if (i === pinned) {
                col = Math.max(col, area.col);
            }
            this.items[i].area = area.with({ col });
        });
    }

    /** Throws if the grid is in a state that its operations should never leave it in. */
    verifyInvariants() {
        const fail = (message) => {
            throw new Error(`grid invariant: ${message}`);
        };

        this.items.forEach(({ key, area: a }, i) => {
            if (!(a.cols >= 1 && a.rows >= 1)) fail(`empty area ${JSON.stringify(a)}`);
            if (!(a.row >= 0 && a.bottom <= this.rows))
                fail(`area outside rows ${JSON.stringify(a)}`);
            if (this.gravity && a.col < 0)
                fail(`area before column 0 with gravity ${JSON.stringify(a)}`);
            if (this.items.slice(0, i).some((other) => other.key === key)) fail("duplicate window");
            for (const { area: b } of this.items.slice(i + 1)) {
                if (a.overlaps(b))
                    fail(`overlapping areas ${JSON.stringify(a)} and ${JSON.stringify(b)}`);
            }
        });

        if (this.pinned !== null && this.idx(this.pinned) === -1) {
            fail("pinned window must be in the grid");
        }

        // No column is empty between the start of the grid with gravity, or the leftmost window
        // without, and the rightmost window. A pinned window can leave empty columns until it's
        // released.
        const start = this.gravity
            ? 0
            : this.items.length > 0
              ? Math.min(...this.items.map(({ area }) => area.col))
              : 0;
        for (let c = start; c < this.width(); c++) {
            if (
                this.pinned === null &&
                !this.items.some(({ area }) => area.col <= c && c < area.right)
            ) {
                fail(`empty column ${c}`);
            }
        }

        if (this.gravity) {
            for (const { key, area: a } of this.items) {
                if (key === this.pinned) {
                    continue;
                }
                const touches =
                    a.col === 0 ||
                    this.items.some(
                        ({ area: b }) => b.right === a.col && b.row < a.bottom && a.row < b.bottom,
                    );
                if (!touches) fail(`window could slide left ${JSON.stringify(a)}`);
            }
        }
    }
}

function sameItems(a, b) {
    return (
        a.length === b.length &&
        a.every(({ key, area }, n) => key === b[n].key && area.equals(b[n].area))
    );
}

/** Pixel geometry of the cell grid for a given view size. */
export class CellMetrics {
    /**
     * Computes cells that fill the view height with `rows` rows and are as close to square as
     * possible while fitting a whole, even number of columns in the view width.
     *
     * The column count is even so that two half-width windows fit the view exactly.
     */
    constructor(viewSize, rows, gap) {
        const cellH = Math.max((viewSize.h - gap * (rows + 1)) / rows, 1);

        // n columns take n * cellW + (n + 1) * gap.
        const squareCols = (viewSize.w - gap) / (cellH + gap);
        const n = Math.max(round(squareCols / 2) * 2, 2);
        const cellW = Math.max((viewSize.w - gap * (n + 1)) / n, 1);

        /** Cell width in logical pixels. */
        this.cellW = cellW;
        /** Cell height in logical pixels. */
        this.cellH = cellH;
        /** Gap between cells and around the edges. */
        this.gap = gap;
        /** Number of columns that exactly fill the view width. */
        this.colsPerView = n;
        /** Number of rows that exactly fill the view height. */
        this.rows = rows;
        Object.freeze(this);
    }

    /** Rectangle of an area, relative to the top-left corner of the strip, gaps included. */
    rect(area) {
        return {
            x: this.gap + area.col * (this.cellW + this.gap),
            y: this.gap + area.row * (this.cellH + this.gap),
            w: area.cols * this.cellW + (area.cols - 1) * this.gap,
            h: area.rows * this.cellH + (area.rows - 1) * this.gap,
        };
    }

    /**
     * Number of columns that fit in `proportion` of the view width; at least one.
     *
     * Rounds down so that windows sized by complementary proportions fit together.
     */
    colsForProportion(proportion) {
        return clamp(spanForProportion(proportion, this.colsPerView), 1, MAX_COLS);
    }

    /** Number of rows that fit in `proportion` of the view height; at least one. */
    rowsForProportion(proportion) {
        return clamp(spanForProportion(proportion, this.rows), 1, this.rows);
    }
}

/** Rounds down, allowing for proportions written with limited precision like 0.33333. */
function spanForProportion(proportion, total) {
    return toInt(Math.floor(proportion * total + 0.01));
}

/**
 * Number of cells to grow or shrink by for `amount`, where a cell takes up `cell`.
 *
 * Rounds to the nearest cell, but a nonzero amount changes the size by at least one cell.
 */
export function cellsForDelta(amount, cell) {
    const cells = round(amount / cell);
    return cells === 0 && amount !== 0 ? Math.sign(amount) : cells;
}

/** Scales a span of `n` cells out of `of` to a view `total` cells across. */
export function scaleSpan(n, of, total) {
    if (of === total || of <= 0) {
        return n;
    }
    return round((n * total) / of);
}

/**
 * Next span when cycling through presets: the smallest one larger than `current` going forwards,
 * or the largest one smaller going backwards, wrapping around.
 */
export function nextPresetSpan(current, spans, forwards) {
    const sorted = [...new Set(spans)].sort((a, b) => a - b);
    if (sorted.length === 0) {
        return current;
    }
    if (forwards) {
        return sorted.find((span) => span > current) ?? sorted[0];
    }
    return [...sorted].reverse().find((span) => span < current) ?? sorted[sorted.length - 1];
}

// =================================================================================================
// Part 2: the layout
// =================================================================================================

/** How many windows that left to float or be dragged elsewhere to remember the size of. */
const REMEMBERED_WINDOWS = 64;

export default function create(ws) {
    // ---------------------------------------------------------------------------------------------
    // State
    // ---------------------------------------------------------------------------------------------

    // Every window is an item on the grid of its own, except for tabs: a tabbed area is one item
    // with several windows, of which one shows. Items are the keys of the grid.
    //
    // An item also keeps the size the user or window rules chose, which carries over to grids
    // with different cells, like on another monitor:
    //
    // - `width` is niri's column width: `{ proportion }` of the view, or `{ fixed }` tile pixels.
    // - `fullWidth` is set by maximize-column. The window keeps its own width to come back to.
    // - `cols` and `rows` are spans that override the width and the full height: `{ preset }`
    //   from a window rule, or `{ cells: n, of }` when the window was resized on a grid that was
    //   `of` cells across. `null` follows the width, or takes the full height.

    /** Cell grid, with one item per window or tabbed area. */
    let grid = new Grid(DEFAULT_ROWS);
    /** Items sorted top-left first by their place on the grid, see `sync`. */
    let items = [];
    /** Item with the focused window, or `null` when there are no windows. */
    let active = null;
    /** Row and column that directional focus is travelling along, while `anchor.item` is active. */
    let anchor = null;
    /** Item to go back to when `opened` closes, if focus didn't move in between. */
    let prevOnRemoval = null;
    /** Camera offset from `item` to go back to when it stops being fullscreen or maximized. */
    let restore = null;
    /** Ongoing pointer resize, with the grid as it was when it started. */
    let resize = null;
    /** Size of windows that left to float or to be dragged, by window id, for when they're back. */
    const remembered = new Map();
    /** Tab bars bento shows, by key, with what they showed, to only send changes. */
    const tabBars = new Map();
    let nextItemId = 1;

    // View geometry, see `updateGeometry`.
    let viewSize;
    /** Part of the view not covered by panels. Maximized windows fill it. */
    let parentArea;
    /** `parentArea` without the struts from the config, where tiles go. */
    let workingArea;
    let metrics;

    updateGeometry();
    grid.setGravity(gravityOption());

    function gravityOption() {
        return ws.options.gravity !== false;
    }

    function updateGeometry() {
        viewSize = ws.view.size;
        parentArea = ws.view.workingArea;
        workingArea = computeWorkingArea(parentArea, ws.view.scale, ws.config.struts);
        metrics = new CellMetrics(workingArea, DEFAULT_ROWS, ws.config.gaps);
    }

    // ---------------------------------------------------------------------------------------------
    // Items and their sizes
    // ---------------------------------------------------------------------------------------------

    function makeItem(windows, intent) {
        return {
            /** Key of the item's tab bar, see `flush`. */
            key: `area-${nextItemId++}`,
            windows,
            /** Index of the tab that shows. */
            active: 0,
            tabbed: ws.config.defaultColumnDisplay === "tabbed",
            width: intent.width,
            fullWidth: intent.fullWidth,
            cols: intent.cols,
            rows: intent.rows,
            fullscreen: false,
            maximized: false,
            /** Row to go back to when the item stops being fullscreen or maximized. */
            rowToRestore: null,
        };
    }

    function intentOf(item) {
        return { width: item.width, fullWidth: item.fullWidth, cols: item.cols, rows: item.rows };
    }

    /** Size of a new window from the window rules and the config. */
    function intentForNewWindow(win) {
        const rules = win.rules;
        let width;
        if (rules.defaultWidth) {
            width = widthFromPreset(rules.defaultWidth, win.borders.w);
        } else if (win.size.w > 0) {
            // Like niri, a window without a default width keeps the width it opened with.
            width = { fixed: win.size.w };
        } else {
            width = { proportion: 0.5 };
        }
        return {
            width,
            fullWidth: false,
            cols: null,
            rows: rules.defaultHeight ? { preset: rules.defaultHeight } : null,
        };
    }

    /** Size of a window that already has one, like from the layout before this one. */
    function intentForSize(win, size) {
        if (win.mode !== "normal" || !(size.w > 0 && size.h > 0)) {
            return intentForNewWindow(win);
        }
        return {
            width: { fixed: size.w },
            fullWidth: false,
            cols: null,
            rows: { cells: rowsForHeight(size.h), of: metrics.rows },
        };
    }

    /** Like niri, a fixed width is for the window, so the borders go on top of it for the tile. */
    function widthFromPreset(preset, borders) {
        return "proportion" in preset
            ? { proportion: preset.proportion }
            : { fixed: preset.fixed + borders };
    }

    function modeOf(item) {
        return item.fullscreen ? "fullscreen" : item.maximized ? "maximized" : "normal";
    }

    function itemOf(win) {
        if (!win) return null;
        return items.find((item) => item.windows.some((w) => w.id === win.id)) ?? null;
    }

    /** The item of `target`, for actions on a specific window, or the focused one. */
    function itemFor(target) {
        return target ? itemOf(target) : active;
    }

    function colsForWidth(width) {
        const m = metrics;
        const cols =
            "proportion" in width
                ? m.colsForProportion(width.proportion)
                : round((width.fixed + m.gap) / (m.cellW + m.gap));
        return clamp(cols, 1, MAX_COLS);
    }

    function colsForPreset(preset, borders) {
        return colsForWidth(widthFromPreset(preset, borders));
    }

    function rowsForHeight(height) {
        const m = metrics;
        return clamp(round((height + m.gap) / (m.cellH + m.gap)), 1, m.rows);
    }

    /** Like niri, a fixed height is for the window, so the borders go on top of it. */
    function rowsForPreset(preset, borders) {
        return "proportion" in preset
            ? metrics.rowsForProportion(preset.proportion)
            : rowsForHeight(preset.fixed + borders);
    }

    function colsForSpan(span, borders) {
        return "cells" in span
            ? clamp(scaleSpan(span.cells, span.of, metrics.colsPerView), 1, MAX_COLS)
            : colsForPreset(span.preset, borders);
    }

    function rowsForSpan(span, borders) {
        return "cells" in span
            ? clamp(scaleSpan(span.cells, span.of, metrics.rows), 1, metrics.rows)
            : rowsForPreset(span.preset, borders);
    }

    /** Grid size `[cols, rows]` for a size `intent` of a window with `borders`. */
    function spanOf(intent, borders) {
        const cols = intent.fullWidth
            ? metrics.colsPerView
            : intent.cols
              ? colsForSpan(intent.cols, borders.w)
              : colsForWidth(intent.width);
        const rows = intent.rows ? rowsForSpan(intent.rows, borders.h) : metrics.rows;
        return [cols, rows];
    }

    /** Grid size `[cols, rows]` that an item should have, from the size of its window. */
    function spanFor(item) {
        return spanOf(item, item.windows[0].borders);
    }

    /**
     * Grid size that an item takes in its sizing mode: its own size, or when fullscreen or
     * maximized, enough columns to cover its width and all rows.
     */
    function spanInMode(item) {
        const mode = modeOf(item);
        if (mode === "normal") {
            return spanFor(item);
        }
        const m = metrics;
        const width = mode === "fullscreen" ? viewSize.w : parentArea.w;

        // Cover the width and the gap after it, so that the next window starts past it.
        const cols = toInt(Math.ceil((width + m.gap) / (m.cellW + m.gap)));
        return [clamp(cols, 1, MAX_COLS), m.rows];
    }

    // ---------------------------------------------------------------------------------------------
    // From cells to pixels
    // ---------------------------------------------------------------------------------------------

    /** X of a grid column on the canvas. Column 0 starts at x = 0. */
    function gridX(col) {
        return ws.round(col * (metrics.cellW + metrics.gap));
    }

    /** Tile geometry for an area: `y` relative to the top of the working area, and the size. */
    function cellGeometry(area) {
        const m = metrics;
        const rect = m.rect(area);

        // Round the edges rather than the sizes so that neighbors line up exactly.
        const left = gridX(area.col);
        const right = gridX(area.right) - m.gap;
        const top = ws.round(rect.y);
        const bottom = ws.round(rect.y + rect.h);

        // With gaps too large to fit the rows, cells bottom out at 1 px and the rows overflow.
        // Keep tiles within the working area like niri does.
        const maxH = Math.max(workingArea.h - m.gap * 2, 1);

        return {
            y: top,
            w: Math.max(right - left, 0),
            h: Math.max(Math.min(bottom - top, maxH), 0),
        };
    }

    /** Rect of an area on the canvas. */
    function areaRect(area) {
        const g = cellGeometry(area);
        return { x: gridX(area.col), y: workingArea.y + g.y, w: g.w, h: g.h };
    }

    function itemX(item) {
        return gridX(grid.area(item).col);
    }

    /**
     * Rect an item covers on the canvas: its cells, or when fullscreen or maximized, the view or
     * the area without panels at the x of its cells.
     */
    function itemRect(item) {
        const x = itemX(item);
        switch (modeOf(item)) {
            case "fullscreen":
                return { x, y: 0, w: viewSize.w, h: viewSize.h };
            case "maximized":
                return { x, y: parentArea.y, w: parentArea.w, h: parentArea.h };
            default:
                return areaRect(grid.area(item));
        }
    }

    /** Width of an item on screen, which the camera keeps in view, tab indicator included. */
    function itemWidth(item) {
        return itemRect(item).w;
    }

    /**
     * Space the tab indicator takes inside a tabbed area of `count` tabs, as `{ w, h }`, when the
     * config places it within the area rather than outside it.
     */
    function tabIndicatorSize(count) {
        const config = ws.config.tabIndicator;
        if (
            !config ||
            config.off ||
            !config.placeWithinColumn ||
            (config.hideWhenSingleTab && count === 1)
        ) {
            return { w: 0, h: 0 };
        }
        const size = Math.max(0, ws.round(config.width) + ws.round(config.gap));
        return config.position === "left" || config.position === "right"
            ? { w: size, h: 0 }
            : { w: 0, h: size };
    }

    /**
     * Rect of the windows of an item. Tabs share the item's rect, minus the space for the tab
     * indicator, which goes on its left or top side, or right or bottom.
     */
    function windowRect(item) {
        const rect = itemRect(item);
        if (!item.tabbed || modeOf(item) !== "normal") {
            return rect;
        }
        const extra = tabIndicatorSize(item.windows.length);
        const position = ws.config.tabIndicator?.position;
        const before = position === "left" || position === "top";
        return {
            x: rect.x + (before ? extra.w : 0),
            y: rect.y + (before ? extra.h : 0),
            w: Math.max(rect.w - extra.w, 1),
            h: Math.max(rect.h - extra.h, 1),
        };
    }

    /** Area of `cols` × `rows` cells nearest to a window with its top-left corner at `x, y`. */
    function areaAt(x, y, cols, rows) {
        const m = metrics;
        const col = round(x / (m.cellW + m.gap));
        const row = clamp(round((y - workingArea.y - m.gap) / (m.cellH + m.gap)), 0, m.rows - rows);
        return new Area(col, row, cols, rows);
    }

    // ---------------------------------------------------------------------------------------------
    // Applying changes
    // ---------------------------------------------------------------------------------------------

    /**
     * Brings the items in line with the grid after it changed.
     *
     * Sorts them top-left first, so that the order is stable; nothing should rely on it meaning
     * anything spatial beyond that. And the grid has the final say on sizes: if a window's size
     * resolves to a different span than its area, the user resized it, so remember the new size.
     * Fullscreen and maximized windows take more space for the time being.
     */
    function sync() {
        sortBy(items, (item) => {
            const area = grid.area(item);
            return [area.col, area.row];
        });

        for (const item of items) {
            if (modeOf(item) !== "normal") {
                continue;
            }
            const area = grid.area(item);
            const [cols, rows] = spanFor(item);
            if (area.cols !== cols) {
                item.fullWidth = false;
                item.cols = { cells: area.cols, of: metrics.colsPerView };
            }
            if (area.rows !== rows) {
                item.rows = { cells: area.rows, of: metrics.rows };
            }
        }
    }

    /**
     * Tells bento where every window goes, how it's sized, whether it shows and what has focus.
     *
     * Runs after every call that can change something. It only places windows that move, so the
     * others stay still.
     */
    function flush(animate = true) {
        const bars = new Map();
        for (const item of items) {
            const rect = windowRect(item);
            const mode = modeOf(item);
            item.windows.forEach((win, i) => {
                ws.setMode(win, mode);
                ws.setVisible(win, !item.tabbed || i === item.active);
                if (!sameRect(win.rect, rect)) {
                    ws.place(win, rect, { animate });
                }
            });

            // Tabbed areas get a tab indicator, except while they cover the view.
            if (item.tabbed && mode === "normal") {
                bars.set(item.key, {
                    area: rect,
                    windows: item.windows,
                    active: item.windows[item.active],
                });
            }
        }

        for (const [key, bar] of bars) {
            const shown = [
                bar.area.x,
                bar.area.y,
                bar.area.w,
                bar.area.h,
                bar.active.id,
                ...bar.windows.map((w) => w.id),
            ].join();
            if (tabBars.get(key) !== shown) {
                ws.setTabBar(key, bar);
                tabBars.set(key, shown);
            }
        }
        for (const key of [...tabBars.keys()]) {
            if (!bars.has(key)) {
                ws.setTabBar(key, null);
                tabBars.delete(key);
            }
        }

        const focus = active ? active.windows[active.active] : null;
        if (ws.focused !== focus) {
            ws.focus(focus);
            if (focus) {
                // Draw the focused window above any it overlaps, like a window too big for its
                // cells.
                ws.raise(focus);
            }
        }
    }

    /** Wraps a layout call so that bento gets what it changed. */
    function changes(call) {
        return (...args) => {
            const result = call(...args);
            flush();
            return result;
        };
    }

    // ---------------------------------------------------------------------------------------------
    // The camera
    // ---------------------------------------------------------------------------------------------
    //
    // Like niri, the camera position for a window is worked out as an offset from the window's x,
    // with `computeFit` keeping it in view and `computeCentered` centering it.

    function isCentering() {
        return (
            ws.config.centerFocusedColumn === "always" ||
            (ws.config.alwaysCenterSingleColumn && isSingleColumn())
        );
    }

    /**
     * Whether no two windows are side by side, like a single column in niri.
     *
     * Windows stacked on top of each other count as a single column even if their widths differ,
     * as long as some column of cells runs through all of them.
     */
    function isSingleColumn() {
        if (grid.length === 0) {
            return true;
        }
        const left = Math.max(...grid.items.map(({ area }) => area.col));
        const right = Math.min(...grid.items.map(({ area }) => area.right));
        return left < right;
    }

    /** Offset from `x` of the camera that keeps a window at `x` of `width` in view. */
    function computeFit(targetX, x, width, mode) {
        if (mode === "fullscreen") {
            return 0;
        }
        const [area, padding] =
            mode === "maximized" ? [parentArea, 0] : [workingArea, ws.config.gaps];
        const cur = (targetX ?? ws.camera.x) + area.x;

        // Non-fullscreen windows are always offset at least by the working area position.
        return computeNewViewOffset(cur, area.w, x, width, padding) - area.x;
    }

    /** Offset from `x` of the camera that centers a window at `x` of `width`. */
    function computeCentered(targetX, x, width, mode) {
        if (mode === "fullscreen") {
            return computeFit(targetX, x, width, mode);
        }
        const area = mode === "maximized" ? parentArea : workingArea;

        // Windows wider than the view are left-aligned (the fit code can deal with that).
        if (area.w <= width) {
            return computeFit(targetX, x, width, mode);
        }
        return -(area.w - width) / 2 - area.x;
    }

    function fitOffset(targetX, item) {
        return computeFit(targetX, itemX(item), itemWidth(item), modeOf(item));
    }

    function centeredOffset(targetX, item) {
        return computeCentered(targetX, itemX(item), itemWidth(item), modeOf(item));
    }

    /**
     * Camera offset from `item` for focusing it, coming from `prev`, following the config's
     * centering.
     */
    function viewOffsetFor(targetX, item, prev) {
        if (isCentering()) {
            return centeredOffset(targetX, item);
        }
        if (ws.config.centerFocusedColumn !== "on-overflow" || prev === null || prev === item) {
            return fitOffset(targetX, item);
        }

        // Always take the window next to the target on the side we're coming from as the source.
        // Moving up or down within the same columns of cells doesn't need any centering.
        const x = itemX(item);
        const right = x + itemWidth(item);
        const prevX = itemX(prev);
        const prevRight = prevX + itemWidth(prev);
        let dir;
        if (right <= prevX) {
            dir = Right;
        } else if (prevRight <= x) {
            dir = Left;
        } else {
            return fitOffset(targetX, item);
        }

        const source = grid.adjacent(item, dir, null);
        if (source === null) {
            return fitOffset(targetX, item);
        }
        const sourceX = itemX(source);
        const sourceRight = sourceX + itemWidth(source);
        const total = Math.max(right, sourceRight) - Math.min(x, sourceX) + ws.config.gaps * 2;

        // If it fits together, do a normal fit, otherwise center the new window.
        return total <= workingArea.w ? fitOffset(targetX, item) : centeredOffset(targetX, item);
    }

    /** Moves the camera to `x`, unless it's going there already. */
    function moveCameraTo(x) {
        if (Math.abs(x - ws.camera.x) < 1 / ws.view.scale) {
            return;
        }
        ws.moveCamera({ x });
    }

    /** Scrolls to `item` if it isn't in view, or centers it, depending on the config. */
    function cameraToItem(item, prev = null) {
        moveCameraTo(itemX(item) + viewOffsetFor(null, item, prev));
    }

    // ---------------------------------------------------------------------------------------------
    // Focus
    // ---------------------------------------------------------------------------------------------

    /** Focuses `item`, scrolling to it from the item that had focus before. */
    function activateItem(item) {
        if (active === item) {
            return;
        }
        cameraToItem(item, active);
        active = item;

        // A different window got focus.
        prevOnRemoval = null;
        restore = null;
        stopResize();
    }

    function activateTab(item, idx) {
        if (item.active === idx) {
            return false;
        }
        item.active = idx;
        return true;
    }

    function activateWindow(win) {
        const item = itemOf(win);
        if (item === null) {
            return;
        }
        activateTab(
            item,
            item.windows.findIndex((w) => w.id === win.id),
        );
        activateItem(item);
    }

    /** The focused item with the row and column of cells that focus is travelling along. */
    function focusLines() {
        if (active === null) {
            return null;
        }
        const area = grid.area(active);
        const a = anchor?.item === active ? anchor : null;
        return { item: active, row: a?.row ?? area.row, col: a?.col ?? area.col };
    }

    /**
     * Focuses the nearest window in `dir`, or with `adjacentOnly`, only one sharing rows or
     * columns of cells with the focused window.
     */
    function focusGrid(dir, adjacentOnly = false) {
        if (active === null) {
            return false;
        }
        const area = grid.area(active);
        const a = anchor?.item === active ? anchor : null;
        const row = a?.row ?? null;
        const col = a?.col ?? null;

        const along = isHorizontal(dir) ? row : col;
        const target = adjacentOnly
            ? grid.adjacent(active, dir, along)
            : grid.neighbor(active, dir, along);
        if (target === null) {
            return false;
        }
        const t = grid.area(target);

        // Keep travelling along the same row or column, and remember which side of the new window
        // we entered from.
        const lines = {
            [Right]: [row ?? area.row, t.col],
            [Left]: [row ?? area.row, t.right - 1],
            [Down]: [t.row, col ?? area.col],
            [Up]: [t.bottom - 1, col ?? area.col],
        }[dir];

        activateItem(target);
        anchor = { item: target, row: lines[0], col: lines[1] };
        return true;
    }

    /** Focuses the next tab down, or up, of the focused item if it's tabbed. */
    function focusTab(down) {
        if (active === null || !active.tabbed) {
            return false;
        }
        const idx = down
            ? Math.min(active.active + 1, active.windows.length - 1)
            : Math.max(active.active - 1, 0);
        return activateTab(active, idx);
    }

    /**
     * Focuses the window below that shares columns of cells with the focused one, like the tile
     * below in a niri column, rather than any window further down. For actions that do something
     * else when there's no window below.
     */
    function focusDownAdjacent() {
        return focusTab(true) || focusGrid(Down, true);
    }

    function focusUpAdjacent() {
        return focusTab(false) || focusGrid(Up, true);
    }

    /** Focuses `target`, travelling along `row` and `col` where it covers them. */
    function focusGridWindow(target, row, col) {
        const area = grid.area(target);
        activateItem(target);
        anchor = {
            item: target,
            row: clamp(row, area.row, area.bottom - 1),
            col: clamp(col, area.col, area.right - 1),
        };
    }

    /** Focuses the leftmost or rightmost window in the row that focus is travelling along. */
    function focusGridEnd(last) {
        const lines = focusLines();
        if (lines === null) {
            return false;
        }
        const inRow = grid.items.filter(
            ({ area }) => area.row <= lines.row && lines.row < area.bottom,
        );
        const target = last
            ? maxBy(inRow, ({ area }) => [area.right, -area.row])
            : minBy(inRow, ({ area }) => [area.col, area.row]);
        if (target === null) {
            return false;
        }
        const before = active;
        focusGridWindow(target.key, lines.row, last ? target.area.right - 1 : target.area.col);
        return active !== before;
    }

    /**
     * Windows in the stack along column of cells `col`, top to bottom, with tabs in order, as
     * `[item, tab index]`.
     */
    function stackTiles(col) {
        return grid.stack(col).flatMap((item) => item.windows.map((_, tab) => [item, tab]));
    }

    /**
     * Focuses a window in the stack along the column of cells that focus is travelling along,
     * counting tabs, at the index that `pick` returns for the stack's length.
     */
    function focusInStack(pick) {
        const lines = focusLines();
        if (lines === null) {
            return false;
        }
        const stack = stackTiles(lines.col);
        if (stack.length === 0) {
            return false;
        }

        const [target, tab] = stack[pick(stack.length)];
        const before = active.windows[active.active];
        const area = grid.area(target);
        focusGridWindow(target, clamp(lines.row, area.row, area.bottom - 1), lines.col);
        activateTab(active, tab);
        return active.windows[active.active] !== before;
    }

    /**
     * Window in grid column `index`, counting from 1 like the actions do: the one in the row that
     * focus is travelling along, or the top one. Indices past the end give the last column.
     */
    function gridColumnWindow(index) {
        const lines = focusLines();
        if (lines === null) {
            return null;
        }
        const starts = grid.columnStarts();
        const n = Number.isFinite(index) ? toInt(index) : 1;
        const start = starts[clamp(n - 1, 0, starts.length - 1)];

        const stack = grid.stack(start);
        const inRow = stack.find((item) => {
            const area = grid.area(item);
            return area.row <= lines.row && lines.row < area.bottom;
        });
        return inRow ?? stack[0] ?? null;
    }

    function focusColumn(index) {
        const target = gridColumnWindow(index);
        if (target === null) {
            return false;
        }
        const before = active;
        focusGridWindow(target, focusLines().row, grid.area(target).col);
        return active !== before;
    }

    // ---------------------------------------------------------------------------------------------
    // Adding and removing windows
    // ---------------------------------------------------------------------------------------------

    function addWindow(win, how) {
        // Windows coming back from floating or a drag keep the size they had, and windows that
        // were on the workspace before this layout keep theirs.
        const saved = remembered.get(win.id) ?? null;
        remembered.delete(win.id);
        let intent = saved ?? intentForNewWindow(win);
        if (saved === null && how.reason === "adopt" && how.rect) {
            intent = intentForSize(win, how.rect);
        }

        const item = makeItem([win], intent);
        item.fullscreen = win.mode === "fullscreen";
        item.maximized = win.mode === "maximized" || (item.fullscreen && saved?.maximized === true);

        const wasEmpty = items.length === 0;
        const [cols, rows] = spanFor(item);

        if (how.reason === "drop" && how.rect) {
            // Where the pointer dropped it, after pushing and gravity, see `dropTarget`.
            const plan = dropPlan(grid, how.pointer, how.rect);
            const areas = [[item, plan.area]];
            if (plan.split !== null) {
                areas.push(plan.split);
            }
            grid.place(areas);
        } else if (how.rect && (how.reason === "tile" || how.reason === "adopt")) {
            // A window that stops floating goes to the cells under where it was, at its size from
            // the last time it was tiled; one from the layout before this one, at its own size.
            grid.place([[item, areaAt(how.rect.x, how.rect.y, cols, rows)]]);
        } else {
            // New windows open right of the focused one, or of the window they belong to, lined
            // up with its top row, pushing the windows in the way to the right.
            const after = itemOf(how.nextTo) ?? active;
            grid.insertAfter(item, after, cols, rows);
        }

        items.push(item);
        sync();

        // A window that is already fullscreen or maximized pushes the others aside right away.
        if (modeOf(item) !== "normal") {
            fitToMode(item);
        }

        if (!how.activate) {
            if (active === null) {
                active = item;
            }
            return;
        }

        if (wasEmpty) {
            // The first window on an empty workspace: whatever camera position was left over
            // doesn't matter, so jump to where it would be for this window.
            const x = itemX(item);
            ws.moveCamera({ x: x + viewOffsetFor(x, item, null) }, { animate: false });
        }

        const prev = wasEmpty ? null : { opened: item, prev: active };
        activateItem(item);
        prevOnRemoval = prev;
    }

    function removeWindow(win, how) {
        const item = itemOf(win);
        if (item === null) {
            return;
        }

        // A window that floats or is dragged keeps its size for when it's tiled again.
        if (how.reason === "float" || how.reason === "drag") {
            remember(win.id, { ...intentOf(item), maximized: item.maximized });
        }
        if (resize?.win.id === win.id) {
            stopResize();
        }

        if (item.windows.length > 1) {
            removeTab(item, win);
        } else {
            removeItem(item);
        }
    }

    function remember(id, intent) {
        remembered.delete(id);
        remembered.set(id, intent);
        if (remembered.size > REMEMBERED_WINDOWS) {
            remembered.delete(remembered.keys().next().value);
        }
    }

    /** Removes a tab from a tabbed area, which keeps its cells, so nothing else moves. */
    function removeTab(item, win) {
        const idx = item.windows.findIndex((w) => w.id === win.id);
        item.windows.splice(idx, 1);
        if (idx < item.active || item.active === item.windows.length) {
            item.active -= 1;
        }
    }

    /**
     * Removes an item, which leaves a hole on the grid, collapses columns of cells that became
     * empty, and lets gravity pull windows left. The camera stays still, so the windows that move
     * slide into place, unless the focused window would leave the view.
     */
    function removeItem(item) {
        const before = grid.clone();
        const wasActive = item === active;

        const idx = items.indexOf(item);
        items.splice(idx, 1);
        const neighbor = items[Math.min(idx, items.length - 1)] ?? null;
        grid.remove(item);
        sync();

        if (resize?.item === item) {
            resize = null;
        }

        const prev = prevOnRemoval;
        prevOnRemoval = null;

        if (items.length === 0) {
            active = null;
            anchor = null;
            restore = null;
            return;
        }

        if (!wasActive) {
            // Keep going back to the previous window, unless that's the one that was removed.
            if (prev !== null && prev.prev !== item) {
                prevOnRemoval = prev;
            }

            // Gravity may have pulled the focused window out of view.
            cameraToItem(active);
            return;
        }

        restore = null;

        // Go back to the window that this one was opened from, if there were no focus changes
        // in-between. The view only scrolls if that window isn't in view.
        if (prev !== null && prev.opened === item && items.includes(prev.prev)) {
            active = prev.prev;
            cameraToItem(active);
            return;
        }

        // Otherwise, go to a neighbor of the removed window.
        const a = anchor?.item === item ? anchor : null;
        active = removalFallback(before, item, a) ?? neighbor;
        cameraToItem(active);
    }

    /**
     * Item to focus after removing the focused `removed` from `before`.
     *
     * Prefers the window below or above, like the tile below or above in a niri column, then the
     * one to the right or left, like the next or previous column.
     */
    function removalFallback(before, removed, a) {
        const row = a?.row ?? null;
        const col = a?.col ?? null;
        for (const [dir, along] of [
            [Down, col],
            [Up, col],
            [Right, row],
            [Left, row],
        ]) {
            const found = before.adjacent(removed, dir, along);
            if (found !== null) return found;
        }
        for (const [dir, along] of [
            [Right, row],
            [Left, row],
        ]) {
            const found = before.neighbor(removed, dir, along);
            if (found !== null) return found;
        }
        return null;
    }

    /**
     * Takes an item off the grid without the focus and camera handling of removing a window, for
     * its windows to go to another item. Keeps the same item focused if it can.
     */
    function takeItem(item) {
        const keep = active;
        items.splice(items.indexOf(item), 1);
        grid.remove(item);
        stopResizeFor(item);
        active = items.includes(keep) ? keep : (items[0] ?? null);
        prevOnRemoval = null;
    }

    /** Adds a new item with `win`, sized like `from`, at `area` on the grid, pushing others. */
    function putItem(win, from, area) {
        const item = makeItem([win], intentOf(from));
        // It was a tab, and is a window of its own now.
        item.tabbed = false;
        items.push(item);
        grid.insert(item, area);
        return item;
    }

    // ---------------------------------------------------------------------------------------------
    // Moving windows
    // ---------------------------------------------------------------------------------------------

    /** Syncs items after moving the focused window, then scrolls to it if it went out of view. */
    function finishMove() {
        sync();
        cameraToItem(active);
    }

    /** Moves the focused window by one cell in `dir`, swapping with windows it runs into. */
    function moveGrid(dir) {
        if (active === null || !grid.moveWindow(active, dir)) {
            return false;
        }
        finishMove();
        return true;
    }

    /** Moves the focused window up or down; tabs reorder within their area instead. */
    function moveVertical(dir) {
        if (active === null) {
            return false;
        }
        if (active.tabbed && active.windows.length > 1) {
            const from = active.active;
            const to = clamp(from + (dir === Down ? 1 : -1), 0, active.windows.length - 1);
            if (from === to) {
                return false;
            }
            [active.windows[from], active.windows[to]] = [active.windows[to], active.windows[from]];
            active.active = to;
            return true;
        }
        return moveGrid(dir);
    }

    /**
     * Moves the focused window to the column of cells from `place(area)`, which also gives the
     * direction to push others in.
     */
    function moveToColumn(place) {
        if (active === null) {
            return false;
        }
        const area = grid.area(active);
        const [col, dir] = place(area);
        if (col === area.col) {
            return false;
        }

        let moved = grid.setArea(active, area.with({ col }), dir);

        // Pushing left can run into the left edge of the grid. Push right instead, from the same
        // place.
        if (!moved && dir === Left) {
            moved = grid.setArea(active, area.with({ col: Math.max(col, 0) }), Right);
        }
        finishMove();
        return moved;
    }

    /** Moves the focused window to where the window in grid column `index` is. */
    function moveToIndex(index) {
        const target = gridColumnWindow(index);
        if (target === null) {
            return false;
        }
        const t = grid.area(target);
        return moveToColumn((area) =>
            t.col <= area.col
                ? // Take the target's place and push it right.
                  [t.col, Right]
                : // Line up with the target's right edge and push it left.
                  [t.right - area.cols, Left],
        );
    }

    /**
     * Applies a grid change from `change`, if it returns true, scrolling to the focused window if
     * it went out of view.
     */
    function changeGrid(change) {
        if (!change()) {
            return false;
        }
        if (active !== null) {
            stopResizeFor(active);
        }
        sync();
        if (active !== null) {
            cameraToItem(active);
        }
        return true;
    }

    // ---------------------------------------------------------------------------------------------
    // Consume and expel, tabs
    // ---------------------------------------------------------------------------------------------
    //
    // On the grid, a column of windows is a stack: windows on top of each other in the same
    // columns of cells. Consuming splits rows, expelling gives them back, and tabbed areas are
    // single items with several windows.

    /**
     * Expels `target` (or the focused window) from its stack towards `dir`, or if it's not
     * stacked with other windows, consumes it into the stack of the window beside it towards
     * `dir`.
     */
    function consumeOrExpel(target, dir) {
        const item = itemFor(target);
        if (item === null) {
            return false;
        }
        const area = grid.area(item);

        // A tab leaves its tabbed area.
        if (item.windows.length > 1) {
            const tab = target ? item.windows.findIndex((w) => w.id === target.id) : item.active;
            return changeGrid(() => expelTab(item, tab, dir));
        }

        const stacked = [Up, Down].some((d) => grid.adjacent(item, d, null) !== null);
        if (stacked) {
            return changeGrid(() => expel(item, dir));
        }

        const row = anchor?.item === item ? anchor.row : area.row;
        const into = grid.adjacent(item, dir, row);
        return into !== null && changeGrid(() => consume(item, into));
    }

    /**
     * Puts `item` in the column of cells of `into`, which keeps the top half of its rows, giving
     * `item` the bottom half, like consuming a window into a column in niri.
     */
    function consume(item, into) {
        // Fullscreen and maximized windows keep their place until they're back to normal.
        if (item === into || modeOf(item) !== "normal" || modeOf(into) !== "normal") {
            return false;
        }

        // A tabbed area takes the window as another tab.
        if (into.tabbed) {
            return consumeAsTab(item, into);
        }

        if (grid.area(into).rows < 2) {
            return false;
        }

        // Take the window out first so that its cells don't get in the way. Gravity can move the
        // target then, so look it up again after every change.
        grid.remove(item);
        let target = grid.area(into);
        const top = Math.floor((target.rows + 1) / 2);
        const bottom = target.rows - top;
        grid.setArea(into, target.with({ rows: top }), Up);

        target = grid.area(into);
        grid.insert(item, new Area(target.col, target.bottom, target.cols, bottom));
        return true;
    }

    /**
     * Takes `item` out of its stack, full height beside it towards `dir`, and gives its rows to
     * the window it was split from, like expelling a window from a column in niri.
     */
    function expel(item, dir) {
        if (modeOf(item) !== "normal") {
            return false;
        }
        const area = grid.area(item);

        // The window right above or below in exactly the same columns, if any, gets the rows
        // back.
        const vertical = [Up, Down]
            .map((d) => grid.adjacent(item, d, null))
            .filter((k) => k !== null);
        const partner =
            vertical.find((k) => {
                const other = grid.area(k);
                return other.col === area.col && other.cols === area.cols;
            }) ?? null;
        if (vertical.length === 0) {
            // Not stacked with anything.
            return false;
        }

        grid.remove(item);
        if (partner !== null) {
            const other = grid.area(partner);
            const row = Math.min(other.row, area.row);
            const bottom = Math.max(other.bottom, area.bottom);
            grid.setArea(partner, other.with({ row, rows: bottom - row }), Right);
        }

        // Go past everything sharing columns with where the window was, or before all of it.
        const stack = grid.items.filter(({ area: b }) => b.col < area.right && area.col < b.right);
        let col;
        if (dir === Left) {
            col = stack.length > 0 ? Math.min(...stack.map(({ area: b }) => b.col)) : area.col;
        } else {
            col = stack.length > 0 ? Math.max(...stack.map(({ area: b }) => b.right)) : area.right;
        }
        grid.insert(item, new Area(col, 0, area.cols, grid.rows));
        return true;
    }

    /**
     * Moves the windows of `item` into the tabbed `into` as tabs after its own, focusing the first
     * of them if `item` was focused.
     */
    function consumeAsTab(item, into) {
        const wasActive = active === item;
        takeItem(item);

        const first = into.windows.length;
        into.windows.push(...item.windows);
        if (wasActive) {
            into.active = first;
            active = into;
        }
        return true;
    }

    /**
     * Takes the tab at `tab` out of the tabbed `item`, putting it beside the area towards `dir` at
     * the same size, and focusing it if it was focused.
     */
    function expelTab(item, tab, dir) {
        if (modeOf(item) !== "normal") {
            return false;
        }
        const area = grid.area(item);
        const wasActive = item === active && tab === item.active;
        const [win] = item.windows.splice(tab, 1);
        if (tab < item.active || item.active === item.windows.length) {
            item.active -= 1;
        }

        const added = putItem(win, item, area.with({ col: dir === Left ? area.col : area.right }));
        if (wasActive) {
            active = added;
        }
        return true;
    }

    /**
     * Switches the focused item between tabbed and normal display.
     *
     * Going tabbed gathers the windows stacked right above and below it in exactly the same
     * columns of cells into its area as tabs. Going normal splits the tabs back into a stack,
     * sharing the area's rows.
     */
    function setDisplay(tabbed) {
        if (active === null || active.tabbed === tabbed || modeOf(active) !== "normal") {
            return false;
        }
        const item = active;
        return changeGrid(() => (tabbed ? tabStack(item) : untab(item)));
    }

    function tabStack(item) {
        const area = grid.area(item);
        const inSameColumns = (other) => other.col === area.col && other.cols === area.cols;

        // Nearest first.
        const above = [];
        let top = area;
        for (;;) {
            const next = grid.items.find(({ area: a }) => inSameColumns(a) && a.bottom === top.row);
            if (!next) break;
            above.push(next.key);
            top = next.area;
        }
        const below = [];
        let bottom = area;
        for (;;) {
            const next = grid.items.find(
                ({ area: a }) => inSameColumns(a) && a.row === bottom.bottom,
            );
            if (!next) break;
            below.push(next.key);
            bottom = next.area;
        }

        item.tabbed = true;
        for (const other of above) {
            takeItem(other);
            item.windows.unshift(...other.windows);
            item.active += other.windows.length;
        }
        for (const other of below) {
            takeItem(other);
            item.windows.push(...other.windows);
        }

        const merged = area.with({ row: top.row, rows: bottom.bottom - top.row });
        grid.setArea(item, merged, Down);
        active = item;
        return true;
    }

    /** Splits the tabs of `item` into a stack sharing its rows. Tabs without a row go after it. */
    function untab(item) {
        const area = grid.area(item);
        const count = item.windows.length;
        const activeTab = item.active;

        const tabs = item.windows.splice(1);
        item.active = 0;
        item.tabbed = false;

        // Split the rows as evenly as possible, giving the remainder to the top ones.
        const stacked = Math.min(count, area.rows);
        const base = Math.floor(area.rows / stacked);
        const extra = area.rows % stacked;
        const rowsOf = (i) => base + (i < extra ? 1 : 0);

        let row = area.row;
        const slices = [];
        for (let i = 0; i < count; i++) {
            if (i < stacked) {
                slices.push(area.with({ row, rows: rowsOf(i) }));
                row += rowsOf(i);
            } else {
                slices.push(area.with({ col: area.right }));
            }
        }

        grid.setArea(item, slices[0], Down);
        let focused = item;
        tabs.forEach((win, i) => {
            const added = putItem(win, item, slices[i + 1]);
            if (i + 1 === activeTab) {
                focused = added;
            }
        });
        active = focused;
        return true;
    }

    // ---------------------------------------------------------------------------------------------
    // Sizes
    // ---------------------------------------------------------------------------------------------

    /** Resizes an item to a new grid span, pushing others aside. */
    function resizeItem(item, cols, rows) {
        // Fullscreen and maximized windows take their size from the view until they're back.
        if (item === null || modeOf(item) !== "normal") {
            return false;
        }
        const area = grid.area(item);
        if (!grid.resize(item, cols ?? area.cols, rows ?? area.rows)) {
            return false;
        }
        sync();

        // Make sure the resized window stays in view.
        if (item === active) {
            cameraToItem(active);
        }
        return true;
    }

    function isSizeChange(change) {
        return (
            change !== null &&
            typeof change === "object" &&
            (change.op === "set" || change.op === "adjust") &&
            (change.unit === "px" || change.unit === "%") &&
            Number.isFinite(change.value)
        );
    }

    function setWidth(target, change) {
        const item = itemFor(target);
        if (item === null || !isSizeChange(change)) {
            return false;
        }
        const m = metrics;
        const cols = grid.area(item).cols;

        let n;
        if (change.op === "set" && change.unit === "px") {
            // Like in niri, a fixed width is for the window, not counting its border.
            n = colsForWidth({ fixed: change.value + item.windows[0].borders.w });
        } else if (change.op === "set") {
            n = m.colsForProportion(change.value / 100);
        } else if (change.unit === "px") {
            n = cols + cellsForDelta(change.value, m.cellW + m.gap);
        } else {
            n = cols + cellsForDelta((change.value / 100) * m.colsPerView, 1);
        }
        return resizeItem(item, Math.max(n, 1), null);
    }

    function setHeight(target, change) {
        const item = itemFor(target);
        if (item === null || !isSizeChange(change)) {
            return false;
        }
        const m = metrics;
        const rows = grid.area(item).rows;

        let n;
        if (change.op === "set" && change.unit === "px") {
            // Like in niri, a fixed height is for the window, not counting its border.
            n = rowsForHeight(change.value + item.windows[0].borders.h);
        } else if (change.op === "set") {
            n = m.rowsForProportion(change.value / 100);
        } else if (change.unit === "px") {
            n = rows + cellsForDelta(change.value, m.cellH + m.gap);
        } else {
            n = rows + cellsForDelta((change.value / 100) * m.rows, 1);
        }
        return resizeItem(item, null, clamp(n, 1, m.rows));
    }

    function switchPresetWidth(target, forwards) {
        const item = itemFor(target);
        if (item === null) {
            return false;
        }
        const borders = item.windows[0].borders.w;
        const spans = ws.config.presetColumnWidths.map((preset) => colsForPreset(preset, borders));
        return resizeItem(item, nextPresetSpan(grid.area(item).cols, spans, forwards), null);
    }

    function switchPresetHeight(target, forwards) {
        const item = itemFor(target);
        if (item === null) {
            return false;
        }
        const borders = item.windows[0].borders.h;
        const spans = ws.config.presetWindowHeights.map((preset) => rowsForPreset(preset, borders));
        return resizeItem(item, null, nextPresetSpan(grid.area(item).rows, spans, forwards));
    }

    /** Toggles the focused window between the full view width and the width it had before. */
    function toggleFullWidth() {
        if (active === null || modeOf(active) !== "normal") {
            return false;
        }

        // The window keeps its own width while full-width, so it comes back to it.
        active.fullWidth = !active.fullWidth;
        resizeItem(active, spanFor(active)[0], null);
        return true;
    }

    /**
     * Grows the focused window into the empty cells beside it in its rows, right and left, up to
     * the next window or the edge of the view, without pushing anything.
     */
    function expandToAvailableWidth() {
        if (active === null || modeOf(active) !== "normal" || active.fullWidth) {
            return false;
        }
        const area = grid.area(active);

        // Columns of cells fully inside the working area, once the camera gets where it's going.
        const m = metrics;
        const step = m.cellW + m.gap;
        const viewLeft = ws.camera.x + workingArea.x;
        const viewRight = viewLeft + workingArea.w;
        const firstVisible = toInt(Math.ceil(viewLeft / step));
        const endVisible = toInt(Math.floor((viewRight - m.cellW) / step)) + 1;

        // Edges of the nearest windows on either side in the same rows.
        const others = grid.items.filter(
            ({ key, area: b }) => key !== active && b.row < area.bottom && area.row < b.bottom,
        );
        const rightLimit = Math.min(
            ...others.map(({ area: b }) => b.col).filter((col) => col >= area.right),
        );
        // With gravity, the grid starts at column 0. Without, it goes on to the left too.
        const lefts = others.map(({ area: b }) => b.right).filter((right) => right <= area.col);
        const leftLimit = lefts.length > 0 ? Math.max(...lefts) : grid.gravity ? 0 : -Infinity;

        const left = Math.min(area.col, Math.max(leftLimit, firstVisible));
        const right = Math.max(area.right, Math.min(rightLimit, endVisible));
        const expanded = area.with({ col: left, cols: right - left });
        if (expanded.equals(area)) {
            return false;
        }

        stopResizeFor(active);
        grid.setArea(active, expanded, Right);
        sync();
        cameraToItem(active);
        return true;
    }

    // ---------------------------------------------------------------------------------------------
    // Fullscreen and maximized windows
    // ---------------------------------------------------------------------------------------------

    function requestMode(win, mode) {
        const item = itemOf(win);
        if (item === null) {
            return;
        }
        const fullscreen = mode === "fullscreen";
        const maximized = mode === "maximized" || (fullscreen && item.maximized);
        changeMode(item, fullscreen, maximized);
    }

    /**
     * Makes an item fullscreen, maximized or normal. Fullscreen and maximized windows push the
     * windows beside them aside on the grid, and the camera goes back to where it was when they
     * stop.
     */
    function changeMode(item, fullscreen, maximized) {
        if (item.fullscreen === fullscreen && item.maximized === maximized) {
            return false;
        }
        stopResizeFor(item);

        const wasNormal = modeOf(item) === "normal";
        item.fullscreen = fullscreen;
        item.maximized = maximized;
        fitToMode(item);

        if (item !== active) {
            return true;
        }

        // When the focused window goes fullscreen, remember the camera to restore later.
        const isNormal = modeOf(item) === "normal";
        if (wasNormal && !isNormal) {
            restore = { item, offset: ws.camera.x - itemX(item) };
        }

        if (!wasNormal && isNormal && restore?.item === item) {
            moveCameraTo(itemX(item) + restore.offset);
        }
        if (isNormal) {
            restore = null;
        }

        cameraToItem(item);
        return true;
    }

    /**
     * Gives an item that is going fullscreen or maximized enough cells to cover its width, across
     * all rows, or gives it back its own size and row when it's going back to normal.
     */
    function fitToMode(item) {
        const area = grid.area(item);
        const [cols, rows] = spanInMode(item);

        let next;
        if (modeOf(item) === "normal") {
            const row = item.rowToRestore ?? area.row;
            item.rowToRestore = null;
            next = new Area(area.col, row, cols, rows);
        } else {
            item.rowToRestore ??= area.row;
            next = new Area(area.col, 0, cols, rows);
        }
        if (next.equals(area)) {
            return;
        }

        grid.setArea(item, next, Right);
        sync();
    }

    // ---------------------------------------------------------------------------------------------
    // The view
    // ---------------------------------------------------------------------------------------------

    function centerColumn() {
        if (active === null) {
            return false;
        }
        moveCameraTo(itemX(active) + centeredOffset(null, active));
        stopResizeFor(active);
        return true;
    }

    /** Centers the windows that are fully in view together, by their span on the grid. */
    function centerVisibleColumns() {
        if (active === null || isCentering()) {
            return false;
        }

        const viewX = ws.camera.x;
        const { x: workingX, w: workingW } = workingArea;
        const gap = ws.config.gaps;

        let span = null;
        let activeVisible = false;
        for (const item of items) {
            const x = itemX(item);
            const right = x + itemWidth(item);
            if (x < viewX + workingX + gap || viewX + workingX + workingW < right + gap) {
                // The window goes off-screen.
                continue;
            }
            activeVisible ||= item === active;
            span = span === null ? [x, right] : [Math.min(span[0], x), Math.max(span[1], right)];
        }

        if (!activeVisible) {
            // The focused window isn't fully in view, so there's nothing meaningful to do.
            return false;
        }

        stopResizeFor(active);
        const [left, right] = span;
        const free = workingW - (right - left);
        moveCameraTo(left - free / 2 - workingX);
        // Just in case.
        cameraToItem(active);
        return true;
    }

    /**
     * Snaps the camera after a swipe to the window edges nearest to where it would coast, and
     * focuses the furthest window in view in the direction of the swipe, preferring the row focus
     * was in.
     */
    function gestureEnd(velocity, projected) {
        if (items.length === 0) {
            return;
        }
        // Snap for where the camera would coast to after the swipe.
        const current = ws.camera.x;
        const target = projected?.x ?? current;
        const gaps = ws.config.gaps;

        // Snapping points: camera positions where the view lines up with window edges on either
        // side, with the window to focus for each.
        const snaps = [];
        if (isCentering()) {
            for (const item of items) {
                const x = itemX(item);
                const w = itemWidth(item);
                const mode = modeOf(item);
                const area = mode === "maximized" ? parentArea : workingArea;
                let pos;
                if (mode === "fullscreen") {
                    pos = x;
                } else if (area.w <= w) {
                    pos = x - area.x;
                } else {
                    pos = x - (area.w - w) / 2 - area.x;
                }
                snaps.push({ pos, item });
            }
        } else {
            const onOverflow = ws.config.centerFocusedColumn === "on-overflow";

            // Width of the window next to `item` in `dir` on the grid, if any.
            const adjacentWidth = (item, dir) => {
                const other = grid.adjacent(item, dir, null);
                return other === null ? null : itemWidth(other);
            };

            const edgesOf = (item) => {
                const x = itemX(item);
                const w = itemWidth(item);
                const mode = modeOf(item);

                // Normal windows align with the working area, but fullscreen ones with the view.
                if (mode === "fullscreen") {
                    return [x, x + w - viewSize.w];
                }

                const area = mode === "maximized" ? parentArea : workingArea;
                const leftStrut = area.x;
                const rightStrut = viewSize.w - area.w - area.x;
                // Logic from computeNewViewOffset.
                const padding = mode === "maximized" ? 0 : clamp((area.w - w) / 2, 0, gaps);
                const center = area.w <= w ? x - leftStrut : x - (area.w - w) / 2 - leftStrut;
                const isOverflowing = (adjacentW) =>
                    onOverflow && adjacentW !== null && adjacentW + 3 * gaps + w > area.w;

                const left = isOverflowing(adjacentWidth(item, Right))
                    ? center
                    : x - padding - leftStrut;
                const right = isOverflowing(adjacentWidth(item, Left))
                    ? center + viewSize.w
                    : x + w + padding + rightStrut;
                return [left, right - viewSize.w];
            };
            const edges = items.map((item) => [item, edgesOf(item)]);

            // Don't snap further than the leftmost or rightmost window, as that's generally not
            // wanted. It's fine if the leftmost snap is right of the rightmost one: that happens
            // when the windows span less than the view.
            const leftmost = minBy(edges, ([, [left]]) => [left]);
            const rightmost = maxBy(edges, ([, [, right]]) => [right]);
            const leftmostSnap = leftmost[1][0];
            const rightmostSnap = rightmost[1][1];
            snaps.push({ pos: leftmostSnap, item: leftmost[0] });
            snaps.push({ pos: rightmostSnap, item: rightmost[0] });

            for (const [item, [left, right]] of edges) {
                for (const pos of [left, right]) {
                    if (leftmostSnap < pos && pos < rightmostSnap) {
                        snaps.push({ pos, item });
                    }
                }
            }
        }

        const snap = minBy(snaps, (s) => [Math.abs(s.pos - target)]);
        let next = snap.item;

        if (!isCentering()) {
            // Focus the furthest window towards the direction of the swipe that is fully in view
            // after snapping, preferring the row focus was in.
            const isVisible = (item) => {
                const x = itemX(item);
                const w = itemWidth(item);
                const mode = modeOf(item);
                if (mode === "fullscreen") {
                    return snap.pos <= x && x + w <= snap.pos + viewSize.w;
                }
                const area = mode === "maximized" ? parentArea : workingArea;
                const padding = mode === "maximized" ? 0 : clamp((area.w - w) / 2, 0, gaps);
                return (
                    snap.pos + area.x <= x - padding &&
                    x + w + padding <= snap.pos + area.x + area.w
                );
            };

            const row = focusLines()?.row ?? null;
            const inRow = (item) => {
                const area = grid.area(item);
                return row !== null && area.row <= row && row < area.bottom;
            };

            const forward = target >= current;
            const furthest = (inRowOnly) =>
                maxBy(
                    items.filter((item) => isVisible(item) && (!inRowOnly || inRow(item))),
                    (item) => {
                        const left = itemX(item);
                        const right = left + itemWidth(item);
                        return forward ? [right, left] : [-left, -right];
                    },
                );
            next = furthest(true) ?? furthest(false) ?? next;
        }

        if (active !== next) {
            restore = null;
            prevOnRemoval = null;
        }
        active = next;

        // bento animates from the speed of the swipe.
        ws.moveCamera({ x: snap.pos });
        // Deal with things like snapping to the right edge of a window wider than the view.
        cameraToItem(next);
    }

    /**
     * Works out the grid again for the view or config, which can change the cells. Windows keep
     * their order, and their sizes resolve to spans on the new cells, so proportional sizes stay
     * proportional.
     */
    function reconfigure() {
        const activeX = active === null ? null : itemX(active);
        const prevColsPerView = metrics.colsPerView;
        updateGeometry();
        rescale(prevColsPerView);

        let animate = false;
        if (grid.gravity !== gravityOption()) {
            grid.setGravity(gravityOption());
            sync();
            animate = true;
        } else {
            sync();
            // Cell size changes move every window, so keep the view on the focused one.
            if (activeX !== null && itemX(active) !== activeX) {
                ws.moveCamera({ x: ws.camera.x + itemX(active) - activeX }, { animate: false });
            }
        }

        // Apply always-center and such right away.
        if (active !== null) {
            cameraToItem(active);
        }
        flush(animate);
    }

    /** Resizes every item to its span on the current cells, if that changed anything. */
    function rescale(prevColsPerView) {
        const spans = new Map(items.map((item) => [item, spanInMode(item)]));
        const resized = items.some((item) => {
            const area = grid.area(item);
            const [cols, rows] = spans.get(item);
            return area.cols !== cols || area.rows !== rows;
        });
        if (resized || metrics.colsPerView !== prevColsPerView) {
            grid.rescale(metrics.colsPerView / prevColsPerView, (item) => spans.get(item));
        }
    }

    // ---------------------------------------------------------------------------------------------
    // Pointer resizing
    // ---------------------------------------------------------------------------------------------
    //
    // The size snaps to cells and neighbours get pushed aside as the pointer moves, always
    // starting over from the grid as it was when the resize started, so windows pushed aside come
    // back when the resize shrinks again. The window being resized is pinned: gravity leaves it
    // where it is until the resize ends.

    function resizeBegin(win, r) {
        if (resize !== null) {
            return false;
        }
        const item = itemOf(win);
        if (item === null || modeOf(item) !== "normal") {
            return false;
        }

        grid.setPinned(item);
        resize = { win, item, edges: r.edges, original: grid.clone() };
        return true;
    }

    function resizeUpdate(win, r) {
        if (resize === null || resize.win.id !== win.id) {
            return;
        }
        resizeOnGrid(r.delta);
    }

    function resizeEnd(win) {
        if (resize === null || resize.win.id !== win.id) {
            return;
        }
        // Scroll to the window right away.
        if (resize.item === active) {
            cameraToItem(active);
        }
        stopResize();
    }

    /** Ends the ongoing resize, if any, and lets the window held by the pointer fall into place. */
    function stopResize() {
        resize = null;
        if (grid.pinned !== null) {
            grid.setPinned(null);
            sync();
        }
    }

    function stopResizeFor(item) {
        if (resize !== null && resize.item === item) {
            stopResize();
        }
    }

    /** Resizes by the pointer moving `delta` since the start, snapping to cells. */
    function resizeOnGrid(delta) {
        const { item, edges, original } = resize;

        // Windows opened or closed during the resize make the snapshot stale, so stop there.
        if (original.length !== items.length || !items.every((it) => original.area(it) !== null)) {
            stopResize();
            return;
        }
        const orig = original.area(item);

        const m = metrics;
        const dx = round(delta.x / (m.cellW + m.gap));
        const dy = round(delta.y / (m.cellH + m.gap));

        let next = original.clone();
        if (edges.top || edges.bottom) {
            if (edges.top) {
                const row = clamp(orig.row + dy, 0, orig.bottom - 1);
                next.setArea(item, orig.with({ row, rows: orig.bottom - row }), Up);
            } else {
                const rows = clamp(orig.rows + dy, 1, m.rows - orig.row);
                next.setArea(item, orig.with({ rows }), Down);
            }
        }
        if (edges.left || edges.right) {
            const cur = next.area(item);
            if (edges.left) {
                // Grow left only as far as the windows to the left can go before the left edge
                // of the grid. The opposite edge stays still.
                let by = Math.min(dx, orig.cols - 1);
                for (;;) {
                    const attempt = next.clone();
                    const area = cur.with({ col: cur.col + by, cols: orig.cols - by });
                    if (attempt.setArea(item, area, Left) || by >= 0) {
                        next = attempt;
                        break;
                    }
                    by += 1;
                }
            } else {
                const cols = Math.max(orig.cols + dx, 1);
                next.setArea(item, cur.with({ cols }), Right);
            }
        }

        if (next.equals(grid)) {
            return;
        }
        grid = next;
        sync();
    }

    // ---------------------------------------------------------------------------------------------
    // Dragging and dropping windows
    // ---------------------------------------------------------------------------------------------

    /**
     * Where a window dragged to `rect` with the pointer at `pointer` drops on grid `g`, as
     * `{ area, split }`.
     *
     * Holding the pointer close to an edge of another window splits that window: the dropped
     * window takes the half of its cells on that side, and `split` is `[item, area]` with the
     * other half. Otherwise the window goes to the cells nearest to its top-left corner, at its
     * size, and `area` is where it lands after pushing and gravity.
     */
    function dropPlan(g, pointer, rect) {
        const split = pointer ? splitAt(g, pointer) : null;
        if (split !== null) {
            return split;
        }

        const m = metrics;
        const cols = clamp(round((rect.w + m.gap) / (m.cellW + m.gap)), 1, MAX_COLS);
        const rows = clamp(round((rect.h + m.gap) / (m.cellH + m.gap)), 1, m.rows);
        const area = areaAt(rect.x, rect.y, cols, rows);

        // Where it lands after pushing and gravity.
        const preview = g.clone();
        const probe = {};
        preview.insert(probe, area);
        return { area: preview.area(probe), split: null };
    }

    /**
     * The split for a window dropped with the pointer at `pointer`: over another window, close to
     * one of its edges.
     *
     * The edges are a cell deep, or a quarter of the window for small windows, so that dropping
     * in the middle always drops normally. A window one cell wide or tall can't be split that way.
     */
    function splitAt(g, pointer) {
        const { x, y } = pointer;
        const m = metrics;

        const under = g.items.find(({ area }) => {
            const r = areaRect(area);
            return r.x <= x && x < r.x + r.w && r.y <= y && y < r.y + r.h;
        });
        if (!under || modeOf(under.key) !== "normal") {
            return null;
        }

        const { key: item, area } = under;
        const r = areaRect(area);
        const zoneW = Math.min(r.w / 4, m.cellW);
        const zoneH = Math.min(r.h / 4, m.cellH);
        const edges = [
            { dir: Left, dist: x - r.x, zone: zoneW, span: area.cols },
            { dir: Right, dist: r.x + r.w - x, zone: zoneW, span: area.cols },
            { dir: Up, dist: y - r.y, zone: zoneH, span: area.rows },
            { dir: Down, dist: r.y + r.h - y, zone: zoneH, span: area.rows },
        ].filter((e) => e.span >= 2 && e.dist < e.zone);
        const edge = minBy(edges, (e) => [e.dist / e.zone]);
        if (edge === null) {
            return null;
        }

        const [dropped, kept] = splitAreas(area, edge.dir);
        return { area: dropped, split: [item, kept] };
    }

    /**
     * Areas that a window dropped to split `a` in `dir` and the window at `a` get. The dropped
     * window takes the half on the side in `dir`, rounding down.
     */
    function splitAreas(a, dir) {
        const halfCols = Math.floor(a.cols / 2);
        const halfRows = Math.floor(a.rows / 2);
        switch (dir) {
            case Left:
                return [
                    new Area(a.col, a.row, halfCols, a.rows),
                    new Area(a.col + halfCols, a.row, a.cols - halfCols, a.rows),
                ];
            case Right:
                return [
                    new Area(a.right - halfCols, a.row, halfCols, a.rows),
                    new Area(a.col, a.row, a.cols - halfCols, a.rows),
                ];
            case Up:
                return [
                    new Area(a.col, a.row, a.cols, halfRows),
                    new Area(a.col, a.row + halfRows, a.cols, a.rows - halfRows),
                ];
            case Down:
                return [
                    new Area(a.col, a.bottom - halfRows, a.cols, halfRows),
                    new Area(a.col, a.row, a.cols, a.rows - halfRows),
                ];
        }
    }

    /** The cells a dragged window would land on, to highlight them. */
    function dropTarget(win, pointer, rect) {
        // The dragged window doesn't count, in case it's still in the layout.
        const g = grid.clone();
        const own = itemOf(win);
        if (own !== null) {
            g.remove(own);
        }

        const plan = dropPlan(g, pointer, rect);
        const probe = {};
        g.place([[probe, plan.area], ...(plan.split ? [plan.split] : [])]);
        const landing = areaRect(g.area(probe));

        if (g.length > 1) {
            return landing;
        }

        // The first window on an empty workspace moves the camera, see `addWindow`. Show the
        // highlight where the window will be on screen then.
        const fit = isCentering() ? computeCentered : computeFit;
        const camera = landing.x + fit(landing.x, landing.x, landing.w, "normal");
        return { ...landing, x: landing.x - camera + ws.camera.x };
    }

    // ---------------------------------------------------------------------------------------------
    // Moving between workspaces, reloading, switching layouts
    // ---------------------------------------------------------------------------------------------

    /** Reads a size intent from saved or transferred data, or `null` if it doesn't make sense. */
    function readIntent(data) {
        const isPreset = (p) =>
            p !== null &&
            typeof p === "object" &&
            (Number.isFinite(p.proportion) || Number.isFinite(p.fixed));
        const isSpan = (s) =>
            s === null ||
            (typeof s === "object" &&
                ((Number.isInteger(s.cells) && Number.isInteger(s.of)) ||
                    isPreset(s.preset ?? null)));
        if (
            !data ||
            !isPreset(data.width ?? null) ||
            !isSpan(data.cols ?? null) ||
            !isSpan(data.rows ?? null)
        ) {
            return null;
        }
        return {
            width: data.width,
            fullWidth: data.fullWidth === true,
            cols: data.cols ?? null,
            rows: data.rows ?? null,
            maximized: data.maximized === true,
        };
    }

    /** A window moves to another workspace on its own, with its size. */
    function transferOut(win) {
        const item = itemOf(win);
        if (item === null) {
            return { windows: [win] };
        }
        return {
            windows: [win],
            data: {
                layout: "bento",
                windows: [{ id: win.id, ...intentOf(item), maximized: item.maximized }],
            },
        };
    }

    function transferIn(windows, data, how) {
        const sizes = data?.layout === "bento" && Array.isArray(data.windows) ? data.windows : [];
        windows.forEach((win, i) => {
            const intent = readIntent(sizes.find((size) => size?.id === win.id));
            if (intent !== null) {
                remember(win.id, intent);
            }
            addWindow(win, {
                reason: "move",
                activate: how.activate && i === 0,
                rect: null,
                pointer: null,
                nextTo: null,
            });
        });
    }

    /** State to come back to when the layout reloads. */
    function save() {
        const ref = (item) => (item && items.includes(item) ? item.windows[0].id : null);
        return {
            layout: "bento",
            version: 1,
            gravity: grid.gravity,
            colsPerView: metrics.colsPerView,
            items: items.map((item) => ({
                key: item.key,
                windows: item.windows.map((win) => win.id),
                active: item.active,
                tabbed: item.tabbed,
                area: grid.area(item).toJSON(),
                ...intentOf(item),
                fullscreen: item.fullscreen,
                maximized: item.maximized,
                rowToRestore: item.rowToRestore,
            })),
            active: ref(active),
            anchor:
                anchor && ref(anchor.item) !== null
                    ? { item: ref(anchor.item), row: anchor.row, col: anchor.col }
                    : null,
            prevOnRemoval:
                prevOnRemoval &&
                ref(prevOnRemoval.opened) !== null &&
                ref(prevOnRemoval.prev) !== null
                    ? { opened: ref(prevOnRemoval.opened), prev: ref(prevOnRemoval.prev) }
                    : null,
            restore:
                restore && ref(restore.item) !== null
                    ? { item: ref(restore.item), offset: restore.offset }
                    : null,
            remembered: [...remembered],
        };
    }

    /**
     * Takes over windows: from saved state after a reload, or otherwise from where they are, like
     * when switching from another layout.
     */
    function adopt(windows, focused, state) {
        // The grid is a horizontal strip, so the camera stays at the top.
        if (ws.camera.y !== 0) {
            ws.moveCamera({ y: 0 }, { animate: false });
        }

        const restored = restoreState(state, windows);
        adoptWindows(windows.filter((win) => !restored.has(win.id)));
        sync();
        // Fullscreen and maximized windows push the others aside.
        for (const item of items) {
            if (modeOf(item) !== "normal") {
                fitToMode(item);
            }
        }

        const focusedItem = itemOf(focused);
        if (focusedItem !== null) {
            activateTab(
                focusedItem,
                focusedItem.windows.findIndex((w) => w.id === focused.id),
            );
            active = focusedItem;
        }
        active ??= items[0] ?? null;
        if (active !== null) {
            cameraToItem(active);
        }
    }

    /** Puts windows back where `state` from `save` had them. Returns the ids of the windows. */
    function restoreState(state, windows) {
        const restored = new Set();
        if (state?.layout !== "bento" || state.version !== 1 || !Array.isArray(state.items)) {
            return restored;
        }
        const byId = new Map(windows.map((win) => [win.id, win]));
        const byWindow = new Map();

        const placed = [];
        for (const saved of state.items) {
            const intent = readIntent(saved);
            const wins = (Array.isArray(saved.windows) ? saved.windows : [])
                .map((id) => byId.get(id))
                .filter((win) => win && !restored.has(win.id));
            const area = saved.area;
            const validArea =
                area &&
                [area.col, area.row, area.cols, area.rows].every(Number.isInteger) &&
                area.cols > 0 &&
                area.rows > 0;
            if (intent === null || wins.length === 0 || !validArea) {
                continue;
            }

            const item = makeItem(wins, intent);
            // Keep the tab bar's key, so that bento keeps its animations.
            const id = /^area-(\d+)$/.exec(saved.key ?? "")?.[1];
            if (id !== undefined && !placed.some(([other]) => other.key === saved.key)) {
                item.key = saved.key;
                nextItemId = Math.max(nextItemId, Number(id) + 1);
            }
            item.tabbed = saved.tabbed === true;
            item.active = clamp(toInt(saved.active), 0, wins.length - 1);
            item.fullscreen = saved.fullscreen === true;
            item.maximized = saved.maximized === true;
            item.rowToRestore = Number.isInteger(saved.rowToRestore) ? saved.rowToRestore : null;
            for (const win of wins) {
                restored.add(win.id);
                byWindow.set(win.id, item);
            }
            placed.push([item, Area.from(area)]);
        }

        // The saved areas came from a grid, so they don't overlap, unless the state is broken.
        // Put them back as they were, then apply the current cells and gravity.
        grid.gravity = state.gravity === true;
        const overlapping = placed.some(([, a], i) =>
            placed.slice(0, i).some(([, b]) => a.overlaps(b)),
        );
        if (overlapping) {
            for (const [item, area] of placed) grid.insert(item, area);
        } else {
            grid.place(placed);
        }
        items = placed.map(([item]) => item);
        sync();
        rescale(
            Number.isInteger(state.colsPerView) && state.colsPerView > 0
                ? state.colsPerView
                : metrics.colsPerView,
        );
        grid.setGravity(gravityOption());
        sync();

        const find = (id) => byWindow.get(id) ?? null;
        active = find(state.active);
        if (state.anchor && find(state.anchor.item)) {
            anchor = {
                item: find(state.anchor.item),
                row: toInt(state.anchor.row),
                col: toInt(state.anchor.col),
            };
        }
        if (
            state.prevOnRemoval &&
            find(state.prevOnRemoval.opened) &&
            find(state.prevOnRemoval.prev)
        ) {
            prevOnRemoval = {
                opened: find(state.prevOnRemoval.opened),
                prev: find(state.prevOnRemoval.prev),
            };
        }
        if (state.restore && find(state.restore.item) && Number.isFinite(state.restore.offset)) {
            restore = { item: find(state.restore.item), offset: state.restore.offset };
        }
        for (const [id, saved] of Array.isArray(state.remembered) ? state.remembered : []) {
            const intent = readIntent(saved);
            if (intent !== null) remember(id, intent);
        }
        return restored;
    }

    /**
     * Adds windows that the layout takes over without saved state. Windows with a place on the
     * canvas go to the cells nearest to it at their size, left to right; the others go after
     * them, in order.
     */
    function adoptWindows(windows) {
        const withRects = sortBy(
            windows.filter((win) => win.rect),
            (win) => [win.rect.x, win.rect.y],
        );
        for (const win of withRects) {
            const item = makeItem([win], intentForSize(win, win.rect));
            item.fullscreen = win.mode === "fullscreen";
            item.maximized = win.mode === "maximized";
            const [cols, rows] = spanFor(item);
            grid.place([[item, areaAt(win.rect.x, win.rect.y, cols, rows)]]);
            items.push(item);
        }

        let last = null;
        for (const win of windows.filter((win) => !win.rect)) {
            const item = makeItem([win], intentForSize(win, win.size));
            item.fullscreen = win.mode === "fullscreen";
            item.maximized = win.mode === "maximized";
            const [cols, rows] = spanFor(item);
            grid.insertAfter(item, last, cols, rows);
            items.push(item);
            last = item;
        }
    }

    // ---------------------------------------------------------------------------------------------
    // Queries
    // ---------------------------------------------------------------------------------------------

    /**
     * Opens windows at the size of the cells they will cover, as tile sizes like every size in
     * the API, minus the tab indicator if they open as tabs.
     */
    function newWindowSize(info) {
        const borders = info.borders;
        const extra =
            ws.config.defaultColumnDisplay === "tabbed" ? tabIndicatorSize(1) : { w: 0, h: 0 };
        const width = info.rules.defaultWidth
            ? cellGeometry(new Area(0, 0, colsForPreset(info.rules.defaultWidth, borders.w), 1)).w -
              extra.w
            : 0;
        const rows = info.rules.defaultHeight
            ? rowsForPreset(info.rules.defaultHeight, borders.h)
            : metrics.rows;
        const height = cellGeometry(new Area(0, 0, 1, rows)).h - extra.h;
        let size = {
            w: width > 0 ? Math.max(1, Math.floor(width)) : 0,
            h: Math.max(1, Math.floor(height)),
        };

        // A window with a fixed size gets it, and the minimum height always holds.
        const { minSize: min, maxSize: max } = info;
        const limit = (value, lo, hi) => Math.max(hi > 0 ? Math.min(value, hi) : value, lo);
        size = {
            w: size.w === 0 ? 0 : limit(size.w, min.w, max.w),
            h: min.h === max.h && min.h > 0 ? limit(size.h, min.h, max.h) : Math.max(size.h, min.h),
        };
        return size;
    }

    /**
     * Column and place in the stack, counting from 1: the grid's columns are where windows start,
     * and a window's stack is the windows and tabs covering the column of cells it starts in.
     */
    function ipcPosition(win) {
        const item = itemOf(win);
        if (item === null) {
            return null;
        }
        const area = grid.area(item);
        const column = grid.columnStarts().indexOf(area.col);
        const tab = item.windows.findIndex((w) => w.id === win.id);
        const place = stackTiles(area.col).findIndex(([it, t]) => it === item && t === tab);
        return column === -1 || place === -1 ? null : [column + 1, place + 1];
    }

    // ---------------------------------------------------------------------------------------------
    // Actions
    // ---------------------------------------------------------------------------------------------
    //
    // niri's layout actions by their config names, with their meanings on the grid. Each returns
    // whether it did anything; bento falls back to monitors and workspaces for focus and move
    // actions that return false.

    const actions = {
        "focus-column-left": () => focusGrid(Left),
        "focus-column-right": () => focusGrid(Right),
        "focus-column-or-monitor-left": () => focusGrid(Left),
        "focus-column-or-monitor-right": () => focusGrid(Right),
        "focus-column-first": () => focusGridEnd(false),
        "focus-column-last": () => focusGridEnd(true),
        "focus-column-right-or-first": () => focusGrid(Right) || focusGridEnd(false),
        "focus-column-left-or-last": () => focusGrid(Left) || focusGridEnd(true),
        "focus-column": ([index]) => focusColumn(index),

        // Up and down can reach the nearest window further down or up, even diagonally. Actions
        // that fall back to something else only count windows sharing columns of cells.
        "focus-window-down": () => focusTab(true) || focusGrid(Down),
        "focus-window-up": () => focusTab(false) || focusGrid(Up),
        "focus-window-or-workspace-down": () => focusDownAdjacent(),
        "focus-window-or-workspace-up": () => focusUpAdjacent(),
        "focus-window-or-monitor-down": () => focusDownAdjacent(),
        "focus-window-or-monitor-up": () => focusUpAdjacent(),
        "focus-window-down-or-column-left": () => focusDownAdjacent() || focusGrid(Left),
        "focus-window-down-or-column-right": () => focusDownAdjacent() || focusGrid(Right),
        "focus-window-up-or-column-left": () => focusUpAdjacent() || focusGrid(Left),
        "focus-window-up-or-column-right": () => focusUpAdjacent() || focusGrid(Right),

        // Within the stack: the windows covering the same column of cells, tabs in order.
        "focus-window-top": () => focusInStack(() => 0),
        "focus-window-bottom": () => focusInStack((n) => n - 1),
        "focus-window-down-or-top": () => focusDownAdjacent() || focusInStack(() => 0),
        "focus-window-up-or-bottom": () => focusUpAdjacent() || focusInStack((n) => n - 1),
        "focus-window-in-column": ([index]) =>
            focusInStack((n) => clamp((Number.isFinite(index) ? toInt(index) : 1) - 1, 0, n - 1)),

        "move-column-left": () => moveGrid(Left),
        "move-column-right": () => moveGrid(Right),
        "move-column-left-or-to-monitor-left": () => moveGrid(Left),
        "move-column-right-or-to-monitor-right": () => moveGrid(Right),
        "swap-window-left": () => moveGrid(Left),
        "swap-window-right": () => moveGrid(Right),
        "move-window-down": () => moveVertical(Down),
        "move-window-up": () => moveVertical(Up),
        "move-window-down-or-to-workspace-down": () => moveVertical(Down),
        "move-window-up-or-to-workspace-up": () => moveVertical(Up),
        // Go to the start of the strip in the window's own rows, pushing others right, or past
        // its end.
        "move-column-to-first": () => moveToColumn(() => [0, Right]),
        "move-column-to-last": () => {
            const width = grid.width();
            return moveToColumn(() => [width, Right]);
        },
        "move-column-to-index": ([index]) => moveToIndex(index),

        "consume-or-expel-window-left": (args, target) => consumeOrExpel(target, Left),
        "consume-or-expel-window-right": (args, target) => consumeOrExpel(target, Right),
        // Brings the window on the right under the focused one.
        "consume-window-into-column": () => {
            const lines = focusLines();
            const right = lines && grid.adjacent(lines.item, Right, lines.row);
            return !!right && changeGrid(() => consume(right, lines.item));
        },
        "expel-window-from-column": () => {
            if (active === null) return false;
            const item = active;
            return item.windows.length > 1
                ? changeGrid(() => expelTab(item, item.active, Right))
                : changeGrid(() => expel(item, Right));
        },
        "toggle-column-tabbed-display": () => active !== null && setDisplay(!active.tabbed),
        "set-column-display": ([display]) => setDisplay(display === "tabbed"),

        "center-column": () => centerColumn(),
        // Only the focused window can reasonably be centered.
        "center-window": (args, target) => itemFor(target) === active && centerColumn(),
        "center-visible-columns": () => centerVisibleColumns(),

        "set-column-width": ([change], target) => setWidth(target, change),
        "set-window-width": ([change], target) => setWidth(target, change),
        "set-window-height": ([change], target) => setHeight(target, change),
        "reset-window-height": (args, target) => resizeItem(itemFor(target), null, metrics.rows),
        "switch-preset-column-width": () => switchPresetWidth(null, true),
        "switch-preset-column-width-back": () => switchPresetWidth(null, false),
        "switch-preset-window-width": (args, target) => switchPresetWidth(target, true),
        "switch-preset-window-width-back": (args, target) => switchPresetWidth(target, false),
        "switch-preset-window-height": (args, target) => switchPresetHeight(target, true),
        "switch-preset-window-height-back": (args, target) => switchPresetHeight(target, false),
        "maximize-column": () => toggleFullWidth(),
        "expand-column-to-available-width": () => expandToAvailableWidth(),

        "fullscreen-window": (args, target) => {
            const item = itemFor(target);
            return item !== null && changeMode(item, !item.fullscreen, item.maximized);
        },
        "maximize-window-to-edges": (args, target) => {
            const item = itemFor(target);
            return item !== null && changeMode(item, item.fullscreen, !item.maximized);
        },
    };

    function runAction(name, args, target) {
        const run = Object.hasOwn(actions, name) ? actions[name] : null;
        return run !== null && !!run(Array.isArray(args) ? args : [], target ?? null);
    }

    return {
        windowAdded: changes(addWindow),
        windowRemoved: changes(removeWindow),
        adopt: changes(adopt),
        save,
        action: changes(runAction),
        activate: changes(activateWindow),
        modeRequested: changes(requestMode),
        // The grid decides window sizes. A window that can't take the size of its cells, like
        // one with a larger minimum size, is drawn at its own size where its cells are.
        windowChanged() {},
        viewChanged: changes(reconfigure),
        configChanged: changes(reconfigure),
        resizeBegin: changes(resizeBegin),
        resizeUpdate: changes(resizeUpdate),
        resizeEnd: changes(resizeEnd),
        gestureEnd: changes(gestureEnd),
        transferIn: changes(transferIn),
        // Queries.
        dropTarget,
        newWindowSize,
        transferOut,
        ipcPosition,
    };
}

// =================================================================================================
// Helpers
// =================================================================================================

/**
 * Camera offset from a window at `newColX` of `newColWidth` that keeps it in view, given the
 * camera's left edge at `curX` and the view `viewWidth` wide. Like niri: windows already in view
 * stay where they are, and others scroll in on the side that moves the view the least.
 */
function computeNewViewOffset(curX, viewWidth, newColX, newColWidth, gaps) {
    // If the window is wider than the view, always left-align it.
    if (viewWidth <= newColWidth) {
        return 0;
    }

    // Compute the padding in case it needs to be smaller due to large tile width.
    const padding = clamp((viewWidth - newColWidth) / 2, 0, gaps);

    // Compute the desired new X with padding.
    const newX = newColX - padding;
    const newRightX = newColX + newColWidth + padding;

    // If the window is already fully visible, leave the view as is.
    if (curX <= newX && newRightX <= curX + viewWidth) {
        return -(newColX - curX);
    }

    // Otherwise, prefer the alignment that results in less motion from the current position.
    const distToLeft = Math.abs(curX - newX);
    const distToRight = Math.abs(curX + viewWidth - newRightX);
    return distToLeft <= distToRight ? -padding : -(viewWidth - padding - newColWidth);
}

/** The working area: the part of the view without panels, minus the struts from the config. */
function computeWorkingArea(parent, scale, struts) {
    const x = parent.x + struts.left;
    const y = parent.y + struts.top;
    const w = Math.max(0, parent.w - struts.left - struts.right);
    const h = Math.max(0, parent.h - struts.top - struts.bottom);

    // Start at a physical pixel.
    const px = Math.ceil(x * scale) / scale;
    const py = Math.ceil(y * scale) / scale;
    return { x: px, y: py, w: w - Math.min(w, px - x), h: h - Math.min(h, py - y) };
}

function sameRect(a, b) {
    return a !== null && a.x === b.x && a.y === b.y && a.w === b.w && a.h === b.h;
}
