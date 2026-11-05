use std::collections::HashMap;

use niri_config::animations::{Curve, EasingParams, Kind, SpringParams};
use niri_config::{Animations, Color};
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

/// A border drawn inside a node's rect.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Border {
    pub width: f64,
    pub color: Color,
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
    pub background: Option<Option<Color>>,
    pub border: Option<Option<Border>>,
    pub text: Option<String>,
    pub font: Option<String>,
    pub color: Option<Color>,
    pub image: Option<String>,
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

/// Parses `#rgb`, `#rrggbb` and `#rrggbbaa`.
pub fn parse_color(text: &str) -> Result<Color, String> {
    let invalid = || format!("invalid colour `{text}`: expected #rgb, #rrggbb or #rrggbbaa");

    let digits = text.strip_prefix('#').ok_or_else(invalid)?;
    let digits = digits
        .chars()
        .map(|c| c.to_digit(16).map(|d| d as u8))
        .collect::<Option<Vec<_>>>()
        .ok_or_else(invalid)?;

    let channels: Vec<u8> = match digits.len() {
        3 => digits.iter().map(|d| d * 17).collect(),
        6 | 8 => digits
            .chunks(2)
            .map(|pair| pair[0] * 16 + pair[1])
            .collect(),
        _ => return Err(invalid()),
    };
    let alpha = channels.get(3).copied().unwrap_or(255);

    Ok(Color::from_rgba8_unpremul(
        channels[0],
        channels[1],
        channels[2],
        alpha,
    ))
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
    color: String,
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
    #[serde(default, deserialize_with = "nullable")]
    background: Option<Option<String>>,
    #[serde(default, deserialize_with = "nullable")]
    border: Option<Option<RawBorder>>,
    text: Option<String>,
    font: Option<String>,
    color: Option<String>,
    image: Option<String>,
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
            .map(|color| color.as_deref().map(parse_color).transpose())
            .transpose()?;

        let border = raw
            .border
            .map(|border| {
                border
                    .map(|border| {
                        Ok::<_, String>(Border {
                            width: non_negative("border.width", border.width)?,
                            color: parse_color(&border.color)?,
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
            color: raw.color.as_deref().map(parse_color).transpose()?,
            image: raw.image,
            interactive: raw.interactive,
            transitions: raw.transitions.map(Transitions::from_raw).transpose()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn parse(value: serde_json::Value) -> Result<Update, String> {
        Update::deserialize(&value).map_err(|err| err.to_string())
    }

    #[test]
    fn colors_parse_in_all_three_spellings() {
        assert_eq!(
            parse_color("#fa0").unwrap(),
            Color::from_rgba8_unpremul(255, 170, 0, 255)
        );
        assert_eq!(
            parse_color("#1a2b3c").unwrap(),
            Color::from_rgba8_unpremul(0x1a, 0x2b, 0x3c, 255)
        );
        assert_eq!(
            parse_color("#1A2B3C80").unwrap(),
            Color::from_rgba8_unpremul(0x1a, 0x2b, 0x3c, 0x80)
        );
    }

    #[test]
    fn colors_reject_everything_else() {
        for bad in [
            "", "#", "fff", "#ff", "#ffff", "#fffff", "#ggg", "#+12", "#ééé", "red",
        ] {
            assert!(parse_color(bad).is_err(), "{bad:?} should be invalid");
        }
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
            Some(Some(Color::from_rgba8_unpremul(0, 0, 0, 255)))
        );
        assert_eq!(update.border.unwrap().unwrap().width, 2.);
        assert_eq!(update.interactive, Some(true));
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
            json!({ "border": { "width": -1, "color": "#fff" } }),
            json!({ "color": "#12" }),
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
