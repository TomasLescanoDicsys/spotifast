//! The pro analyser's window: a large oscilloscope, a spectrum and a
//! spectrogram, each with its scales and a readout under the pointer, and
//! the levels read out above them. It shows the sound playing on this
//! computer, post-equalizer and pre-volume like every visualiser; a paused
//! song holds the picture still so it can be read.

use std::time::Instant;

use egui::{
    Align2, Color32, CursorIcon, FontId, Key, Modifiers, Painter, Pos2, Rect, Response, Sense,
    Shape, Stroke, StrokeKind, TextureId, TextureOptions, Ui, pos2, vec2,
};

use crate::analyser::{
    DIVISIONS, FLOOR_DB, GAINS, HISTORY, HOP, ROWS, TIME_BASES, band_level, bin_hz, decibels,
    hz_at, note, position_of,
};
use crate::app::{App, NowPlaying};
use crate::i18n::gettext;
use crate::model::Action;

/// The window's size when it first opens, and the least it shrinks to.
const SIZE: [f32; 2] = [1120.0, 800.0];
const MIN_SIZE: [f32; 2] = [760.0, 560.0];

/// An instrument's screen: near-black glass, a dim green grid, a phosphor
/// trace, amber peaks, and a sweep from cyan bass to violet treble.
const GLASS: Color32 = Color32::from_rgb(7, 10, 8);
const PANEL: Color32 = Color32::from_rgb(12, 17, 14);
const EDGE: Color32 = Color32::from_rgb(32, 46, 38);
const GRID: Color32 = Color32::from_rgb(22, 33, 27);
const GRID_MAJOR: Color32 = Color32::from_rgb(40, 60, 48);
const LABEL: Color32 = Color32::from_rgb(104, 134, 116);
const TEXT: Color32 = Color32::from_rgb(206, 232, 214);
const TRACE: Color32 = Color32::from_rgb(70, 255, 150);
const PEAK: Color32 = Color32::from_rgb(255, 190, 70);
const CLIP: Color32 = Color32::from_rgb(255, 82, 82);
const BASS: Color32 = Color32::from_rgb(46, 210, 255);
const TREBLE: Color32 = Color32::from_rgb(178, 112, 255);

/// The frequencies the axes name.
const NAMED_HZ: [f32; 10] = [
    20.0, 50.0, 100.0, 200.0, 500.0, 1_000.0, 2_000.0, 5_000.0, 10_000.0, 20_000.0,
];
/// The spectrum's scale, a line every 12 dB.
const DB_STEP: f32 = 12.0;
/// The scope's rows of divisions.
const ROWS_OF_DIVISIONS: usize = 8;
/// The level meters' range, in dBFS.
const METER_FLOOR: f32 = -60.0;
/// The spectrogram's colours from the floor to full scale, after the
/// perceptually even "inferno" map.
const INFERNO: [[u8; 3]; 9] = [
    [0, 0, 4],
    [31, 12, 72],
    [85, 15, 109],
    [136, 34, 106],
    [186, 54, 85],
    [227, 89, 51],
    [249, 140, 10],
    [249, 201, 50],
    [252, 255, 164],
];
/// Wheel travel that moves a setting one step; a mouse notch is 120.
const SCROLL_STEP: f32 = 60.0;
/// Room each panel keeps around its plot for the title and the scales:
/// left, top, right, bottom.
const MARGINS: [f32; 4] = [58.0, 34.0, 16.0, 24.0];

/// Shows the analyser's window while it is open. Its close button, Esc,
/// and the shortcut close it again.
pub fn show(app: &mut App, ctx: &egui::Context) {
    if !app.settings.pro_analyser_open {
        return;
    }
    let title = gettext(app.locale, "Pro analyser");
    let builder = egui::ViewportBuilder::default()
        .with_title(format!("{title} · Spotifast"))
        .with_inner_size(SIZE)
        .with_min_inner_size(MIN_SIZE);
    ctx.show_viewport_immediate(
        egui::ViewportId::from_hash_of("spotifast-pro-analyser"),
        builder,
        |ui, _| window(app, ui),
    );
}

