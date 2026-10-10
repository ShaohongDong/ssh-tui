//! Small, shared UI primitives. All measurements use terminal cell widths.
use crate::theme::{UiPalette, readable};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
};
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

pub fn clean(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

pub fn width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

pub fn truncate(text: &str, cells: usize) -> String {
    let text = clean(text);
    if width(&text) <= cells {
        return text;
    }
    if cells == 0 {
        return String::new();
    }
    let mut result = String::new();
    let mut used = 0;
    for part in text.graphemes(true) {
        let n = width(part);
        if used + n > cells - 1 {
            break;
        }
        result.push_str(part);
        used += n;
    }
    result.push('…');
    result
}

pub fn padded(text: &str, cells: usize) -> String {
    let mut text = truncate(text, cells);
    text.push_str(&" ".repeat(cells.saturating_sub(width(&text))));
    text
}

/// Wrap once for both height measurement and rendering, including CJK text.
pub fn wrapped(text: &str, cells: usize) -> Vec<String> {
    if cells == 0 {
        return Vec::new();
    }
    let mut result = Vec::new();
    for source in text.lines() {
        let source = clean(source);
        let mut line = String::new();
        let mut used = 0;
        // Keep words together when they fit. Unicode word boundaries also
        // allow CJK wrapping; long words still split at grapheme boundaries.
        for word in source.split_word_bounds() {
            if width(word) <= cells && used + width(word) > cells && !line.is_empty() {
                result.push(line.trim_end().to_owned());
                line.clear();
                used = 0;
            }
            if line.is_empty() && word.trim().is_empty() {
                continue;
            }
            for part in word.graphemes(true) {
                let n = width(part);
                if used + n > cells && !line.is_empty() {
                    result.push(std::mem::take(&mut line));
                    used = 0;
                }
                if n <= cells {
                    line.push_str(part);
                    used += n;
                }
            }
        }
        result.push(line.trim_end().to_owned());
    }
    result
}

/// Scroll long input horizontally so the insertion marker is always visible.
pub fn input_window(text: &str, cursor: usize, cells: usize) -> String {
    if cells == 0 {
        return String::new();
    }
    if cells == 1 {
        return "▏".into();
    }
    let before = clean(&text[..cursor]);
    let after = clean(&text[cursor..]);
    let parts: Vec<_> = before.graphemes(true).collect();
    let mut start = parts.len();
    let mut used = 1;
    // Reserve one cell for a left overflow marker if scrolling is needed.
    let budget = cells - usize::from(width(&before) + 1 > cells);
    while start > 0 && used + width(parts[start - 1]) <= budget {
        start -= 1;
        used += width(parts[start]);
    }
    let mut out = if start > 0 {
        "‹".to_string()
    } else {
        String::new()
    };
    for part in &parts[start..] {
        out.push_str(part);
    }
    out.push('▏');
    out.push_str(&truncate(&after, cells.saturating_sub(width(&out))));
    out
}

pub fn visible_range(total: usize, selected: usize, rows: usize) -> Range<usize> {
    let rows = rows.min(total);
    let start = selected
        .saturating_add(1)
        .saturating_sub(rows)
        .min(total.saturating_sub(rows));
    start..start + rows
}

#[derive(Clone, Copy)]
pub struct Panel {
    pub outer: Rect,
    pub inner: Rect,
}
impl Panel {
    pub fn new(area: Rect, preferred_width: u16, content_height: usize) -> Self {
        let framed = area.width >= 40 && area.height >= 8;
        let margin = if framed { 4 } else { 0 };
        let w = preferred_width.min(area.width.saturating_sub(margin));
        let h = (content_height.saturating_add(if framed { 2 } else { 0 })).min(usize::from(
            area.height.saturating_sub(if framed { 2 } else { 0 }),
        )) as u16;
        let outer = Rect::new(
            area.x + (area.width - w) / 2,
            area.y + (area.height - h) / 2,
            w,
            h,
        );
        let inset = u16::from(framed);
        let inner = Rect::new(
            outer.x + inset * 2,
            outer.y + inset,
            w.saturating_sub(inset * 4),
            h.saturating_sub(inset * 2),
        );
        Self { outer, inner }
    }
    pub fn render(self, f: &mut Frame, ui: UiPalette) {
        f.render_widget(Clear, self.outer);
        let framed = self.inner.x != self.outer.x;
        f.render_widget(
            Block::default()
                .style(ui.panel())
                .borders(if framed { Borders::ALL } else { Borders::NONE })
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(ui.border).bg(ui.surface)),
            self.outer,
        );
    }
    pub fn heading(self) -> Rect {
        row(self.inner, 0)
    }
    pub fn footer(self) -> Rect {
        row(self.inner, self.inner.height.saturating_sub(1))
    }
    pub fn body(self) -> Rect {
        Rect::new(
            self.inner.x,
            self.inner.y.saturating_add(1),
            self.inner.width,
            self.inner.height.saturating_sub(2),
        )
    }
}

