use std::io::{BufRead as _, ErrorKind};
use std::iter::Peekable;
use std::path::Path;
use std::{env, slice};

use anyhow::{anyhow, bail, Context};
use bento_ipc::socket::Socket;
use bento_ipc::{
    Cast, CastKind, CastTarget, Event, Get, KeyboardInteractivity, Layer, LayerSurface, Mode,
    Monitor, OutputConfigChanged, Request, Response, SizingMode, Transform, Window, Workspace,
};
use niri_ipc::{
    LayoutEngine, LayoutEngineAction, LayoutEngineOptionType, LayoutEngineOptionValue,
    LayoutEngineSource,
};
use serde_json::{json, Map, Value};

use crate::cli::Msg;
use crate::utils::version;

pub fn handle_msg(msg: Msg, json: bool, print_request: bool) -> anyhow::Result<()> {
    if let Msg::RawRequest = msg {
        return raw_request(print_request);
    }

    let request = make_request(&msg)?;

    if print_request {
        let json_str =
            serde_json::to_string(&request).context("error formatting request as JSON")?;
        println!("{json_str}");
        return Ok(());
    }

    let mut socket = Socket::connect().context("error connecting to the bento socket")?;

    let result = socket.send(&request);

    // For errors that can be caused by a version mismatch between the running bento instance and
    // the bento msg CLI, we will try to fetch and compare the versions.
    let check_compositor_version = match &result {
        Err(err) => {
            // Response JSON parsing errors.
            matches!(
                err.kind(),
                ErrorKind::InvalidData | ErrorKind::UnexpectedEof
            )
        }
        // Error returned from bento.
        Ok(Err(_)) => true,
        _ => false,
    };

    let compositor_version = if check_compositor_version && !matches!(msg, Msg::Version) {
        Socket::connect()
            .and_then(|mut socket| socket.send(&Request::Get { what: Get::Version }))
            .ok()
    } else {
        None
    };

    // Default SIGPIPE so that our prints don't panic on stdout closing.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }

    // Check for CLI-server version mismatch to add helpful context.
    match compositor_version {
        Some(Ok(Response::Version(compositor_version))) => {
            let cli_version = version();
            if cli_version != compositor_version {
                eprintln!("Running bento compositor has a different version from the bento CLI:");
                eprintln!("Compositor version: {compositor_version}");
                eprintln!("CLI version:        {cli_version}");
                eprintln!("Did you forget to restart bento after an update?");
                eprintln!();
            }
        }
        Some(_) => {
            eprintln!("Unable to get the running bento compositor version.");
            eprintln!("Did you forget to restart bento after an update?");
            eprintln!();
        }
        None => {
            // Communication error, or the original request was already a version request, or the
            // original request had succeeded. Don't add irrelevant context.
        }
    }

    let reply = result.context("error communicating with bento")?;
    let response = reply.map_err(|err_msg| anyhow!(err_msg).context("bento returned an error"))?;

    // What the request got back as JSON, for `--json`.
    macro_rules! print_json {
        ($value:expr) => {
            if json {
                let value = serde_json::to_string(&$value).context("error formatting response")?;
                println!("{value}");
                return Ok(());
            }
        };
    }

    match response {
        Response::Handled => {
            if let Msg::Subscribe { .. } = msg {
                if !json {
                    println!("Started reading events.");
                }

                let mut read_event = socket.read_events();
                loop {
                    let event = read_event().context("error reading event from bento")?;

                    if json {
                        let event =
                            serde_json::to_string(&event).context("error formatting event")?;
                        println!("{event}");
                    } else {
                        println!("{}", describe_event(&event));
                    }
                }
            }
        }
        Response::Error(_) => unreachable!("errors are split off by `into_result`"),
        Response::Version(compositor_version) => {
            let cli_version = version();

            if json {
                println!(
                    "{}",
                    json!({
                        "compositor": compositor_version,
                        "cli": cli_version,
                    })
                );
                return Ok(());
            }

            if cli_version != compositor_version {
                eprintln!("Running bento compositor has a different version from the bento CLI.");
                eprintln!("Did you forget to restart bento after an update?");
                eprintln!();
            }

            println!("Compositor version: {compositor_version}");
            println!("CLI version:        {cli_version}");
        }
        Response::Monitors(mut monitors) => {
            print_json!(monitors);

            monitors.sort_unstable_by(|a, b| a.name.cmp(&b.name));
            for monitor in monitors {
                print_monitor(monitor);
                println!();
            }
        }
        Response::Workspaces(mut workspaces) => {
            print_json!(workspaces);

            if workspaces.is_empty() {
                println!("No workspaces.");
                return Ok(());
            }

            workspaces.sort_by(|a, b| a.monitor.cmp(&b.monitor).then(a.index.cmp(&b.index)));

            let mut current_monitor = None;
            for workspace in &workspaces {
                if current_monitor != Some(&workspace.monitor) {
                    if current_monitor.is_some() {
                        println!();
                    }
                    match &workspace.monitor {
                        Some(monitor) => println!("Monitor \"{monitor}\":"),
                        None => println!("No monitor:"),
                    }
                    current_monitor = Some(&workspace.monitor);
                }

                print_workspace(workspace);
            }
        }
        Response::Windows(mut windows) => {
            print_json!(windows);

            windows.sort_unstable_by_key(|window| window.id);
            for window in windows {
                print_window(&window);
                println!();
            }
        }
        Response::Groups(mut groups) => {
            print_json!(groups);

            if groups.is_empty() {
                println!("No groups.");
                return Ok(());
            }

            groups.sort_unstable_by_key(|group| group.id);
            for group in groups {
                println!("Group ID {}:", group.id);
                let windows: Vec<_> = group.windows.iter().map(u64::to_string).collect();
                println!("  Windows: {}", windows.join(", "));
                println!("  Active window: {}", group.active);
                println!();
            }
        }
        Response::Layers(mut layers) => {
            print_json!(layers);

            layers.sort_by(|a, b| {
                Ord::cmp(&a.monitor, &b.monitor)
                    .then_with(|| Ord::cmp(&a.layer, &b.layer))
                    .then_with(|| Ord::cmp(&a.namespace, &b.namespace))
            });
            let mut iter = layers.iter().peekable();

            let print = |surface: &LayerSurface| {
                println!("    Surface:");
                println!("      Namespace: \"{}\"", surface.namespace);

                let interactivity = match surface.keyboard_interactivity {
                    KeyboardInteractivity::None => "none",
                    KeyboardInteractivity::Exclusive => "exclusive",
                    KeyboardInteractivity::OnDemand => "on-demand",
                };
                println!("      Keyboard interactivity: {interactivity}");
            };

            let print_layer = |iter: &mut Peekable<slice::Iter<LayerSurface>>,
                               monitor: &str,
                               layer| {
                let mut empty = true;
                while let Some(surface) = iter.next_if(|s| s.monitor == monitor && s.layer == layer)
                {
                    empty = false;
                    println!();
                    print(surface);
                }
                if empty {
                    println!(" (empty)\n");
                } else {
                    println!();
                }
            };

            while let Some(surface) = iter.peek() {
                let monitor = &surface.monitor;
                println!("Monitor \"{monitor}\":");

                print!("  Background layer:");
                print_layer(&mut iter, monitor, Layer::Background);

                print!("  Bottom layer:");
                print_layer(&mut iter, monitor, Layer::Bottom);

                print!("  Top layer:");
                print_layer(&mut iter, monitor, Layer::Top);

                print!("  Overlay layer:");
                print_layer(&mut iter, monitor, Layer::Overlay);
            }
        }
        Response::Focused(focused) => {
            print_json!(focused);

            if let Some(monitor) = focused.monitor {
                println!("Focused monitor: \"{}\"", monitor.name);
            } else {
                println!("No monitor is focused.");
            }
            if let Some(window) = focused.window {
                print_window(&window);
            } else {
                println!("No window is focused.");
            }
        }
        Response::KeyboardLayouts(layouts) => {
            print_json!(layouts);

            let current_idx = usize::from(layouts.current_idx);

            println!("Keyboard layouts:");
            for (idx, name) in layouts.names.iter().enumerate() {
                let is_active = if idx == current_idx { " * " } else { "   " };
                println!("{is_active}{idx} {name}");
            }
        }
        Response::Overview(overview) => {
            print_json!(overview);

            if overview.is_open {
                println!("Overview is open.");
            } else {
                println!("Overview is closed.");
            }
        }
        Response::Casts(mut casts) => {
            print_json!(casts);

            if casts.is_empty() {
                println!("No screencasts.");
                return Ok(());
            }

            casts.sort_by_key(|c| (c.session_id, c.stream_id));
            for cast in casts {
                print_cast(&cast);
                println!();
            }
        }
        Response::Engine(engine) => {
            print_json!(engine);

            print_layout_engine(&engine);
        }
        Response::EngineState(state) => {
            // The state's shape is the engine's own, so there's no better reading of it.
            if json {
                println!("{state}");
            } else {
                let state =
                    serde_json::to_string_pretty(&state).context("error formatting response")?;
                println!("{state}");
            }
        }
        Response::Actions(actions) => {
            print_json!(actions);

            print_actions(&actions);
        }
        Response::PickedWindow(window) => {
            print_json!(window);

            if let Some(window) = window {
                print_window(&window);
            } else {
                println!("No window selected.");
            }
        }
        Response::PickedColor(color) => {
            print_json!(color);

            if let Some(color) = color {
                let [r, g, b] = color.rgb.map(|v| (v.clamp(0., 1.) * 255.).round() as u8);

                println!("Picked color: rgb({r}, {g}, {b})",);
                println!("Hex: #{r:02x}{g:02x}{b:02x}");
            } else {
                println!("No color was picked.");
            }
        }
        Response::OutputConfigChanged(changed) => {
            print_json!(changed);

            if let (OutputConfigChanged::OutputWasMissing, Msg::Output { output, .. }) =
                (changed, &msg)
            {
                println!("Monitor \"{output}\" is not connected.");
                println!("The change will apply when it is connected.");
            }
        }
    }

    Ok(())
}

