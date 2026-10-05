//! The pro analyser's window. On the left, the pictures with their scales
//! and a readout under the pointer: an oscilloscope with a phosphor's
//! afterglow, a spectrum with the note's harmonics in colour, and a
//! spectrogram with the note's line through it. On the right, the
//! instruments: the note, loudness, and the sound's modulations and
//! filtering, each with the formula it measures. It shows the sound
//! playing on this computer, post-equalizer and pre-volume like every
//! visualiser; a paused song holds the picture still so it can be read.

mod cards;

use std::time::Instant;

use egui::{
    Align2, Color32, CursorIcon, FontId, Key, Modifiers, Painter, Pos2, Rect, Response, Sense,
    Shape, Stroke, StrokeKind, TextureId, TextureOptions, Ui, pos2, vec2,
};

use crate::analyser::{
    DIVISIONS, FLOOR_DB, GAINS, HISTORY, HOP, NAMED_CLARITY, ROWS, TIME_BASES, band_level, bin_hz,
    decibels, hz_at, note, position_of,
};
use crate::app::{App, NowPlaying};
use crate::i18n::{Locale, gettext, pgettext};
use crate::model::Action;

/// The window's size when it first opens, and the least it shrinks to.
const SIZE: [f32; 2] = [1400.0, 820.0];
const MIN_SIZE: [f32; 2] = [1080.0, 700.0];

/// An instrument's screen: near-black glass, a dim green grid, a phosphor
/// trace, amber peaks.
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
/// The spectrum itself stays a quiet sea green, so the harmonics stand out.
const SPECTRUM: Color32 = Color32::from_rgb(110, 190, 160);
/// One colour for each harmonic, the fundamental first.
const HARMONIC: [Color32; 8] = [
    Color32::from_rgb(255, 214, 92),
    Color32::from_rgb(255, 140, 64),
    Color32::from_rgb(255, 92, 120),
    Color32::from_rgb(214, 96, 255),
    Color32::from_rgb(124, 124, 255),
    Color32::from_rgb(64, 180, 255),
    Color32::from_rgb(64, 230, 200),
    Color32::from_rgb(150, 255, 110),
];

/// The frequencies the axes name.
const NAMED_HZ: [f32; 10] = [
    20.0, 50.0, 100.0, 200.0, 500.0, 1_000.0, 2_000.0, 5_000.0, 10_000.0, 20_000.0,
];
/// Where the bass ends and the treble begins, as engineers divide the band.
const BASS_TOP_HZ: f32 = 250.0;
const TREBLE_FOOT_HZ: f32 = 4_000.0;
/// The spectrum's scale, a line every 12 dB.
const DB_STEP: f32 = 12.0;
/// The scope's rows of divisions.
const ROWS_OF_DIVISIONS: usize = 8;
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
/// Space between panels, and the instruments' column width.
const GAP: f32 = 10.0;
const INSTRUMENTS_WIDTH: [f32; 2] = [330.0, 400.0];

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
    if let Some(now) = &now {
        app.pro_analyser.follow(&now.uri);
    }
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
    let header = Rect::from_min_size(inner.min, vec2(inner.width(), 30.0));
    header_block(app, &painter, header, now.as_ref(), sounding);

    let body = Rect::from_min_max(pos2(inner.left(), header.bottom() + GAP), inner.max);
    let instruments = (body.width() * 0.27).clamp(INSTRUMENTS_WIDTH[0], INSTRUMENTS_WIDTH[1]);
    let pictures = Rect::from_min_max(
        body.min,
        pos2(body.right() - instruments - GAP, body.bottom()),
    );
    let column = Rect::from_min_max(pos2(pictures.right() + GAP, body.top()), body.max);

    let usable = pictures.height() - 2.0 * GAP;
    let scope_rect = Rect::from_min_size(pictures.min, vec2(pictures.width(), usable * 0.42));
    let spectrum_rect = Rect::from_min_size(
        pos2(pictures.left(), scope_rect.bottom() + GAP),
        vec2(pictures.width(), usable * 0.31),
    );
    let spectrogram_rect = Rect::from_min_max(
        pos2(pictures.left(), spectrum_rect.bottom() + GAP),
        pictures.max,
    );
    scope(app, ui, &painter, scope_rect);
    spectrum(app, ui, &painter, spectrum_rect);
    spectrogram(app, ui, &painter, spectrogram_rect, texture);
    cards::instruments(app, &painter, column);
}

