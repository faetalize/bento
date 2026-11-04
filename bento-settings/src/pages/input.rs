use niri_config::input::{AccelProfile, ClickMethod, ScrollMethod, TrackLayout};

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
        .keywords(&["xkb", "typing", "language"])
        .group(
            Group::new("Layout")
                .setting(
                    Setting::text("Layouts", "input.keyboard.xkb.layout", |c| {
                        c.input.keyboard.xkb.layout.clone()
                    })
                    .keywords(&["language", "us", "qwerty", "azerty"]),
                )
                .setting(
                    Setting::text("Variants", "input.keyboard.xkb.variant", |c| {
                        c.input.keyboard.xkb.variant.clone()
                    })
                    .keywords(&["dvorak", "colemak"]),
                )
                .setting(
                    Setting::text("Options", "input.keyboard.xkb.options", |c| {
                        c.input.keyboard.xkb.options.clone().unwrap_or_default()
                    })
                    .keywords(&["compose", "caps lock", "switch layout"]),
                )
                .setting(
                    Setting::choice("Switch Layouts", "input.keyboard.track-layout", |c| {
                        Some(match c.input.keyboard.track_layout {
                            TrackLayout::Global => "global",
                            TrackLayout::Window => "window",
                        })
                    })
                    .option(Some("global"), "Everywhere")
                    .option(Some("window"), "Per Window")
                    .keywords(&["track layout"]),
                ),
        )
        .group(
            Group::new("Typing")
                .setting(
                    Setting::number("Repeat Delay", "input.keyboard.repeat-delay", |c| {
                        c.input.keyboard.repeat_delay.into()
                    })
                    .range(100., 2000., 25.)
                    .keywords(&["key repeat", "hold"]),
                )
                .setting(
                    Setting::number("Repeat Rate", "input.keyboard.repeat-rate", |c| {
                        c.input.keyboard.repeat_rate.into()
                    })
                    .range(1., 100., 1.)
                    .keywords(&["key repeat", "speed"]),
                )
                .setting(
                    Setting::switch("Num Lock", "input.keyboard.numlock", |c| {
                        c.input.keyboard.numlock
                    })
                    .keywords(&["numpad", "number"]),
                ),
        )
}

pub fn mouse() -> Category {
    Category::settings("mouse", "Mouse")
        .icon("input-mouse-symbolic")
        .keywords(&["pointer"])
        .group(
            Group::untitled()
                .setting(
                    Setting::number("Speed", "input.mouse.accel-speed", |c| {
                        c.input.mouse.accel_speed.0
                    })
                    .range(-1., 1., 0.05)
                    .digits(2)
                    .keywords(&["sensitivity", "dpi"]),
                )
                .setting(
                    Setting::choice("Acceleration", "input.mouse.accel-profile", |c| {
                        accel_profile(c.input.mouse.accel_profile)
                    })
                    .option(None, "Default")
                    .option(Some("adaptive"), "Adaptive")
                    .option(Some("flat"), "Flat")
                    .keywords(&["profile", "raw input"]),
                )
                .setting(
                    Setting::switch("Natural Scrolling", "input.mouse.natural-scroll", |c| {
                        c.input.mouse.natural_scroll
                    })
                    .style(BoolStyle::Presence)
                    .keywords(&["reverse", "invert"]),
                )
                .setting(
                    Setting::switch("Left-Handed", "input.mouse.left-handed", |c| {
                        c.input.mouse.left_handed
                    })
                    .style(BoolStyle::Presence)
                    .keywords(&["swap buttons"]),
                )
                .setting(
                    Setting::switch(
                        "Middle-Click Emulation",
                        "input.mouse.middle-emulation",
                        |c| c.input.mouse.middle_emulation,
                    )
                    .style(BoolStyle::Presence)
                    .keywords(&["middle button"]),
                ),
        )
}

