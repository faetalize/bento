use std::collections::HashMap;

use niri_config::animations::{Curve, EasingParams, Kind, SpringParams};
use niri_config::{resolve_gradient, Animations, Color, Gradient};
use niri_ipc::LayoutEngineGradient;
use serde::{Deserialize, Deserializer};
use smithay::utils::{Logical, Point, Rectangle, Size};

/// Scales a node around the centre of its rect, then moves it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NodeTransform {
    pub scale: f64,
    pub x: f64,
    pub y: f64,
}

impl NodeTransform {
    pub const IDENTITY: Self = Self {
        scale: 1.,
        x: 0.,
        y: 0.,
    };
}

/// What a fill or a border is painted with: a colour or a gradient.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Paint {
    Color(Color),
    Gradient(Gradient),
}

impl Paint {
    /// The gradient it paints: a colour is a gradient of one colour.
    pub fn gradient(self) -> Gradient {
        match self {
            Paint::Color(color) => Gradient::from(color),
            Paint::Gradient(gradient) => gradient,
        }
    }

    /// The colour to paint with where gradients can't be drawn: the colour itself, or the start
    /// of the gradient.
    pub fn solid(self) -> Color {
        match self {
            Paint::Color(color) => color,
            Paint::Gradient(gradient) => gradient.from,
        }
    }

    /// Whether it paints anything.
    pub fn is_visible(self) -> bool {
        match self {
            Paint::Color(color) => color.a > 0.,
            Paint::Gradient(gradient) => gradient.from.a > 0. || gradient.to.a > 0.,
        }
    }

    /// The same paint, fully transparent.
    pub fn transparent(self) -> Self {
        let clear = |color: Color| Color { a: 0., ..color };
        match self {
            Paint::Color(color) => Paint::Color(clear(color)),
            Paint::Gradient(gradient) => Paint::Gradient(Gradient {
                from: clear(gradient.from),
                to: clear(gradient.to),
                ..gradient
            }),
        }
    }
}

/// A border drawn inside a node's rect.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Border {
    pub width: f64,
    pub color: Paint,
}

/// How a property animates when it changes.
#[derive(Debug, Clone, PartialEq)]
pub enum Transition {
    /// One of the animations in the config's `animations {}`, by its name there.
    Named(String),
    Config(niri_config::Animation),
}

impl Transition {
    pub fn resolve(&self, animations: &Animations) -> Option<niri_config::Animation> {
        match self {
            Transition::Named(name) => named_animation(animations, name),
            Transition::Config(config) => Some(*config),
        }
    }
}

/// The transitions of the properties that animate.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Transitions {
    pub rect: Option<Transition>,
    pub opacity: Option<Transition>,
    pub transform: Option<Transition>,
    pub corner_radius: Option<Transition>,
    pub background: Option<Transition>,
}

/// Properties to change on a node. What's left out stays as it is.
#[derive(Debug, Default, Clone, PartialEq, Deserialize)]
#[serde(try_from = "RawProps")]
pub struct Update {
    pub rect: Option<Rectangle<f64, Logical>>,
    pub opacity: Option<f64>,
    pub transform: Option<NodeTransform>,
    pub clip: Option<bool>,
    pub corner_radius: Option<f64>,
    pub background: Option<Option<Paint>>,
    pub border: Option<Option<Border>>,
    pub text: Option<String>,
    pub font: Option<String>,
    pub color: Option<Color>,
    pub image: Option<String>,
    pub placeholder: Option<String>,
    pub interactive: Option<bool>,
    pub transitions: Option<Transitions>,
}

