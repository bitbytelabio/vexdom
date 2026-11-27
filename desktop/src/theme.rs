//! The appliance face (`.impeccable/surfaces/desktop-src-ui-rs.md`), in two lights that follow
//! macOS's appearance. Light: warm grey enamel with raised off-white faces. Dark: JetBrains
//! Islands' layout of the same idea, rounded islands floating on a near-black canvas, lit by a
//! hairline edge instead of a shadow. Both keep a lamp that is green when a browser is connected,
//! amber while waiting and unlit when off. One grotesk, Archivo, bundled so the window looks the
//! same on every Mac.

use std::borrow::Cow;

use gpui::{
    App, BoxShadow, Div, ElementId, Font, FontFeatures, FontWeight, Global, Hsla, SharedString,
    Stateful, Window, WindowAppearance, div, point, prelude::*, px, rgb, rgba,
};

use crate::settings::Appearance;

pub const FAMILY: &str = "Archivo";

/// The log and the command line: the system's monospace face.
pub const MONO: &str = "Menlo";

/// Registers the bundled faces (SIL OFL 1.1, `desktop/fonts/OFL.txt`).
pub fn load_fonts(cx: &App) {
    let faces: Vec<Cow<'static, [u8]>> = vec![
        Cow::Borrowed(include_bytes!("../fonts/Archivo-Regular.ttf")),
        Cow::Borrowed(include_bytes!("../fonts/Archivo-Medium.ttf")),
        Cow::Borrowed(include_bytes!("../fonts/Archivo-Bold.ttf")),
    ];
    if let Err(error) = cx.text_system().add_fonts(faces) {
        eprintln!(
            "vexdom-desktop: the bundled fonts did not load ({error}); using the system font"
        );
    }
}

/// Every colour the window paints, for one appearance.
#[derive(Clone, Copy)]
pub struct Palette {
    /// The window's ground: enamel, or the islands' canvas.
    pub ground: Hsla,
    /// A raised face or island.
    pub face: Hsla,
    /// The hairline round a face; transparent in the light, where the shadow lifts it.
    pub edge: Hsla,
    /// Recessed parts: fields, the switch's well, secondary buttons, list hairlines.
    pub well: Hsla,
    pub well_hover: Hsla,
    /// Text, and the primary (non-power) button.
    pub ink: Hsla,
    pub ink_hover: Hsla,
    /// Labels and secondary text.
    pub label: Hsla,
    pub lamp_on: Hsla,
    pub lamp_waiting: Hsla,
    pub lamp_off: Hsla,
    /// What went wrong, in text.
    pub fault: Hsla,
    pub selection: Hsla,
    /// `None` in the dark: islands are lit by their edge, not lifted by a shadow.
    shadow: Option<Hsla>,
}

impl Global for Palette {}

impl Palette {
    pub fn light() -> Self {
        Self {
            ground: rgb(0xe8e6e1).into(),
            face: rgb(0xf6f5f2).into(),
            edge: rgba(0x0000_0000).into(),
            well: rgb(0xe8e6e1).into(),
            well_hover: rgb(0xdedbd5).into(),
            ink: rgb(0x2f2f2d).into(),
            ink_hover: rgb(0x484845).into(),
            // 6.1:1 on `face`, 5.4:1 on `ground` and `well`, 4.8:1 on `well_hover` (the tab track).
            label: rgb(0x5e5c56).into(),
            lamp_on: rgb(0x2fa84f).into(),
            lamp_waiting: rgb(0xd98a1a).into(),
            lamp_off: rgb(0xc6c3bc).into(),
            fault: rgb(0xb3361b).into(),
            selection: rgba(0x2f2f2d2e).into(),
            shadow: Some(rgba(0x0000_0014).into()),
        }
    }

    pub fn dark() -> Self {
        Self {
            ground: rgb(0x131417).into(),
            face: rgb(0x1e1f22).into(),
            edge: rgb(0x2b2d30).into(),
            well: rgb(0x2b2d30).into(),
            well_hover: rgb(0x393b40).into(),
            ink: rgb(0xdfe1e5).into(),
            ink_hover: rgb(0xc9ccd2).into(),
            // 7.2:1 on `face`, 6.1:1 on `well`, 4.9:1 on `well_hover` (the tab track).
            label: rgb(0xa9acb2).into(),
            lamp_on: rgb(0x3fbf62).into(),
            lamp_waiting: rgb(0xe39a2e).into(),
            lamp_off: rgb(0x4a4c51).into(),
            fault: rgb(0xf0785a).into(),
            selection: rgba(0xdfe1e53d).into(),
            shadow: None,
        }
    }

    /// Port, pid and versions: the same face with tabular figures, so numbers line up.
    pub fn figures(&self) -> Font {
        Font {
            family: FAMILY.into(),
            features: FontFeatures(std::sync::Arc::new(vec![("tnum".into(), 1)])),
            fallbacks: None,
            weight: FontWeight::NORMAL,
            style: gpui::FontStyle::Normal,
        }
    }

    /// The soft offset shadow under a face; none in the dark.
    pub fn lift(&self) -> Vec<BoxShadow> {
        let Some(shadow) = self.shadow else {
            return Vec::new();
        };
        vec![
            BoxShadow {
                color: shadow,
                offset: point(px(0.), px(1.)),
                blur_radius: px(2.),
                spread_radius: px(0.),
            },
            BoxShadow {
                color: shadow.opacity(0.75),
                offset: point(px(0.), px(6.)),
                blur_radius: px(16.),
                spread_radius: px(-4.),
            },
        ]
    }

    /// A raised face (an island in the dark): the container every work area sits on.
    pub fn panel(&self) -> Div {
        div()
            .flex()
            .flex_col()
            .rounded(px(14.))
            .bg(self.face)
            .border_1()
            .border_color(self.edge)
            .shadow(self.lift())
    }

    pub fn button(
        &self,
        id: impl Into<ElementId>,
        label: impl Into<SharedString>,
        tone: Tone,
    ) -> Stateful<Div> {
        let base = div()
            .id(id.into())
            .px(px(14.))
            .py(px(7.))
            .rounded(px(7.))
            .cursor_pointer()
            .text_size(px(13.))
            .font_weight(FontWeight::MEDIUM)
            .child(label.into());
        match tone {
            Tone::Primary => {
                let hover = self.ink_hover;
                base.bg(self.ink)
                    .text_color(self.face)
                    .hover(move |style| style.bg(hover))
            }
            Tone::Secondary => {
                let hover = self.well_hover;
                base.bg(self.well)
                    .text_color(self.ink)
                    .hover(move |style| style.bg(hover))
            }
        }
    }
}

/// The appearance picked in Settings → General (`settings.rs`).
#[derive(Clone, Copy)]
pub struct Chosen(pub Appearance);

impl Global for Chosen {}

/// Sets the palette for `window`: what Settings picked, else macOS's appearance. The caller
/// refreshes the window.
pub fn paint(window: &Window, cx: &mut App) {
    let chosen = cx
        .try_global::<Chosen>()
        .map_or(Appearance::System, |chosen| chosen.0);
    let system_dark = matches!(
        window.appearance(),
        WindowAppearance::Dark | WindowAppearance::VibrantDark
    );
    cx.set_global(if chosen.is_dark(system_dark) {
        Palette::dark()
    } else {
        Palette::light()
    });
}

/// The palette for the window's current appearance.
pub fn palette(cx: &App) -> Palette {
    cx.try_global::<Palette>()
        .copied()
        .unwrap_or_else(Palette::light)
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    /// The one action a face asks for.
    Primary,
    /// Recessed into the face.
    Secondary,
}
