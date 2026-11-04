use niri_config::input::{AccelProfile, ClickMethod, ScrollMethod, TapButtonMap, TrackLayout};

use crate::model::{BoolStyle, Category, Group, Setting};

fn accel_profile(profile: Option<AccelProfile>) -> Option<&'static str> {
    profile.map(|profile| match profile {
        AccelProfile::Adaptive => "adaptive",
        AccelProfile::Flat => "flat",
    })
}

pub fn keyboard() -> Category {
    Category::settings("keyboard", "Keyboard")
        .icon("input-keyboard-symbolic")
        .description("Layouts and key repeat.")
        .keywords(&["xkb", "typing", "language"])
        .group(
            Group::new("Layouts")
                .description("Leave these empty to use the system's keyboard settings.")
                .setting(
                    Setting::text("Layouts", "input.keyboard.xkb.layout", |c| {
                        c.input.keyboard.xkb.layout.clone()
                    })
                    .keywords(&["language", "us", "qwerty", "azerty", "xkb"]),
                )
                .setting(
                    Setting::text("Variants", "input.keyboard.xkb.variant", |c| {
                        c.input.keyboard.xkb.variant.clone()
                    })
                    .keywords(&["dvorak", "colemak", "xkb"]),
                )
                .setting(
                    Setting::text("Options", "input.keyboard.xkb.options", |c| {
                        c.input.keyboard.xkb.options.clone().unwrap_or_default()
                    })
                    .keywords(&[
                        "compose",
                        "caps lock",
                        "grp",
                        "switch layout",
                        "xkb",
                    ]),
                )
                .setting(
                    Setting::choice("Remember layout", "input.keyboard.track-layout", |c| {
                        Some(match c.input.keyboard.track_layout {
                            TrackLayout::Global => "global",
                            TrackLayout::Window => "window",
                        })
                    })
                    .option(Some("global"), "For everything")
                    .option(Some("window"), "Per window")
                    .keywords(&["switch layout", "track"]),
                ),
        )
        .group(
            Group::new("Typing")
                .setting(
                    Setting::number("Repeat delay", "input.keyboard.repeat-delay", |c| {
                        c.input.keyboard.repeat_delay.into()
                    })
                    .subtitle("How long to hold a key before it repeats, in milliseconds")
                    .range(100., 2000., 25.)
                    .keywords(&["key repeat", "hold"]),
                )
                .setting(
                    Setting::number("Repeat rate", "input.keyboard.repeat-rate", |c| {
                        c.input.keyboard.repeat_rate.into()
                    })
                    .subtitle("Characters per second while a key is held")
                    .range(1., 100., 1.)
                    .keywords(&["key repeat", "speed"]),
                )
                .setting(
                    Setting::switch("Num Lock on at startup", "input.keyboard.numlock", |c| {
                        c.input.keyboard.numlock
                    })
                    .keywords(&["numpad", "number"]),
                ),
        )
}

pub fn mouse() -> Category {
    Category::settings("mouse", "Mouse")
        .icon("input-mouse-symbolic")
        .description("Pointer speed and scrolling for mice.")
        .keywords(&["pointer", "cursor speed"])
        .group(
            Group::new("Pointer")
                .setting(
                    Setting::number("Speed", "input.mouse.accel-speed", |c| {
                        c.input.mouse.accel_speed.0
                    })
                    .subtitle("From -1, slowest, to 1, fastest")
                    .range(-1., 1., 0.05)
                    .digits(2)
                    .keywords(&["acceleration", "sensitivity", "dpi"]),
                )
                .setting(
                    Setting::choice("Acceleration", "input.mouse.accel-profile", |c| {
                        accel_profile(c.input.mouse.accel_profile)
                    })
                    .option(None, "Device default")
                    .option(Some("adaptive"), "Adaptive")
                    .option(Some("flat"), "Flat")
                    .keywords(&["profile", "raw input", "gaming"]),
                )
                .setting(
                    Setting::switch("Left-handed", "input.mouse.left-handed", |c| {
                        c.input.mouse.left_handed
                    })
                    .style(BoolStyle::Presence)
                    .subtitle("Swap the left and right buttons")
                    .keywords(&["buttons", "swap"]),
                )
                .setting(
                    Setting::switch(
                        "Middle-click by pressing both buttons",
                        "input.mouse.middle-emulation",
                        |c| c.input.mouse.middle_emulation,
                    )
                    .style(BoolStyle::Presence)
                    .keywords(&["middle button", "emulation"]),
                ),
        )
        .group(
            Group::new("Scrolling").setting(
                Setting::switch("Natural scrolling", "input.mouse.natural-scroll", |c| {
                    c.input.mouse.natural_scroll
                })
                .style(BoolStyle::Presence)
                .subtitle("Content moves the way the wheel turns")
                .keywords(&["reverse", "invert", "direction"]),
            ),
        )
}