fn window(app: &mut App, ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    let close = ctx.input_mut(|input| {
        input.viewport().close_requested()
            | input.consume_key(Modifiers::NONE, Key::Escape)
            | input.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::O)
    });
    if close {
        app.actions.push(Action::ToggleProAnalyser);
    }
    // Space plays and pauses here as it does in the main window.
    if ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Space)) {
        app.actions.push(Action::TogglePlay);
    }
    let now = app.now_playing();
    let sounding = now
        .as_ref()
        .is_some_and(|now| (now.playing || now.loading) && now.local);
    if sounding {
        app.pro_analyser.step(&app.winamp.tap, Instant::now());
        ctx.request_repaint();
    } else {
        app.pro_analyser.pause();
    }
    let texture = spectrogram_texture(app, &ctx);

    let rect = ui.max_rect();
    let painter = ui.painter().clone();
    painter.rect_filled(rect, 0.0, GLASS);
    let inner = rect.shrink(14.0);
    let header = Rect::from_min_size(inner.min, vec2(inner.width(), 64.0));
    header_block(app, &painter, header, now.as_ref(), sounding);

    let body = Rect::from_min_max(pos2(inner.left(), header.bottom() + 12.0), inner.max);
    let gap = 10.0;
    let usable = body.height() - 2.0 * gap;
    let scope_rect = Rect::from_min_size(body.min, vec2(body.width(), usable * 0.38));
    let spectrum_rect = Rect::from_min_size(
        pos2(body.left(), scope_rect.bottom() + gap),
        vec2(body.width(), usable * 0.34),
    );
    let spectrogram_rect =
        Rect::from_min_max(pos2(body.left(), spectrum_rect.bottom() + gap), body.max);
    scope(app, ui, &painter, scope_rect);
    spectrum(app, ui, &painter, spectrum_rect);
    spectrogram(app, ui, &painter, spectrogram_rect, texture);
}

/// The window's head: what is playing on the left, the levels on the right.
fn header_block(
    app: &App,
    painter: &Painter,
    rect: Rect,
    now: Option<&NowPlaying>,
    sounding: bool,
) {
    let locale = app.locale;
    let card = 152.0;
    let gap = 8.0;
    let cards_left = rect.right() - 4.0 * card - 3.0 * gap;

    painter.text(
        rect.left_top() + vec2(0.0, 4.0),
        Align2::LEFT_TOP,
        gettext(locale, "Pro analyser").to_uppercase(),
        FontId::proportional(11.0),
        LABEL,
    );
    let song = now.map_or_else(String::new, |now| {
        if now.subtitle.is_empty() {
            now.title.clone()
        } else {
            format!("{}  ·  {}", now.title, now.subtitle)
        }
    });
    let width = (cards_left - rect.left() - 16.0).max(40.0);
    let song_rect = Rect::from_min_size(rect.left_top() + vec2(0.0, 22.0), vec2(width, 22.0));
    let galley = crate::bidi::layout(
        painter,
        &song,
        FontId::proportional(17.0),
        TEXT,
        width,
        1,
        Some(crate::bidi::ELLIPSIS),
    );
    painter.galley(crate::bidi::galley_pos(song_rect, &galley), galley, TEXT);
    if !sounding && now.is_some() {
        let y = rect.top() + 54.0;
        painter.circle_filled(pos2(rect.left() + 4.0, y), 3.5, PEAK);
        painter.text(
            pos2(rect.left() + 14.0, y),
            Align2::LEFT_CENTER,
            gettext(locale, "Paused: the picture holds still"),
            FontId::proportional(11.5),
            PEAK,
        );
    }

    let levels = app.pro_analyser.levels();
    let mut left = cards_left;
    let mut next_card = || {
        let at = Rect::from_min_size(pos2(left, rect.top()), vec2(card, rect.height()));
        left += card + gap;
        at
    };
    let clipping = levels.held > -0.1;
    readout_card(
        painter,
        next_card(),
        &gettext(locale, "Peak"),
        &db_label(levels.peak),
        None,
        if clipping { CLIP } else { TEXT },
        Some((levels.peak, levels.held)),
    );
    readout_card(
        painter,
        next_card(),
        "RMS",
        &db_label(levels.rms),
        None,
        TEXT,
        Some((levels.rms, FLOOR_DB)),
    );
    let crest = if levels.peak > FLOOR_DB {
        format!("{:.1} dB", (levels.peak - levels.rms).max(0.0))
    } else {
        "-".into()
    };
    readout_card(
        painter,
        next_card(),
        &gettext(locale, "Crest factor"),
        &crest,
        None,
        TEXT,
        None,
    );
    let dominant = app.pro_analyser.dominant();
    readout_card(
        painter,
        next_card(),
        &gettext(locale, "Dominant frequency"),
        &dominant.map_or_else(|| "-".into(), hz_precise),
        dominant.and_then(note_label).as_deref(),
        TEXT,
        None,
    );
}

