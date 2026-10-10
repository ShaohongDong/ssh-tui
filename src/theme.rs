use crate::i18n::{Context, Message, Text};
use anyhow::{Result, ensure};
use ratatui::style::{Color, Style};
use serde::Deserialize;
use std::path::Path;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Theme {
    pub name: String,
    pub foreground: String,
    pub background: String,
    pub accent: String,
    pub muted: String,
    pub palette: Vec<String>,
    #[serde(default)]
    pub ui: UiOverrides,
}

/// Optional UI roles in the same theme. Terminal colors remain untouched.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UiOverrides {
    pub surface: Option<String>,
    pub border: Option<String>,
    pub selection_background: Option<String>,
    pub selection_foreground: Option<String>,
    pub success: Option<String>,
    pub attention: Option<String>,
    pub failure: Option<String>,
}

#[derive(Clone, Copy, Debug)]
pub struct UiPalette {
    pub background: Color,
    pub surface: Color,
    pub border: Color,
    pub foreground: Color,
    pub muted: Color,
    pub accent: Color,
    pub selection_background: Color,
    pub selection_foreground: Color,
    pub success: Color,
    pub attention: Color,
    pub failure: Color,
}

impl UiPalette {
    pub fn base(self) -> Style {
        Style::reset()
            .fg(readable(self.foreground, self.background))
            .bg(self.background)
    }
    pub fn panel(self) -> Style {
        Style::reset().fg(self.foreground).bg(self.surface)
    }
    pub fn secondary(self) -> Style {
        self.panel().fg(self.muted)
    }
    pub fn selected(self) -> Style {
        Style::reset()
            .fg(self.selection_foreground)
            .bg(self.selection_background)
    }
}

fn rgb(c: Color) -> [u8; 3] {
    match c {
        Color::Rgb(r, g, b) => [r, g, b],
        _ => unreachable!("theme colors are validated RGB"),
    }
}

fn blend(a: Color, b: Color, alpha: f64) -> Color {
    let a = rgb(a);
    let b = rgb(b);
    let c: [u8; 3] = std::array::from_fn(|i| {
        (f64::from(a[i]) * alpha + f64::from(b[i]) * (1.0 - alpha)).round() as u8
    });
    Color::Rgb(c[0], c[1], c[2])
}

