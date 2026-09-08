//! Rendering of the application state onto a ratatui frame.

use std::time::Duration;

use ratatui_core::buffer::Buffer;
use ratatui_core::layout::{Constraint, Layout, Margin, Rect};
use ratatui_core::style::{Color, Modifier, Style};
use ratatui_core::terminal::Frame;
use ratatui_core::text::{Line, Span, Text};
use ratatui_core::widgets::StatefulWidget;
use ratatui_widgets::block::Block;
use ratatui_widgets::borders::BorderType;
use ratatui_widgets::clear::Clear;
use ratatui_widgets::paragraph::{Paragraph, Wrap};
use ratatui_widgets::scrollbar::{Scrollbar, ScrollbarOrientation, ScrollbarState};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::app::{App, Phase, Row};
use crate::tree::{group_digits, human_size, Kind, Tree};

const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const PARTIAL: [&str; 8] = ["", "▏", "▎", "▍", "▌", "▋", "▊", "▉"];

const SIZE_W: u16 = 8;
const BAR_W: u16 = 10;
const PCT_W: u16 = 6;
const GAP: u16 = 2;

/// Column x-offsets for one screen width. Narrow terminals drop the bar, then the percentage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Columns {
    size_x: u16,
    bar_x: Option<u16>,
    pct_x: Option<u16>,
    name_x: u16,
    name_w: u16,
}

impl Columns {
    fn for_width(width: u16) -> Self {
        let mut x = 1;
        let size_x = x;
        x += SIZE_W + GAP;
        let bar_x = (width >= 60).then_some(x);
        if bar_x.is_some() {
            x += BAR_W + 1;
        }
        let pct_x = (width >= 40).then_some(x);
        if pct_x.is_some() {
            x += PCT_W + GAP;
        }
        Columns {
            size_x,
            bar_x,
            pct_x,
            name_x: x,
            name_w: width.saturating_sub(x + 1),
        }
    }
}

/// Draw the whole screen.
pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    match app.phase {
        Phase::Scanning(_) => draw_scanning(frame, app, area),
        Phase::Failed(_) => draw_failed(frame, app, area),
        Phase::Ready(_) => draw_ready(frame, app, area),
    }
    if app.show_help {
        draw_help(frame, area);
    }
}

fn accent() -> Style {
    Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD)
}

fn dim() -> Style {
    Style::new().fg(Color::DarkGray)
}

fn panel(title: &str) -> Block<'static> {
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(dim())
        .title(Line::from(format!(" {title} ")).style(accent()))
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    )
}

/// Human readable duration, e.g. `3.2 s` or `2m 05s`.
pub fn format_duration(d: Duration) -> String {
    let secs = d.as_secs_f64();
    if secs < 60.0 {
        format!("{secs:.1} s")
    } else {
        let whole = d.as_secs();
        format!("{}m {:02}s", whole / 60, whole % 60)
    }
}

/// Truncate `s` to at most `width` columns, ending with `…` if cut.
pub fn fit(s: &str, width: usize) -> String {
    if s.width() <= width {
        return s.to_string();
    }
    if width == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 0;
    for ch in s.chars() {
        let w = ch.width().unwrap_or(0);
        if used + w > width - 1 {
            break;
        }
        out.push(ch);
        used += w;
    }
    out.push('…');
    out
}

/// Truncate from the left, keeping the tail (useful for long paths).
pub fn fit_tail(s: &str, width: usize) -> String {
    if s.width() <= width {
        return s.to_string();
    }
    if width == 0 {
        return String::new();
    }
    let mut kept = Vec::new();
    let mut used = 0;
    for ch in s.chars().rev() {
        let w = ch.width().unwrap_or(0);
        if used + w > width - 1 {
            break;
        }
        kept.push(ch);
        used += w;
    }
    let mut out = String::from("…");
    out.extend(kept.into_iter().rev());
    out
}

// ----- scanning / failed ---------------------------------------------------