/// One reading in a card: its name, its value, and along the foot either
/// an aside or a meter of `(level, held)`.
fn readout_card(
    painter: &Painter,
    rect: Rect,
    label: &str,
    value: &str,
    aside: Option<&str>,
    colour: Color32,
    meter: Option<(f32, f32)>,
) {
    painter.rect_filled(rect, 8.0, PANEL);
    painter.rect_stroke(rect, 8.0, Stroke::new(1.0, EDGE), StrokeKind::Inside);
    let inner = rect.shrink2(vec2(11.0, 8.0));
    painter.text(
        inner.left_top(),
        Align2::LEFT_TOP,
        label.to_uppercase(),
        FontId::proportional(10.0),
        LABEL,
    );
    painter.text(
        pos2(inner.left(), inner.center().y + 1.0),
        Align2::LEFT_CENTER,
        value,
        FontId::monospace(18.0),
        colour,
    );
    if let Some(aside) = aside {
        painter.text(
            inner.left_bottom() + vec2(0.0, 2.0),
            Align2::LEFT_BOTTOM,
            aside,
            FontId::monospace(11.0),
            PEAK,
        );
    }
    let Some((level, held)) = meter else {
        return;
    };
    let track = Rect::from_min_max(
        pos2(inner.left(), inner.bottom() - 4.0),
        pos2(inner.right(), inner.bottom()),
    );
    painter.rect_filled(track, 2.0, GRID);
    let reach = |db: f32| ((db - METER_FLOOR) / -METER_FLOOR).clamp(0.0, 1.0);
    let filled = Rect::from_min_max(
        track.min,
        pos2(track.left() + track.width() * reach(level), track.bottom()),
    );
    let fill = if level > -3.0 {
        CLIP
    } else if level > -12.0 {
        PEAK
    } else {
        TRACE
    };
    painter.rect_filled(filled, 2.0, fill);
    if held > METER_FLOOR {
        let x = track.left() + track.width() * reach(held);
        painter.line_segment(
            [pos2(x, track.top() - 2.0), pos2(x, track.bottom() + 1.0)],
            Stroke::new(2.0, PEAK),
        );
    }
}

/// A panel and its title. Returns the plot inside the room for the scales.
fn panel(painter: &Painter, rect: Rect, title: &str, margins: [f32; 4]) -> Rect {
    painter.rect_filled(rect, 8.0, PANEL);
    painter.rect_stroke(rect, 8.0, Stroke::new(1.0, EDGE), StrokeKind::Inside);
    painter.text(
        rect.left_top() + vec2(12.0, 10.0),
        Align2::LEFT_TOP,
        title.to_uppercase(),
        FontId::proportional(11.0),
        LABEL,
    );
    Rect::from_min_max(
        pos2(rect.left() + margins[0], rect.top() + margins[1]),
        pos2(rect.right() - margins[2], rect.bottom() - margins[3]),
    )
}

/// A small setting at a panel's head, its right edge at `right`. Returns
/// its left edge and its response.
fn chip(ui: &Ui, painter: &Painter, right: f32, y: f32, text: String, id: &str) -> (f32, Response) {
    let galley = painter.layout_no_wrap(text, FontId::monospace(11.0), TEXT);
    let size = galley.size() + vec2(14.0, 6.0);
    let rect = Rect::from_min_size(pos2(right - size.x, y - size.y / 2.0), size);
    let response = ui.interact(rect, ui.id().with(id), Sense::click());
    let fill = if response.hovered() { GRID_MAJOR } else { GRID };
    painter.rect_filled(rect, 4.0, fill);
    painter.rect_stroke(rect, 4.0, Stroke::new(1.0, EDGE), StrokeKind::Inside);
    painter.galley(rect.center() - galley.size() / 2.0, galley, TEXT);
    if response.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    (rect.left() - 6.0, response)
}

/// How far a chip moves its setting: on with a click, back with a
/// right-click.
fn chip_step(response: &Response) -> i32 {
    i32::from(response.clicked()) - i32::from(response.secondary_clicked())
}

/// Whole steps of wheel travel over `response`, kept between frames so a
/// trackpad's small movements add up.
fn wheel_steps(ui: &Ui, response: &Response, id: &str) -> (i32, bool) {
    if !response.hovered() {
        return (0, false);
    }
    let (delta, shift) = ui.input(|input| (input.smooth_scroll_delta, input.modifiers.shift));
    let id = ui.id().with(id);
    let steps = ui.data_mut(|data| {
        let travel = data.get_temp_mut_or_default::<f32>(id);
        *travel += delta.x + delta.y;
        let steps = (*travel / SCROLL_STEP).trunc();
        *travel -= steps * SCROLL_STEP;
        steps as i32
    });
    (steps, shift)
}

fn step_index(index: usize, step: i32, len: usize) -> usize {
    (index as i32 + step).clamp(0, len as i32 - 1) as usize
}

/// A box of text by the pointer, kept inside the plot.
fn readout(painter: &Painter, plot: Rect, pointer: Pos2, text: String) {
    let galley = painter.layout_no_wrap(text, FontId::monospace(11.5), TEXT);
    let size = galley.size() + vec2(12.0, 8.0);
    let mut min = pointer + vec2(14.0, -size.y - 10.0);
    if min.x + size.x > plot.right() {
        min.x = pointer.x - 14.0 - size.x;
    }
    if min.y < plot.top() {
        min.y = pointer.y + 14.0;
    }
    let rect = Rect::from_min_size(min, size);
    painter.rect_filled(rect, 4.0, Color32::from_rgba_unmultiplied(4, 8, 6, 235));
    painter.rect_stroke(rect, 4.0, Stroke::new(1.0, GRID_MAJOR), StrokeKind::Inside);
    painter.galley(rect.min + vec2(6.0, 4.0), galley, TEXT);
}

