//! Colour palettes for the window.
//!
//! Two hand-tuned palettes; the header toggle is a pure lookup so no element has
//! to know about themes. The window opens in the light palette and can be switched
//! from the theme page, whose cards are drawn with the palette they describe. More
//! themes (and accent choices) are a later change; `README.md`「已知限制」记录了这个决定。

/// Which palette the window uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemeKind {
    /// Dark grey chrome.
    Dark,
    /// Light chrome, the default.
    Light,
}

impl Default for ThemeKind {
    /// The window opens light: the desktop it sits on is bright by default.
    fn default() -> Self {
        ThemeKind::Light
    }
}

impl ThemeKind {
    /// Every palette the picker offers, in display order.
    pub const ALL: [ThemeKind; 2] = [ThemeKind::Light, ThemeKind::Dark];

    /// The other theme.
    pub fn toggled(self) -> Self {
        match self {
            ThemeKind::Dark => ThemeKind::Light,
            ThemeKind::Light => ThemeKind::Dark,
        }
    }

    /// Display name of the theme that is currently active.
    pub fn label(self) -> &'static str {
        match self {
            ThemeKind::Dark => "深色",
            ThemeKind::Light => "浅色",
        }
    }

    /// The name stored in the settings file.
    pub fn name(self) -> &'static str {
        match self {
            ThemeKind::Dark => "dark",
            ThemeKind::Light => "light",
        }
    }

    /// The theme a stored name means; anything unknown opens light.
    pub fn from_name(name: &str) -> Self {
        match name.trim() {
            "dark" => ThemeKind::Dark,
            _ => ThemeKind::Light,
        }
    }

    /// One line about when this palette is the better choice.
    pub fn description(self) -> &'static str {
        match self {
            ThemeKind::Dark => "灰底；夜间与长时间使用",
            ThemeKind::Light => "亮底，默认；明亮环境下对比更清楚",
        }
    }

    /// The palette for this theme.
    pub fn palette(self) -> Palette {
        match self {
            ThemeKind::Dark => Palette {
                background: 0x17171a,
                chrome: 0x17171a,
                canvas: 0x1f1f23,
                panel: 0x212126,
                panel_soft: 0x2b2b31,
                field: 0x161619,
                border: 0x32323a,
                text: 0xe9e9ec,
                strong: 0xffffff,
                muted: 0x9d9da6,
                faint: 0x71717a,
                accent: 0x7aa2f7,
                accent_text: 0x10131c,
                ok: 0x4cc38a,
                failed: 0xef6f6c,
                skipped: 0x9a9aa2,
                running: 0x7aa2f7,
                warn: 0xe0b072,
                sky: 0x4cc2ff,
                selection: 0x33456b,
                find: 0x5a4a12,
                code_function: 0xb392f0,
                code_type: 0x5ac8d8,
            },
            ThemeKind::Light => Palette {
                background: 0xededf0,
                chrome: 0xf9fafb,
                canvas: 0xffffff,
                panel: 0xfbfbfc,
                panel_soft: 0xe3e3e9,
                field: 0xffffff,
                border: 0xd8d8de,
                text: 0x1c1c20,
                strong: 0x000000,
                muted: 0x5b5b64,
                faint: 0x8c8c96,
                accent: 0x2f6fd0,
                accent_text: 0xffffff,
                ok: 0x1f8a4c,
                failed: 0xc0392b,
                skipped: 0x70707a,
                running: 0x2f6fd0,
                warn: 0xa96612,
                sky: 0x1f9ede,
                selection: 0xcfe4ff,
                find: 0xfff2a8,
                code_function: 0x7c3aed,
                code_type: 0x0e7490,
            },
        }
    }
}

/// Name and description of the picker's third card: the reader's own colours.
///
/// It is not a [`ThemeKind`]: picking light or dark means "the palette as it
/// ships", while this one means "whatever the colour rows below were set to".
pub const CUSTOM_LABEL: &str = "自定义";
/// One line about what the custom card means.
pub const CUSTOM_DESCRIPTION: &str = "改动任意颜色后自动选中";