pub fn row(area: Rect, offset: u16) -> Rect {
    Rect::new(
        area.x,
        area.y.saturating_add(offset),
        area.width,
        u16::from(offset < area.height),
    )
}

pub fn heading(f: &mut Frame, area: Rect, ui: UiPalette, title: &str, detail: &str) {
    let title = truncate(title, usize::from(area.width));
    f.render_widget(Paragraph::new(title.clone()).style(ui.panel().bold()), area);
    let detail = clean(detail);
    let n = width(&detail) as u16;
    if !detail.is_empty() && width(&title) + width(&detail) + 2 <= usize::from(area.width) {
        f.render_widget(
            Paragraph::new(detail).style(ui.secondary()),
            Rect::new(area.right() - n, area.y, n, area.height),
        );
    }
}

/// Keep complete shortcut labels; omit lower-priority hints at narrow widths.
pub fn hints(f: &mut Frame, area: Rect, ui: UiPalette, actions: &[(&str, &str)]) {
    f.render_widget(
        Paragraph::new(hint_line(ui, actions, usize::from(area.width))).style(ui.panel()),
        area,
    );
}

pub fn hint_line(ui: UiPalette, actions: &[(&str, &str)], cells: usize) -> Line<'static> {
    let mut spans = Vec::new();
    let mut used = 0;
    for (key, description) in actions {
        let n = width(key) + 1 + width(description) + if spans.is_empty() { 0 } else { 3 };
        if used + n > cells {
            break;
        }
        if !spans.is_empty() {
            spans.push(Span::styled(" · ", ui.secondary()));
        }
        spans.push(Span::styled(format!("{key} "), ui.panel().bold()));
        spans.push(Span::styled(description.to_string(), ui.secondary()));
        used += n;
    }
    Line::from(spans)
}

#[derive(Clone)]
pub struct ListRow {
    pub name: String,
    pub detail: String,
    pub extra: String,
}