/// The window's head: what is playing, and that the picture holds while
/// the song is paused.
fn header_block(
    app: &App,
    painter: &Painter,
    rect: Rect,
    now: Option<&NowPlaying>,
    sounding: bool,
) {
    let locale = app.locale;
    let title = painter.text(
        pos2(rect.left(), rect.center().y),
        Align2::LEFT_CENTER,
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
    let paused = gettext(locale, "Paused: the picture holds still");
    let paused_width = if !sounding && now.is_some() {
        let width = painter
            .layout_no_wrap(paused.to_string(), FontId::proportional(11.5), PEAK)
            .size()
            .x;
        let x = rect.right() - width;
        painter.circle_filled(pos2(x - 10.0, rect.center().y), 3.5, PEAK);
        painter.text(
            pos2(x, rect.center().y),
            Align2::LEFT_CENTER,
            paused.as_ref(),
            FontId::proportional(11.5),
            PEAK,
        );
        width + 30.0
    } else {
        0.0
    };
    let left = title.right() + 14.0;
    let width = (rect.right() - paused_width - left).max(40.0);
    let galley = crate::bidi::layout(
        painter,
        &song,
        FontId::proportional(16.0),
        TEXT,
        width,
        1,
        Some(crate::bidi::ELLIPSIS),
    );
    let song_rect = Rect::from_min_size(
        pos2(left, rect.center().y - galley.size().y / 2.0),
        vec2(width, galley.size().y),
    );
    painter.galley(crate::bidi::galley_pos(song_rect, &galley), galley, TEXT);
}

/// A panel and its title, with an aside in small type after it. Returns
/// the plot inside the room for the scales.
fn panel(painter: &Painter, rect: Rect, title: &str, aside: &str, margins: [f32; 4]) -> Rect {
    painter.rect_filled(rect, 8.0, PANEL);
    painter.rect_stroke(rect, 8.0, Stroke::new(1.0, EDGE), StrokeKind::Inside);
    let title = painter.text(
        rect.left_top() + vec2(12.0, 10.0),
        Align2::LEFT_TOP,
        title.to_uppercase(),
        FontId::proportional(11.0),
        LABEL,
    );
    if !aside.is_empty() {
        painter.text(
            pos2(title.right() + 12.0, title.center().y),
            Align2::LEFT_CENTER,
            aside,
            FontId::monospace(10.5),
            LABEL.gamma_multiply(0.8),
        );
    }
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
/// trackpad's small movements add up, and whether Shift is held.
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

/// The wave as points across `plot` at `gain`, the edges at full scale.
/// With more samples than pixels each column spans its samples' lowest to
/// highest, so no peak goes missing between pixels.
fn trace_points(trace: &[f32], plot: Rect, gain: f32) -> Vec<Pos2> {
    let centre = plot.center().y;
    let y_of = |sample: f32| centre - (sample * gain).clamp(-1.0, 1.0) * plot.height() / 2.0;
    let width = plot.width();
    let count = trace.len();
    if count as f32 > width {
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
    }
}

/// The scope: the wave over ten divisions at the chosen time base, held
/// still on a rising edge when there is one, its gain fitted to the wave
/// unless one is chosen, with the last few traces fading behind it.
fn scope(app: &mut App, ui: &Ui, painter: &Painter, rect: Rect) {
    let locale = app.locale;
    let plot = panel(
        painter,
        rect,
        &gettext(locale, "Oscilloscope"),
        "x(t)",
        MARGINS,
    );
    let hint = gettext(
        locale,
        "Click to step, right-click to step back, or scroll over the scope",
    );
    let pro = &mut app.pro_analyser;
    let head = rect.top() + 16.0;
    let gain_text = if pro.gain_index() == 0 {
        format!("{} ×{:.1}", gettext(locale, "Auto"), pro.gain())
    } else {
        format!("×{}", GAINS[pro.gain_index()])
    };
    let (left, gain) = chip(
        ui,
        painter,
        rect.right() - 12.0,
        head,
        gain_text,
        "scope-gain",
    );
    let (left, base) = chip(
        ui,
        painter,
        left,
        head,
        format!("{}/div", duration_label(TIME_BASES[pro.time_base()])),
        "scope-base",
    );
    let plot_response = ui.interact(plot, ui.id().with("scope-plot"), Sense::hover());
    let (wheel, shift) = wheel_steps(ui, &plot_response, "scope-wheel");
    // The wheel zooms in on the wave: up shortens the time base, or with
    // Shift raises the gain.
    let (base_step, gain_step) = if shift { (0, wheel) } else { (-wheel, 0) };
    pro.set_time_base(step_index(
        pro.time_base(),
        chip_step(&base) + base_step,
        TIME_BASES.len(),
    ));
    pro.set_gain_index(step_index(
        pro.gain_index(),
        chip_step(&gain) + gain_step,
        GAINS.len(),
    ));
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

    let gain = pro.gain();
    let base = TIME_BASES[pro.time_base()];
    graticule(painter, plot);
    for j in (0..=ROWS_OF_DIVISIONS).step_by(2) {
        let y = plot.top() + plot.height() * j as f32 / ROWS_OF_DIVISIONS as f32;
        let value = (1.0 - j as f32 / (ROWS_OF_DIVISIONS / 2) as f32) / gain;
        let text = if value.abs() < 1e-6 {
            "0".to_owned()
        } else if value.abs() >= 10.0 {
            format!("{value:+.0}")
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
    // The phosphor's afterglow: earlier traces, fainter the older.
    let trails = pro.trails();
    for (age, earlier) in trails.iter().rev().skip(1).enumerate() {
        let fade = 1.0 - (age + 1) as f32 / trails.len().max(1) as f32;
        clip.add(Shape::line(
            trace_points(earlier, plot, gain),
            Stroke::new(1.2, TRACE.gamma_multiply(0.04 + 0.3 * fade * fade)),
        ));
    }
    let points = trace_points(trace, plot, gain);
    clip.add(Shape::line(
        points.clone(),
        Stroke::new(6.0, TRACE.gamma_multiply(0.10)),
    ));
    clip.add(Shape::line(points.clone(), Stroke::new(1.6, TRACE)));
    let count = trace.len();
    let width = plot.width();
    // Few enough samples to see one by one: mark each.
    if (count as f32) <= width / 6.0 {
        for point in &points {
            clip.circle_filled(*point, 2.4, TRACE);
        }
    }
    // A chosen gain can push the wave off the screen: say where.
    if gain * trace.iter().fold(0.0f32, |most, s| most.max(s.abs())) > 1.0 {
        let columns = (width as usize).max(1);
        let mut over = vec![(false, false); columns];
        let last = count.saturating_sub(1).max(1);
        for (i, sample) in trace.iter().enumerate() {
            let column = (i * (columns - 1) / last).min(columns - 1);
            if sample * gain > 1.0 {
                over[column].0 = true;
            } else if sample * gain < -1.0 {
                over[column].1 = true;
            }
        }
        for (column, (top, bottom)) in over.into_iter().enumerate() {
            let x = plot.left() + column as f32;
            if top {
                clip.line_segment(
                    [pos2(x, plot.top()), pos2(x, plot.top() + 4.0)],
                    Stroke::new(1.0, CLIP),
                );
            }
            if bottom {
                clip.line_segment(
                    [pos2(x, plot.bottom() - 4.0), pos2(x, plot.bottom())],
                    Stroke::new(1.0, CLIP),
                );
            }
        }
    }

    if let Some(pointer) = plot_response.hover_pos() {
        ui.ctx().set_cursor_icon(CursorIcon::Crosshair);
        let t = ((pointer.x - plot.left()) / width).clamp(0.0, 1.0);
        let last = count.saturating_sub(1).max(1);
        let index = ((t * last as f32).round() as usize).min(count - 1);
        let sample = trace[index];
        let x = plot.left() + width * index as f32 / last as f32;
        let y = plot.center().y - (sample * gain).clamp(-1.0, 1.0) * plot.height() / 2.0;
        clip.line_segment(
            [pos2(x, plot.top()), pos2(x, plot.bottom())],
            Stroke::new(1.0, LABEL),
        );
        clip.circle_filled(pos2(x, y), 3.5, PEAK);
        let at = base * DIVISIONS as f32 * index as f32 / last as f32;
        readout(
            painter,
            plot,
            pointer,
            format!(
                "t = {}   x = {:+.4}   {}",
                duration_label(at),
                sample,
                db_label(decibels(sample.abs()))
            ),
        );
    }
}

/// An oscilloscope's graticule: ten divisions across, eight down, with
/// fine ticks along the centre lines.
fn graticule(painter: &Painter, plot: Rect) {
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
}

/// A dashed upright line across `plot` at `x`.
fn dashed(painter: &Painter, plot: Rect, x: f32, colour: Color32) {
    let mut y = plot.top();
    while y < plot.bottom() {
        painter.line_segment(
            [pos2(x, y), pos2(x, (y + 4.0).min(plot.bottom()))],
            Stroke::new(1.0, colour),
        );
        y += 8.0;
    }
}

/// The spectrum: dBFS against a logarithmic frequency axis, the held peaks
/// above it, the fundamental's harmonics each in its colour, and the
/// centroid and the 85% rolloff marked.
fn spectrum(app: &App, ui: &Ui, painter: &Painter, rect: Rect) {
    let locale = app.locale;
    let plot = panel(
        painter,
        rect,
        &gettext(locale, "Spectrum analyser"),
        &format!(
            "X[k] = Σ x[n]·w[n]·e^(−j2πkn/N)   N = {}   Δf = fs/N = {:.1} Hz",
            crate::analyser::FFT_SIZE,
            bin_hz()
        ),
        MARGINS,
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
    let x_of_hz = |hz: f32| plot.left() + plot.width() * position_of(hz);
    let clip = painter.with_clip_rect(plot);

    // Which harmonic, if any, each column belongs to: a quarter-tone either
    // side of k·f₀.
    let pitch = pro.pitch().filter(|found| found.clarity >= NAMED_CLARITY);
    let quarter_tone = 2f32.powf(1.0 / 24.0);
    let harmonic_of = |column: usize| {
        let hz = hz_at(column as f32 / columns as f32);
        let found = pitch?;
        let k = (hz / found.hz).round();
        let close = k >= 1.0 && k <= HARMONIC.len() as f32 && {
            let centre = found.hz * k;
            hz >= centre / quarter_tone && hz <= centre * quarter_tone
        };
        close.then(|| k as usize - 1)
    };

    let mut fill = egui::Mesh::default();
    let mut line = Vec::with_capacity(columns + 1);
    for column in 0..=columns {
        let top = pos2(x_of(column), y_of_db(plot, band(column, pro.spectrum())));
        let (colour, strength) = match harmonic_of(column) {
            Some(k) => (HARMONIC[k], 0.75),
            None => (SPECTRUM, 0.32),
        };
        let base = fill.vertices.len() as u32;
        fill.colored_vertex(top, colour.gamma_multiply(strength));
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
        Stroke::new(1.0, PEAK.gamma_multiply(0.7)),
    ));

    // The shape filters move: the centroid and the 85% rolloff.
    if let Some(shape) = pro.shape() {
        for (hz, label) in [
            (shape.centroid, gettext(locale, "Brightness").to_string()),
            (shape.rolloff, "85%".to_owned()),
        ] {
            let x = x_of_hz(hz);
            dashed(&clip, plot, x, LABEL.gamma_multiply(0.8));
            clip.text(
                pos2(x + 4.0, plot.bottom() - 4.0),
                Align2::LEFT_BOTTOM,
                label,
                FontId::proportional(10.0),
                LABEL,
            );
        }
    }

    // Each harmonic's mark and name, k·f₀, the fundamental's with its note.
    if let Some(found) = pitch {
        let levels = pro.harmonics();
        let mut last_label = f32::MIN;
        for (k, colour) in HARMONIC.iter().enumerate() {
            let hz = found.hz * (k + 1) as f32;
            if hz > crate::analyser::HIGH_HZ {
                break;
            }
            let x = x_of_hz(hz);
            let y = y_of_db(plot, levels[k]);
            clip.circle_filled(pos2(x, y), 3.0, *colour);
            if x - last_label < 34.0 {
                continue;
            }
            last_label = x;
            let label = if k == 0 {
                match note(hz) {
                    Some((name, _)) => format!("f₀ {name}"),
                    None => "f₀".to_owned(),
                }
            } else {
                format!("{}f₀", k + 1)
            };
            clip.text(
                pos2(x, (y - 8.0).max(plot.top() + 12.0)),
                Align2::CENTER_BOTTOM,
                label,
                FontId::monospace(10.5),
                *colour,
            );
        }
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
        let mut text = format!(
            "{}   {}   {}",
            hz_precise(hz),
            note_label(hz).unwrap_or_default(),
            db_label(level)
        );
        if let Some(found) = pitch {
            text.push_str(&format!("   f/f₀ = {:.2}", hz / found.hz));
        }
        readout(painter, plot, pointer, text);
    }
}

/// The spectrogram: the last ten seconds, newest on the right, bass at the
/// foot, brighter for louder, with the band's regions named, the note's
/// line through it, and its colour scale beside it.
fn spectrogram(app: &App, ui: &Ui, painter: &Painter, rect: Rect, texture: TextureId) {
    let locale = app.locale;
    let mut margins = MARGINS;
    margins[2] = 64.0;
    let plot = panel(
        painter,
        rect,
        &gettext(locale, "Spectrogram"),
        &gettext(
            locale,
            "Time → · Frequency ↑ · Colour is loudness · White line is the note",
        ),
        margins,
    );
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
    let y_of_hz = |hz: f32| plot.bottom() - plot.height() * position_of(hz);
    let clip = painter.with_clip_rect(plot);

    // The bass, the mids and the treble, divided where engineers divide them.
    for hz in [BASS_TOP_HZ, TREBLE_FOOT_HZ] {
        let y = y_of_hz(hz);
        let mut x = plot.left();
        while x < plot.right() {
            clip.line_segment(
                [pos2(x, y), pos2((x + 6.0).min(plot.right()), y)],
                Stroke::new(1.0, Color32::from_white_alpha(70)),
            );
            x += 12.0;
        }
    }
    for (label, low, high) in [
        (
            gettext(locale, "Treble"),
            TREBLE_FOOT_HZ,
            crate::analyser::HIGH_HZ,
        ),
        (gettext(locale, "Mids"), BASS_TOP_HZ, TREBLE_FOOT_HZ),
        (
            gettext(locale, "Bass"),
            crate::analyser::LOW_HZ,
            BASS_TOP_HZ,
        ),
    ] {
        let y = (y_of_hz(low) + y_of_hz(high)) / 2.0;
        let galley = painter.layout_no_wrap(label.to_uppercase(), FontId::proportional(10.0), TEXT);
        let tag = Rect::from_min_size(
            pos2(plot.left() + 6.0, y - galley.size().y / 2.0 - 2.0),
            galley.size() + vec2(10.0, 4.0),
        );
        clip.rect_filled(tag, 3.0, Color32::from_black_alpha(170));
        clip.galley(tag.min + vec2(5.0, 2.0), galley, TEXT);
    }

    // The note's line: each column's named fundamental, and its name where
    // it changes and holds.
    let column_x =
        |offset: usize| plot.left() + plot.width() * (offset as f32 + 0.5) / HISTORY as f32;
    let mut held: Option<(String, usize)> = None;
    let mut last_label = f32::MIN;
    for offset in 0..HISTORY {
        let index = (next + offset) % HISTORY;
        let Some(hz) = pro.column_note(index) else {
            held = None;
            continue;
        };
        let x = column_x(offset);
        let y = y_of_hz(hz);
        clip.rect_filled(
            Rect::from_center_size(pos2(x, y), vec2(2.5, 2.5)),
            0.0,
            Color32::from_white_alpha(230),
        );
        let name = note(hz).map(|(name, _)| name).unwrap_or_default();
        let run = match &mut held {
            Some((current, run)) if *current == name => {
                *run += 1;
                *run
            }
            _ => {
                held = Some((name.clone(), 1));
                1
            }
        };
        // Named once it has held for 100 ms.
        if run == 5 && x - last_label > 44.0 {
            last_label = x;
            clip.text(
                pos2(x, y - 5.0),
                Align2::CENTER_BOTTOM,
                name,
                FontId::monospace(10.0),
                Color32::WHITE,
            );
        }
    }

    let seconds = (HISTORY as u32 * HOP.as_millis() as u32) / 1000;
    for second in (0..=seconds).step_by(2) {
        let x = plot.right() - plot.width() * second as f32 / seconds as f32;
        painter.line_segment(
            [pos2(x, plot.top()), pos2(x, plot.bottom())],
            Stroke::new(1.0, Color32::from_white_alpha(18)),
        );
        let label = if second == 0 {
            gettext(locale, "now").to_string()
        } else {
            gettext(locale, "{seconds} s ago").replace("{seconds}", &second.to_string())
        };
        let align = if second == seconds {
            Align2::LEFT_TOP
        } else if second == 0 {
            Align2::RIGHT_TOP
        } else {
            Align2::CENTER_TOP
        };
        painter.text(
            pos2(x, plot.bottom() + 5.0),
            align,
            label,
            FontId::monospace(10.5),
            LABEL,
        );
    }

    // The colour scale, from full scale at the top to the floor.
    let bar = Rect::from_min_max(
        pos2(plot.right() + 12.0, plot.top() + 12.0),
        pos2(plot.right() + 22.0, plot.bottom() - 12.0),
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
    painter.text(
        pos2(bar.center().x, bar.top() - 3.0),
        Align2::CENTER_BOTTOM,
        gettext(locale, "loud").as_ref(),
        FontId::proportional(9.5),
        LABEL,
    );
    painter.text(
        pos2(bar.center().x, bar.bottom() + 3.0),
        Align2::CENTER_TOP,
        gettext(locale, "silence").as_ref(),
        FontId::proportional(9.5),
        LABEL,
    );
    for (t, label) in [(0.0, "0 dB"), (0.5, "-48"), (1.0, "-96")] {
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
        clip.line_segment(
            [pos2(pointer.x, plot.top()), pos2(pointer.x, plot.bottom())],
            Stroke::new(1.0, Color32::from_white_alpha(60)),
        );
        clip.line_segment(
            [pos2(plot.left(), pointer.y), pos2(plot.right(), pointer.y)],
            Stroke::new(1.0, Color32::from_white_alpha(60)),
        );
        let mut text = format!("-{ago:.2} s   {}   {}", hz_precise(hz), db_label(level));
        if let Some((name, _)) = pro.column_note(column).and_then(note) {
            text.push_str(&format!("   {name}"));
        }
        readout(painter, plot, pointer, text);
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
            let colour = if upright {
                Color32::from_white_alpha(if multiple == 1 { 34 } else { 12 })
            } else if multiple == 1 {
                GRID_MAJOR
            } else {
                GRID
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
    // Names too close to the last one drawn on a short plot are left out.
    let mut last_y = f32::MAX;
    for hz in NAMED_HZ {
        let t = position_of(hz);
        let label = if hz >= 1_000.0 {
            format!("{:.0}k", hz / 1_000.0)
        } else {
            format!("{hz:.0}")
        };
        if upright {
            let y = plot.bottom() - plot.height() * t;
            if last_y - y < 11.0 {
                continue;
            }
            last_y = y;
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

/// A pitch class's name in the reader's language, with its sharp: Do♯ or
/// C#. Each language names the seven natural notes its own way.
fn note_name(locale: Locale, class: usize) -> String {
    // The natural note each class sharpens, and whether it is sharp.
    const NATURAL: [(usize, bool); 12] = [
        (0, false),
        (0, true),
        (1, false),
        (1, true),
        (2, false),
        (3, false),
        (3, true),
        (4, false),
        (4, true),
        (5, false),
        (5, true),
        (6, false),
    ];
    let (natural, sharp) = NATURAL[class % 12];
    let name = match natural {
        // Translators: A note's name. Languages that sing Do, Re, Mi name
        // it Do; others keep the letter.
        0 => pgettext(locale, "note name", "C"),
        // Translators: A note's name: Re in solfège, D as a letter.
        1 => pgettext(locale, "note name", "D"),
        // Translators: A note's name: Mi in solfège, E as a letter.
        2 => pgettext(locale, "note name", "E"),
        // Translators: A note's name: Fa in solfège, F as a letter.
        3 => pgettext(locale, "note name", "F"),
        // Translators: A note's name: Sol in solfège, G as a letter.
        4 => pgettext(locale, "note name", "G"),
        // Translators: A note's name: La in solfège, A as a letter.
        5 => pgettext(locale, "note name", "A"),
        // Translators: A note's name: Si in solfège, B (H in German) as a
        // letter.
        _ => pgettext(locale, "note name", "B"),
    };
    if sharp {
        format!("{name}#")
    } else {
        name.to_string()
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

    #[test]
    fn notes_are_named_in_the_readers_language() {
        assert_eq!(note_name(Locale::default(), 9), "A");
        assert_eq!(note_name(Locale::default(), 10), "A#");
        assert_eq!(note_name(Locale::default(), 0), "C");
        assert_eq!(note_name(Locale::Spanish, 9), "La");
        assert_eq!(note_name(Locale::Spanish, 6), "Fa#");
        assert_eq!(note_name(Locale::German, 11), "H");
    }
}