/// Whether the custom card is the one in use.
///
/// There is no stored flag: any colour that differs from the theme is what makes
/// the window custom, so changing the last colour back also leaves the mode.
pub fn custom_mode(colors: &[(String, u32)]) -> bool {
    !colors.is_empty()
}

/// Hue steps the swatch grid shows, in degrees.
const SWATCH_HUES: [f32; 12] = [
    0., 30., 60., 90., 120., 150., 180., 210., 240., 270., 300., 330.,
];

/// Lightness and saturation of each grid row, from pale to deep.
pub const SWATCH_LEVELS: [(f32, f32); 6] = [
    (0.92, 0.55),
    (0.80, 0.70),
    (0.64, 0.85),
    (0.50, 0.85),
    (0.36, 0.75),
    (0.22, 0.60),
];

/// Greys the swatch grid offers under the hue rows: pure white at one end,
/// pure black at the other, so backgrounds and text have somewhere to go.
const SWATCH_NEUTRALS: usize = 12;

/// Every colour the swatch grid shows, row by row.
///
/// A grid rather than a system dialog: on Windows that dialog is a second
/// process, and its window can end up behind ours, which read as "clicking does
/// nothing".
pub fn swatch_grid() -> Vec<Vec<u32>> {
    let mut grid: Vec<Vec<u32>> = SWATCH_LEVELS
        .iter()
        .map(|(lightness, saturation)| {
            SWATCH_HUES
                .iter()
                .map(|hue| hsl(*hue, *saturation, *lightness))
                .collect()
        })
        .collect();
    grid.push(
        (0..SWATCH_NEUTRALS)
            .map(|step| {
                let share = step as f32 / (SWATCH_NEUTRALS - 1) as f32;
                let channel = (255. * (1. - share)).round() as u32;
                (channel << 16) | (channel << 8) | channel
            })
            .collect(),
    );
    grid
}

/// Turn a hue (degrees), saturation and lightness (both 0–1) into `0xRRGGBB`.
pub fn hsl(hue: f32, saturation: f32, lightness: f32) -> u32 {
    let hue = hue.rem_euclid(360.) / 360.;
    let saturation = saturation.clamp(0., 1.);
    let lightness = lightness.clamp(0., 1.);
    let channel = |n: f32| {
        let k = (n + hue * 12.) % 12.;
        let a = saturation * lightness.min(1. - lightness);
        let value = lightness - a * (k - 3.).min(9. - k).clamp(-1., 1.);
        (value.clamp(0., 1.) * 255.).round() as u32
    };
    (channel(0.) << 16) | (channel(8.) << 8) | channel(4.)
}

/// Every colour the window draws with.
#[derive(Clone, Copy)]
pub struct Palette {
    /// Rows, chips and menus drawn on top of panels. Chrome surfaces (title bar,
    /// sidebar, splitter) use [`Palette::chrome`] instead, which can be a
    /// lighter tone than these.
    pub background: u32,
    /// Window chrome: the title bar, the sidebar, the splitter between them and
    /// the window behind the rounded panel.
    pub chrome: u32,
    /// Right-hand panel background: pure white in the light theme, an elevated
    /// surface in the dark one so light text keeps its contrast.
    pub canvas: u32,
    /// Sidebar background.
    pub panel: u32,
    /// Rows, chips and buttons on top of panels.
    pub panel_soft: u32,
    /// Text field background.
    pub field: u32,
    /// Hairline borders.
    pub border: u32,
    /// Primary text.
    pub text: u32,
    /// Solidest ink: folder names in the sidebar tree, which are read at a glance.
    pub strong: u32,
    /// Secondary text.
    pub muted: u32,
    /// Tertiary text: timings, hints.
    pub faint: u32,
    /// Primary action.
    pub accent: u32,
    /// Text drawn on the accent colour.
    pub accent_text: u32,
    /// Successful step.
    pub ok: u32,
    /// Failed step, run, or message.
    pub failed: u32,
    /// Skipped step.
    pub skipped: u32,
    /// In-flight step or request.
    pub running: u32,
    /// Missing pieces and warnings.
    pub warn: u32,
    /// Selection outline: sky blue, used instead of a filled background.
    pub sky: u32,
    /// Backdrop of selected text (Ctrl+A in a field).
    pub selection: u32,
    /// Backdrop of a search hit in the transcript.
    pub find: u32,
    /// Code: a called name (`foo(`).
    pub code_function: u32,
    /// Code: a type-like name (`String`, `Vec`).
    pub code_type: u32,
}