pub fn list(
    f: &mut Frame,
    area: Rect,
    ui: UiPalette,
    items: &[ListRow],
    selected: usize,
    empty: &str,
) {
    if area.is_empty() {
        return;
    }
    if items.is_empty() {
        f.render_widget(
            Paragraph::new(truncate(empty, usize::from(area.width))).style(ui.secondary()),
            row(area, 0),
        );
        return;
    }
    let available = usize::from(area.width).saturating_sub(3);
    let show_detail = available >= 32;
    let show_extra = available >= 64;
    let name_width = if show_detail {
        items
            .iter()
            .map(|r| width(&clean(&r.name)))
            .max()
            .unwrap_or(0)
            .clamp(12, (available / 3).max(12))
    } else {
        available
    };
    let extra_width = if show_extra {
        items
            .iter()
            .map(|r| width(&clean(&r.extra)))
            .max()
            .unwrap_or(0)
            .min(22)
    } else {
        0
    };
    let detail_width =
        available.saturating_sub(name_width + 2 + if show_extra { extra_width + 2 } else { 0 });
    let range = visible_range(items.len(), selected, usize::from(area.height));
    for (offset, i) in range.clone().enumerate() {
        let r = &items[i];
        let active = i == selected;
        let style = if active { ui.selected() } else { ui.panel() };
        let secondary = style.fg(readable(
            ui.muted,
            if active {
                ui.selection_background
            } else {
                ui.surface
            },
        ));
        let mut spans = vec![
            Span::styled(if active { "› " } else { "  " }, style),
            Span::styled(
                padded(&r.name, name_width),
                style.add_modifier(if active {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
            ),
        ];
        if show_detail {
            spans.push(Span::styled(
                format!("  {}", padded(&r.detail, detail_width)),
                secondary,
            ));
        }
        if show_extra {
            spans.push(Span::styled(
                format!("  {}", padded(&r.extra, extra_width)),
                secondary,
            ));
        }
        f.render_widget(
            Paragraph::new(Line::from(spans)).style(style),
            row(area, offset as u16),
        );
    }
    for (show, symbol, y) in [
        (range.start > 0, "↑", area.y),
        (range.end < items.len(), "↓", area.bottom() - 1),
    ] {
        if show {
            // Use the right margin, preserving the leading focus marker.
            f.render_widget(
                Paragraph::new(symbol).style(ui.panel().fg(ui.accent)),
                Rect::new(area.right() - 1, y, 1, 1),
            );
        }
    }
}

#[derive(Clone)]
pub struct Tab {
    pub name: String,
    pub exited: bool,
    pub unread: bool,
}

pub fn tabs(f: &mut Frame, area: Rect, ui: UiPalette, items: &[Tab], active: usize) {
    if area.is_empty() {
        return;
    }
    if items.is_empty() {
        f.render_widget(Paragraph::new(" SSH TUI").style(ui.base().bold()), area);
        return;
    }
    let available = usize::from(area.width);
    let active = active.min(items.len() - 1);
    let label = |i: usize| {
        let t = &items[i];
        format!(
            " {}:{}{}{} ",
            i + 1,
            truncate(&t.name, 17),
            if t.exited { " ×" } else { "" },
            if t.unread { " •" } else { "" }
        )
    };
    // Reserve markers on both ends when the tab strip overflows.
    let total: usize = (0..items.len()).map(|i| width(&label(i))).sum();
    let budget = available.saturating_sub(if total > available { 2 } else { 0 });
    let mut start = active;
    let mut end = active + 1;
    let mut used = width(&label(active)).min(budget);
    while start > 0 && used + width(&label(start - 1)) <= budget {
        start -= 1;
        used += width(&label(start));
    }
    while end < items.len() && used + width(&label(end)) <= budget {
        used += width(&label(end));
        end += 1;
    }
    let mut spans = Vec::new();
    if start > 0 {
        spans.push(Span::styled("‹", ui.base()));
    }
    for (i, _) in items.iter().enumerate().take(end).skip(start) {
        let style = if i == active {
            ui.selected().bold()
        } else {
            ui.base().fg(readable(ui.muted, ui.background))
        };
        let text = if i == active {
            truncate(&label(i), budget)
        } else {
            label(i)
        };
        spans.push(Span::styled(text, style));
    }
    if end < items.len() {
        spans.push(Span::styled("›", ui.base()));
    }
    f.render_widget(Paragraph::new(Line::from(spans)).style(ui.base()), area);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wrapping_keeps_english_words_and_unicode_graphemes_intact() {
        assert_eq!(wrapped("alpha beta gamma", 10), ["alpha beta", "gamma"]);
        let source = "SSH process running means the local process is alive;";
        let lines = wrapped(source, 40);
        assert_eq!(lines.join(" "), source);
        assert!(lines.iter().all(|line| width(line) <= 40));
        assert_eq!(
            wrapped("e\u{301}e\u{301}e\u{301}", 2),
            ["e\u{301}e\u{301}", "e\u{301}"]
        );
        for cells in 1..15 {
            let lines = wrapped("中文目录/averylongfilenameabcdef", cells);
            assert!(lines.iter().all(|line| width(line) <= cells));
        }
        assert!(wrapped("text", 0).is_empty());
    }
    #[test]
    fn unicode_truncation_and_cursor_window_use_cells() {
        assert_eq!(truncate("中文abcd", 5), "中文…");
        assert_eq!(truncate("e\u{301}abcdef", 3), "e\u{301}a…");
        assert_eq!(truncate("\nabc", 0), "");
        for n in 1..20 {
            let text = "中文长路径/alpha/beta";
            for (cursor, _) in text
                .char_indices()
                .chain(std::iter::once((text.len(), ' ')))
            {
                let shown = input_window(text, cursor, n);
                assert!(width(&shown) <= n, "{shown:?}");
                assert!(shown.contains('▏'));
            }
        }
    }
    #[test]
    fn overflowing_tabs_keep_the_active_tab_and_its_style_visible() {
        use ratatui::{Terminal, backend::TestBackend};
        let ui = crate::theme::builtin()[0].ui();
        let items: Vec<_> = (0..24)
            .map(|i| Tab {
                name: format!("生产服务器-{i}"),
                exited: i == 3,
                unread: i == 23,
            })
            .collect();
        for cells in [12, 24, 48, 80, 120] {
            let mut terminal = Terminal::new(TestBackend::new(cells, 1)).unwrap();
            terminal
                .draw(|f| tabs(f, f.area(), ui, &items, 18))
                .unwrap();
            let buffer = terminal.backend().buffer();
            let text: String = buffer.content.iter().map(|c| c.symbol()).collect();
            assert!(text.contains("19:"), "{text}");
            assert!(text.contains('‹') && text.contains('›'), "{text}");
            let cell = buffer.content.iter().find(|c| c.symbol() == "9").unwrap();
            assert_eq!(cell.bg, ui.selection_background);
            assert_eq!(cell.fg, ui.selection_foreground);
        }
    }
}
