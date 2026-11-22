use niri_config::{Action, Config, WorkspaceReference};
use smithay::output::Output;
use smithay::utils::{Logical, Point};
use wayland_client::protocol::wl_surface::WlSurface;

use super::client::ClientId;
use super::*;

/// Opens a window, on whatever monitor it goes to.
fn open(f: &mut Fixture, id: ClientId) -> WlSurface {
    let window = f.client(id).create_window();
    let surface = window.surface.clone();
    window.commit();
    f.roundtrip(id);
    map(f, id, &surface);
    surface
}

fn map(f: &mut Fixture, id: ClientId, surface: &WlSurface) {
    let window = f.client(id).window(surface);
    window.attach_new_buffer();
    window.set_size(100, 100);
    window.ack_last_and_commit();
    f.double_roundtrip(id);
}

/// Two monitors side by side, each with a window, the first one's with the focus and the pointer
/// on it.
fn set_up() -> (Fixture, ClientId) {
    set_up_with_config(Config::default())
}

fn set_up_with_config(config: Config) -> (Fixture, ClientId) {
    let mut f = Fixture::with_config(config);
    f.add_output(1, (1920, 1080));
    f.add_output(2, (1280, 720));
    let id = f.add_client();
    for n in [2, 1] {
        point_at(&mut f, id, n);
        f.niri_focus_output(n);
        f.double_roundtrip(id);
        open(&mut f, id);
    }
    assert_eq!(active(&mut f), f.niri_output(1));
    (f, id)
}

/// Moves the pointer to the middle of a monitor, as a mouse does.
fn point_at(f: &mut Fixture, id: ClientId, n: u8) {
    let at = if n == 1 { 960 } else { 1920 + 640 };
    let pointer = f.client(id).state.virtual_pointer();
    pointer.motion_absolute(0, at, 300, 1920 + 1280, 1080);
    pointer.frame();
    f.double_roundtrip(id);
}

fn active(f: &mut Fixture) -> Output {
    f.niri().layout.active_output().unwrap().clone()
}

fn pointer(f: &mut Fixture) -> Point<f64, Logical> {
    let pointer = f.niri().seat.get_pointer().unwrap();
    pointer.current_location()
}

/// Which of its workspaces each monitor shows.
fn shown(f: &mut Fixture) -> [usize; 2] {
    [1, 2].map(|n| {
        let output = f.niri_output(n);
        let mon = f.niri().layout.monitor_for_output(&output).unwrap();
        mon.active_workspace_idx()
    })
}

fn act(f: &mut Fixture, id: ClientId, action: Action) {
    f.niri_state().do_action(action, false);
    f.double_roundtrip(id);
}

/// The windows on a monitor.
fn windows_on(f: &mut Fixture, n: u8) -> usize {
    let output = f.niri_output(n);
    let mon = f.niri().layout.monitor_for_output(&output).unwrap();
    mon.windows().count()
}

#[test]
fn workspaces_switch_on_the_pointers_monitor_and_the_keyboard_stays() {
    let (mut f, id) = set_up();
    let focused = f.niri().layout.focus().unwrap().id();
    point_at(&mut f, id, 2);

    act(&mut f, id, Action::FocusWorkspaceDown);
    assert_eq!(shown(&mut f), [0, 1]);
    act(&mut f, id, Action::FocusWorkspaceUp);
    assert_eq!(shown(&mut f), [0, 0]);
    let second = WorkspaceReference::Index(2);
    act(&mut f, id, Action::FocusWorkspace(second));
    assert_eq!(shown(&mut f), [0, 1]);
    act(&mut f, id, Action::FocusWorkspacePrevious);
    assert_eq!(shown(&mut f), [0, 0]);

    assert_eq!(active(&mut f), f.niri_output(1));
    assert_eq!(f.niri().layout.focus().unwrap().id(), focused);
}

#[test]
fn the_monitor_that_got_the_focus_last_is_current_again() {
    let (mut f, id) = set_up();
    point_at(&mut f, id, 2);
    // The focus goes to the second monitor and back, and the pointer stays.
    for n in [2, 1] {
        f.niri_focus_output(n);
        f.double_roundtrip(id);
    }
    act(&mut f, id, Action::FocusWorkspaceDown);
    assert_eq!(shown(&mut f), [1, 0]);

    // The pointer moving over the monitor it's on doesn't change that, and coming onto another
    // does.
    point_at(&mut f, id, 1);
    point_at(&mut f, id, 2);
    act(&mut f, id, Action::FocusWorkspaceDown);
    assert_eq!(shown(&mut f), [1, 1]);
}

#[test]
fn workspace_actions_are_about_the_current_monitors_workspace() {
    let (mut f, id) = set_up();
    point_at(&mut f, id, 2);
    act(&mut f, id, Action::SetWorkspaceName(String::from("web")));
    let (_, ws) = f.niri().layout.find_workspace_by_name("web").unwrap();
    assert_eq!(ws.current_output().cloned(), Some(f.niri_output(2)));
    // Under another that stays.
    act(&mut f, id, Action::FocusWorkspaceDown);
    act(&mut f, id, Action::SetWorkspaceName(String::from("chat")));
    act(&mut f, id, Action::FocusWorkspaceUp);
    act(&mut f, id, Action::MoveWorkspaceDown);
    assert_eq!(shown(&mut f), [0, 1]);
    act(&mut f, id, Action::UnsetWorkspaceName);
    assert!(f.niri().layout.find_workspace_by_name("web").is_none());
    assert_eq!(active(&mut f), f.niri_output(1));
}