/// The scope: the wave over ten divisions at the chosen time base, held
/// still on a rising edge when there is one.
fn scope(app: &mut App, ui: &Ui, painter: &Painter, rect: Rect) {
    let locale = app.locale;
    let plot = panel(painter, rect, &gettext(locale, "Oscilloscope"), MARGINS);
    let hint = gettext(
        locale,
        "Click to step, right-click to step back, or scroll over the scope",
    );
    let pro = &mut app.pro_analyser;
    let head = rect.top() + 16.0;
    let (left, gain) = chip(
        ui,
        painter,
        rect.right() - 12.0,
        head,
        format!("×{:.0}", GAINS[pro.gain]),
        "scope-gain",
    );
    let (left, base) = chip(
        ui,
        painter,
        left,
        head,
        format!("{}/div", duration_label(TIME_BASES[pro.time_base])),
        "scope-base",
    );
    let plot_response = ui.interact(plot, ui.id().with("scope-plot"), Sense::hover());
    let (wheel, shift) = wheel_steps(ui, &plot_response, "scope-wheel");
    // The wheel zooms in on the wave: up shortens the time base, or with
    // Shift raises the gain.
    let (base_step, gain_step) = if shift { (0, wheel) } else { (-wheel, 0) };
    pro.time_base = step_index(
        pro.time_base,
        chip_step(&base) + base_step,
        TIME_BASES.len(),
    );
    pro.gain = step_index(pro.gain, chip_step(&gain) + gain_step, GAINS.len());
    base.on_hover_text(hint.as_ref());
    gain.on_hover_text(hint.as_ref());

    let (trace, triggered) = pro.trace();
    painter.text(
        pos2(left, head),
        Align2::RIGHT_CENTER,
        if triggered { "TRIG ↑ 0" } else { "FREE RUN" },
        FontId::monospace(11.0),
        if triggered { TRACE } else { LABEL },
    );

    let gain = GAINS[pro.gain];
    let base = TIME_BASES[pro.time_base];
    // The graticule: ten divisions across, eight down, with fine ticks
    // along the centre lines as an instrument's screen has.
    for i in 0..=DIVISIONS {
        let x = plot.left() + plot.width() * i as f32 / DIVISIONS as f32;
        let colour = if i == 0 || i == DIVISIONS {
            GRID_MAJOR
        } else {
            GRID
        };
        painter.line_segment(
            [pos2(x, plot.top()), pos2(x, plot.bottom())],
            Stroke::new(1.0, colour),
        );
    }
    for j in 0..=ROWS_OF_DIVISIONS {
        let y = plot.top() + plot.height() * j as f32 / ROWS_OF_DIVISIONS as f32;
        let colour = if j == 0 || j == ROWS_OF_DIVISIONS || j == ROWS_OF_DIVISIONS / 2 {
            GRID_MAJOR
        } else {
            GRID
        };
        painter.line_segment(
            [pos2(plot.left(), y), pos2(plot.right(), y)],
            Stroke::new(1.0, colour),
        );
    }
    let centre = plot.center();
    for k in 0..=DIVISIONS * 5 {
        let x = plot.left() + plot.width() * k as f32 / (DIVISIONS * 5) as f32;
        painter.line_segment(
            [pos2(x, centre.y - 3.0), pos2(x, centre.y + 3.0)],
            Stroke::new(1.0, GRID_MAJOR),
        );
    }
    for k in 0..=ROWS_OF_DIVISIONS * 5 {
        let y = plot.top() + plot.height() * k as f32 / (ROWS_OF_DIVISIONS * 5) as f32;
        painter.line_segment(
            [pos2(centre.x - 3.0, y), pos2(centre.x + 3.0, y)],
            Stroke::new(1.0, GRID_MAJOR),
        );
    }
    for j in (0..=ROWS_OF_DIVISIONS).step_by(2) {
        let y = plot.top() + plot.height() * j as f32 / ROWS_OF_DIVISIONS as f32;
        let value = (1.0 - j as f32 / (ROWS_OF_DIVISIONS / 2) as f32) / gain;
        let text = if value == 0.0 {
            "0".to_owned()
        } else {
            format!("{value:+.2}")
        };
        painter.text(
            pos2(plot.left() - 8.0, y),
            Align2::RIGHT_CENTER,
            text,
            FontId::monospace(10.5),
            LABEL,
        );
    }
    for i in (0..=DIVISIONS).step_by(2) {
        let x = plot.left() + plot.width() * i as f32 / DIVISIONS as f32;
        painter.text(
            pos2(x, plot.bottom() + 5.0),
            Align2::CENTER_TOP,
            duration_label(base * i as f32),
            FontId::monospace(10.5),
            LABEL,
        );
    }

    let clip = painter.with_clip_rect(plot.expand(1.0));
    let y_of = |sample: f32| centre.y - (sample * gain).clamp(-1.02, 1.02) * plot.height() / 2.0;
    let width = plot.width();
    let count = trace.len();
    let points: Vec<Pos2> = if count as f32 > width {
        // More samples than pixels: each column spans its samples' lowest
        // to highest, so no peak goes missing between pixels.
        let columns = (width as usize).max(1);
        (0..columns)
            .flat_map(|column| {
                let from = column * count / columns;
                let to = ((column + 1) * count / columns).max(from + 1).min(count);
                let (low, high) = trace[from..to]
                    .iter()
                    .fold((f32::MAX, f32::MIN), |(low, high), sample| {
                        (low.min(*sample), high.max(*sample))
                    });
                let x = plot.left() + column as f32 + 0.5;
                let (first, second) = if column % 2 == 0 {
                    (high, low)
                } else {
                    (low, high)
                };
                [pos2(x, y_of(first)), pos2(x, y_of(second))]
            })
            .collect()
    } else {
        let last = count.saturating_sub(1).max(1) as f32;
        trace
            .iter()
            .enumerate()
            .map(|(i, sample)| pos2(plot.left() + width * i as f32 / last, y_of(*sample)))
            .collect()
    };
    clip.add(Shape::line(
        points.clone(),
        Stroke::new(6.0, TRACE.gamma_multiply(0.10)),
    ));
    clip.add(Shape::line(points.clone(), Stroke::new(1.6, TRACE)));
    // Few enough samples to see one by one: mark each.
    if (count as f32) <= width / 6.0 {
        for point in &points {
            clip.circle_filled(*point, 2.4, TRACE);
        }
    }

    if let Some(pointer) = plot_response.hover_pos() {
        ui.ctx().set_cursor_icon(CursorIcon::Crosshair);
        let t = ((pointer.x - plot.left()) / width).clamp(0.0, 1.0);
        let index = ((t * count.saturating_sub(1) as f32).round() as usize).min(count - 1);
        let sample = trace[index];
        let x = plot.left() + width * index as f32 / count.saturating_sub(1).max(1) as f32;
        clip.line_segment(
            [pos2(x, plot.top()), pos2(x, plot.bottom())],
            Stroke::new(1.0, LABEL),
        );
        clip.circle_filled(pos2(x, y_of(sample)), 3.5, PEAK);
        let at = base * DIVISIONS as f32 * index as f32 / count.saturating_sub(1).max(1) as f32;
        readout(
            painter,
            plot,
            pointer,
            format!(
                "{}   {:+.4}   {}",
                duration_label(at),
                sample,
                db_label(decibels(sample.abs()))
            ),
        );
    }
}

