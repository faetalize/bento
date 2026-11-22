use niri_config::Config;

use super::*;

#[test]
fn the_shell_knows_where_windows_are_on_every_workspace() {
    let mut f = Fixture::with_config(Config::default());
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    for _ in 0..2 {
        let window = f.client(id).create_window();
        let surface = window.surface.clone();
        window.commit();
        f.roundtrip(id);
        let window = f.client(id).window(&surface);
        window.attach_new_buffer();
        window.set_size(100, 100);
        window.ack_last_and_commit();
        f.double_roundtrip(id);
    }
    // The second goes to the second workspace, and the first workspace stays on screen.
    f.niri().layout.move_to_workspace_down(false);
    f.double_roundtrip(id);

    let windows = f.niri().shell_windows();
    assert_eq!(windows.len(), 2);
    assert_ne!(windows[0]["workspace"], windows[1]["workspace"]);
    for window in &windows {
        assert_eq!(window["active"], true, "each is its workspace's: {window}");
        let rect = &window["rect"];
        assert!(rect["width"].as_f64().unwrap() > 0., "{window}");
        assert!(rect["x"].as_f64().unwrap() >= 0., "{window}");
    }
    let focused = windows.iter().filter(|w| w["focused"] == true).count();
    assert!(focused <= 1, "{windows:?}");
}
