//! The game's UMG widget trees (`ui/<Widget>.json` from `f13-export ui`) and their layout: each widget's rectangle in
//! the designer's units (the HUD is a 1920×1080 canvas), as Slate arranges it.
//!
//! A user widget (a `..._C` type naming its class) keeps its own name, visibility and slot, with its class's tree as
//! its only child. Properties left at their class defaults aren't in the export, so the defaults below are UMG's:
//! a canvas slot's offsets are (0, 0, 100, 30), a border's content padding (4, 2), an overlay slot sits top-left,
//! box slots fill across and take their content's size along the box, an image's brush is 32×32 and a text's font
//! 24 points.
//!
//! Text sizes are estimated (Slate measures the shaped text): only autosized canvas children and boxes depend on
//! them, and the HUD's texts are left-aligned.

use serde_json::Value;
use std::path::Path;

#[derive(Clone, Debug)]
pub struct Widget {
    /// The widget's type: CanvasPanel, Image, TextBlock, ... or the user widget's class (`SCStamina_Widget_C`).
    pub kind: String,
    pub name: String,
    /// The user widget's class (its tree is the only child).
    pub class: Option<String>,
    pub props: Value,
    /// The slot's type (CanvasPanelSlot, HorizontalBoxSlot, ...) and properties; empty for a root.
    pub slot_kind: String,
    pub slot: Value,
    pub children: Vec<Widget>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// A widget arranged: its rectangle (absolute, in designer units) and its children's, in drawing order (ZOrder, then
/// the tree's order).
#[derive(Debug)]
pub struct Laid<'a> {
    pub widget: &'a Widget,
    pub rect: Rect,
    pub children: Vec<Laid<'a>>,
}