/// The spectrum: dBFS against a logarithmic frequency axis, the held peaks
/// above it and the dominant frequency marked.
fn spectrum(app: &App, ui: &Ui, painter: &Painter, rect: Rect) {
    let locale = app.locale;
    let plot = panel(
        painter,
        rect,
        &gettext(locale, "Spectrum analyser"),
        MARGINS,
    );
    painter.text(
        pos2(rect.right() - 14.0, rect.top() + 16.0),
        Align2::RIGHT_CENTER,
        format!(
            "dBFS · FFT {} · {:.1} Hz/bin",
            crate::analyser::FFT_SIZE,
            bin_hz()
        ),
        FontId::monospace(11.0),
        LABEL,
    );
    frequency_grid(painter, plot, false);
    let mut level = 0.0;
    while level >= FLOOR_DB {
        let y = y_of_db(plot, level);
        let colour = if level == 0.0 { GRID_MAJOR } else { GRID };
        painter.line_segment(
            [pos2(plot.left(), y), pos2(plot.right(), y)],
            Stroke::new(1.0, colour),
        );
        painter.text(
            pos2(plot.left() - 8.0, y),
            Align2::RIGHT_CENTER,
            format!("{level:.0}"),
            FontId::monospace(10.5),
            LABEL,
        );
        level -= DB_STEP;
    }

    let pro = &app.pro_analyser;
    let columns = (plot.width() as usize).max(2);
    let band = |column: usize, levels: &[f32]| {
        let t = column as f32 / columns as f32;
        let half = 0.5 / columns as f32;
        band_level(
            levels,
            hz_at((t - half).max(0.0)),
            hz_at((t + half).min(1.0)),
        )
    };
    let x_of = |column: usize| plot.left() + plot.width() * column as f32 / columns as f32;
    let clip = painter.with_clip_rect(plot);
    let mut fill = egui::Mesh::default();
    let mut line = Vec::with_capacity(columns + 1);
    for column in 0..=columns {
        let colour = lerp(BASS, TREBLE, column as f32 / columns as f32);
        let top = pos2(x_of(column), y_of_db(plot, band(column, pro.spectrum())));
        let base = fill.vertices.len() as u32;
        fill.colored_vertex(top, colour.gamma_multiply(0.42));
        fill.colored_vertex(pos2(top.x, plot.bottom()), colour.gamma_multiply(0.03));
        if column > 0 {
            fill.add_triangle(base - 2, base - 1, base);
            fill.add_triangle(base - 1, base, base + 1);
        }
        line.push((top, colour));
    }
    clip.add(Shape::mesh(fill));
    for pair in line.windows(2) {
        clip.line_segment([pair[0].0, pair[1].0], Stroke::new(1.6, pair[1].1));
    }
    let peaks: Vec<Pos2> = (0..=columns)
        .map(|column| pos2(x_of(column), y_of_db(plot, band(column, pro.peaks()))))
        .collect();
    clip.add(Shape::line(
        peaks,
        Stroke::new(1.0, PEAK.gamma_multiply(0.8)),
    ));

    if let Some(hz) = pro.dominant() {
        let x = plot.left() + plot.width() * position_of(hz);
        let level = band_level(pro.spectrum(), hz * 0.995, hz * 1.005);
        let y = y_of_db(plot, level) - 6.0;
        clip.add(Shape::convex_polygon(
            vec![pos2(x - 5.0, y - 8.0), pos2(x + 5.0, y - 8.0), pos2(x, y)],
            PEAK,
            Stroke::NONE,
        ));
        let text = match note_label(hz) {
            Some(note) => format!("{}  {note}", hz_precise(hz)),
            None => hz_precise(hz),
        };
        let anchor = if position_of(hz) > 0.85 {
            Align2::RIGHT_BOTTOM
        } else {
            Align2::LEFT_BOTTOM
        };
        clip.text(
            pos2(x, (y - 10.0).max(plot.top() + 14.0)),
            anchor,
            text,
            FontId::monospace(11.0),
            PEAK,
        );
    }

    let response = ui.interact(plot, ui.id().with("spectrum-plot"), Sense::hover());
    if let Some(pointer) = response.hover_pos() {
        ui.ctx().set_cursor_icon(CursorIcon::Crosshair);
        let t = ((pointer.x - plot.left()) / plot.width()).clamp(0.0, 1.0);
        let column = (t * columns as f32).round() as usize;
        let hz = hz_at(t);
        let level = band(column, pro.spectrum());
        let y = y_of_db(plot, level);
        clip.line_segment(
            [pos2(pointer.x, plot.top()), pos2(pointer.x, plot.bottom())],
            Stroke::new(1.0, LABEL),
        );
        clip.line_segment(
            [pos2(plot.left(), y), pos2(plot.right(), y)],
            Stroke::new(1.0, GRID_MAJOR),
        );
        clip.circle_filled(pos2(pointer.x, y), 3.5, PEAK);
        let note = note_label(hz).unwrap_or_default();
        readout(
            painter,
            plot,
            pointer,
            format!("{}   {note}   {}", hz_precise(hz), db_label(level)),
        );
    }
}

