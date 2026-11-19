// The workspace editor: a field in the middle of the output with the name of its workspace,
// which is renamed as the name is typed. Its buttons take the name away, and put back the one it
// had when the editor opened.

import { Surface, action, focusedOutput, onAction, outputs, workspaces } from "bento";
import { createSignal, Show } from "solid-js";
import { Button, grows, growsWith, popup } from "bento:modules/ui";

const RADIUS = 24;

/** Whether another workspace is called `name` already: names are told apart without their case. */
function taken(name: string, id: number) {
  const wanted = name.toLowerCase();
  return outputs()
    .flatMap((output) => workspaces(output))
    .find((ws) => ws.id !== id && (ws.name ?? "").toLowerCase() === wanted);
}

/** The editor, opened by its shortcut for the focused workspace. */
export function WorkspaceEditor() {
  // The workspace it edits, as it was when it opened.
  const [editing, setEditing] = createSignal<{ output: string; id: number; index: number; original: string } | null>(null);
  onAction("edit-workspace", ({ output }) => {
    if (popup.is("workspace-editor")) {
      popup.close("workspace-editor");
      return;
    }
    const where = output ?? focusedOutput() ?? outputs()[0];
    const ws = workspaces(where).find((one) => one.active);
    if (!ws) return;
    setEditing({ output: where, id: ws.id, index: ws.index, original: ws.name ?? "" });
    popup.open("workspace-editor");
  });
  return (
    <Show when={popup.is("workspace-editor") && editing()} keyed>
      {(edit) => <Editor edit={edit} />}
    </Show>
  );
}

function Editor(props: { edit: { output: string; id: number; index: number; original: string } }) {
  const edit = props.edit;
  const [name, setName] = createSignal(edit.original);
  const [typed, setTyped] = createSignal(edit.original);
  const other = () => {
    const wanted = typed().trim();
    return wanted ? taken(wanted, edit.id) : undefined;
  };
  let field: any;
  const rename = (value: string) => {
    setTyped(value);
    const wanted = value.trim();
    if ((wanted && taken(wanted, edit.id)) || wanted === name()) return;
    setName(wanted);
    if (wanted) action("set-workspace-name", [wanted], { workspace: edit.id });
    else action("unset-workspace-name", [], { workspace: edit.id });
  };
  const close = () => popup.close("workspace-editor");
  return (
    <Surface
      output={edit.output}
      layer="overlay"
      z={-1}
      keyboard="exclusive"
      onEscape={close}
      onOutside={close}
    >
      <box
        style={{ material: "glass", borderRadius: RADIUS, padding: 18, overflow: "hidden" }}
        {...grows(RADIUS)}
      >
        <box style={{ flexDirection: "column", gap: 12, width: 380 }} {...growsWith}>
          <box style={{ flexDirection: "row", alignItems: "center", gap: 10 }}>
            <image icon="document-edit-symbolic" size={18} style={{ color: "accent" }} />
            <text style={{ fontSize: 16, fontWeight: 700 }}>{`Workspace ${edit.index + 1}`}</text>
          </box>
          <box style={{ padding: [9, 12], borderRadius: 12, background: "fg/0.08", fontSize: 17 }}>
            <input
              ref={field}
              placeholder="Name"
              value={edit.original}
              autofocus
              onInput={rename}
              onSubmit={close}
            />
          </box>
          <box style={{ height: 16 }}>
            <text style={{ fontSize: 12, color: "#e5484d" }}>
              {other() ? `"${other()!.name}" is another workspace already.` : ""}
            </text>
          </box>
          <box style={{ flexDirection: "row", justifyContent: "end", gap: 8 }}>
            <Show when={typed().trim() !== edit.original}>
              <Button
                icon="edit-undo-symbolic"
                label="Undo"
                style={{ background: "fg/0.08" }}
                onClick={() => {
                  field?.setText(edit.original);
                  rename(edit.original);
                }}
              />
            </Show>
            <Show when={typed().trim()}>
              <Button
                icon="user-trash-symbolic"
                label="Remove Name"
                style={{ background: "fg/0.08" }}
                onClick={() => {
                  field?.setText("");
                  rename("");
                }}
              />
            </Show>
            <Button label="Done" active onClick={close} />
          </box>
        </box>
      </box>
    </Surface>
  );
}