#[test]
fn new_windows_open_on_the_current_monitor_and_take_the_focus() {
    let (mut f, id) = set_up();
    point_at(&mut f, id, 2);
    open(&mut f, id);
    assert_eq!([windows_on(&mut f, 1), windows_on(&mut f, 2)], [1, 2]);
    assert_eq!(active(&mut f), f.niri_output(2));
}

#[test]
fn going_to_a_workspace_on_the_pointers_monitor_leaves_the_pointer() {
    let (mut f, id) = set_up();
    point_at(&mut f, id, 2);
    for n in [2, 1] {
        f.niri_focus_output(n);
        f.double_roundtrip(id);
    }
    let at = pointer(&mut f);
    let output = f.niri_output(2);
    let mon = f.niri().layout.monitor_for_output(&output).unwrap();
    let ws = mon.active_workspace_ref().id().get();
    act(
        &mut f,
        id,
        Action::FocusWorkspace(WorkspaceReference::Id(ws)),
    );
    assert_eq!(pointer(&mut f), at);

    // From another monitor, it still comes along.
    let output = f.niri_output(1);
    let mon = f.niri().layout.monitor_for_output(&output).unwrap();
    let ws = mon.active_workspace_ref().id().get();
    act(
        &mut f,
        id,
        Action::FocusWorkspace(WorkspaceReference::Id(ws)),
    );
    assert!(pointer(&mut f).x < 1920.);
    assert_eq!(active(&mut f), f.niri_output(1));
}

#[test]
fn a_program_bento_started_opens_where_it_was_started() {
    let (mut f, id) = set_up();
    point_at(&mut f, id, 2);
    let token = f.niri().launch_token();
    point_at(&mut f, id, 1);

    let window = f.client(id).create_window();
    let surface = window.surface.clone();
    // As its app does with the token it was started with.
    let data = f.niri().activation_state.data_for_token(&token).unwrap();
    let data = data.clone();
    f.roundtrip(id);
    let unmapped = f.niri().unmapped_windows.values_mut().next().unwrap();
    unmapped.activation_token_data = Some(data);
    f.client(id).window(&surface).commit();
    f.roundtrip(id);
    map(&mut f, id, &surface);
    assert_eq!([windows_on(&mut f, 1), windows_on(&mut f, 2)], [1, 2]);
}

#[test]
fn a_dialog_opens_by_its_parent_and_leaves_the_focus() {
    let (mut f, id) = set_up();
    // The second monitor's window gets a dialog, and the pointer is there.
    point_at(&mut f, id, 2);
    let parent = f.client(id).state.windows[0].xdg_toplevel.clone();
    let window = f.client(id).create_window();
    window.set_parent(Some(&parent));
    let surface = window.surface.clone();
    window.commit();
    f.roundtrip(id);
    map(&mut f, id, &surface);
    assert_eq!([windows_on(&mut f, 1), windows_on(&mut f, 2)], [1, 2]);
    assert_eq!(active(&mut f), f.niri_output(1));
}

#[test]
fn a_monitor_thats_gone_isnt_current() {
    let (mut f, id) = set_up();
    point_at(&mut f, id, 2);
    let gone = f.niri_output(2);
    f.niri().remove_output(&gone);
    f.double_roundtrip(id);
    assert_eq!(f.niri().current_output().cloned(), Some(f.niri_output(1)));
}

#[test]
fn moving_a_workspace_to_another_monitor_takes_the_focus_along() {
    let (mut f, id) = set_up();
    point_at(&mut f, id, 2);
    act(&mut f, id, Action::MoveWorkspaceToMonitorLeft);
    assert_eq!([windows_on(&mut f, 1), windows_on(&mut f, 2)], [2, 0]);
    assert_eq!(active(&mut f), f.niri_output(1));
    assert!(pointer(&mut f).x < 1920.);
    assert_eq!(f.niri().current_output().cloned(), Some(f.niri_output(1)));
}

#[test]
fn the_pointer_comes_along_with_the_focus_unless_its_kept() {
    let (mut f, id) = set_up();
    act(&mut f, id, Action::FocusMonitorRight);
    assert!(pointer(&mut f).x > 1920.);

    let mut config = Config::default();
    config.input.keep_mouse_on_monitor = true;
    let (mut f, id) = set_up_with_config(config);
    let at = pointer(&mut f);
    act(&mut f, id, Action::FocusMonitorRight);
    assert_eq!(active(&mut f), f.niri_output(2));
    assert_eq!(pointer(&mut f), at);
    // The monitor that got the focus is current, and stays so as the mouse is nudged where it is.
    assert_eq!(f.niri().current_output().cloned(), Some(f.niri_output(2)));
    point_at(&mut f, id, 1);
    assert_eq!(f.niri().current_output().cloned(), Some(f.niri_output(2)));
    act(&mut f, id, Action::FocusWorkspaceDown);
    assert_eq!(shown(&mut f), [0, 1]);

    // Going to a window or a workspace there leaves it too.
    act(&mut f, id, Action::FocusMonitorLeft);
    let output = f.niri_output(2);
    let mon = f.niri().layout.monitor_for_output(&output).unwrap();
    let ws = mon.active_workspace_ref().id().get();
    act(
        &mut f,
        id,
        Action::FocusWorkspace(WorkspaceReference::Id(ws)),
    );
    assert_eq!(active(&mut f), f.niri_output(2));
    assert!(pointer(&mut f).x < 1920.);
}