/// Sends the JSON request on stdin as it is, and prints what comes back.
fn raw_request(print_request: bool) -> anyhow::Result<()> {
    let mut line = String::new();
    std::io::stdin()
        .lock()
        .read_line(&mut line)
        .context("error reading from stdin")?;

    if print_request {
        println!("{}", line.trim_end());
        return Ok(());
    }

    let is_subscription = serde_json::from_str::<Value>(&line)
        .context("error parsing request JSON from stdin")?
        .get("type")
        == Some(&Value::from("subscribe"));

    let mut socket = Socket::connect().context("error connecting to the bento socket")?;
    let reply = socket
        .send_line(&line)
        .context("error communicating with bento")?;

    // Default SIGPIPE so that our prints don't panic on stdout closing.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }

    println!("{reply}");

    if is_subscription {
        loop {
            match socket.read_line() {
                Ok(event) => println!("{event}"),
                Err(err) if err.kind() == ErrorKind::UnexpectedEof => return Ok(()),
                Err(err) => return Err(err).context("error reading event from bento"),
            }
        }
    }

    Ok(())
}

fn make_request(msg: &Msg) -> anyhow::Result<Request> {
    let get = |what| Request::Get { what };

    let request = match msg {
        Msg::Version => get(Get::Version),
        Msg::Monitors => get(Get::Monitors),
        Msg::Workspaces => get(Get::Workspaces),
        Msg::Windows => get(Get::Windows),
        Msg::Groups => get(Get::Groups),
        Msg::Layers => get(Get::Layers),
        Msg::Focused => get(Get::Focused),
        Msg::KeyboardLayouts => get(Get::KeyboardLayouts),
        Msg::Overview => get(Get::Overview),
        Msg::Casts => get(Get::Casts),
        Msg::Engine => get(Get::Engine),
        Msg::EngineState => get(Get::EngineState),
        Msg::Actions => get(Get::Actions),
        Msg::PickWindow => Request::PickWindow,
        Msg::PickColor => Request::PickColor,
        Msg::Action {
            window,
            workspace,
            properties,
            name,
            args,
        } => {
            let mut parsed = Map::new();
            for property in properties {
                let (key, value) = property
                    .split_once('=')
                    .ok_or_else(|| anyhow!("property {property:?} isn't written NAME=VALUE"))?;
                let mut value = parse_value(value);
                // For paths, prepend the bento CLI's working directory.
                if key == "path" {
                    if let Value::String(path) = &mut value {
                        ensure_absolute_path(path).context("error making the path absolute")?;
                    }
                }
                parsed.insert(String::from(key), value);
            }

            Request::Action {
                name: name.clone(),
                args: args.iter().map(|arg| parse_value(arg)).collect(),
                properties: parsed,
                window: *window,
                workspace: *workspace,
            }
        }
        Msg::Output { output, action } => Request::Output {
            output: output.clone(),
            action: action.clone(),
        },
        Msg::FocusWindow { id } => Request::FocusWindow { id: *id },
        Msg::WindowUrgent { id, urgent } => Request::SetWindowUrgent {
            id: *id,
            urgent: *urgent,
        },
        Msg::StopCast { session_id } => Request::StopCast {
            session_id: *session_id,
        },
        Msg::LoadConfigFile { path } => {
            let mut path = path.clone();
            if let Some(path) = &mut path {
                ensure_absolute_path(path).context("error making the path absolute")?;
            }
            Request::LoadConfigFile { path }
        }
        Msg::Subscribe { events } => {
            let events = if events.is_empty() {
                None
            } else {
                let events = events
                    .iter()
                    .map(|event| {
                        serde_json::from_value(Value::String(event.clone()))
                            .with_context(|| format!("invalid event kind {event:?}"))
                    })
                    .collect::<anyhow::Result<_>>()?;
                Some(events)
            };
            Request::Subscribe { events }
        }
        Msg::RawRequest => unreachable!("raw requests are sent as they are"),
    };

    Ok(request)
}