/// The spectrogram: the last ten seconds, newest on the right, bass at the
/// foot, brighter for louder, with its colour scale beside it.
fn spectrogram(app: &App, ui: &Ui, painter: &Painter, rect: Rect, texture: TextureId) {
    let locale = app.locale;
    let mut margins = MARGINS;
    margins[2] = 54.0;
    let plot = panel(painter, rect, &gettext(locale, "Spectrogram"), margins);
    let pro = &app.pro_analyser;
    // The ring's oldest column is the next one written: from there to the
    // end, then from the start, reads left to right in time.
    let next = pro.next_column();
    let split = (HISTORY - next) as f32 / HISTORY as f32;
    let u = next as f32 / HISTORY as f32;
    let seam = plot.left() + plot.width() * split;
    painter.image(
        texture,
        Rect::from_min_max(plot.min, pos2(seam, plot.bottom())),
        Rect::from_min_max(pos2(u, 0.0), pos2(1.0, 1.0)),
        Color32::WHITE,
    );
    if next > 0 {
        painter.image(
            texture,
            Rect::from_min_max(pos2(seam, plot.top()), plot.max),
            Rect::from_min_max(pos2(0.0, 0.0), pos2(u, 1.0)),
            Color32::WHITE,
        );
    }
    frequency_grid(painter, plot, true);
    let seconds = (HISTORY as u32 * HOP.as_millis() as u32) / 1000;
    for second in (0..=seconds).step_by(2) {
        let x = plot.right() - plot.width() * second as f32 / seconds as f32;
        painter.line_segment(
            [pos2(x, plot.top()), pos2(x, plot.bottom())],
            Stroke::new(1.0, Color32::from_white_alpha(18)),
        );
        let label = if second == 0 {
            "0 s".to_owned()
        } else {
            format!("-{second} s")
        };
        painter.text(
            pos2(x, plot.bottom() + 5.0),
            Align2::CENTER_TOP,
            label,
            FontId::monospace(10.5),
            LABEL,
        );
    }

    // The colour scale, from full scale at the top to the floor.
    let bar = Rect::from_min_max(
        pos2(plot.right() + 12.0, plot.top()),
        pos2(plot.right() + 22.0, plot.bottom()),
    );
    let mut mesh = egui::Mesh::default();
    let steps = 32;
    for step in 0..=steps {
        let t = step as f32 / steps as f32;
        let y = bar.top() + bar.height() * t;
        let colour = inferno(((1.0 - t) * 255.0).round() as u8);
        let base = mesh.vertices.len() as u32;
        mesh.colored_vertex(pos2(bar.left(), y), colour);
        mesh.colored_vertex(pos2(bar.right(), y), colour);
        if step > 0 {
            mesh.add_triangle(base - 2, base - 1, base);
            mesh.add_triangle(base - 1, base, base + 1);
        }
    }
    painter.add(Shape::mesh(mesh));
    painter.rect_stroke(bar, 0.0, Stroke::new(1.0, EDGE), StrokeKind::Outside);
    for (t, label) in [(0.0, "0"), (0.5, "-48"), (1.0, "-96")] {
        painter.text(
            pos2(bar.right() + 4.0, bar.top() + bar.height() * t),
            Align2::LEFT_CENTER,
            label,
            FontId::monospace(9.5),
            LABEL,
        );
    }

    let response = ui.interact(plot, ui.id().with("spectrogram-plot"), Sense::hover());
    if let Some(pointer) = response.hover_pos() {
        ui.ctx().set_cursor_icon(CursorIcon::Crosshair);
        let across = ((pointer.x - plot.left()) / plot.width()).clamp(0.0, 0.9999);
        let up = ((plot.bottom() - pointer.y) / plot.height()).clamp(0.0, 0.9999);
        let column = (next + (across * HISTORY as f32) as usize) % HISTORY;
        let row = (up * ROWS as f32) as usize;
        let level = f32::from(pro.column(column)[row]) / 255.0 * -FLOOR_DB + FLOOR_DB;
        let hz = hz_at((row as f32 + 0.5) / ROWS as f32);
        let ago = (1.0 - across) * seconds as f32;
        let clip = painter.with_clip_rect(plot);
        clip.line_segment(
            [pos2(pointer.x, plot.top()), pos2(pointer.x, plot.bottom())],
            Stroke::new(1.0, Color32::from_white_alpha(60)),
        );
        clip.line_segment(
            [pos2(plot.left(), pointer.y), pos2(plot.right(), pointer.y)],
            Stroke::new(1.0, Color32::from_white_alpha(60)),
        );
        let note = note_label(hz).unwrap_or_default();
        readout(
            painter,
            plot,
            pointer,
            format!(
                "-{ago:.2} s   {}   {note}   {}",
                hz_precise(hz),
                db_label(level)
            ),
        );
    }
}