impl Laid<'_> {
    /// Scales the laid-out subtree by (sx, sy) about the origin, then moves it by (ox, oy).
    fn transform(&mut self, ox: f32, oy: f32, sx: f32, sy: f32) {
        let r = self.rect;
        self.rect = Rect { x: ox + r.x * sx, y: oy + r.y * sy, w: r.w * sx, h: r.h * sy };
        for c in &mut self.children {
            c.transform(ox, oy, sx, sy);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Vis {
    Visible,
    /// Drawn nowhere but still takes its space.
    Hidden,
    /// Takes no space.
    Collapsed,
}

/// Slate draws text at 96 DPI from point sizes.
pub const POINTS_TO_PX: f32 = 96.0 / 72.0;

fn f(v: &Value, default: f32) -> f32 {
    v.as_f64().map_or(default, |x| x as f32)
}

/// A margin ({Left, Top, Right, Bottom}), missing sides from `default`.
pub fn margin(v: &Value, default: [f32; 4]) -> [f32; 4] {
    [f(&v["Left"], default[0]), f(&v["Top"], default[1]), f(&v["Right"], default[2]), f(&v["Bottom"], default[3])]
}

/// A linear color ({R, G, B, A}, or a slate color's {SpecifiedColor}); `default` when absent.
pub fn color(v: &Value, default: [f32; 4]) -> [f32; 4] {
    let c = if v["SpecifiedColor"].is_object() { &v["SpecifiedColor"] } else { v };
    if !c.is_object() {
        return default;
    }
    [f(&c["R"], 0.0), f(&c["G"], 0.0), f(&c["B"], 0.0), f(&c["A"], 1.0)]
}

impl Widget {
    /// Loads `<dir>/ui/<name>.json` with its user widgets' trees.
    pub fn load(dir: &Path, name: &str) -> Option<Widget> {
        Self::load_depth(dir, name, 0)
    }

    fn load_depth(dir: &Path, name: &str, depth: u32) -> Option<Widget> {
        let text = std::fs::read_to_string(dir.join("ui").join(format!("{name}.json"))).ok()?;
        let v: Value = serde_json::from_str(&text).ok()?;
        Some(Self::from_json(dir, &v["Root"], depth))
    }

    fn from_json(dir: &Path, v: &Value, depth: u32) -> Widget {
        let mut w = Widget {
            kind: v["Type"].as_str().unwrap_or("").to_string(),
            name: v["Name"].as_str().unwrap_or("").to_string(),
            class: v["Class"].as_str().map(String::from),
            props: v["Props"].clone(),
            slot_kind: v["Slot"]["Type"].as_str().unwrap_or("").to_string(),
            slot: v["Slot"]["Props"].clone(),
            children: v["Children"].as_array().map_or(vec![], |c| c.iter().map(|c| Self::from_json(dir, c, depth)).collect()),
        };
        if let Some(class) = &w.class {
            if depth < 16 {
                w.children = Self::load_depth(dir, class, depth + 1).into_iter().collect();
            }
        }
        w
    }

    /// From parsed parts (tests).
    pub fn new(kind: &str, name: &str, props: Value, slot_kind: &str, slot: Value, children: Vec<Widget>) -> Widget {
        Widget { kind: kind.into(), name: name.into(), class: None, props, slot_kind: slot_kind.into(), slot, children }
    }

    pub fn visibility(&self) -> Vis {
        match self.props["Visibility"].as_str().unwrap_or("") {
            "ESlateVisibility::Hidden" => Vis::Hidden,
            "ESlateVisibility::Collapsed" => Vis::Collapsed,
            _ => Vis::Visible,
        }
    }

    /// The first widget named `name` (depth first).
    pub fn find(&self, name: &str) -> Option<&Widget> {
        if self.name == name {
            return Some(self);
        }
        self.children.iter().find_map(|c| c.find(name))
    }

    pub fn find_mut(&mut self, name: &str) -> Option<&mut Widget> {
        if self.name == name {
            return Some(self);
        }
        self.children.iter_mut().find_map(|c| c.find_mut(name))
    }

    /// Sets the widget's visibility (as the game's code does at runtime before the HUD is laid out).
    pub fn set_visibility(&mut self, vis: Vis) {
        let s = match vis {
            Vis::Visible => "ESlateVisibility::Visible",
            Vis::Hidden => "ESlateVisibility::Hidden",
            Vis::Collapsed => "ESlateVisibility::Collapsed",
        };
        if let Some(o) = self.props.as_object_mut() {
            o.insert("Visibility".into(), Value::String(s.into()));
        } else {
            self.props = serde_json::json!({ "Visibility": s });
        }
    }

    /// The text's string.
    pub fn text(&self) -> &str {
        self.props["Text"]["LocalizedString"].as_str().or(self.props["Text"]["SourceString"].as_str()).unwrap_or("")
    }

    /// The text's size in pixels.
    pub fn font_px(&self) -> f32 {
        f(&self.props["Font"]["Size"], 24.0) * POINTS_TO_PX
    }

    /// The size the widget asks for.
    pub fn desired(&self) -> (f32, f32) {
        if self.visibility() == Vis::Collapsed {
            return (0.0, 0.0);
        }
        let p = &self.props;
        match self.kind.as_str() {
            "Image" => (f(&p["Brush"]["ImageSize"]["X"], 32.0), f(&p["Brush"]["ImageSize"]["Y"], 32.0)),
            "TextBlock" => {
                let px = self.font_px();
                let lines = self.text().lines().count().max(1);
                let widest = self.text().lines().map(|l| l.chars().count()).max().unwrap_or(0);
                (widest as f32 * px * 0.5, lines as f32 * px * 1.2)
            }
            "Spacer" => (f(&p["Size"]["X"], 1.0), f(&p["Size"]["Y"], 1.0)),
            "ProgressBar" => {
                let s = &p["WidgetStyle"];
                let (bx, by) = (f(&s["BackgroundImage"]["ImageSize"]["X"], 32.0), f(&s["BackgroundImage"]["ImageSize"]["Y"], 32.0));
                let (fx, fy) = (f(&s["FillImage"]["ImageSize"]["X"], 32.0), f(&s["FillImage"]["ImageSize"]["Y"], 32.0));
                (bx.max(fx), by.max(fy))
            }
            "CanvasPanel" => {
                let (mut w, mut h) = (0.0f32, 0.0f32);
                for c in &self.children {
                    let l = &c.slot["LayoutData"];
                    let o = margin(&l["Offsets"], [0.0, 0.0, 100.0, 30.0]);
                    let (dw, dh) = if c.slot["bAutoSize"].as_bool().unwrap_or(false) { c.desired() } else { (o[2], o[3]) };
                    w = w.max(o[0] + dw);
                    h = h.max(o[1] + dh);
                }
                (w, h)
            }
            "HorizontalBox" | "VerticalBox" => {
                let horizontal = self.kind == "HorizontalBox";
                let (mut along, mut across) = (0.0f32, 0.0f32);
                for c in &self.children {
                    let pad = margin(&c.slot["Padding"], [0.0; 4]);
                    let (dw, dh) = c.desired();
                    let (w, h) = (dw + pad[0] + pad[2], dh + pad[1] + pad[3]);
                    if horizontal {
                        along += w;
                        across = across.max(h);
                    } else {
                        along += h;
                        across = across.max(w);
                    }
                }
                if horizontal { (along, across) } else { (across, along) }
            }
            "SizeBox" => {
                let (cw, ch) = self.children.first().map_or((0.0, 0.0), |c| c.desired());
                (f(&p["WidthOverride"], cw), f(&p["HeightOverride"], ch))
            }
            _ => {
                // Border, Overlay, ScaleBox, a user widget...: the largest child and its padding.
                let (mut w, mut h) = (0.0f32, 0.0f32);
                for c in &self.children {
                    let pad = margin(&c.slot["Padding"], self.default_padding());
                    let (dw, dh) = c.desired();
                    w = w.max(dw + pad[0] + pad[2]);
                    h = h.max(dh + pad[1] + pad[3]);
                }
                (w, h)
            }
        }
    }

    fn default_padding(&self) -> [f32; 4] {
        if self.kind == "Border" { [4.0, 2.0, 4.0, 2.0] } else { [0.0; 4] }
    }

    /// Arranges the widget in `rect` (and its children).
    pub fn layout(&self, rect: Rect) -> Laid<'_> {
        let mut children: Vec<(i64, Laid)> = vec![];
        match self.kind.as_str() {
            "CanvasPanel" => {
                for c in &self.children {
                    let r = canvas_child(c, rect);
                    children.push((c.slot["ZOrder"].as_i64().unwrap_or(0), c.layout(r)));
                }
            }
            "HorizontalBox" | "VerticalBox" => {
                let horizontal = self.kind == "HorizontalBox";
                let total = if horizontal { rect.w } else { rect.h };
                // Automatic slots take their size; fill slots share what's left by their values.
                let mut fixed = 0.0;
                let mut fill = 0.0;
                for c in &self.children {
                    let pad = margin(&c.slot["Padding"], [0.0; 4]);
                    let (dw, dh) = c.desired();
                    match fill_value(c) {
                        Some(v) => fill += v,
                        None => fixed += if horizontal { dw } else { dh },
                    }
                    fixed += if horizontal { pad[0] + pad[2] } else { pad[1] + pad[3] };
                }
                let spare = (total - fixed).max(0.0);
                let mut at = if horizontal { rect.x } else { rect.y };
                for c in &self.children {
                    let pad = margin(&c.slot["Padding"], [0.0; 4]);
                    let (dw, dh) = c.desired();
                    let along = match fill_value(c) {
                        Some(v) if fill > 0.0 => spare * v / fill,
                        _ => if horizontal { dw } else { dh },
                    };
                    let r = if horizontal {
                        let (y, h) = align(c.slot["VerticalAlignment"].as_str(), rect.y + pad[1], rect.h - pad[1] - pad[3], dh, "Fill");
                        Rect { x: at + pad[0], y, w: along, h }
                    } else {
                        let (x, w) = align(c.slot["HorizontalAlignment"].as_str(), rect.x + pad[0], rect.w - pad[0] - pad[2], dw, "Fill");
                        Rect { x, y: at + pad[1], w, h: along }
                    };
                    at += along + if horizontal { pad[0] + pad[2] } else { pad[1] + pad[3] };
                    children.push((0, c.layout(r)));
                }
            }
            "ScaleBox" if self.children.first().is_some_and(|c| c.desired().0 > 0.0 && c.desired().1 > 0.0) => {
                // SScaleBox: the child laid out at its desired size, scaled by Stretch (ScaleToFit unless set:
                // uniform, to fit; Fill: each axis to fill; ScaleToFill; ScaleToFitX / Y; None) within its
                // StretchDirection, and placed by its slot's alignment (centred; a uniform scale can't fill).
                let c = &self.children[0];
                let (dw, dh) = c.desired();
                let (fx, fy) = (rect.w / dw, rect.h / dh);
                let stretch = self.props["Stretch"].as_str().unwrap_or("EStretch::ScaleToFit").rsplit(':').next().unwrap_or("ScaleToFit");
                let (mut sx, mut sy) = match stretch {
                    "None" => (1.0, 1.0),
                    "Fill" => (fx, fy),
                    "ScaleToFill" => (fx.max(fy), fx.max(fy)),
                    "ScaleToFitX" => (fx, fx),
                    "ScaleToFitY" => (fy, fy),
                    "UserSpecified" => (f(&self.props["UserSpecifiedScale"], 1.0), f(&self.props["UserSpecifiedScale"], 1.0)),
                    _ => (fx.min(fy), fx.min(fy)),
                };
                match self.props["StretchDirection"].as_str().unwrap_or("").rsplit(':').next().unwrap_or("") {
                    "DownOnly" => (sx, sy) = (sx.min(1.0), sy.min(1.0)),
                    "UpOnly" => (sx, sy) = (sx.max(1.0), sy.max(1.0)),
                    _ => {}
                }
                let (w, h) = (dw * sx, dh * sy);
                let centre = |a: Option<&str>| match a.map(|a| a.rsplit('_').next().unwrap_or("")) {
                    Some("Left") | Some("Top") => 0.0,
                    Some("Right") | Some("Bottom") => 1.0,
                    _ => 0.5,
                };
                let ox = rect.x + (rect.w - w) * centre(c.slot["HorizontalAlignment"].as_str());
                let oy = rect.y + (rect.h - h) * centre(c.slot["VerticalAlignment"].as_str());
                let mut laid = c.layout(Rect { x: 0.0, y: 0.0, w: dw, h: dh });
                laid.transform(ox, oy, sx, sy);
                children.push((0, laid));
            }
            _ => {
                // One child (or an overlay's several) aligned in the padded rectangle.
                let default = if self.kind == "Overlay" { "Left" } else { "Fill" };
                for c in &self.children {
                    let pad = margin(&c.slot["Padding"], self.default_padding());
                    let (dw, dh) = c.desired();
                    let (x, w) = align(c.slot["HorizontalAlignment"].as_str(), rect.x + pad[0], rect.w - pad[0] - pad[2], dw, default);
                    let (y, h) = align(c.slot["VerticalAlignment"].as_str(), rect.y + pad[1], rect.h - pad[1] - pad[3], dh, default);
                    children.push((0, c.layout(Rect { x, y, w, h })));
                }
            }
        }
        children.sort_by_key(|(z, _)| *z);
        Laid { widget: self, rect, children: children.into_iter().map(|(_, l)| l).collect() }
    }
}

/// A box slot's fill value (None for an automatic slot).
fn fill_value(c: &Widget) -> Option<f32> {
    let size = &c.slot["Size"];
    match size["SizeRule"].as_str() {
        Some(r) if r.ends_with("Fill") => Some(f(&size["Value"], 1.0)),
        _ => None,
    }
}

/// Places `size` in the span [start, start + len) by a slot alignment (HAlign_Left / Center / Right / Fill, or the
/// VAlign_ ones; `default` when unset): the start and length.
fn align(a: Option<&str>, start: f32, len: f32, size: f32, default: &str) -> (f32, f32) {
    let a = a.map_or(default, |a| a.rsplit('_').next().unwrap_or(default));
    match a {
        "Left" | "Top" => (start, size),
        "Center" => (start + (len - size) / 2.0, size),
        "Right" | "Bottom" => (start + len - size, size),
        _ => (start, len),
    }
}

/// A canvas child's rectangle: anchors on the parent, offsets (position and size when the anchors meet, margins when
/// they stretch), alignment (the pivot), autosize (the desired size).
fn canvas_child(c: &Widget, parent: Rect) -> Rect {
    let l = &c.slot["LayoutData"];
    let o = margin(&l["Offsets"], [0.0, 0.0, 100.0, 30.0]);
    let (min_x, min_y) = (f(&l["Anchors"]["Minimum"]["X"], 0.0), f(&l["Anchors"]["Minimum"]["Y"], 0.0));
    let (max_x, max_y) = (f(&l["Anchors"]["Maximum"]["X"], 0.0), f(&l["Anchors"]["Maximum"]["Y"], 0.0));
    let (al_x, al_y) = (f(&l["Alignment"]["X"], 0.0), f(&l["Alignment"]["Y"], 0.0));
    let auto = c.slot["bAutoSize"].as_bool().unwrap_or(false);
    let (dw, dh) = if auto { c.desired() } else { (o[2], o[3]) };
    let axis = |start: f32, len: f32, amin: f32, amax: f32, near: f32, far: f32, desired: f32, alignment: f32| {
        if amin == amax {
            let size = desired;
            (start + amin * len + near - alignment * size, size)
        } else {
            let a = start + amin * len + near;
            let b = start + amax * len - far;
            (a, b - a)
        }
    };
    let (x, w) = axis(parent.x, parent.w, min_x, max_x, o[0], o[2], dw, al_x);
    let (y, h) = axis(parent.y, parent.h, min_y, max_y, o[1], o[3], dh, al_y);
    Rect { x, y, w, h }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn image(name: &str, slot: Value) -> Widget {
        Widget::new("Image", name, json!({"Brush": {"ImageSize": {"X": 50.0, "Y": 20.0}}}), "CanvasPanelSlot", slot, vec![])
    }

    #[test]
    fn canvas_anchors_offsets_and_alignment() {
        let canvas = Widget::new(
            "CanvasPanel",
            "root",
            json!({}),
            "",
            json!({}),
            vec![
                // Bottom-right anchor, pivot at its bottom middle (the stamina ring).
                image("ring", json!({"LayoutData": {"Offsets": {"Left": -208.0, "Top": -60.0, "Right": 280.0, "Bottom": 140.0},
                    "Anchors": {"Minimum": {"X": 1.0, "Y": 1.0}, "Maximum": {"X": 1.0, "Y": 1.0}}, "Alignment": {"X": 0.5, "Y": 1.0}}})),
                // Stretched over the whole canvas.
                image("fill", json!({"LayoutData": {"Offsets": {"Right": 0.0, "Bottom": 0.0}, "Anchors": {"Maximum": {"X": 1.0, "Y": 1.0}}}})),
                // Default offsets (0, 0, 100, 30), autosized to the brush.
                image("auto", json!({"bAutoSize": true, "ZOrder": -1})),
                image("plain", json!({})),
            ],
        );
        let laid = canvas.layout(Rect { x: 0.0, y: 0.0, w: 1920.0, h: 1080.0 });
        let r: Vec<(&str, Rect)> = laid.children.iter().map(|l| (l.widget.name.as_str(), l.rect)).collect();
        // ZOrder -1 first.
        assert_eq!(r[0], ("auto", Rect { x: 0.0, y: 0.0, w: 50.0, h: 20.0 }));
        assert_eq!(r[1], ("ring", Rect { x: 1920.0 - 208.0 - 140.0, y: 1080.0 - 60.0 - 140.0, w: 280.0, h: 140.0 }));
        assert_eq!(r[2], ("fill", Rect { x: 0.0, y: 0.0, w: 1920.0, h: 1080.0 }));
        assert_eq!(r[3], ("plain", Rect { x: 0.0, y: 0.0, w: 100.0, h: 30.0 }));
        // The ring reaches 80 down from its offset (-60 + 140).
        assert_eq!(canvas.desired(), (100.0, 80.0));
    }

    #[test]
    fn boxes_and_borders() {
        let hbox = Widget::new(
            "HorizontalBox",
            "row",
            json!({}),
            "",
            json!({}),
            vec![
                Widget::new("Image", "a", json!({"Brush": {"ImageSize": {"X": 50.0, "Y": 20.0}}}), "HorizontalBoxSlot", json!({"Padding": {"Right": 5.0}}), vec![]),
                Widget::new("Image", "b", json!({"Visibility": "ESlateVisibility::Collapsed"}), "HorizontalBoxSlot", json!({}), vec![]),
                Widget::new("Image", "c", json!({}), "HorizontalBoxSlot", json!({"Size": {"SizeRule": "ESlateSizeRule::Fill"}, "VerticalAlignment": "VAlign_Center"}), vec![]),
            ],
        );
        let laid = hbox.layout(Rect { x: 10.0, y: 0.0, w: 200.0, h: 40.0 });
        assert_eq!(laid.children[0].rect, Rect { x: 10.0, y: 0.0, w: 50.0, h: 40.0 });
        assert_eq!(laid.children[1].rect.w, 0.0);
        assert_eq!(laid.children[2].rect, Rect { x: 65.0, y: 4.0, w: 145.0, h: 32.0 });
        assert_eq!(hbox.desired(), (50.0 + 5.0 + 32.0, 32.0));

        // A border's content is padded (4, 2) unless the slot says otherwise.
        let border = Widget::new("Border", "b", json!({}), "", json!({}), vec![Widget::new("Image", "i", json!({}), "BorderSlot", json!({}), vec![])]);
        assert_eq!(border.desired(), (40.0, 36.0));
        assert_eq!(border.layout(Rect { x: 0.0, y: 0.0, w: 100.0, h: 50.0 }).children[0].rect, Rect { x: 4.0, y: 2.0, w: 92.0, h: 46.0 });
    }

    #[test]
    fn colors_and_margins() {
        assert_eq!(color(&json!({"SpecifiedColor": {"R": 1.0, "G": 0.5, "B": 0.0, "A": 0.25}}), [1.0; 4]), [1.0, 0.5, 0.0, 0.25]);
        assert_eq!(color(&json!({"R": 0.5, "G": 0.5, "B": 0.5}), [0.0; 4]), [0.5, 0.5, 0.5, 1.0]);
        assert_eq!(color(&Value::Null, [1.0; 4]), [1.0; 4]);
        assert_eq!(margin(&json!({"Bottom": 8.0}), [4.0, 2.0, 4.0, 2.0]), [4.0, 2.0, 4.0, 8.0]);
    }
}