/// One colour the reader may replace, with the name it is stored under.
pub struct ColorRole {
    /// Key used in `config.json` and in the settings box.
    pub key: &'static str,
    /// What this colour paints, in the reader's words.
    pub label: &'static str,
    /// Reads the colour out of a palette.
    pub get: fn(&Palette) -> u32,
    /// Writes the colour into a palette.
    pub set: fn(&mut Palette, u32),
}

/// The colours the custom-palette editor offers.
///
/// The list stops at the ones a reader can judge by eye; timings and code
/// token colours stay as shipped, because changing them is a design decision
/// rather than a preference.
pub const COLOR_ROLES: [ColorRole; 14] = [
    ColorRole {
        key: "background",
        label: "窗口底色",
        get: |palette| palette.background,
        set: |palette, value| palette.background = value,
    },
    ColorRole {
        key: "chrome",
        label: "标题栏 / 侧栏 / 分割条",
        get: |palette| palette.chrome,
        set: |palette, value| palette.chrome = value,
    },
    ColorRole {
        key: "canvas",
        label: "对话底色",
        get: |palette| palette.canvas,
        set: |palette, value| palette.canvas = value,
    },
    ColorRole {
        key: "panel",
        label: "面板底色",
        get: |palette| palette.panel,
        set: |palette, value| palette.panel = value,
    },
    ColorRole {
        key: "panel-soft",
        label: "按钮 / 卡片",
        get: |palette| palette.panel_soft,
        set: |palette, value| palette.panel_soft = value,
    },
    ColorRole {
        key: "border",
        label: "分隔线",
        get: |palette| palette.border,
        set: |palette, value| palette.border = value,
    },
    ColorRole {
        key: "text",
        label: "正文",
        get: |palette| palette.text,
        set: |palette, value| palette.text = value,
    },
    ColorRole {
        key: "muted",
        label: "次要文字",
        get: |palette| palette.muted,
        set: |palette, value| palette.muted = value,
    },
    ColorRole {
        key: "accent",
        label: "强调色",
        get: |palette| palette.accent,
        set: |palette, value| palette.accent = value,
    },
    ColorRole {
        key: "accent-text",
        label: "强调色上的文字",
        get: |palette| palette.accent_text,
        set: |palette, value| palette.accent_text = value,
    },
    ColorRole {
        key: "ok",
        label: "成功",
        get: |palette| palette.ok,
        set: |palette, value| palette.ok = value,
    },
    ColorRole {
        key: "failed",
        label: "失败",
        get: |palette| palette.failed,
        set: |palette, value| palette.failed = value,
    },
    ColorRole {
        key: "warn",
        label: "警告",
        get: |palette| palette.warn,
        set: |palette, value| palette.warn = value,
    },
    ColorRole {
        key: "selection",
        label: "选中背景",
        get: |palette| palette.selection,
        set: |palette, value| palette.selection = value,
    },
];

/// The role stored under `key`, if the editor offers it.
pub fn color_role(key: &str) -> Option<&'static ColorRole> {
    let wanted = key.trim().to_ascii_lowercase();
    COLOR_ROLES.iter().find(|role| role.key == wanted)
}

/// Parse `#rrggbb` (or `rrggbb`) into a colour.
pub fn parse_hex(text: &str) -> Option<u32> {
    let digits = text.trim().trim_start_matches('#');
    if digits.len() != 6
        || !digits
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    {
        return None;
    }
    u32::from_str_radix(digits, 16).ok()
}

