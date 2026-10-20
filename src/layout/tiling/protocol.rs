//! JSON that bento and layouts exchange, see `resources/layouts/prelude.js`.

use serde::{Deserialize, Deserializer, Serialize};
use smithay::utils::{Logical, Point, Rectangle, Size};

/// Largest coordinate or size bento accepts from a layout, to keep arithmetic sane.
const MAX_COORD: f64 = 1e7;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub fn from_rect(rect: Rectangle<f64, Logical>) -> Self {
        Self {
            x: rect.loc.x,
            y: rect.loc.y,
            w: rect.size.w,
            h: rect.size.h,
        }
    }

    /// The rect as a rectangle, with coordinates kept in a sane range and a size of at least 1.
    pub fn to_rect(self) -> Rectangle<f64, Logical> {
        let coord = |v: f64| v.clamp(-MAX_COORD, MAX_COORD);
        let size = |v: f64| v.clamp(1., MAX_COORD);
        Rectangle::new(
            Point::from((coord(self.x), coord(self.y))),
            Size::from((size(self.w), size(self.h))),
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Xy {
    pub x: f64,
    pub y: f64,
}

impl Xy {
    pub fn from_point(point: Point<f64, Logical>) -> Self {
        Self {
            x: point.x,
            y: point.y,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Wh {
    pub w: f64,
    pub h: f64,
}

impl Wh {
    pub fn from_size(size: Size<f64, Logical>) -> Self {
        Self {
            w: size.w,
            h: size.h,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Normal,
    Fullscreen,
    Maximized,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Preset {
    Proportion(f64),
    Fixed(f64),
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Rules {
    pub default_width: Option<Preset>,
    pub default_height: Option<Preset>,
}

/// What bento tells the layout about a window.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowInfo {
    pub id: u64,
    pub app_id: Option<String>,
    pub title: Option<String>,
    pub size: Wh,
    pub min_size: Wh,
    pub max_size: Wh,
    pub borders: Wh,
    pub mode: Mode,
    pub rules: Rules,
    pub is_urgent: bool,
    /// Where the window is, as a hint when a layout takes over.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rect: Option<Rect>,
}

/// What a layout call changed.
#[derive(Debug, Deserialize)]
pub struct Output {
    #[serde(default)]
    pub result: serde_json::Value,
    pub error: Option<String>,
    #[serde(default)]
    pub logs: Vec<String>,
    #[serde(default)]
    pub windows: Vec<WindowChange>,
    /// Window ids, bottom to top.
    pub order: Option<Vec<u64>>,
    #[serde(default, deserialize_with = "present")]
    pub focused: Option<Option<u64>>,
    pub camera: Option<CameraChange>,
    #[serde(default, deserialize_with = "present")]
    pub hint: Option<Option<Rect>>,
    pub ipc: Option<Vec<(u64, Option<(usize, usize)>)>>,
    /// All tab bars, when they changed.
    pub tabs: Option<Vec<TabBarOut>>,
}

#[derive(Debug, Deserialize)]
pub struct TabBarOut {
    pub key: String,
    pub area: Rect,
    pub windows: Vec<u64>,
    pub active: u64,
}

#[derive(Debug, Deserialize)]
pub struct WindowChange {
    pub id: u64,
    pub rect: Rect,
    pub visible: bool,
    pub mode: Mode,
    pub animate: bool,
}

#[derive(Debug, Deserialize)]
pub struct CameraChange {
    pub x: f64,
    pub y: f64,
    pub animate: bool,
}

#[derive(Debug, Deserialize)]
pub struct TransferOut {
    pub windows: Vec<u64>,
    #[serde(default)]
    pub data: serde_json::Value,
}

/// Tells "present, but null" apart from "missing".
fn present<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}