fn draw_scanning(frame: &mut Frame, app: &mut App, area: Rect) {
    let Phase::Scanning(progress) = &app.phase else {
        return;
    };
    let spinner = SPINNER[(app.tick as usize) % SPINNER.len()];
    let boxed = centered(area, 64, 9);
    let block = panel("Scanning");
    let inner = block.inner(boxed);
    frame.render_widget(block, boxed);
    let width = inner.width.saturating_sub(2) as usize;
    let lines = vec![
        Line::from(vec![
            Span::styled(format!(" {spinner} "), accent()),
            Span::styled(fit_tail(&app.root.to_string_lossy(), width.saturating_sub(3)), Style::new().bold()),
        ]),
        Line::default(),
        Line::from(format!(
            " {} dirs · {} files · {}",
            group_digits(progress.dirs),
            group_digits(progress.files),
            human_size(progress.bytes)
        )),
        Line::from(format!(" elapsed {}", format_duration(app.elapsed()))),
        Line::from(Span::styled(
            format!(" {}", fit_tail(&progress.current.to_string_lossy(), width.saturating_sub(1))),
            dim(),
        )),
        Line::default(),
        Line::from(vec![Span::styled(" q", accent()), Span::styled(" quit", dim())]),
    ];
    frame.render_widget(Paragraph::new(Text::from(lines)), inner);
}

fn draw_failed(frame: &mut Frame, app: &mut App, area: Rect) {
    let Phase::Failed(message) = &app.phase else {
        return;
    };
    let boxed = centered(area, 64, 8);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(Color::Red))
        .title(Line::from(" Scan failed ").style(Style::new().fg(Color::Red).bold()));
    let inner = block.inner(boxed);
    frame.render_widget(block, boxed);
    let lines = vec![
        Line::from(format!(" {message}")),
        Line::default(),
        Line::from(vec![Span::styled(" q", accent()), Span::styled(" quit", dim())]),
    ];
    frame.render_widget(Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false }), inner);
}

// ----- main view -------------------------------------------------------------

fn draw_ready(frame: &mut Frame, app: &mut App, area: Rect) {
    let [main, footer] = Layout::vertical([Constraint::Min(3), Constraint::Length(1)]).areas(area);
    let counts = app.counts_of(app.view_root);
    let elapsed = app.elapsed();
    let Some(tree) = app.tree() else {
        return;
    };
    let view_root = app.view_root;
    let root_node = tree.node(view_root);
    let path = tree.path(view_root).to_string_lossy().into_owned();

    let summary = format!(
        " {} · {} files · {} dirs ",
        human_size(root_node.size),
        group_digits(counts.files),
        group_digits(counts.dirs)
    );
    let title_w = main.width.saturating_sub(summary.width() as u16 + 6) as usize;
    let mut block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(dim())
        .title(Line::from(format!(" {} ", fit_tail(&path, title_w))).style(accent()))
        .title_top(Line::from(summary).style(Style::new().bold()).right_aligned());
    if app.finished.is_some() {
        block = block.title_bottom(
            Line::from(format!(" scanned in {} ", format_duration(elapsed)))
                .style(dim())
                .right_aligned(),
        );
    }
    let inner = block.inner(main);
    frame.render_widget(block, main);
    if inner.height == 0 || inner.width == 0 {
        return;
    }

    let [header, rows_area] = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(inner);
    let cols = Columns::for_width(inner.width);
    draw_column_header(frame.buffer_mut(), header, cols);

    app.ensure_visible(rows_area.height as usize);
    app.rows_area = rows_area;
    let tree = app.tree().expect("ready");
    let buf = frame.buffer_mut();
    if app.rows.is_empty() && rows_area.height > 0 {
        buf.set_string(rows_area.x + 1, rows_area.y, "(empty directory)", dim());
    }
    for (i, row) in app.rows.iter().skip(app.offset).take(rows_area.height as usize).enumerate() {
        let line_area = Rect::new(rows_area.x, rows_area.y + i as u16, rows_area.width, 1);
        let selected = app.offset + i == app.selected;
        draw_row(buf, line_area, cols, tree, row, selected);
    }

    if app.rows.len() > rows_area.height as usize {
        let mut state = ScrollbarState::new(app.rows.len().saturating_sub(rows_area.height as usize))
            .position(app.offset);
        Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None)
            .track_style(dim())
            .thumb_style(Style::new().fg(Color::Cyan))
            .render(main.inner(Margin::new(0, 1)), buf, &mut state);
    }

    draw_footer(buf, footer, app);
}

