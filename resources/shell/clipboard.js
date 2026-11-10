// The clipboard's history, which the right island opens into from the clipboard's button or the
// clipboard's shortcut: what was copied, newest first, found by typing, and copied again with a
// click or Enter. The data program keeps it, for the session only.

import {
  box, text, image, input, button, expand, refill, on, data, last, focus, focusedOutput, outputs,
} from "bento:ui";

const WIDTH = 380;
// How many things show at once; typing finds the others.
const ROWS = 8;
// How tall a picture's row is.
const PICTURE = 96;

const history = () => last("clipboard") ?? { available: false, entries: [] };

// A text on one line.
const flat = (value) => value.replace(/\s+/g, " ").trim();

// Each output's clipboard panel, for the shortcut to open the focused one's.
const panels = new Map();

// The clipboard panel of `output`'s right island `pill`. `onToggle(open)` hears it open and
// close.
export function clipboardPanel(output, pill, onToggle) {
  let query = "";
  let shown = [];
  let rows = [];
  let selected = 0;

  const field = input({
    placeholder: "Search the clipboard",
    size: 15,
    onChange: (typed) => {
      query = typed;
      selected = 0;
      fill();
    },
    onSubmit: () => pick(selected),
  });
  const clear = button({
    icon: "edit-clear-all-symbolic", fill: "fg/0.08", height: 34, radius: 12,
    onClick: () => data("clipboard-clear"),
  });
  const header = box({ gap: 8, align: "center" }, [
    // A column, so that the field stretches across it.
    box({
      grow: 1, shrink: 1, minWidth: 0, direction: "column", padding: [9, 12], radius: 12,
      fill: "fg/0.08",
    }, [field]),
    clear,
  ]);
  const list = box({ direction: "column", gap: 2 });
  const note = text("", { color: "fg/0.6" });
  const noteRow = box({ padding: [6, 12], display: "none" }, [note]);
  const content = box({ direction: "column", gap: 10, width: WIDTH }, [header, noteRow, list]);

  const select = (index) => {
    if (rows.length === 0) return;
    selected = (index + rows.length) % rows.length;
    rows.forEach((row, i) => row.set({ fill: i === selected ? "fg/0.1" : null }));
  };

  const pick = (index) => {
    const entry = shown[index];
    if (!entry) return;
    data("clipboard-copy", { id: entry.id });
    panel.close();
  };

  const row = (entry, index) => {
    const what = entry.kind === "image"
      ? box({ grow: 1, shrink: 1, minWidth: 0, height: PICTURE, radius: 10, clip: true }, [
          image(entry.image, { width: WIDTH - 60, height: PICTURE }),
        ])
      : box({ grow: 1, shrink: 1, minWidth: 0 }, [text(flat(entry.text), { lines: 1 })]);
    return box(
      {
        align: "center", gap: 8, padding: [5, 5, 5, 12], radius: 12, cursor: "pointer",
        onClick: () => pick(index),
        onHover: ({ inside }) => inside && select(index),
      },
      [
        what,
        button({
          icon: "window-close-symbolic", size: 14, height: 26, radius: 13,
          onClick: () => data("clipboard-delete", { id: entry.id }),
        }),
      ],
    );
  };

  // Lists what matches what's typed.
  const fill = () => {
    const { available, entries } = history();
    const wanted = query.trim().toLowerCase();
    shown = entries
      .filter((entry) => !wanted || (entry.text ?? "").toLowerCase().includes(wanted))
      .slice(0, ROWS);
    rows = shown.map(row);
    refill(list, rows);
    const why = !available ? "The clipboard can't be watched here."
      : entries.length === 0 ? "Nothing's been copied yet."
      : shown.length === 0 ? "Nothing copied matches."
      : "";
    note.set({ text: why });
    noteRow.set({ display: why ? "flex" : "none" });
    list.set({ display: rows.length ? "flex" : "none" });
    clear.set({ display: entries.length ? "flex" : "none" });
    select(Math.min(selected, rows.length - 1));
    panel.fit();
  };

  const panel = expand(pill, content, {
    hold: "end",
    onToggle: (open) => {
      if (open) {
        query = "";
        selected = 0;
        field.setText("");
        fill();
        focus(field);
      }
      onToggle?.(open);
    },
  });

  on("clipboard", () => {
    if (panel.open) fill();
  });
  on("key", (event) => {
    if (!panel.open || !event.pressed) return;
    if (event.key === "Down" || (event.key === "Tab" && !event.shift)) {
      select(selected + 1);
    } else if (event.key === "Up" || event.key === "ISO_Left_Tab") {
      select(selected - 1);
    } else if (event.key === "Delete" && event.shift && shown[selected]) {
      data("clipboard-delete", { id: shown[selected].id });
    }
  });

  panels.set(output, panel);
  return panel;
}

on("toggle-clipboard", (event) => {
  const output = event.output ?? focusedOutput() ?? outputs()[0];
  const panel = panels.get(output) ?? panels.values().next().value;
  panel?.toggle();
});