pub fn touchpad() -> Category {
    Category::settings("touchpad", "Touchpad")
        .icon("input-touchpad-symbolic")
        .description("Tapping, scrolling and clicking on touchpads.")
        .keywords(&["trackpad", "laptop", "pointer"])
        .group(
            Group::new("Tapping")
                .setting(
                    Setting::switch("Tap to click", "input.touchpad.tap", |c| {
                        c.input.touchpad.tap
                    })
                    .style(BoolStyle::Presence)
                    .keywords(&["tap", "click"]),
                )
                .setting(
                    Setting::choice(
                        "Two- and three-finger taps",
                        "input.touchpad.tap-button-map",
                        |c| {
                            c.input.touchpad.tap_button_map.map(|map| match map {
                                TapButtonMap::LeftRightMiddle => "left-right-middle",
                                TapButtonMap::LeftMiddleRight => "left-middle-right",
                            })
                        },
                    )
                    .option(None, "Device default")
                    .option(Some("left-right-middle"), "Right-click, then middle-click")
                    .option(Some("left-middle-right"), "Middle-click, then right-click")
                    .enabled_if(|c| c.input.touchpad.tap)
                    .keywords(&["right click", "middle click", "button map"]),
                )
                .setting(
                    Setting::switch("Tap and drag", "input.touchpad.drag", |c| {
                        c.input.touchpad.drag.unwrap_or(true)
                    })
                    .style(BoolStyle::Arg)
                    .subtitle("Tap, then touch again and move to drag")
                    .enabled_if(|c| c.input.touchpad.tap),
                )
                .setting(
                    Setting::switch("Drag lock", "input.touchpad.drag-lock", |c| {
                        c.input.touchpad.drag_lock
                    })
                    .style(BoolStyle::Presence)
                    .subtitle("Keep dragging briefly after lifting your finger")
                    .enabled_if(|c| c.input.touchpad.tap),
                ),
        )
        .group(
            Group::new("Scrolling")
                .setting(
                    Setting::switch("Natural scrolling", "input.touchpad.natural-scroll", |c| {
                        c.input.touchpad.natural_scroll
                    })
                    .style(BoolStyle::Presence)
                    .subtitle("Content moves with your fingers")
                    .keywords(&["reverse", "invert", "direction"]),
                )
                .setting(
                    Setting::choice("Scroll with", "input.touchpad.scroll-method", |c| {
                        c.input.touchpad.scroll_method.map(|method| match method {
                            ScrollMethod::NoScroll => "no-scroll",
                            ScrollMethod::TwoFinger => "two-finger",
                            ScrollMethod::Edge => "edge",
                            ScrollMethod::OnButtonDown => "on-button-down",
                        })
                    })
                    .option(None, "Device default")
                    .option(Some("two-finger"), "Two fingers")
                    .option(Some("edge"), "Edge of the touchpad")
                    .option(Some("on-button-down"), "Moving while holding a button")
                    .option(Some("no-scroll"), "Nothing")
                    .keywords(&["two finger", "edge scrolling"]),
                ),
        )
        .group(
            Group::new("Pointer")
                .setting(
                    Setting::number("Speed", "input.touchpad.accel-speed", |c| {
                        c.input.touchpad.accel_speed.0
                    })
                    .subtitle("From -1, slowest, to 1, fastest")
                    .range(-1., 1., 0.05)
                    .digits(2)
                    .keywords(&["acceleration", "sensitivity"]),
                )
                .setting(
                    Setting::choice("Acceleration", "input.touchpad.accel-profile", |c| {
                        accel_profile(c.input.touchpad.accel_profile)
                    })
                    .option(None, "Device default")
                    .option(Some("adaptive"), "Adaptive")
                    .option(Some("flat"), "Flat")
                    .keywords(&["profile"]),
                )
                .setting(
                    Setting::choice("Right-click by", "input.touchpad.click-method", |c| {
                        c.input.touchpad.click_method.map(|method| match method {
                            ClickMethod::ButtonAreas => "button-areas",
                            ClickMethod::Clickfinger => "clickfinger",
                        })
                    })
                    .option(None, "Device default")
                    .option(Some("button-areas"), "Pressing the bottom-right corner")
                    .option(Some("clickfinger"), "Pressing with two fingers")
                    .keywords(&[
                        "click method",
                        "secondary click",
                        "clickfinger",
                    ]),
                )
                .setting(
                    Setting::switch("Off while typing", "input.touchpad.dwt", |c| {
                        c.input.touchpad.dwt
                    })
                    .style(BoolStyle::Presence)
                    .keywords(&["disable while typing", "palm", "dwt"]),
                )
                .setting(
                    Setting::switch(
                        "Off while a mouse is plugged in",
                        "input.touchpad.disabled-on-external-mouse",
                        |c| c.input.touchpad.disabled_on_external_mouse,
                    )
                    .style(BoolStyle::Presence)
                    .keywords(&["external mouse", "disable"]),
                )
                .setting(
                    Setting::switch("Left-handed", "input.touchpad.left-handed", |c| {
                        c.input.touchpad.left_handed
                    })
                    .style(BoolStyle::Presence)
                    .keywords(&["buttons", "swap"]),
                ),
        )
}

pub fn cursor() -> Category {
    Category::settings("cursor", "Cursor")
        .icon("find-location-symbolic")
        .description("The pointer's theme and size, and when it hides.")
        .keywords(&["pointer", "mouse", "xcursor"])
        .group(
            Group::new("Look")
                .setting(
                    Setting::text("Theme", "cursor.xcursor-theme", |c| {
                        c.cursor.xcursor_theme.clone()
                    })
                    .keywords(&["xcursor", "icons"]),
                )
                .setting(
                    Setting::number("Size", "cursor.xcursor-size", |c| {
                        c.cursor.xcursor_size.into()
                    })
                    .range(8., 255., 1.)
                    .keywords(&["big", "large", "accessibility"]),
                ),
        )
        .group(
            Group::new("Hiding").setting(
                Setting::switch("Hide while typing", "cursor.hide-when-typing", |c| {
                    c.cursor.hide_when_typing
                })
                .keywords(&["hide", "keyboard"]),
            ),
        )
}
