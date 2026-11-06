// A mirror of bento's search for the test host: how a query ranks windows, and how typing edits
// the field. It does what `src/layout/search/` does, and has to stay in step with it.

/** How well a window matches a query, as bento reports it: the greater the better. */
export const RANK = { any: 0, letters: 1, substring: 2, prefix: 3, word: 4 };

/** Most bytes a query can have. */
const MAX_LEN = 1024;

const WORD_CHAR = /[\p{Alphabetic}\p{N}]/u;
const CONTROL = /\p{Cc}/u;
const segmenter = new Intl.Segmenter("und", { granularity: "grapheme" });
const encoder = new TextEncoder();

/** The grapheme clusters of `text`, as strings. */
function clusters(text) {
    return [...segmenter.segment(text)].map((part) => part.segment);
}

/** How `query` matches `text`, both lowercase and `query` not empty; null if it doesn't. */
function rankText(query, text) {
    let best = null;
    for (let at = text.indexOf(query); at !== -1; at = text.indexOf(query, at + query.length)) {
        const before = [...text.slice(0, at)].at(-1);
        const after = [...text.slice(at + query.length)][0];
        const wordStart = before === undefined || !WORD_CHAR.test(before);
        const wordEnd = after === undefined || !WORD_CHAR.test(after);
        const rank = wordStart && wordEnd ? RANK.word : wordStart ? RANK.prefix : RANK.substring;
        best = Math.max(best ?? 0, rank);
        if (best === RANK.word) {
            return best;
        }
    }
    if (best !== null) {
        return best;
    }

    // The letters of the query, with no whitespace, in order.
    const letters = [...query].filter((c) => !/\s/u.test(c));
    let next = 0;
    for (const c of text) {
        if (letters[next] === c) {
            next++;
        }
    }
    return next === letters.length ? RANK.letters : null;
}

/** How `query` matches a window with this title and app id, or null if it doesn't. */
export function rank(query, title, appId) {
    const lowered = query.trim().toLowerCase();
    if (lowered === "") {
        return RANK.any;
    }
    const ranks = [title, appId]
        .filter((text) => typeof text === "string")
        .map((text) => rankText(lowered, text.toLowerCase()))
        .filter((r) => r !== null);
    return ranks.length === 0 ? null : Math.max(...ranks);
}

/** The text of a search field, with its caret and selection, as indexes in the string. */
export class SearchField {
    text = "";
    caret = 0;
    anchor = 0;

    get selection() {
        return [Math.min(this.caret, this.anchor), Math.max(this.caret, this.anchor)];
    }

    get selected() {
        const [from, to] = this.selection;
        return from === to ? null : this.text.slice(from, to);
    }

    /** Types `text` in place of the selection. */
    insert(text) {
        const [from, to] = this.selection;
        const room = MAX_LEN - (encoder.encode(this.text).length - encoder.encode(this.text.slice(from, to)).length);
        let typed = "";
        let used = 0;
        for (const c of text) {
            if (CONTROL.test(c)) continue;
            const size = encoder.encode(c).length;
            if (used + size > room) break;
            typed += c;
            used += size;
        }
        if (typed === "") return;
        this.text = this.text.slice(0, from) + typed + this.text.slice(to);
        this.caret = this.anchor = from + typed.length;
    }

    backspace() {
        if (this.caret === this.anchor) this.anchor = this.leftOf(this.caret);
        this.removeSelection();
    }

    delete() {
        if (this.caret === this.anchor) this.anchor = this.rightOf(this.caret);
        this.removeSelection();
    }

    removeSelection() {
        const [from, to] = this.selection;
        this.text = this.text.slice(0, from) + this.text.slice(to);
        this.caret = this.anchor = from;
    }

    left(select = false) {
        this.step(false, select);
    }

    right(select = false) {
        this.step(true, select);
    }

    home(select = false) {
        this.moveTo(0, select);
    }

    end(select = false) {
        this.moveTo(this.text.length, select);
    }

    selectAll() {
        this.anchor = 0;
        this.caret = this.text.length;
    }

    /** Moves a character, or with a selection that isn't extended, to its edge that way. */
    step(forward, select) {
        const [from, to] = this.selection;
        const collapse = !select && from !== to;
        const target = collapse ? (forward ? to : from) : forward ? this.rightOf(this.caret) : this.leftOf(this.caret);
        this.moveTo(target, select);
    }

    moveTo(to, select) {
        this.caret = to;
        if (!select) this.anchor = to;
    }

    leftOf(at) {
        const last = clusters(this.text.slice(0, at)).at(-1);
        return last === undefined ? 0 : at - last.length;
    }

    rightOf(at) {
        const first = clusters(this.text.slice(at))[0];
        return first === undefined ? this.text.length : at + first.length;
    }
}