fn draw_column_header(buf: &mut Buffer, area: Rect, cols: Columns) {
    if area.height == 0 {
        return;
    }
    let style = dim().add_modifier(Modifier::BOLD);
    buf.set_string(area.x + cols.size_x, area.y, format!("{:>w$}", "SIZE", w = SIZE_W as usize), style);
    if let Some(x) = cols.bar_x {
        buf.set_string(area.x + x, area.y, "SHARE", style);
    } else if let Some(x) = cols.pct_x {
        buf.set_string(area.x + x, area.y, format!("{:>w$}", "SHARE", w = PCT_W as usize), style);
    }
    if cols.name_w > 0 {
        buf.set_string(area.x + cols.name_x, area.y, "NAME", style);
    }
}

/// Fixed-width usage bar; the colour encodes how much of the parent this entry takes.
fn bar_spans(share: f64) -> Vec<Span<'static>> {
    let cells = share * BAR_W as f64;
    let full = (cells.floor() as usize).min(BAR_W as usize);
    let eighths = if full < BAR_W as usize {
        ((cells - full as f64) * 8.0).round() as usize
    } else {
        0
    };
    let (eighths, full) = if eighths == 8 { (0, full + 1) } else { (eighths, full) };
    let filled = format!("{}{}", "█".repeat(full), PARTIAL[eighths]);
    let used = full + usize::from(eighths > 0);
    let track = "░".repeat((BAR_W as usize).saturating_sub(used));
    vec![
        Span::styled(filled, Style::new().fg(share_color(share))),
        Span::styled(track, Style::new().fg(Color::DarkGray).add_modifier(Modifier::DIM)),
    ]
}

fn share_color(share: f64) -> Color {
    if share >= 0.5 {
        Color::Red
    } else if share >= 0.2 {
        Color::Yellow
    } else if share >= 0.05 {
        Color::Green
    } else {
        Color::Gray
    }
}

fn draw_row(buf: &mut Buffer, area: Rect, cols: Columns, tree: &Tree, row: &Row, selected: bool) {
    let node = tree.node(row.id);
    if selected {
        buf.set_style(area, Style::new().bg(Color::DarkGray).fg(Color::White));
    }
    let base = if selected {
        Style::new().fg(Color::White).add_modifier(Modifier::BOLD)
    } else {
        Style::new()
    };

    // size
    let size = format!("{:>w$}", human_size(node.size), w = SIZE_W as usize);
    buf.set_string(area.x + cols.size_x, area.y, size, base);

    // bar + percentage
    if let Some(x) = cols.bar_x {
        let line = Line::from(bar_spans(row.share));
        buf.set_line(area.x + x, area.y, &line, BAR_W);
    }
    if let Some(x) = cols.pct_x {
        let pct = format!("{:>5.1}%", row.share * 100.0);
        let style = if selected { base } else { Style::new().fg(share_color(row.share)) };
        buf.set_string(area.x + x, area.y, pct, style);
    }

    // guides + expander + name
    let mut prefix = String::new();
    for &ancestor_last in &row.guides {
        prefix.push_str(if ancestor_last { "   " } else { "│  " });
    }
    prefix.push_str(if row.last { "└─ " } else { "├─ " });
    let expander = match node.kind {
        Kind::Dir if row.expanded => "▾ ",
        Kind::Dir => "▸ ",
        _ => "  ",
    };
    let name = match node.kind {
        Kind::Dir => format!("{}/", node.name),
        Kind::Symlink => format!("{}@", node.name),
        _ => node.name.clone(),
    };
    let name_style = if selected {
        base
    } else {
        match node.kind {
            Kind::Dir => Style::new().fg(Color::Blue).add_modifier(Modifier::BOLD),
            Kind::File => Style::new(),
            Kind::Symlink => Style::new().fg(Color::Magenta),
            Kind::Other => dim(),
        }
    };
    let guide_style = if selected { base } else { dim() };
    let mut spans = vec![
        Span::styled(prefix, guide_style),
        Span::styled(expander, name_style),
        Span::styled(name, name_style),
    ];
    if let Some(err) = &node.error {
        spans.push(Span::styled(format!("  ⚠ {err}"), Style::new().fg(Color::Yellow)));
    }
    let line = Line::from(spans);
    let width = cols.name_w as usize;
    if line.width() > width {
        // Truncate the composed text while keeping the span styles of the visible part.
        let mut remaining = width;
        let mut kept = Vec::new();
        for span in line.spans {
            let w = span.content.width();
            if w <= remaining && remaining > 0 {
                remaining -= w;
                kept.push(span);
            } else {
                kept.push(Span::styled(fit(&span.content, remaining), span.style));
                break;
            }
        }
        buf.set_line(area.x + cols.name_x, area.y, &Line::from(kept), cols.name_w);
    } else {
        buf.set_line(area.x + cols.name_x, area.y, &line, cols.name_w);
    }
}