/// The logarithmic frequency axis' lines and names along `plot`, across
/// it for the spectrum or up it for the spectrogram.
fn frequency_grid(painter: &Painter, plot: Rect, upright: bool) {
    let mut decade = 10.0;
    while decade < 20_000.0 {
        for multiple in 1..10 {
            let hz = decade * multiple as f32;
            if !(20.0..=20_000.0).contains(&hz) {
                continue;
            }
            let t = position_of(hz);
            let colour = if multiple == 1 { GRID_MAJOR } else { GRID };
            let colour = if upright {
                Color32::from_white_alpha(if multiple == 1 { 34 } else { 12 })
            } else {
                colour
            };
            let line = if upright {
                let y = plot.bottom() - plot.height() * t;
                [pos2(plot.left(), y), pos2(plot.right(), y)]
            } else {
                let x = plot.left() + plot.width() * t;
                [pos2(x, plot.top()), pos2(x, plot.bottom())]
            };
            painter.line_segment(line, Stroke::new(1.0, colour));
        }
        decade *= 10.0;
    }
    for hz in NAMED_HZ {
        let t = position_of(hz);
        let label = if hz >= 1_000.0 {
            format!("{:.0}k", hz / 1_000.0)
        } else {
            format!("{hz:.0}")
        };
        if upright {
            let y = plot.bottom() - plot.height() * t;
            painter.text(
                pos2(plot.left() - 8.0, y),
                Align2::RIGHT_CENTER,
                label,
                FontId::monospace(10.5),
                LABEL,
            );
        } else {
            let x = plot.left() + plot.width() * t;
            painter.text(
                pos2(x, plot.bottom() + 5.0),
                Align2::CENTER_TOP,
                label,
                FontId::monospace(10.5),
                LABEL,
            );
        }
    }
}

