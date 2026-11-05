use niri_ipc::SizeChange;

use crate::appearance::{Border, FocusRing, Shadow, DEFAULT_BACKGROUND_COLOR};
use crate::utils::{Flag, MergeWith};
use crate::{BorderRule, Color, FloatOrInt, ShadowRule};

#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    pub focus_ring: FocusRing,
    pub border: Border,
    pub shadow: Shadow,
    pub empty_workspace_above_first: bool,
    pub gaps: f64,
    pub struts: Struts,
    pub background_color: Color,
    pub fullscreen_backdrop_color: Color,
    /// Image drawn behind each workspace in place of the background color, as written in the
    /// config. Paths starting with `~` are expanded when the image loads.
    pub background_image: Option<String>,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            focus_ring: FocusRing::default(),
            border: Border::default(),
            shadow: Shadow::default(),
            empty_workspace_above_first: false,
            gaps: 16.,
            struts: Struts::default(),
            background_color: DEFAULT_BACKGROUND_COLOR,
            fullscreen_backdrop_color: Color::from_array_unpremul([0., 0., 0., 1.]),
            background_image: None,
        }
    }
}

impl MergeWith<LayoutPart> for Layout {
    fn merge_with(&mut self, part: &LayoutPart) {
        merge!(
            (self, part),
            focus_ring,
            border,
            shadow,
            empty_workspace_above_first,
            gaps,
        );

        merge_clone!(
            (self, part),
            struts,
            background_color,
            fullscreen_backdrop_color,
        );

        if let Some(path) = &part.background_image {
            self.background_image = Some(path.clone());
        }
    }
}

#[derive(knuffel::Decode, Debug, Default, Clone, PartialEq)]
pub struct LayoutPart {
    #[knuffel(child)]
    pub focus_ring: Option<BorderRule>,
    #[knuffel(child)]
    pub border: Option<BorderRule>,
    #[knuffel(child)]
    pub shadow: Option<ShadowRule>,
    #[knuffel(child)]
    pub empty_workspace_above_first: Option<Flag>,
    #[knuffel(child, unwrap(argument))]
    pub gaps: Option<FloatOrInt<0, 65535>>,
    #[knuffel(child)]
    pub struts: Option<Struts>,
    #[knuffel(child)]
    pub background_color: Option<Color>,
    #[knuffel(child)]
    pub fullscreen_backdrop_color: Option<Color>,
    #[knuffel(child, unwrap(argument))]
    pub background_image: Option<String>,
}

/// A size to give a window: a proportion of the working area, or logical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PresetSize {
    Proportion(f64),
    Fixed(i32),
}

impl From<PresetSize> for SizeChange {
    fn from(value: PresetSize) -> Self {
        match value {
            PresetSize::Proportion(prop) => SizeChange::SetProportion(prop * 100.),
            PresetSize::Fixed(fixed) => SizeChange::SetFixed(fixed),
        }
    }
}

#[derive(knuffel::Decode, Debug, Default, Clone, Copy, PartialEq)]
pub struct Struts {
    #[knuffel(child, unwrap(argument), default)]
    pub left: FloatOrInt<-65535, 65535>,
    #[knuffel(child, unwrap(argument), default)]
    pub right: FloatOrInt<-65535, 65535>,
    #[knuffel(child, unwrap(argument), default)]
    pub top: FloatOrInt<-65535, 65535>,
    #[knuffel(child, unwrap(argument), default)]
    pub bottom: FloatOrInt<-65535, 65535>,
}
