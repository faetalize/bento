//! Turns an `action` request into the action a bind would run.
//!
//! The request is written out as a bind and parsed by the config's own parser, so the same things
//! are valid and the same errors come back.

use std::fmt::Write as _;
use std::path::Path;

use miette::{Diagnostic, GraphicalReportHandler, GraphicalTheme};
use niri_config::actions::native_actions;
use niri_config::{Action, Config, ConfigIncludeError, WorkspaceReference};
use niri_ipc::{LayoutEngineActionArg as Arg, LayoutEngineActionArgType as Type};
use serde_json::{Map, Value};

/// What an `action` request says.
pub struct ActionRequest {
    pub name: String,
    pub args: Vec<Value>,
    pub properties: Map<String, Value>,
    pub window: Option<u64>,
    pub workspace: Option<u64>,
}

/// The action that `request` asks for. `engine_args` gives the arguments of the running engine's
/// action with a name, if it has one.
pub fn parse(
    mut request: ActionRequest,
    engine_args: impl Fn(&str) -> Option<Vec<Arg>>,
) -> Result<Action, String> {
    // `layout-action` is how the command line spells an engine's action: by its name, in the
    // first argument.
    if request.name == "layout-action" {
        if request.args.is_empty() {
            return Err(String::from("layout-action needs an engine action's name"));
        }
        let Value::String(name) = request.args.remove(0) else {
            return Err(String::from("the engine action's name must be a string"));
        };
        request.name = name;
    }

    let name = request.name.as_str();
    if !is_identifier(name) {
        return Err(format!("invalid action name {name:?}"));
    }

    // An action can't share its name between bento and the engine, so the first match is the one.
    let (declared, native) = match (
        native_actions().into_iter().find(|a| a.name == name),
        engine_args(name),
    ) {
        (Some(action), _) => (action.args, true),
        (None, Some(args)) => (args, false),
        (None, None) => {
            return Err(format!(
                "no action named {name:?}, in bento or in the running engine"
            ));
        }
    };

    let mut properties = request.properties;
    // A path isn't something a bind can say, but a script can ask for a screenshot to go to one.
    let path = match properties.remove("path") {
        Some(path) if takes_path(name) => Some(absolute_path(path)?),
        Some(path) => {
            properties.insert(String::from("path"), path);
            None
        }
        None => None,
    };

    let positional: Vec<&Arg> = declared.iter().filter(|arg| !arg.property).collect();
    let args = request.args.iter().enumerate().map(|(index, value)| {
        // A command takes all the arguments that are left.
        let arg = positional.get(index).or_else(|| {
            positional
                .last()
                .filter(|arg| arg.r#type == Some(Type::Command))
        });
        coerce(value, arg.and_then(|arg| arg.r#type), native)
    });
    let properties = properties.iter().map(|(key, value)| {
        let arg = declared.iter().find(|arg| arg.property && arg.name == *key);
        (key, coerce(value, arg.and_then(|arg| arg.r#type), native))
    });

    let mut node = String::from(name);
    for value in args {
        node.push(' ');
        node.push_str(&kdl_value(&value)?);
    }
    for (key, value) in properties {
        if !is_identifier(key) {
            return Err(format!("invalid property name {key:?}"));
        }
        write!(node, " {key}={}", kdl_value(&value)?).unwrap();
    }

    // The engine's file is the one where the engine's actions are bound by name.
    let text = format!("binds {{\n    F35 {{ {node}; }}\n}}\n");
    let config = Config::parse_mem_with_engine("", &text).map_err(render_error)?;
    let action = config
        .binds
        .0
        .into_iter()
        .next()
        .and_then(|bind| bind.into_action(true))
        .ok_or_else(|| format!("{name} didn't make an action"))?;

    let action = match path {
        Some(path) => with_path(action, path),
        None => action,
    };
    target(action, request.window, request.workspace)
}

fn is_identifier(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

fn takes_path(name: &str) -> bool {
    matches!(
        name,
        "screenshot" | "screenshot-screen" | "screenshot-window"
    )
}

fn absolute_path(path: Value) -> Result<String, String> {
    let Value::String(path) = path else {
        return Err(String::from("path must be a string"));
    };
    // A relative path would resolve against bento's working directory, which is almost certainly
    // not what the caller means.
    if !Path::new(&path).is_absolute() {
        return Err(format!("path must be absolute: {path}"));
    }
    Ok(path)
}

fn with_path(action: Action, path: String) -> Action {
    match action {
        Action::Screenshot(show_pointer, _) => Action::Screenshot(show_pointer, Some(path)),
        Action::ScreenshotScreen(write_to_disk, show_pointer, _) => {
            Action::ScreenshotScreen(write_to_disk, show_pointer, Some(path))
        }
        Action::ScreenshotWindow(write_to_disk, show_pointer, _) => {
            Action::ScreenshotWindow(write_to_disk, show_pointer, Some(path))
        }
        action => action,
    }
}

/// Makes a value the type the argument declares where that's unambiguous, since the command line
/// only has strings and bind syntax tells `3` from `"3"`.
fn coerce(value: &Value, r#type: Option<Type>, native: bool) -> Value {
    match (r#type, value) {
        (Some(Type::String | Type::Choice | Type::Command), Value::Number(number)) => {
            Value::String(number.to_string())
        }
        (Some(Type::String | Type::Choice | Type::Command), Value::Bool(value)) => {
            Value::String(value.to_string())
        }
        // Bento's own size arguments are read from text, like `"800"`.
        (Some(Type::Size), Value::Number(number)) if native => Value::String(number.to_string()),
        (Some(Type::Number), Value::String(string)) => string
            .parse::<i64>()
            .map(Value::from)
            .or_else(|_| string.parse::<f64>().map(Value::from))
            .unwrap_or_else(|_| value.clone()),
        (Some(Type::Boolean), Value::String(string)) => match string.as_str() {
            "true" => Value::Bool(true),
            "false" => Value::Bool(false),
            _ => value.clone(),
        },
        _ => value.clone(),
    }
}

/// A value as KDL writes it.
fn kdl_value(value: &Value) -> Result<String, String> {
    match value {
        Value::String(string) => Ok(kdl_string(string)),
        Value::Bool(value) => Ok(value.to_string()),
        Value::Number(number) => Ok(number.to_string()),
        value => Err(format!(
            "arguments are strings, numbers or booleans, not {value}"
        )),
    }
}

fn kdl_string(string: &str) -> String {
    let mut out = String::from("\"");
    for c in string.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => write!(out, "\\u{{{:x}}}", u32::from(c)).unwrap(),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn render_error(err: ConfigIncludeError) -> String {
    let handler = GraphicalReportHandler::new_themed(GraphicalTheme::unicode_nocolor());
    let mut out = String::new();
    match handler.render_report(&mut out, &err as &dyn Diagnostic) {
        Ok(()) => out,
        Err(_) => err.to_string(),
    }
}

/// Aims `action` at the window or workspace the request names instead of the focused one.
fn target(action: Action, window: Option<u64>, workspace: Option<u64>) -> Result<Action, String> {
    let action = match (window, action) {
        (None, action) => action,
        (Some(id), Action::CloseWindow) => Action::CloseWindowById(id),
        (Some(id), Action::FullscreenWindow) => Action::FullscreenWindowById(id),
        (Some(id), Action::ToggleWindowedFullscreen) => Action::ToggleWindowedFullscreenById(id),
        (Some(id), Action::MaximizeWindowToEdges) => Action::MaximizeWindowToEdgesById(id),
        (Some(id), Action::ToggleWindowFloating) => Action::ToggleWindowFloatingById(id),
        (Some(id), Action::MoveWindowToFloating) => Action::MoveWindowToFloatingById(id),
        (Some(id), Action::MoveWindowToTiling) => Action::MoveWindowToTilingById(id),
        (Some(id), Action::ToggleWindowRuleOpacity) => Action::ToggleWindowRuleOpacityById(id),
        (Some(id), Action::SetDynamicCastWindow) => Action::SetDynamicCastWindowById(id),
        (Some(id), Action::ScreenshotWindow(write_to_disk, show_pointer, path)) => {
            Action::ScreenshotWindowById {
                id,
                write_to_disk,
                show_pointer,
                path,
            }
        }
        (Some(window_id), Action::MoveWindowToWorkspace(reference, focus)) => {
            Action::MoveWindowToWorkspaceById {
                window_id,
                reference,
                focus,
            }
        }
        (Some(id), Action::MoveWindowToMonitor(output)) => {
            Action::MoveWindowToMonitorById { id, output }
        }
        (Some(_), _) => return Err(String::from("this action doesn't take a window")),
    };

    let action = match (workspace.map(WorkspaceReference::Id), action) {
        (None, action) => action,
        (Some(reference), Action::MoveWorkspaceToIndex(new_idx)) => {
            Action::MoveWorkspaceToIndexByRef { new_idx, reference }
        }
        (Some(reference), Action::MoveWorkspaceToMonitor(output_name)) => {
            Action::MoveWorkspaceToMonitorByRef {
                output_name,
                reference,
            }
        }
        (Some(reference), Action::SetWorkspaceName(name)) => {
            Action::SetWorkspaceNameByRef { name, reference }
        }
        (Some(reference), Action::UnsetWorkspaceName) => Action::UnsetWorkSpaceNameByRef(reference),
        (Some(_), _) => return Err(String::from("this action doesn't take a workspace")),
    };

    Ok(action)
}

#[cfg(test)]
mod tests {
    use niri_config::TilingOption;
    use serde_json::json;

    use super::*;

    fn request(name: &str, args: Value) -> ActionRequest {
        ActionRequest {
            name: String::from(name),
            args: args.as_array().unwrap().clone(),
            properties: Map::new(),
            window: None,
            workspace: None,
        }
    }

    fn arg(name: &str, r#type: Type) -> Arg {
        Arg {
            name: String::from(name),
            r#type: Some(r#type),
            default: None,
            optional: false,
            property: false,
            choices: Vec::new(),
            description: None,
        }
    }

    /// An engine that has `resize <cells> <side>`.
    fn engine(name: &str) -> Option<Vec<Arg>> {
        (name == "resize").then(|| vec![arg("cells", Type::Number), arg("side", Type::String)])
    }

    #[test]
    fn a_workspace_is_an_index_or_a_name() {
        let action = parse(request("focus-workspace", json!([3])), engine).unwrap();
        assert_eq!(action, Action::FocusWorkspace(WorkspaceReference::Index(3)));

        let action = parse(request("focus-workspace", json!(["web"])), engine).unwrap();
        assert_eq!(
            action,
            Action::FocusWorkspace(WorkspaceReference::Name(String::from("web")))
        );
    }

    #[test]
    fn properties_are_written_like_in_a_bind() {
        let mut request = request("move-window-to-workspace", json!([2]));
        request
            .properties
            .insert(String::from("focus"), json!(false));
        let action = parse(request, engine).unwrap();
        assert_eq!(
            action,
            Action::MoveWindowToWorkspace(WorkspaceReference::Index(2), false)
        );
    }

    #[test]
    fn a_string_argument_takes_a_number_from_the_command_line() {
        let action = parse(request("set-workspace-name", json!([5])), engine).unwrap();
        assert_eq!(action, Action::SetWorkspaceName(String::from("5")));
    }

    #[test]
    fn a_command_takes_every_argument() {
        let action = parse(
            request("spawn", json!(["sh", "-c", "echo \"hi\"", 3])),
            engine,
        );
        assert_eq!(
            action.unwrap(),
            Action::Spawn(vec![
                String::from("sh"),
                String::from("-c"),
                String::from("echo \"hi\""),
                String::from("3"),
            ])
        );
    }

    #[test]
    fn engine_actions_go_to_the_engine() {
        let action = parse(request("resize", json!([2, "right"])), engine).unwrap();
        assert_eq!(
            action,
            Action::LayoutAction(
                String::from("resize"),
                vec![
                    TilingOption::Number(2.),
                    TilingOption::String(String::from("right"))
                ]
            )
        );
    }

    #[test]
    fn layout_action_names_the_engine_action_first() {
        let action = parse(
            request("layout-action", json!(["resize", "2", "right"])),
            engine,
        );
        assert_eq!(
            action.unwrap(),
            Action::LayoutAction(
                String::from("resize"),
                vec![
                    TilingOption::Number(2.),
                    TilingOption::String(String::from("right"))
                ]
            )
        );
    }

    #[test]
    fn unknown_actions_are_refused() {
        assert!(parse(request("no-such-action", json!([])), engine).is_err());
        assert!(parse(request("focus-workspace; spawn", json!([])), engine).is_err());
        // Wrong arguments get the config's errors.
        assert!(parse(request("focus-workspace", json!([])), engine).is_err());
    }

    #[test]
    fn a_window_replaces_the_focused_one() {
        let mut request = request("close-window", json!([]));
        request.window = Some(7);
        assert_eq!(parse(request, engine).unwrap(), Action::CloseWindowById(7));

        let mut request = self::request("focus-monitor-left", json!([]));
        request.window = Some(7);
        assert!(parse(request, engine).is_err());
    }

    #[test]
    fn a_workspace_can_be_given_by_id() {
        let mut request = request("set-workspace-name", json!(["web"]));
        request.workspace = Some(4);
        assert_eq!(
            parse(request, engine).unwrap(),
            Action::SetWorkspaceNameByRef {
                name: String::from("web"),
                reference: WorkspaceReference::Id(4),
            }
        );
    }

    #[test]
    fn screenshot_paths_must_be_absolute() {
        let mut relative = request("screenshot-screen", json!([]));
        relative
            .properties
            .insert(String::from("path"), json!("shot.png"));
        assert!(parse(relative, engine).is_err());

        let mut absolute = request("screenshot-screen", json!([]));
        absolute
            .properties
            .insert(String::from("path"), json!("/tmp/shot.png"));
        assert_eq!(
            parse(absolute, engine).unwrap(),
            Action::ScreenshotScreen(true, true, Some(String::from("/tmp/shot.png")))
        );
    }

    #[test]
    fn strings_are_escaped() {
        assert_eq!(kdl_string("a \"b\" \\ \n"), r#""a \"b\" \\ \n""#);
        assert_eq!(kdl_string("\u{1}"), r#""\u{1}""#);
    }
}