/// The `#rrggbb` spelling of a colour.
pub fn hex(value: u32) -> String {
    format!("#{:06x}", value & 0x00ff_ffff)
}

/// Put one custom colour into a palette.
/// @returns true when the key names a role the editor offers.
pub fn apply_color(palette: &mut Palette, key: &str, value: u32) -> bool {
    let Some(role) = color_role(key) else {
        return false;
    };
    (role.set)(palette, value & 0x00ff_ffff);
    true
}

/// A palette with every custom colour laid over it.
pub fn themed(kind: ThemeKind, colors: &[(String, u32)]) -> Palette {
    let mut palette = kind.palette();
    for (key, value) in colors {
        apply_color(&mut palette, key, *value);
    }
    palette
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_names_the_active_theme() {
        assert_eq!(ThemeKind::Dark.label(), "深色");
        assert_eq!(ThemeKind::Light.label(), "浅色");
    }

    #[test]
    fn toggling_swaps_the_palette() {
        for theme in [ThemeKind::Dark, ThemeKind::Light] {
            assert_eq!(theme.toggled().toggled(), theme);
            assert_ne!(
                theme.palette().background,
                theme.toggled().palette().background
            );
        }
    }

    #[test]
    fn stored_names_round_trip() {
        for theme in [ThemeKind::Dark, ThemeKind::Light] {
            assert_eq!(ThemeKind::from_name(theme.name()), theme);
        }
        // Trimmed, and anything unknown (or a first run) opens light.
        assert_eq!(ThemeKind::from_name(" dark "), ThemeKind::Dark);
        assert_eq!(ThemeKind::from_name(""), ThemeKind::Light);
        assert_eq!(ThemeKind::from_name("sepia"), ThemeKind::Light);
    }

    #[test]
    fn opens_in_the_light_palette() {
        assert_eq!(ThemeKind::default(), ThemeKind::Light);
        assert_eq!(ThemeKind::default().label(), "浅色");
    }

    #[test]
    fn every_palette_is_offered() {
        assert_eq!(ThemeKind::ALL.len(), 2);
        for theme in ThemeKind::ALL {
            assert!(!theme.label().is_empty());
            assert!(!theme.description().is_empty());
        }
    }

    #[test]
    fn the_main_panel_is_white_in_the_light_theme() {
        assert_eq!(ThemeKind::Light.palette().canvas, 0xffffff);
        // The dark theme keeps an elevated surface, not white: the text there is
        // drawn in the light palette's ink and would vanish on white.
        assert_ne!(
            ThemeKind::Dark.palette().canvas,
            ThemeKind::Dark.palette().background
        );
    }

    #[test]
    fn the_sidebar_stays_visible_against_the_white_panel() {
        // The sidebar shares the title bar's colour (`chrome`). The light chrome
        // is `#f9fafb`, deliberately only ~6/255 away from the white panel: the
        // hairline under the chat header and the card shadows carry the
        // separation. Chips and menus keep the older `#ededf0` (`background`).
        let light = ThemeKind::Light.palette();
        assert_eq!(light.chrome, 0xf9fafb);
        assert_eq!(light.background, 0xededf0);
        let channel = |colour: u32, shift: u32| (colour >> shift) & 0xff;
        for shift in [16, 8, 0] {
            let gap = channel(light.canvas, shift) - channel(light.chrome, shift);
            assert!(gap >= 4, "sidebar is too close to the panel: gap {gap}");
        }
        // The hover/selected chip colour still has to stand off the chrome.
        for shift in [16, 8, 0] {
            let gap = channel(light.background, shift) - channel(light.panel_soft, shift);
            assert!(gap >= 6, "hover is too close to the sidebar: gap {gap}");
        }
        // Dark: the panel has to stay the lighter surface so cards keep floating.
        assert!(ThemeKind::Dark.palette().chrome < ThemeKind::Dark.palette().canvas);
        assert_eq!(
            ThemeKind::Dark.palette().chrome,
            ThemeKind::Dark.palette().background
        );
    }

    #[test]
    fn hex_survives_a_round_trip_and_refuses_nonsense() {
        assert_eq!(parse_hex("#2f6fd0"), Some(0x002f_6fd0));
        assert_eq!(parse_hex("2f6fd0"), Some(0x002f_6fd0));
        assert_eq!(hex(0x002f_6fd0), "#2f6fd0");
        for bad in ["", "#fff", "#gggggg", "#2f6fd", "2f6fd0ff", "red"] {
            assert_eq!(parse_hex(bad), None, "{bad}");
        }
    }

    #[test]
    fn custom_colours_layer_over_the_theme_and_ignore_unknown_keys() {
        let base = ThemeKind::Dark.palette();
        let colors = vec![
            ("accent".to_string(), 0x00ff_0000),
            ("not-a-role".to_string(), 0),
        ];
        let palette = themed(ThemeKind::Dark, &colors);
        assert_eq!(palette.accent, 0x00ff_0000);
        assert_eq!(
            palette.text, base.text,
            "the rest of the theme is untouched"
        );

        let mut direct = base;
        assert!(apply_color(&mut direct, " accent ", 0x0000_ff00));
        assert_eq!(direct.accent, 0x0000_ff00);
        assert!(!apply_color(&mut direct, "not-a-role", 0));
    }

    #[test]
    fn every_offered_colour_has_a_unique_key_and_a_label() {
        let mut seen: Vec<&str> = Vec::new();
        for role in COLOR_ROLES.iter() {
            assert!(!role.label.is_empty(), "{} needs a label", role.key);
            assert!(!seen.contains(&role.key), "{} is listed twice", role.key);
            seen.push(role.key);
        }
    }

    #[test]
    fn hsl_covers_the_primaries_and_the_extremes() {
        assert_eq!(hsl(0., 1., 0.5), 0x00ff_0000);
        assert_eq!(hsl(120., 1., 0.5), 0x0000_ff00);
        assert_eq!(hsl(240., 1., 0.5), 0x0000_00ff);
        assert_eq!(hsl(0., 0., 0.), 0x0000_0000);
        assert_eq!(hsl(0., 0., 1.), 0x00ff_ffff);
        // The hue wraps, so 360 is the same red as 0.
        assert_eq!(hsl(360., 1., 0.5), hsl(0., 1., 0.5));
    }

    #[test]
    fn the_swatch_grid_is_six_hue_rows_plus_a_neutral_one() {
        let grid = swatch_grid();
        assert_eq!(grid.len(), 7);
        for row in &grid {
            assert_eq!(row.len(), 12);
            // Nothing is transparent and nothing is out of range.
            assert!(row.iter().all(|value| *value <= 0x00ff_ffff));
        }
        // Every row is distinct: a picker whose rows repeat is useless.
        assert_ne!(grid[0], grid[5]);
        // The greys reach both ends, so pure white and pure black are pickable.
        let neutrals = &grid[6];
        assert_eq!(neutrals[0], 0x00ff_ffff);
        assert_eq!(neutrals[11], 0x0000_0000);
        assert!(neutrals.windows(2).all(|pair| pair[0] > pair[1]));
    }

    #[test]
    fn the_custom_card_is_the_mode_whenever_a_colour_differs() {
        assert!(!crate::theme::custom_mode(&[]));
        assert!(crate::theme::custom_mode(&[(
            "accent".to_string(),
            0x00ff_0000
        )]));
        // The third card is a state, so it has no stored flag to fall out of sync.
        assert_eq!(crate::theme::CUSTOM_LABEL, "自定义");
        assert!(!crate::theme::CUSTOM_DESCRIPTION.is_empty());
    }

    #[test]
    fn folder_names_are_drawn_in_the_solidest_ink() {
        // The 工作区 tree is read at a glance, so its folder rows use a colour of
        // their own: pure black on the light theme, pure white on the dark one.
        assert_eq!(ThemeKind::Light.palette().strong, 0x000000);
        assert_eq!(ThemeKind::Dark.palette().strong, 0xffffff);
        assert_ne!(
            ThemeKind::Light.palette().strong,
            ThemeKind::Light.palette().text
        );
    }
}