pub fn contrast(a: Color, b: Color) -> f64 {
    fn luminance(c: Color) -> f64 {
        let [r, g, b] = rgb(c).map(|v| {
            let x = f64::from(v) / 255.0;
            if x <= 0.04045 {
                x / 12.92
            } else {
                ((x + 0.055) / 1.055).powf(2.4)
            }
        });
        0.2126 * r + 0.7152 * g + 0.0722 * b
    }
    let a = luminance(a);
    let b = luminance(b);
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

/// Preserve the hue as far as possible, correcting only application UI text.
pub fn readable(preferred: Color, background: Color) -> Color {
    if contrast(preferred, background) >= 4.5 {
        return preferred;
    }
    let black = Color::Rgb(0, 0, 0);
    let white = Color::Rgb(255, 255, 255);
    let endpoint = if contrast(black, background) >= contrast(white, background) {
        black
    } else {
        white
    };
    (1..=255)
        .map(|n| blend(endpoint, preferred, f64::from(n) / 255.0))
        .find(|c| contrast(*c, background) >= 4.5)
        .unwrap_or(endpoint)
}

pub fn color(s: &str) -> Result<Color> {
    ensure!(
        s.len() == 7 && s.starts_with('#') && s[1..].bytes().all(|c| c.is_ascii_hexdigit()),
        Message::arg(Text::ColorInvalid, s)
    );
    let value = u32::from_str_radix(&s[1..], 16)?;
    Ok(Color::Rgb(
        (value >> 16) as u8,
        (value >> 8) as u8,
        value as u8,
    ))
}

impl Theme {
    pub fn validate(&self) -> Result<()> {
        ensure!(!self.name.is_empty(), Message::plain(Text::ThemeNameEmpty));
        ensure!(self.palette.len() == 16, Message::plain(Text::ThemePalette));
        for c in [
            &self.foreground,
            &self.background,
            &self.accent,
            &self.muted,
        ]
        .into_iter()
        .chain(self.palette.iter())
        .chain(
            [
                &self.ui.surface,
                &self.ui.border,
                &self.ui.selection_background,
                &self.ui.selection_foreground,
                &self.ui.success,
                &self.ui.attention,
                &self.ui.failure,
            ]
            .into_iter()
            .filter_map(Option::as_ref),
        ) {
            color(c)?;
        }
        Ok(())
    }
    pub fn style(&self) -> Style {
        Style::default().fg(self.fg()).bg(self.bg())
    }
    pub fn fg(&self) -> Color {
        color(&self.foreground).expect("validated theme")
    }
    pub fn bg(&self) -> Color {
        color(&self.background).expect("validated theme")
    }
    pub fn accent(&self) -> Color {
        color(&self.accent).expect("validated theme")
    }
    pub fn muted(&self) -> Color {
        color(&self.muted).expect("validated theme")
    }
    pub fn ui(&self) -> UiPalette {
        let resolve = |value: &Option<String>, fallback: Color| {
            value
                .as_ref()
                .map_or(fallback, |v| color(v).expect("validated UI color"))
        };
        let surface = resolve(&self.ui.surface, blend(self.fg(), self.bg(), 0.045));
        let selection_background = resolve(
            &self.ui.selection_background,
            blend(self.accent(), surface, 0.18),
        );
        UiPalette {
            background: self.bg(),
            surface,
            border: resolve(&self.ui.border, blend(self.fg(), surface, 0.23)),
            foreground: readable(self.fg(), surface),
            muted: readable(self.muted(), surface),
            accent: readable(self.accent(), surface),
            selection_background,
            selection_foreground: readable(
                resolve(&self.ui.selection_foreground, self.fg()),
                selection_background,
            ),
            success: readable(
                resolve(&self.ui.success, color(&self.palette[2]).unwrap()),
                surface,
            ),
            attention: readable(
                resolve(&self.ui.attention, color(&self.palette[3]).unwrap()),
                surface,
            ),
            failure: readable(
                resolve(&self.ui.failure, color(&self.palette[1]).unwrap()),
                surface,
            ),
        }
    }
    pub fn terminal_color(&self, c: vt100::Color, foreground: bool) -> Color {
        match c {
            vt100::Color::Default => {
                if foreground {
                    self.fg()
                } else {
                    self.bg()
                }
            }
            vt100::Color::Idx(i) if i < 16 => {
                color(&self.palette[i as usize]).expect("validated palette")
            }
            vt100::Color::Idx(i) => Color::Indexed(i),
            vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
        }
    }
}

pub fn builtin() -> Vec<Theme> {
    let specs = [
        (
            "Tokyo Night",
            "#c0caf5",
            "#1a1b26",
            "#7aa2f7",
            "#565f89",
            "15161e f7768e 9ece6a e0af68 7aa2f7 bb9af7 7dcfff a9b1d6 414868 f7768e 9ece6a e0af68 7aa2f7 bb9af7 7dcfff c0caf5",
        ),
        (
            "Dracula",
            "#f8f8f2",
            "#282a36",
            "#bd93f9",
            "#6272a4",
            "21222c ff5555 50fa7b f1fa8c bd93f9 ff79c6 8be9fd f8f8f2 6272a4 ff6e6e 69ff94 ffffa4 d6acff ff92df a4ffff ffffff",
        ),
        (
            "Nord",
            "#d8dee9",
            "#2e3440",
            "#88c0d0",
            "#4c566a",
            "3b4252 bf616a a3be8c ebcb8b 81a1c1 b48ead 88c0d0 e5e9f0 4c566a bf616a a3be8c ebcb8b 81a1c1 b48ead 8fbcbb eceff4",
        ),
        (
            "Gruvbox Dark",
            "#ebdbb2",
            "#282828",
            "#fabd2f",
            "#928374",
            "282828 cc241d 98971a d79921 458588 b16286 689d6a a89984 928374 fb4934 b8bb26 fabd2f 83a598 d3869b 8ec07c ebdbb2",
        ),
        (
            "Solarized Dark",
            "#839496",
            "#002b36",
            "#268bd2",
            "#586e75",
            "073642 dc322f 859900 b58900 268bd2 d33682 2aa198 eee8d5 002b36 cb4b16 586e75 657b83 839496 6c71c4 93a1a1 fdf6e3",
        ),
        (
            "Solarized Light",
            "#657b83",
            "#fdf6e3",
            "#268bd2",
            "#93a1a1",
            "073642 dc322f 859900 b58900 268bd2 d33682 2aa198 eee8d5 002b36 cb4b16 586e75 657b83 839496 6c71c4 93a1a1 fdf6e3",
        ),
    ];
    specs
        .into_iter()
        .map(
            |(name, foreground, background, accent, muted, palette)| Theme {
                name: name.into(),
                foreground: foreground.into(),
                background: background.into(),
                accent: accent.into(),
                muted: muted.into(),
                palette: palette
                    .split_whitespace()
                    .map(|c| format!("#{c}"))
                    .collect(),
                ui: UiOverrides::default(),
            },
        )
        .collect()
}

pub fn load(dir: &Path) -> Result<Vec<Theme>> {
    let mut themes = builtin();
    if !dir.exists() {
        return Ok(themes);
    }
    let mut paths = std::fs::read_dir(dir)?
        .map(|e| e.map(|e| e.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    paths.sort();
    for path in paths
        .into_iter()
        .filter(|p| p.extension().is_some_and(|e| e == "toml"))
    {
        let t: Theme = toml::from_str(&std::fs::read_to_string(&path)?)
            .with_context(|| Message::arg(Text::ThemeToml, path.display()))?;
        t.validate()
            .with_context(|| Message::arg(Text::ThemeInvalid, path.display()))?;
        ensure!(
            !themes.iter().any(|x| x.name == t.name),
            Message::arg(Text::DuplicateTheme, &t.name)
        );
        themes.push(t);
    }
    Ok(themes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn palettes_and_explicit_rgb() {
        for t in builtin() {
            t.validate().unwrap();
            assert_eq!(
                t.terminal_color(vt100::Color::Rgb(1, 2, 3), true),
                Color::Rgb(1, 2, 3)
            );
        }
        assert!(color("#你好").is_err());
    }
    #[test]
    fn ui_text_has_measured_contrast_on_each_surface() {
        for mut t in builtin() {
            for background in ["#000000", "#ffffff", "#808080", "#fdf6e3", "#1a1b26"] {
                t.background = background.into();
                let ui = t.ui();
                for c in [
                    ui.foreground,
                    ui.muted,
                    ui.accent,
                    ui.success,
                    ui.attention,
                    ui.failure,
                ] {
                    assert!(contrast(c, ui.surface) >= 4.5, "{} {c:?}", t.name);
                }
                assert!(contrast(ui.selection_foreground, ui.selection_background) >= 4.5);
                assert!(contrast(ui.base().fg.unwrap(), ui.background) >= 4.5);
            }
        }
    }
    #[test]
    fn old_theme_and_optional_ui_overrides_are_compatible() {
        let original = include_str!("../examples/themes/amber.toml");
        let old: Theme = toml::from_str(original).unwrap();
        old.validate().unwrap();
        let custom: Theme = toml::from_str(&format!("{original}\n[ui]\nsurface = '#eeeeee'\nselection_background = '#ffffff'\nselection_foreground = '#eeeeee'\n")).unwrap();
        custom.validate().unwrap();
        assert_eq!(custom.ui().surface, Color::Rgb(238, 238, 238));
        assert!(
            contrast(
                custom.ui().selection_foreground,
                custom.ui().selection_background
            ) >= 4.5
        );
        for c in [
            vt100::Color::Default,
            vt100::Color::Idx(1),
            vt100::Color::Idx(199),
            vt100::Color::Rgb(1, 2, 3),
        ] {
            assert_eq!(old.terminal_color(c, true), custom.terminal_color(c, true));
        }
        let mut bad = custom;
        bad.ui.border = Some("red".into());
        assert!(bad.validate().is_err());
        assert!(
            toml::from_str::<Theme>(&format!("{original}\n[ui]\nunknown_color = '#ffffff'\n"))
                .is_err()
        );
    }
}