/// Looks up an animation of the config by the name it has there.
pub fn named_animation(animations: &Animations, name: &str) -> Option<niri_config::Animation> {
    Some(match name {
        "workspace-switch" => animations.workspace_switch.0,
        "window-open" => animations.window_open.anim,
        "window-close" => animations.window_close.anim,
        "horizontal-view-movement" => animations.horizontal_view_movement.0,
        "window-movement" => animations.window_movement.0,
        "window-resize" => animations.window_resize.anim,
        "config-notification-open-close" => animations.config_notification_open_close.0,
        "exit-confirmation-open-close" => animations.exit_confirmation_open_close.0,
        "screenshot-ui-open" => animations.screenshot_ui_open.0,
        "overview-open-close" => animations.overview_open_close.0,
        "recent-windows-close" => animations.recent_windows_close.0,
        _ => return None,
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTransform {
    scale: Option<f64>,
    x: Option<f64>,
    y: Option<f64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBorder {
    width: f64,
    /// A colour or a gradient.
    color: serde_json::Value,
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
enum RawCurve {
    Linear,
    EaseOutQuad,
    EaseOutCubic,
    EaseOutExpo,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawSpring {
    damping_ratio: f64,
    stiffness: f64,
    epsilon: f64,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RawTransition {
    Named(String),
    Easing { duration: f64, curve: RawCurve },
    Spring { spring: RawSpring },
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawProps {
    rect: Option<RawRect>,
    opacity: Option<f64>,
    transform: Option<RawTransform>,
    clip: Option<bool>,
    corner_radius: Option<f64>,
    /// A colour or a gradient.
    #[serde(default, deserialize_with = "nullable")]
    background: Option<Option<serde_json::Value>>,
    #[serde(default, deserialize_with = "nullable")]
    border: Option<Option<RawBorder>>,
    text: Option<String>,
    font: Option<String>,
    color: Option<String>,
    image: Option<String>,
    placeholder: Option<String>,
    interactive: Option<bool>,
    transitions: Option<HashMap<String, RawTransition>>,
}

/// Tells a property that is `null` apart from one that's left out.
fn nullable<'de, T, D>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: Deserializer<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

/// Reads a colour, written `"#rrggbb"`, or a gradient, written as an object, for property `name`.
fn paint(name: &str, value: serde_json::Value) -> Result<Paint, String> {
    let named = |err: String| format!("`{name}`: {err}");
    match value {
        serde_json::Value::String(text) => Color::from_hex(&text).map(Paint::Color).map_err(named),
        serde_json::Value::Object(_) => {
            let gradient =
                LayoutEngineGradient::deserialize(value).map_err(|e| named(e.to_string()))?;
            resolve_gradient(&gradient)
                .map(Paint::Gradient)
                .map_err(named)
        }
        _ => Err(format!("`{name}` must be a colour or a gradient")),
    }
}

fn non_negative(name: &str, value: f64) -> Result<f64, String> {
    if value.is_finite() && value >= 0. {
        Ok(value)
    } else {
        Err(format!("`{name}` must be a number that isn't negative"))
    }
}

fn finite(name: &str, value: f64) -> Result<f64, String> {
    if value.is_finite() {
        Ok(value)
    } else {
        Err(format!("`{name}` must be a finite number"))
    }
}

impl RawTransition {
    fn resolve(self, property: &str) -> Result<Transition, String> {
        let kind = match self {
            RawTransition::Named(name) => {
                if named_animation(&Animations::default(), &name).is_none() {
                    return Err(format!(
                        "the transition of `{property}` names `{name}`, which isn't an animation"
                    ));
                }
                return Ok(Transition::Named(name));
            }
            RawTransition::Easing { duration, curve } => {
                let duration = non_negative("duration", duration)?;
                Kind::Easing(EasingParams {
                    duration_ms: duration.round().min(f64::from(u32::MAX)) as u32,
                    curve: match curve {
                        RawCurve::Linear => Curve::Linear,
                        RawCurve::EaseOutQuad => Curve::EaseOutQuad,
                        RawCurve::EaseOutCubic => Curve::EaseOutCubic,
                        RawCurve::EaseOutExpo => Curve::EaseOutExpo,
                    },
                })
            }
            RawTransition::Spring { spring } => {
                // A spring without damping never comes to rest, and one without stiffness or
                // epsilon never gets there.
                let positive = |name: &str, value: f64| {
                    if value.is_finite() && value > 0. {
                        Ok(value)
                    } else {
                        Err(format!("`{name}` must be a number above 0"))
                    }
                };
                Kind::Spring(SpringParams {
                    damping_ratio: positive("dampingRatio", spring.damping_ratio)?,
                    stiffness: positive("stiffness", spring.stiffness)?
                        .round()
                        .min(f64::from(u32::MAX)) as u32,
                    epsilon: positive("epsilon", spring.epsilon)?,
                })
            }
        };

        Ok(Transition::Config(niri_config::Animation {
            off: false,
            kind,
        }))
    }
}

impl Transitions {
    fn from_raw(raw: HashMap<String, RawTransition>) -> Result<Self, String> {
        let mut transitions = Transitions::default();
        for (property, transition) in raw {
            let transition = transition.resolve(&property)?;
            let slot = match property.as_str() {
                "rect" => &mut transitions.rect,
                "opacity" => &mut transitions.opacity,
                "transform" => &mut transitions.transform,
                "cornerRadius" => &mut transitions.corner_radius,
                "background" => &mut transitions.background,
                _ => return Err(format!("`{property}` can't have a transition")),
            };
            *slot = Some(transition);
        }
        Ok(transitions)
    }
}

impl TryFrom<RawProps> for Update {
    type Error = String;

    fn try_from(raw: RawProps) -> Result<Self, String> {
        let rect = raw
            .rect
            .map(|rect| {
                Ok::<_, String>(Rectangle::new(
                    Point::from((finite("rect.x", rect.x)?, finite("rect.y", rect.y)?)),
                    Size::from((
                        non_negative("rect.w", rect.w)?,
                        non_negative("rect.h", rect.h)?,
                    )),
                ))
            })
            .transpose()?;

        let transform = raw
            .transform
            .map(|transform| {
                Ok::<_, String>(NodeTransform {
                    scale: non_negative("transform.scale", transform.scale.unwrap_or(1.))?,
                    x: finite("transform.x", transform.x.unwrap_or(0.))?,
                    y: finite("transform.y", transform.y.unwrap_or(0.))?,
                })
            })
            .transpose()?;

        let background = raw
            .background
            .map(|value| value.map(|value| paint("background", value)).transpose())
            .transpose()?;

        let border = raw
            .border
            .map(|border| {
                border
                    .map(|border| {
                        Ok::<_, String>(Border {
                            width: non_negative("border.width", border.width)?,
                            color: paint("border.color", border.color)?,
                        })
                    })
                    .transpose()
            })
            .transpose()?;

        Ok(Update {
            rect,
            opacity: raw
                .opacity
                .map(|opacity| finite("opacity", opacity).map(|x| x.clamp(0., 1.)))
                .transpose()?,
            transform,
            clip: raw.clip,
            corner_radius: raw
                .corner_radius
                .map(|radius| non_negative("cornerRadius", radius))
                .transpose()?,
            background,
            border,
            text: raw.text,
            font: raw.font,
            color: raw
                .color
                .as_deref()
                .map(|color| Color::from_hex(color).map_err(|err| format!("`color`: {err}")))
                .transpose()?,
            image: raw.image,
            placeholder: raw.placeholder,
            interactive: raw.interactive,
            transitions: raw.transitions.map(Transitions::from_raw).transpose()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use niri_config::{GradientInterpolation, GradientRelativeTo};
    use serde_json::json;

    use super::*;

    fn parse(value: serde_json::Value) -> Result<Update, String> {
        Update::deserialize(&value).map_err(|err| err.to_string())
    }

    #[test]
    fn props_parse_into_typed_values() {
        let update = parse(json!({
            "rect": { "x": 1, "y": 2.5, "w": 30, "h": 40 },
            "opacity": 0.5,
            "transform": { "scale": 2 },
            "clip": true,
            "cornerRadius": 8,
            "background": "#000",
            "border": { "width": 2, "color": "#fff" },
            "interactive": true,
            "placeholder": "Search",
        }))
        .unwrap();

        assert_eq!(
            update.rect,
            Some(Rectangle::new(
                Point::from((1., 2.5)),
                Size::from((30., 40.))
            ))
        );
        assert_eq!(update.opacity, Some(0.5));
        assert_eq!(
            update.transform,
            Some(NodeTransform {
                scale: 2.,
                x: 0.,
                y: 0.
            })
        );
        assert_eq!(update.clip, Some(true));
        assert_eq!(update.corner_radius, Some(8.));
        assert_eq!(
            update.background,
            Some(Some(Paint::Color(Color::from_rgba8_unpremul(0, 0, 0, 255))))
        );
        let border = update.border.unwrap().unwrap();
        assert_eq!(border.width, 2.);
        assert_eq!(
            border.color,
            Paint::Color(Color::from_rgba8_unpremul(255, 255, 255, 255))
        );
        assert_eq!(update.interactive, Some(true));
        assert_eq!(update.placeholder.as_deref(), Some("Search"));
    }

    #[test]
    fn fills_and_borders_take_gradients() {
        let update = parse(json!({
            "background": { "from": "#80c8ff", "to": "#c7ff7f80", "angle": 45, "relativeTo": "view", "in": "oklch longer hue" },
            "border": { "width": 3, "color": { "from": "#000", "to": "#fff" } },
        }))
        .unwrap();

        let Some(Some(Paint::Gradient(background))) = update.background else {
            panic!("{:?}", update.background);
        };
        assert_eq!(
            background.from,
            Color::from_rgba8_unpremul(0x80, 0xc8, 0xff, 255)
        );
        assert_eq!(
            background.to,
            Color::from_rgba8_unpremul(0xc7, 0xff, 0x7f, 0x80)
        );
        assert_eq!(background.angle, 45);
        assert_eq!(background.relative_to, GradientRelativeTo::WorkspaceView);
        assert_eq!(
            background.in_,
            "oklch longer hue".parse::<GradientInterpolation>().unwrap()
        );

        // What's left out is niri's: top to bottom, over the node, in sRGB.
        let border = update.border.unwrap().unwrap();
        let Paint::Gradient(border) = border.color else {
            panic!("{border:?}");
        };
        assert_eq!(border.angle, 180);
        assert_eq!(border.relative_to, GradientRelativeTo::Window);
        assert_eq!(border.in_, GradientInterpolation::default());
    }

    #[test]
    fn a_paint_is_a_colour_or_a_gradient_and_a_colour_is_a_flat_gradient() {
        let red = Color::from_rgba8_unpremul(255, 0, 0, 255);
        let blue = Color::from_rgba8_unpremul(0, 0, 255, 255);
        let color = Paint::Color(red);
        let gradient = Paint::Gradient(Gradient {
            from: red,
            to: blue,
            ..Gradient::from(red)
        });

        assert_eq!(color.solid(), red);
        assert_eq!(gradient.solid(), red);
        assert_eq!(color.gradient().from, color.gradient().to);
        assert_eq!(gradient.gradient().to, blue);

        assert!(color.is_visible() && gradient.is_visible());
        assert!(!color.transparent().is_visible());
        assert!(!gradient.transparent().is_visible());
        // Only the ends are clear.
        assert_eq!(color.transparent().solid(), Color { a: 0., ..red });
        let half_clear = Paint::Gradient(Gradient {
            from: Color { a: 0., ..red },
            ..gradient.gradient()
        });
        assert!(half_clear.is_visible());
    }

    #[test]
    fn null_clears_a_nullable_prop_and_a_missing_one_leaves_it() {
        let update = parse(json!({ "background": null, "border": null })).unwrap();
        assert_eq!(update.background, Some(None));
        assert_eq!(update.border, Some(None));

        let update = parse(json!({})).unwrap();
        assert_eq!(update.background, None);
        assert_eq!(update.border, None);
    }

    #[test]
    fn opacity_is_clamped() {
        assert_eq!(parse(json!({ "opacity": 3 })).unwrap().opacity, Some(1.));
        assert_eq!(parse(json!({ "opacity": -1 })).unwrap().opacity, Some(0.));
    }

    #[test]
    fn invalid_props_are_errors() {
        for bad in [
            json!({ "bogus": 1 }),
            json!({ "opacity": "half" }),
            json!({ "rect": { "x": 0, "y": 0, "w": -1, "h": 1 } }),
            json!({ "rect": { "x": 0, "y": 0, "w": 1 } }),
            json!({ "transform": { "scale": -1 } }),
            json!({ "transform": { "rotate": 1 } }),
            json!({ "cornerRadius": -2 }),
            json!({ "background": "blue" }),
            json!({ "background": 5 }),
            json!({ "background": { "from": "#000" } }),
            json!({ "background": { "from": "#000", "to": "#fff", "angle": "up" } }),
            json!({ "background": { "from": "#000", "to": "#fff", "relativeTo": "window" } }),
            json!({ "background": { "from": "#000", "to": "#fff", "in": "hsl" } }),
            json!({ "background": { "from": "#000", "to": "red" } }),
            json!({ "background": { "from": "#000", "to": "#fff", "bogus": 1 } }),
            json!({ "border": { "width": -1, "color": "#fff" } }),
            json!({ "border": { "width": 1, "color": { "from": "#000" } } }),
            json!({ "color": "#12" }),
            // Text has a colour, not a gradient.
            json!({ "color": { "from": "#000", "to": "#fff" } }),
        ] {
            assert!(parse(bad.clone()).is_err(), "{bad} should be invalid");
        }
    }

    #[test]
    fn transitions_take_a_name_an_easing_or_a_spring() {
        let update = parse(json!({
            "transitions": {
                "rect": "window-movement",
                "opacity": { "duration": 150, "curve": "ease-out-quad" },
                "transform": { "spring": { "dampingRatio": 1, "stiffness": 800, "epsilon": 0.0001 } },
            }
        }))
        .unwrap();
        let transitions = update.transitions.unwrap();

        assert_eq!(
            transitions.rect,
            Some(Transition::Named("window-movement".into()))
        );
        assert_eq!(
            transitions.opacity,
            Some(Transition::Config(niri_config::Animation {
                off: false,
                kind: Kind::Easing(EasingParams {
                    duration_ms: 150,
                    curve: Curve::EaseOutQuad
                })
            }))
        );
        assert_eq!(
            transitions.transform,
            Some(Transition::Config(niri_config::Animation {
                off: false,
                kind: Kind::Spring(SpringParams {
                    damping_ratio: 1.,
                    stiffness: 800,
                    epsilon: 0.0001
                })
            }))
        );
        assert_eq!(transitions.corner_radius, None);
    }

    #[test]
    fn invalid_transitions_are_errors() {
        for bad in [
            json!({ "transitions": { "rect": "no-such-animation" } }),
            json!({ "transitions": { "color": "window-movement" } }),
            json!({ "transitions": { "rect": { "duration": -1, "curve": "linear" } } }),
            json!({ "transitions": { "rect": { "duration": 1, "curve": "bounce" } } }),
            json!({ "transitions": { "rect": { "spring": { "dampingRatio": 0, "stiffness": 1, "epsilon": 1 } } } }),
        ] {
            assert!(parse(bad.clone()).is_err(), "{bad} should be invalid");
        }
    }

    #[test]
    fn every_animation_of_the_config_has_a_name() {
        let animations = Animations::default();
        for name in [
            "workspace-switch",
            "window-open",
            "window-close",
            "horizontal-view-movement",
            "window-movement",
            "window-resize",
            "config-notification-open-close",
            "exit-confirmation-open-close",
            "screenshot-ui-open",
            "overview-open-close",
            "recent-windows-close",
        ] {
            assert!(named_animation(&animations, name).is_some(), "{name}");
        }
        assert!(named_animation(&animations, "window-teleport").is_none());
    }
}