fn draw_footer(buf: &mut Buffer, area: Rect, app: &App) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let key = |k: &str| Span::styled(k.to_string(), accent());
    let desc = |d: &str| Span::styled(format!(" {d}  "), dim());
    let hints = Line::from(vec![
        Span::raw(" "),
        key("↑↓"),
        desc("move"),
        key("→/␣"),
        desc("expand"),
        key("←"),
        desc("collapse"),
        key("⏎"),
        desc("zoom in"),
        key("⌫"),
        desc("zoom out"),
        key("s"),
        desc(&format!("sort: {}", app.sort.label())),
        key("?"),
        desc("help"),
        key("q"),
        desc("quit"),
    ]);
    let mut right = String::new();
    if app.unreadable > 0 {
        right = format!("⚠ {} unreadable ", group_digits(app.unreadable));
    }
    let right_w = right.width() as u16;
    let hints_w = area.width.saturating_sub(right_w);
    buf.set_line(area.x, area.y, &hints, hints_w);
    if right_w > 0 && right_w <= area.width {
        buf.set_string(
            area.x + area.width - right_w,
            area.y,
            right,
            Style::new().fg(Color::Yellow),
        );
    }
}

fn draw_help(frame: &mut Frame, area: Rect) {
    let entries: [(&str, &str); 12] = [
        ("↑ ↓  j k", "move selection"),
        ("PgUp PgDn", "move by a page"),
        ("g / G", "first / last row"),
        ("→  l  space", "expand directory"),
        ("←  h", "collapse, else go to parent"),
        ("⏎", "zoom into directory"),
        ("⌫  u", "zoom out to parent"),
        ("s", "cycle sort: size, name, items"),
        ("click / wheel", "select / scroll"),
        ("?", "toggle this help"),
        ("q  esc", "quit"),
        ("", ""),
    ];
    let boxed = centered(area, 52, entries.len() as u16 + 3);
    frame.render_widget(Clear, boxed);
    let block = panel("Keys");
    let inner = block.inner(boxed);
    frame.render_widget(block, boxed);
    let mut lines: Vec<Line> = entries
        .iter()
        .filter(|(k, _)| !k.is_empty())
        .map(|(k, d)| {
            Line::from(vec![
                Span::styled(format!(" {k:<14}"), accent()),
                Span::raw((*d).to_string()),
            ])
        })
        .collect();
    lines.push(Line::default());
    lines.push(Line::from(Span::styled(
        " Sizes are allocated bytes; share is relative to the parent.",
        dim(),
    )));
    frame.render_widget(Paragraph::new(Text::from(lines)), inner);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::{Node, ScanEvent};
    use ratatui_core::backend::TestBackend;
    use ratatui_core::layout::Position;
    use ratatui_core::terminal::Terminal;
    use std::path::PathBuf;

    fn sample() -> Tree {
        let mut t = Tree::new("/root");
        let a = t.add_child(0, Node::new("a", Kind::Dir, 0));
        t.add_child(a, Node::new("small", Kind::File, 10));
        t.add_child(a, Node::new("big", Kind::File, 1000));
        let b = t.add_child(0, Node::new("b", Kind::Dir, 0));
        t.add_child(b, Node::new("x", Kind::File, 5));
        t.add_child(0, Node::new("top", Kind::File, 500));
        t.finalize();
        t
    }

    fn render(app: &mut App, width: u16, height: u16) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|f| draw(f, app)).unwrap();
        let buf = terminal.backend().buffer().clone();
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buf.cell(Position::new(x, y)).unwrap().symbol().to_string())
                    .collect::<String>()
            })
            .collect()
    }

    fn contains(lines: &[String], needle: &str) -> bool {
        lines.iter().any(|l| l.contains(needle))
    }

    #[test]
    fn columns_adapt_to_width() {
        let wide = Columns::for_width(100);
        assert!(wide.bar_x.is_some() && wide.pct_x.is_some());
        let mid = Columns::for_width(50);
        assert!(mid.bar_x.is_none() && mid.pct_x.is_some());
        let narrow = Columns::for_width(30);
        assert!(narrow.bar_x.is_none() && narrow.pct_x.is_none());
        assert_eq!(narrow.name_x, 1 + SIZE_W + GAP);
    }

    #[test]
    fn fit_truncates_with_ellipsis() {
        assert_eq!(fit("hello", 10), "hello");
        assert_eq!(fit("hello world", 6), "hello…");
        assert_eq!(fit("héllo", 3), "hé…");
        assert_eq!(fit("x", 0), "");
        assert_eq!(fit_tail("/a/b/c/d", 5), "…/c/d");
        assert_eq!(fit_tail("abc", 5), "abc");
    }

    #[test]
    fn duration_formats() {
        assert_eq!(format_duration(Duration::from_millis(3210)), "3.2 s");
        assert_eq!(format_duration(Duration::from_secs(125)), "2m 05s");
    }

    #[test]
    fn bar_fills_proportionally() {
        let full: String = bar_spans(1.0).iter().map(|s| s.content.to_string()).collect();
        assert_eq!(full, "█".repeat(10));
        let half: String = bar_spans(0.5).iter().map(|s| s.content.to_string()).collect();
        assert_eq!(half, format!("{}{}", "█".repeat(5), "░".repeat(5)));
        let bit: String = bar_spans(0.05).iter().map(|s| s.content.to_string()).collect();
        assert_eq!(bit, format!("▌{}", "░".repeat(9)));
        let none: String = bar_spans(0.0).iter().map(|s| s.content.to_string()).collect();
        assert_eq!(none, "░".repeat(10));
        for share in [0.0, 0.01, 0.33, 0.5, 0.99, 1.0] {
            let w: usize = bar_spans(share).iter().map(|s| s.content.width()).sum();
            assert_eq!(w, 10, "share {share}");
        }
    }

    #[test]
    fn ready_screen_shows_rows_and_summary() {
        let mut app = App::with_tree(PathBuf::from("/root"), sample());
        let lines = render(&mut app, 80, 10);
        assert!(contains(&lines, "/root"), "{lines:#?}");
        assert!(contains(&lines, "1.5 KiB · 4 files · 2 dirs"), "{lines:#?}");
        assert!(contains(&lines, "SIZE"), "{lines:#?}");
        assert!(contains(&lines, "▸ a/"), "{lines:#?}");
        assert!(contains(&lines, "66.7%"), "{lines:#?}");
        assert!(contains(&lines, "└─   b/") || contains(&lines, "└─ ▸ b/"), "{lines:#?}");
        assert!(contains(&lines, "sort: size"), "{lines:#?}");
        // rows drawn in order a, top, b
        let a = lines.iter().position(|l| l.contains("a/")).unwrap();
        let top = lines.iter().position(|l| l.contains(" top")).unwrap();
        let b = lines.iter().position(|l| l.contains("b/")).unwrap();
        assert!(a < top && top < b);
    }

    #[test]
    fn expanded_rows_render_guides() {
        let mut app = App::with_tree(PathBuf::from("/root"), sample());
        app.expand_selected();
        let lines = render(&mut app, 80, 10);
        assert!(contains(&lines, "▾ a/"), "{lines:#?}");
        assert!(contains(&lines, "│  ├─   big"), "{lines:#?}");
        assert!(contains(&lines, "│  └─   small"), "{lines:#?}");
    }

    #[test]
    fn long_names_are_truncated_to_width() {
        let mut t = Tree::new("/root");
        t.add_child(0, Node::new("x".repeat(200), Kind::File, 1));
        t.finalize();
        let mut app = App::with_tree(PathBuf::from("/root"), t);
        let lines = render(&mut app, 60, 6);
        assert!(contains(&lines, "…"), "{lines:#?}");
        assert!(lines.iter().all(|l| l.width() <= 60));
        assert!(lines.iter().all(|l| !l.contains("xxxx│")), "{lines:#?}");
    }

    #[test]
    fn scanning_and_failed_screens() {
        let mut app = App::new(PathBuf::from("/scan/me"));
        app.apply(ScanEvent::Progress {
            dirs: 12,
            files: 3456,
            bytes: 7 * 1024 * 1024,
            current: PathBuf::from("/scan/me/deep"),
        });
        let lines = render(&mut app, 80, 12);
        assert!(contains(&lines, "Scanning"), "{lines:#?}");
        assert!(contains(&lines, "12 dirs · 3,456 files · 7.0 MiB"), "{lines:#?}");
        assert!(contains(&lines, "/scan/me/deep"), "{lines:#?}");

        app.apply(ScanEvent::Error("no such directory".into()));
        let lines = render(&mut app, 80, 12);
        assert!(contains(&lines, "Scan failed"), "{lines:#?}");
        assert!(contains(&lines, "no such directory"), "{lines:#?}");
    }

    #[test]
    fn help_overlay_and_unreadable_marker() {
        let mut t = sample();
        t.node_mut(3).error = Some("permission denied".into());
        let mut app = App::with_tree(PathBuf::from("/root"), t);
        let lines = render(&mut app, 100, 12);
        assert!(contains(&lines, "⚠ 1 unreadable"), "{lines:#?}");
        app.expand_selected();
        let lines = render(&mut app, 100, 12);
        assert!(contains(&lines, "⚠ permission denied"), "{lines:#?}");
        app.show_help = true;
        let lines = render(&mut app, 100, 20);
        assert!(contains(&lines, "Keys"), "{lines:#?}");
        assert!(contains(&lines, "zoom into directory"), "{lines:#?}");
    }

    #[test]
    fn tiny_terminal_does_not_panic() {
        let mut app = App::with_tree(PathBuf::from("/root"), sample());
        for (w, h) in [(1, 1), (5, 2), (20, 3), (39, 4), (59, 5)] {
            let _ = render(&mut app, w, h);
        }
        app.show_help = true;
        let _ = render(&mut app, 10, 3);
    }

    #[test]
    fn scrolling_follows_selection() {
        let mut t = Tree::new("/root");
        for i in 0..50 {
            t.add_child(0, Node::new(format!("f{i:02}"), Kind::File, 100 - i));
        }
        t.finalize();
        let mut app = App::with_tree(PathBuf::from("/root"), t);
        app.end();
        let lines = render(&mut app, 80, 10);
        assert!(contains(&lines, "f49"), "{lines:#?}");
        assert!(!contains(&lines, "f00"), "{lines:#?}");
        assert!(app.offset > 0);
    }
}