/// A command line argument as the action takes it: bento turns it to what the action declares
/// where that's unambiguous, so only what can't be mistaken for another type becomes one.
fn parse_value(arg: &str) -> Value {
    match arg {
        "true" => return Value::Bool(true),
        "false" => return Value::Bool(false),
        _ => (),
    }

    if let Ok(number) = arg.parse::<i64>() {
        if number.to_string() == arg {
            return Value::from(number);
        }
    }
    if let Ok(number) = arg.parse::<f64>() {
        if number.is_finite() && number.to_string() == arg {
            return Value::from(number);
        }
    }

    Value::String(String::from(arg))
}

fn print_monitor(monitor: Monitor) {
    let Monitor {
        name,
        make,
        model,
        serial,
        physical_size,
        modes,
        current_mode,
        custom_mode,
        vrr_supported,
        vrr_enabled,
        max_bpc,
        enabled: _,
        position,
        size,
        scale,
        transform,
        working_area,
        workspaces,
        active_workspace,
        focused,
    } = monitor;

    let serial = serial.as_deref().unwrap_or("Unknown");
    let focused = if focused { " (focused)" } else { "" };
    println!(r#"Monitor "{make} {model} {serial}" ({name}){focused}"#);

    let print_qualifier = |is_preferred: bool, is_current: bool, is_custom_mode: bool| {
        let mut qualifier = Vec::new();
        if is_current {
            qualifier.push("current");
            if is_custom_mode {
                qualifier.push("custom");
            };
        };

        if is_preferred {
            qualifier.push("preferred");
        };

        if qualifier.is_empty() {
            String::new()
        } else {
            format!(" ({})", qualifier.join(", "))
        }
    };

    if let Some(mode) = current_mode.and_then(|current| modes.get(current)) {
        let Mode {
            width,
            height,
            refresh_rate,
            preferred,
        } = *mode;
        let refresh = refresh_rate as f64 / 1000.;

        // This is technically the current mode, but the println below already specifies that.
        let qualifier = print_qualifier(preferred, false, custom_mode);
        println!("  Current mode: {width}x{height} @ {refresh:.3} Hz{qualifier}");
    } else {
        println!("  Disabled");
    }

    if vrr_supported {
        let enabled = if vrr_enabled { "enabled" } else { "disabled" };
        println!("  Variable refresh rate: supported, {enabled}");
    } else {
        println!("  Variable refresh rate: not supported");
    }

    if let Some(size) = physical_size {
        println!("  Physical size: {}x{} mm", size.w, size.h);
    } else {
        println!("  Physical size: unknown");
    }

    if let Some(position) = position {
        println!("  Logical position: {}, {}", position.x, position.y);
    }
    if let Some(size) = size {
        println!("  Logical size: {}x{}", size.w, size.h);
    }
    if let Some(scale) = scale {
        println!("  Scale: {scale}");
    }
    if let Some(transform) = transform {
        let transform = match transform {
            Transform::Normal => "normal",
            Transform::_90 => "90° counter-clockwise",
            Transform::_180 => "180°",
            Transform::_270 => "270° counter-clockwise",
            Transform::Flipped => "flipped horizontally",
            Transform::Flipped90 => "90° counter-clockwise, flipped horizontally",
            Transform::Flipped180 => "flipped vertically",
            Transform::Flipped270 => "270° counter-clockwise, flipped horizontally",
        };
        println!("  Transform: {transform}");
    }
    if let Some(area) = working_area {
        println!(
            "  Working area: {}x{} at {}, {}",
            fmt_rounded(area.w),
            fmt_rounded(area.h),
            fmt_rounded(area.x),
            fmt_rounded(area.y)
        );
    }

    if !workspaces.is_empty() {
        let ids: Vec<_> = workspaces
            .iter()
            .map(|id| {
                let active = if Some(*id) == active_workspace {
                    "*"
                } else {
                    ""
                };
                format!("{id}{active}")
            })
            .collect();
        println!("  Workspaces (ids, active one marked): {}", ids.join(", "));
    }

    if let Some(max_bpc) = max_bpc {
        println!("  Max bits per channel: {max_bpc}");
    }

    println!("  Available modes:");
    for (idx, mode) in modes.into_iter().enumerate() {
        let Mode {
            width,
            height,
            refresh_rate,
            preferred,
        } = mode;
        let refresh = refresh_rate as f64 / 1000.;

        let is_current = Some(idx) == current_mode;
        let qualifier = print_qualifier(preferred, is_current, custom_mode);

        println!("    {width}x{height}@{refresh:.3}{qualifier}");
    }
}

fn print_workspace(workspace: &Workspace) {
    let is_active = if workspace.active { " * " } else { "   " };
    // Counting from 1, as `focus-workspace` does.
    let index = workspace.index + 1;
    let name = if let Some(name) = workspace.name.as_deref() {
        format!(" \"{name}\"")
    } else {
        String::new()
    };
    let urgent = if workspace.urgent { " (urgent)" } else { "" };
    let windows = workspace.windows.len();
    println!(
        "{is_active}{index}{name} (ID {}, {windows} windows){urgent}",
        workspace.id
    );
}

fn print_window(window: &Window) {
    let focused = if window.focused { " (focused)" } else { "" };
    let urgent = if window.urgent { " (urgent)" } else { "" };
    println!("Window ID {}:{focused}{urgent}", window.id);

    if let Some(title) = &window.title {
        println!("  Title: \"{title}\"");
    } else {
        println!("  Title: (unset)");
    }

    if let Some(app_id) = &window.app_id {
        println!("  App ID: \"{app_id}\"");
    } else {
        println!("  App ID: (unset)");
    }

    println!(
        "  Is floating: {}",
        if window.floating { "yes" } else { "no" }
    );

    let mode = match window.mode {
        SizingMode::Normal => "normal",
        SizingMode::Fullscreen => "fullscreen",
        SizingMode::Maximized => "maximized",
    };
    println!("  Mode: {mode}");

    if let Some(pid) = window.pid {
        println!("  PID: {pid}");
    } else {
        println!("  PID: (unknown)");
    }

    if let Some(workspace) = window.workspace {
        println!("  Workspace ID: {workspace}");
    } else {
        println!("  Workspace ID: (none)");
    }

    if let Some(group) = window.group {
        println!("  Group ID: {group}");
    }

    if let Some(rect) = window.rect {
        println!(
            "  Placed at: {}, {} ({} x {})",
            fmt_rounded(rect.x),
            fmt_rounded(rect.y),
            fmt_rounded(rect.w),
            fmt_rounded(rect.h)
        );
    } else {
        println!("  Placed at: (not placed)");
    }
    println!("  Window size: {} x {}", window.size.w, window.size.h);
}

fn print_cast(cast: &Cast) {
    let active = if cast.active { "" } else { " (inactive)" };
    println!("Cast stream ID {}:{active}", cast.stream_id);
    println!("  Session ID: {}", cast.session_id);

    let kind = match cast.kind {
        CastKind::PipeWire => "PipeWire",
        CastKind::WlrScreencopy => "wlr-screencopy",
        CastKind::ExtImageCopyCapture => "ext-image-copy-capture",
    };
    println!("  Kind: {kind}");

    match &cast.target {
        CastTarget::Nothing => {
            println!("  Target: nothing (cleared)");
        }
        CastTarget::Output { name } => {
            println!("  Target: monitor \"{name}\"");
        }
        CastTarget::Window { id } => {
            println!("  Target: window {id}");
        }
    }

    if cast.dynamic_target {
        println!("  Dynamic cast target");
    }

    if let Some(pid) = cast.pid {
        println!("  PID: {pid}");
    }

    if let Some(node_id) = cast.pw_node_id {
        println!("  PipeWire node ID: {node_id}");
    }
}

fn print_layout_engine(engine: &LayoutEngine) {
    match &engine.source {
        LayoutEngineSource::BuiltIn {} => println!("Layout engine \"{}\" (built-in)", engine.name),
        LayoutEngineSource::File { path } => {
            println!("Layout engine \"{}\" ({path})", engine.name)
        }
    }
    if let Some(description) = &engine.description {
        println!("  {description}");
    }
    if !engine.floating {
        println!("  bento's floating layer is off under this engine.");
    }

    println!();
    if engine.options.is_empty() {
        println!("No options.");
    } else {
        println!("Options:");
    }
    for option in &engine.options {
        let value = fmt_option_value(&option.value);
        if option.value == option.default {
            println!("  {}: {value} (default)", option.name);
        } else {
            let default = fmt_option_value(&option.default);
            println!("  {}: {value} (default: {default})", option.name);
        }
        let list = if option.list { " list" } else { "" };
        let choices = fmt_choices(&option.choices);
        println!("    {}{list}{choices}", fmt_option_type(option.r#type));
        if let Some(description) = &option.description {
            println!("    {description}");
        }
    }

    println!();
    if engine.window_rules.is_empty() {
        println!("No window rules.");
    } else {
        println!("Window rules:");
    }
    for rule in &engine.window_rules {
        let list = if rule.list { " list" } else { "" };
        let choices = fmt_choices(&rule.choices);
        let kind = fmt_option_type(rule.r#type);
        println!("  {}: {kind}{list}{choices}", rule.name);
        if let Some(description) = &rule.description {
            println!("    {description}");
        }
    }

    println!();
    print_actions(&engine.actions);
}

fn print_actions(actions: &[LayoutEngineAction]) {
    if actions.is_empty() {
        println!("No actions.");
    } else {
        println!("Actions:");
    }
    for action in actions {
        // Arguments that can be left out go in brackets.
        let mut usage = action.name.clone();
        for arg in &action.args {
            let name = if arg.choices.is_empty() {
                arg.name.clone()
            } else {
                format!("{}: {}", arg.name, arg.choices.join("|"))
            };
            let name = if arg.property {
                format!("{}=", arg.name)
            } else {
                name
            };
            if arg.r#type.is_some() && arg.default.is_none() && !arg.optional {
                usage.push_str(&format!(" <{name}>"));
            } else {
                usage.push_str(&format!(" [{name}]"));
            }
        }
        println!("  {usage}");
        if let Some(description) = &action.description {
            println!("    {description}");
        }
    }
}

fn fmt_option_value(value: &Option<LayoutEngineOptionValue>) -> String {
    match value {
        // A size that the window decides.
        None => String::from("none"),
        Some(LayoutEngineOptionValue::Boolean(value)) => value.to_string(),
        Some(LayoutEngineOptionValue::Number(value)) => value.to_string(),
        Some(LayoutEngineOptionValue::String(value)) => format!("{value:?}"),
        Some(LayoutEngineOptionValue::List(values)) => {
            let values: Vec<_> = values
                .iter()
                .map(|value| fmt_option_value(&Some(value.clone())))
                .collect();
            values.join(" ")
        }
    }
}

fn fmt_option_type(r#type: LayoutEngineOptionType) -> &'static str {
    match r#type {
        LayoutEngineOptionType::Boolean => "boolean",
        LayoutEngineOptionType::Number => "number",
        LayoutEngineOptionType::String => "string",
        LayoutEngineOptionType::Choice => "choice",
        LayoutEngineOptionType::Size => "size",
    }
}

fn fmt_choices(choices: &[String]) -> String {
    if choices.is_empty() {
        String::new()
    } else {
        format!(": {}", choices.join("|"))
    }
}

/// A line that says what an event is about.
fn describe_event(event: &Event) -> String {
    match event {
        Event::MonitorAdded { monitor } => format!("Monitor added: \"{}\"", monitor.name),
        Event::MonitorChanged { monitor } => format!("Monitor changed: \"{}\"", monitor.name),
        Event::MonitorRemoved { name } => format!("Monitor removed: \"{name}\""),
        Event::WorkspaceAdded { workspace } => format!("Workspace added: {}", workspace.id),
        Event::WorkspaceChanged { workspace } => format!("Workspace changed: {}", workspace.id),
        Event::WorkspaceRemoved { id } => format!("Workspace removed: {id}"),
        Event::WindowAdded { window } => format!("Window added: {}", describe_window(window)),
        Event::WindowChanged { window } => format!("Window changed: {}", describe_window(window)),
        Event::WindowRemoved { id } => format!("Window removed: {id}"),
        Event::FocusChanged { monitor, window } => {
            format!("Focus changed: monitor {monitor:?}, window {window:?}")
        }
        Event::GroupAdded { group } => format!("Group added: {}", group.id),
        Event::GroupChanged { group } => format!("Group changed: {}", group.id),
        Event::GroupRemoved { id } => format!("Group removed: {id}"),
        Event::EngineStateChanged { state } => format!("Engine state changed: {state}"),
        Event::EngineChanged { engine } => format!("Engine changed: \"{}\"", engine.name),
        Event::KeyboardLayoutsChanged { keyboard_layouts } => {
            format!("Keyboard layouts changed: {keyboard_layouts:?}")
        }
        Event::KeyboardLayoutSwitched { index } => format!("Keyboard layout switched: {index}"),
        Event::OverviewChanged { is_open } => format!("Overview toggled: {is_open}"),
        Event::CastsChanged { casts } => format!("Casts changed: {} casts", casts.len()),
        Event::ConfigLoaded { failed } => {
            let status = if *failed {
                "with an error"
            } else {
                "successfully"
            };
            format!("Config loaded {status}")
        }
        Event::ScreenshotCaptured { path } => {
            let mut parts = vec![];
            parts.push("copied to clipboard".to_string());
            if let Some(path) = path {
                parts.push(format!("saved to {path}"));
            }
            let description = parts.join(" and ");
            format!("Screenshot captured: {description}")
        }
        Event::TouchpadToggled { on } => format!("Touchpad enabled: {on}"),
    }
}

fn describe_window(window: &Window) -> String {
    let app_id = window.app_id.as_deref().unwrap_or("(unset)");
    let title = window.title.as_deref().unwrap_or("(unset)");
    format!("{} \"{app_id}\" \"{title}\"", window.id)
}

fn fmt_rounded(x: f64) -> String {
    let r = x.round();
    if (r - x).abs() <= 0.005 {
        format!("{r}")
    } else {
        format!("{x:.2}")
    }
}

fn ensure_absolute_path(path: &mut String) -> anyhow::Result<()> {
    let p = Path::new(path);
    if p.is_relative() {
        let mut cwd = env::current_dir().context("error getting current working directory")?;
        cwd.push(p);
        match cwd.into_os_string().into_string() {
            Ok(absolute) => *path = absolute,
            Err(cwd) => bail!("couldn't convert absolute path to string: {cwd:?}"),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use insta::assert_snapshot;

    use super::*;

    #[test]
    fn test_fmt_rounded() {
        assert_snapshot!(fmt_rounded(1.9), @"1.90");
        assert_snapshot!(fmt_rounded(1.994), @"1.99");
        assert_snapshot!(fmt_rounded(1.996), @"2");
        assert_snapshot!(fmt_rounded(2.0), @"2");
        assert_snapshot!(fmt_rounded(2.004), @"2");
        assert_snapshot!(fmt_rounded(2.006), @"2.01");
        assert_snapshot!(fmt_rounded(2.1), @"2.10");
    }

    #[test]
    fn arguments_keep_the_type_they_can_be_mistaken_for() {
        assert_eq!(parse_value("3"), json!(3));
        assert_eq!(parse_value("-100"), json!(-100));
        assert_eq!(parse_value("1.5"), json!(1.5));
        assert_eq!(parse_value("true"), json!(true));
        // These are strings: `+10` is a change, not the number 10, and `007` isn't `7`.
        assert_eq!(parse_value("+10"), json!("+10"));
        assert_eq!(parse_value("+10%"), json!("+10%"));
        assert_eq!(parse_value("007"), json!("007"));
        assert_eq!(parse_value("web"), json!("web"));
    }
}