pub fn touchpad() -> Category {
    Category::settings("touchpad", "Touchpad")
        .icon("input-touchpad-symbolic")
        .keywords(&["trackpad", "laptop"])
        .group(
            Group::untitled()
                .setting(
                    Setting::switch("Tap to Click", "input.touchpad.tap", |c| {
                        c.input.touchpad.tap
                    })
                    .style(BoolStyle::Presence)
                    .expands(vec![
                        Setting::switch("Tap and Drag", "input.touchpad.drag", |c| {
                            c.input.touchpad.drag.unwrap_or(true)
                        })
                        .style(BoolStyle::Arg),
                        Setting::switch("Drag Lock", "input.touchpad.drag-lock", |c| {
                            c.input.touchpad.drag_lock
                        })
                        .style(BoolStyle::Presence),
                    ]),
                )
                .setting(
                    Setting::choice("Secondary Click", "input.touchpad.click-method", |c| {
                        c.input.touchpad.click_method.map(|method| match method {
                            ClickMethod::ButtonAreas => "button-areas",
                            ClickMethod::Clickfinger => "clickfinger",
                        })
                    })
                    .option(None, "Default")
                    .option(Some("clickfinger"), "Two Fingers")
                    .option(Some("button-areas"), "Corner")
                    .keywords(&["right click", "click method"]),
                ),
        )
        .group(
            Group::new("Scrolling")
                .setting(
                    Setting::switch("Natural Scrolling", "input.touchpad.natural-scroll", |c| {
                        c.input.touchpad.natural_scroll
                    })
                    .style(BoolStyle::Presence)
                    .keywords(&["reverse", "invert"]),
                )
                .setting(
                    Setting::choice("Method", "input.touchpad.scroll-method", |c| {
                        c.input.touchpad.scroll_method.map(|method| match method {
                            ScrollMethod::NoScroll => "no-scroll",
                            ScrollMethod::TwoFinger => "two-finger",
                            ScrollMethod::Edge => "edge",
                            ScrollMethod::OnButtonDown => "on-button-down",
                        })
                    })
                    .option(None, "Default")
                    .option(Some("two-finger"), "Two Fingers")
                    .option(Some("edge"), "Edge")
                    .option(Some("on-button-down"), "Button")
                    .option(Some("no-scroll"), "Off")
                    .keywords(&["two finger", "edge scrolling"]),
                ),
        )
        .group(
            Group::new("Pointer")
                .setting(
                    Setting::number("Speed", "input.touchpad.accel-speed", |c| {
                        c.input.touchpad.accel_speed.0
                    })
                    .range(-1., 1., 0.05)
                    .digits(2)
                    .keywords(&["sensitivity"]),
                )
                .setting(
                    Setting::choice("Acceleration", "input.touchpad.accel-profile", |c| {
                        accel_profile(c.input.touchpad.accel_profile)
                    })
                    .option(None, "Default")
                    .option(Some("adaptive"), "Adaptive")
                    .option(Some("flat"), "Flat"),
                )
                .setting(
                    Setting::switch("Disable While Typing", "input.touchpad.dwt", |c| {
                        c.input.touchpad.dwt
                    })
                    .style(BoolStyle::Presence)
                    .keywords(&["palm", "dwt"]),
                )
                .setting(
                    Setting::switch(
                        "Disable With Mouse",
                        "input.touchpad.disabled-on-external-mouse",
                        |c| c.input.touchpad.disabled_on_external_mouse,
                    )
                    .style(BoolStyle::Presence)
                    .keywords(&["external mouse"]),
                )
                .setting(
                    Setting::switch("Left-Handed", "input.touchpad.left-handed", |c| {
                        c.input.touchpad.left_handed
                    })
                    .style(BoolStyle::Presence)
                    .keywords(&["swap buttons"]),
                ),
        )
}

pub fn cursor() -> Category {
    Category::settings("cursor", "Cursor")
        .icon("find-location-symbolic")
        .keywords(&["pointer", "mouse", "xcursor"])
        .group(
            Group::untitled()
                .setting(
                    Setting::text("Theme", "cursor.xcursor-theme", |c| {
                        c.cursor.xcursor_theme.clone()
                    })
                    .keywords(&["xcursor"]),
                )
                .setting(
                    Setting::number("Size", "cursor.xcursor-size", |c| {
                        c.cursor.xcursor_size.into()
                    })
                    .range(8., 255., 1.)
                    .keywords(&["big", "large", "accessibility"]),
                )
                .setting(Setting::switch(
                    "Hide While Typing",
                    "cursor.hide-when-typing",
                    |c| c.cursor.hide_when_typing,
                )),
        )
}