/// The spectrogram's texture, made in full when the window has none and
/// then brought up to date a column at a time.
fn spectrogram_texture(app: &mut App, ctx: &egui::Context) -> TextureId {
    let fresh = app.pro_analyser.take_fresh();
    if let Some(texture) = app.pro_spectrogram.as_mut() {
        for index in fresh {
            texture.set_partial(
                [index, 0],
                column_image(app.pro_analyser.column(index)),
                TextureOptions::LINEAR,
            );
        }
        return texture.id();
    }
    let mut pixels = vec![Color32::BLACK; HISTORY * ROWS];
    for column in 0..HISTORY {
        for (row, level) in app.pro_analyser.column(column).iter().enumerate() {
            pixels[(ROWS - 1 - row) * HISTORY + column] = inferno(*level);
        }
    }
    let texture = ctx.load_texture(
        "pro-analyser-spectrogram",
        egui::ColorImage::new([HISTORY, ROWS], pixels),
        TextureOptions::LINEAR,
    );
    let id = texture.id();
    app.pro_spectrogram = Some(texture);
    id
}

/// One column of the spectrogram as an image one pixel wide, treble on top.
fn column_image(levels: &[u8]) -> egui::ColorImage {
    egui::ColorImage::new(
        [1, levels.len()],
        levels.iter().rev().map(|level| inferno(*level)).collect(),
    )
}

/// The spectrogram's colour for a level from 0 (the floor) to 255.
fn inferno(level: u8) -> Color32 {
    let t = f32::from(level) / 255.0 * (INFERNO.len() - 1) as f32;
    let below = (t.floor() as usize).min(INFERNO.len() - 2);
    let [r0, g0, b0] = INFERNO[below];
    let [r1, g1, b1] = INFERNO[below + 1];
    lerp(
        Color32::from_rgb(r0, g0, b0),
        Color32::from_rgb(r1, g1, b1),
        t - below as f32,
    )
}

fn lerp(from: Color32, to: Color32, t: f32) -> Color32 {
    let mix = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * t).round() as u8;
    Color32::from_rgb(
        mix(from.r(), to.r()),
        mix(from.g(), to.g()),
        mix(from.b(), to.b()),
    )
}

fn y_of_db(plot: Rect, level: f32) -> f32 {
    plot.top() + plot.height() * (level / FLOOR_DB).clamp(0.0, 1.0)
}

/// A time on the scope's axis: microseconds under a millisecond.
fn duration_label(ms: f32) -> String {
    if ms == 0.0 {
        "0".to_owned()
    } else if ms < 1.0 {
        format!("{:.0} µs", ms * 1_000.0)
    } else if ms.fract().abs() > 1e-3 {
        format!("{ms:.1} ms")
    } else {
        format!("{ms:.0} ms")
    }
}

fn hz_precise(hz: f32) -> String {
    if hz >= 1_000.0 {
        format!("{:.2} kHz", hz / 1_000.0)
    } else {
        format!("{hz:.1} Hz")
    }
}

fn note_label(hz: f32) -> Option<String> {
    note(hz).map(|(name, cents)| format!("{name} {cents:+}¢"))
}

fn db_label(level: f32) -> String {
    if level <= FLOOR_DB {
        "-inf dBFS".to_owned()
    } else {
        format!("{level:.1} dBFS")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_read_as_an_instrument_names_them() {
        assert_eq!(duration_label(0.0), "0");
        assert_eq!(duration_label(0.2), "200 µs");
        assert_eq!(duration_label(2.0), "2 ms");
        assert_eq!(duration_label(2.5), "2.5 ms");
        assert_eq!(hz_precise(440.0), "440.0 Hz");
        assert_eq!(hz_precise(12_345.0), "12.35 kHz");
        assert_eq!(note_label(440.0).as_deref(), Some("A4 +0¢"));
        assert_eq!(db_label(FLOOR_DB), "-inf dBFS");
        assert_eq!(db_label(-6.02), "-6.0 dBFS");
    }

    #[test]
    fn the_colour_scale_runs_dark_to_bright() {
        assert_eq!(inferno(0), Color32::from_rgb(0, 0, 4));
        assert_eq!(inferno(255), Color32::from_rgb(252, 255, 164));
        let brightness = |c: Color32| u32::from(c.r()) + u32::from(c.g()) + u32::from(c.b());
        assert!(brightness(inferno(64)) < brightness(inferno(192)));
    }

    #[test]
    fn a_column_image_puts_the_treble_on_top() {
        let image = column_image(&[255, 0, 0]);
        assert_eq!(image.size, [1, 3]);
        assert_eq!(image.pixels[2], inferno(255), "the bass at the foot");
        assert_eq!(image.pixels[0], inferno(0));
    }
}
