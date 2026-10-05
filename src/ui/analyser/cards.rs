//! The instruments beside the pictures: the note sounding, loudness, and
//! the sound's modulations and filtering. Each shows its formula with the
//! measured values in it, so the reading says what it is.

use egui::{Align2, Color32, FontId, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, pos2, vec2};

use super::{
    EDGE, GAP, GRID, HARMONIC, LABEL, PANEL, PEAK, TEXT, TRACE, db_label, hz_precise, note_name,
};
use crate::analyser::loudness::STREAMING_TARGET;
use crate::analyser::modulation::{ENVELOPE_POINTS, ROLLOFF_SHARE};
use crate::analyser::{FLOOR_DB, NAMED_CLARITY, note_parts};
use crate::app::App;
use crate::i18n::gettext;

/// Loudness meters run from here to full scale, in LUFS.
const LOUDNESS_FLOOR: f32 = -48.0;
/// The tuner's reach either side of the note, in cents, and how close
/// counts as in tune.
const TUNER_CENTS: f32 = 50.0;
const IN_TUNE_CENTS: i32 = 5;
/// A sweep faster than this, in octaves a second, is a filter moving.
const SWEEPING: f32 = 0.5;
/// White keys of one octave, and where each black key sits between them,
/// as pitch classes.
const WHITE_KEYS: [usize; 7] = [0, 2, 4, 5, 7, 9, 11];
const BLACK_KEYS: [(usize, f32); 5] = [(1, 1.0), (3, 2.0), (6, 4.0), (8, 5.0), (10, 6.0)];

/// The column of instruments down `rect`.
pub(super) fn instruments(app: &App, painter: &Painter, rect: Rect) {
    let usable = rect.height() - 2.0 * GAP;
    let note = Rect::from_min_size(rect.min, vec2(rect.width(), usable * 0.40));
    let loudness = Rect::from_min_size(
        pos2(rect.left(), note.bottom() + GAP),
        vec2(rect.width(), usable * 0.25),
    );
    let modulation = Rect::from_min_max(pos2(rect.left(), loudness.bottom() + GAP), rect.max);
    note_card(app, painter, note);
    loudness_card(app, painter, loudness);
    modulation_card(app, painter, modulation);
}

/// A card and its title, with the formula it measures in small type on
/// the right. Returns the room inside.
fn card(painter: &Painter, rect: Rect, title: &str, formula: &str) -> Rect {
    painter.rect_filled(rect, 8.0, PANEL);
    painter.rect_stroke(rect, 8.0, Stroke::new(1.0, EDGE), StrokeKind::Inside);
    painter.text(
        rect.left_top() + vec2(12.0, 10.0),
        Align2::LEFT_TOP,
        title.to_uppercase(),
        FontId::proportional(11.0),
        LABEL,
    );
    painter.text(
        rect.right_top() + vec2(-12.0, 10.0),
        Align2::RIGHT_TOP,
        formula,
        FontId::monospace(10.0),
        LABEL.gamma_multiply(0.8),
    );
    Rect::from_min_max(rect.min + vec2(12.0, 30.0), rect.max - vec2(12.0, 10.0))
}

/// The note: its name large, its frequency and MIDI number, a tuner, the
/// twelve pitch classes on an octave of keys, and its harmonics' levels.
fn note_card(app: &App, painter: &Painter, rect: Rect) {
    let locale = app.locale;
    let inner = card(
        painter,
        rect,
        &gettext(locale, "Note"),
        "n = 69 + 12·log₂(f/440)",
    );
    let pro = &app.pro_analyser;
    let pitch = pro.pitch();
    let named = pitch.filter(|found| found.clarity >= NAMED_CLARITY);
    let parts = pitch.and_then(|found| note_parts(found.hz));
    let mut y = inner.top();

    // The name, large: bright when the sound is built on it, dim when it is
    // only the likeliest guess.
    let (name, cents) = match parts {
        Some((class, octave, cents)) => (format!("{}{octave}", note_name(locale, class)), cents),
        None => ("-".to_owned(), 0),
    };
    let colour = match named {
        Some(_) if cents.abs() <= IN_TUNE_CENTS => TRACE,
        Some(_) => TEXT,
        None => LABEL.gamma_multiply(0.7),
    };
    let big = painter.text(
        pos2(inner.left(), y - 4.0),
        Align2::LEFT_TOP,
        &name,
        FontId::proportional(46.0),
        colour,
    );
    if let Some(found) = pitch {
        let midi = 69.0 + 12.0 * (found.hz / 440.0).log2();
        let letter = crate::analyser::note(found.hz)
            .map(|(letter, _)| letter)
            .unwrap_or_default();
        let lines = [
            format!("f₀ = {}", hz_precise(found.hz)),
            format!("n = {midi:.2}  {letter}"),
            format!("{cents:+} ¢"),
        ];
        for (row, line) in lines.iter().enumerate() {
            painter.text(
                pos2(big.right() + 14.0, big.top() + 8.0 + row as f32 * 15.0),
                Align2::LEFT_TOP,
                line,
                FontId::monospace(11.5),
                if row == 2 { colour } else { TEXT },
            );
        }
    }
    y = big.bottom() + 6.0;

    // The tuner: how far from the note, -50 to +50 cents.
    let tuner = Rect::from_min_size(pos2(inner.left(), y), vec2(inner.width(), 16.0));
    painter.line_segment(
        [
            pos2(tuner.left(), tuner.center().y),
            pos2(tuner.right(), tuner.center().y),
        ],
        Stroke::new(1.0, GRID),
    );
    for tick in [-50.0f32, -25.0, 0.0, 25.0, 50.0] {
        let x = tuner.center().x + tuner.width() / 2.0 * tick / TUNER_CENTS;
        let reach = if tick == 0.0 { 7.0 } else { 4.0 };
        painter.line_segment(
            [
                pos2(x, tuner.center().y - reach),
                pos2(x, tuner.center().y + reach),
            ],
            Stroke::new(1.0, if tick == 0.0 { TRACE } else { LABEL }),
        );
    }
    if named.is_some() {
        let x =
            tuner.center().x + tuner.width() / 2.0 * (cents as f32 / TUNER_CENTS).clamp(-1.0, 1.0);
        painter.add(Shape::convex_polygon(
            vec![
                pos2(x, tuner.center().y + 1.0),
                pos2(x - 5.0, tuner.top() - 2.0),
                pos2(x + 5.0, tuner.top() - 2.0),
            ],
            colour,
            Stroke::NONE,
        ));
    }
    y = tuner.bottom() + 8.0;

    // One octave of keys, each lit by its pitch class's energy, the note's
    // key ringed.
    let room = inner.bottom() - y;
    let keys_height = (room * 0.48).clamp(28.0, 64.0);
    let keys = Rect::from_min_size(pos2(inner.left(), y), vec2(inner.width(), keys_height));
    let chroma = pro.chroma();
    let current = named.and(parts).map(|(class, _, _)| class);
    let white_width = keys.width() / WHITE_KEYS.len() as f32;
    for (index, class) in WHITE_KEYS.iter().enumerate() {
        let key = Rect::from_min_size(
            pos2(keys.left() + white_width * index as f32, keys.top()),
            vec2(white_width - 2.0, keys.height()),
        );
        let lit = chroma[*class].sqrt();
        painter.rect_filled(
            key,
            3.0,
            lerp_colour(Color32::from_rgb(38, 46, 42), TRACE, lit),
        );
        if current == Some(*class) {
            painter.rect_stroke(key, 3.0, Stroke::new(2.0, PEAK), StrokeKind::Inside);
        }
        // Dark on a lit key, light on a dark one.
        let ink = if lit > 0.45 {
            Color32::from_rgb(20, 26, 22)
        } else {
            LABEL
        };
        painter.text(
            pos2(key.center().x, key.bottom() - 3.0),
            Align2::CENTER_BOTTOM,
            note_name(locale, *class),
            FontId::proportional(9.5),
            ink,
        );
    }
    for (class, after) in BLACK_KEYS {
        let width = white_width * 0.6;
        let key = Rect::from_min_size(
            pos2(
                keys.left() + white_width * after - width / 2.0 - 1.0,
                keys.top(),
            ),
            vec2(width, keys.height() * 0.6),
        );
        let lit = chroma[class].sqrt();
        painter.rect_filled(
            key,
            2.0,
            lerp_colour(Color32::from_rgb(8, 10, 9), TRACE, lit * 0.85),
        );
        let ring = if current == Some(class) {
            Stroke::new(2.0, PEAK)
        } else {
            Stroke::new(1.0, EDGE)
        };
        painter.rect_stroke(key, 2.0, ring, StrokeKind::Inside);
    }
    y = keys.bottom() + 8.0;

    // The harmonics: each one's level under the fundamental's, the
    // timbre's fingerprint.
    let bars = Rect::from_min_max(pos2(inner.left(), y), inner.max);
    if bars.height() < 24.0 {
        return;
    }
    painter.text(
        bars.left_top(),
        Align2::LEFT_TOP,
        format!("{}  ·  fₖ = k·f₀", gettext(locale, "Harmonics")),
        FontId::proportional(10.0),
        LABEL,
    );
    let plot = Rect::from_min_max(bars.min + vec2(0.0, 16.0), bars.max - vec2(0.0, 12.0));
    let levels = pro.harmonics();
    let reference = levels[0].max(levels.iter().copied().fold(FLOOR_DB, f32::max));
    let width = plot.width() / levels.len() as f32;
    for (k, level) in levels.iter().enumerate() {
        let x = plot.left() + width * k as f32;
        // A guess's harmonics show faintly; a clear note's in full.
        let reach = if pitch.is_some() {
            ((level - (reference - 60.0)) / 60.0).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let strength = if named.is_some() { 0.85 } else { 0.3 };
        let bar = Rect::from_min_max(
            pos2(x + 2.0, plot.bottom() - plot.height() * reach),
            pos2(x + width - 2.0, plot.bottom()),
        );
        painter.rect_filled(bar, 2.0, HARMONIC[k].gamma_multiply(strength));
        painter.text(
            pos2(x + width / 2.0, plot.bottom() + 2.0),
            Align2::CENTER_TOP,
            (k + 1).to_string(),
            FontId::monospace(9.5),
            HARMONIC[k],
        );
    }
}

/// Loudness: momentary, short-term and integrated, against the level
/// streaming services play songs at, with sample peak, RMS and crest.
fn loudness_card(app: &App, painter: &Painter, rect: Rect) {
    let locale = app.locale;
    let inner = card(
        painter,
        rect,
        &format!("{} (LUFS)", gettext(locale, "Loudness")),
        "L = −0.691 + 10·log₁₀ Σ Gᵢ·zᵢ",
    );
    let meter = app.pro_analyser.loudness();
    let rows = [
        (gettext(locale, "Momentary"), meter.momentary()),
        (gettext(locale, "Short-term"), meter.short_term()),
        (gettext(locale, "Integrated"), meter.integrated()),
    ];
    let row_height = ((inner.height() - 50.0) / 3.0).clamp(14.0, 22.0);
    let label_width = 92.0;
    let value_width = 82.0;
    let target_x = |track: Rect| {
        track.left() + track.width() * (STREAMING_TARGET - LOUDNESS_FLOOR) / -LOUDNESS_FLOOR
    };
    for (row, (label, value)) in rows.iter().enumerate() {
        let y = inner.top() + row_height * (row as f32 + 0.5);
        painter.text(
            pos2(inner.left(), y),
            Align2::LEFT_CENTER,
            label.as_ref(),
            FontId::proportional(11.0),
            TEXT,
        );
        let track = Rect::from_min_max(
            pos2(inner.left() + label_width, y - 4.0),
            pos2(inner.right() - value_width, y + 4.0),
        );
        painter.rect_filled(track, 2.0, GRID);
        if let Some(level) = value {
            let reach = ((level - LOUDNESS_FLOOR) / -LOUDNESS_FLOOR).clamp(0.0, 1.0);
            let colour = if *level > STREAMING_TARGET + 4.0 {
                super::CLIP
            } else if *level > STREAMING_TARGET {
                PEAK
            } else {
                TRACE
            };
            painter.rect_filled(
                Rect::from_min_max(
                    track.min,
                    pos2(track.left() + track.width() * reach, track.bottom()),
                ),
                2.0,
                colour,
            );
        }
        let x = target_x(track);
        painter.line_segment(
            [pos2(x, track.top() - 3.0), pos2(x, track.bottom() + 3.0)],
            Stroke::new(1.0, TEXT),
        );
        painter.text(
            pos2(inner.right(), y),
            Align2::RIGHT_CENTER,
            value.map_or_else(|| "-".to_owned(), |level| format!("{level:.1} LUFS")),
            FontId::monospace(11.5),
            TEXT,
        );
    }
    let mut y = inner.top() + row_height * 3.0 + 6.0;
    if let Some(integrated) = meter.integrated() {
        let gain = STREAMING_TARGET - integrated;
        painter.text(
            pos2(inner.left(), y),
            Align2::LEFT_TOP,
            gettext(locale, "At Spotify's -14 LUFS: {gain} dB")
                .replace("{gain}", &format!("{gain:+.1}")),
            FontId::proportional(11.0),
            PEAK,
        );
    }
    y += 18.0;
    if y + 26.0 > rect.bottom() {
        return;
    }
    // Sample peak, RMS and their ratio, each in a column of its own.
    let levels = app.pro_analyser.levels();
    let crest = if levels.peak > FLOOR_DB {
        format!("{:.1} dB", (levels.peak - levels.rms).max(0.0))
    } else {
        "-".to_owned()
    };
    let peak_colour = if levels.held > -0.1 {
        super::CLIP
    } else {
        TEXT
    };
    let width = inner.width() / 3.0;
    for (column, (label, value, colour)) in [
        (
            gettext(locale, "Peak").to_string(),
            db_label(levels.peak),
            peak_colour,
        ),
        ("RMS".to_owned(), db_label(levels.rms), TEXT),
        (gettext(locale, "Crest factor").to_string(), crest, TEXT),
    ]
    .into_iter()
    .enumerate()
    {
        let x = inner.left() + width * column as f32;
        painter.text(
            pos2(x, y),
            Align2::LEFT_TOP,
            label,
            FontId::proportional(10.0),
            LABEL,
        );
        painter.text(
            pos2(x, y + 13.0),
            Align2::LEFT_TOP,
            value,
            FontId::monospace(11.0),
            colour,
        );
    }
}

/// Modulation and filtering: AM from the envelope, FM from the held note's
/// pitch, and the spectrum's shape, each with its formula and a trace.
fn modulation_card(app: &App, painter: &Painter, rect: Rect) {
    let locale = app.locale;
    let inner = card(
        painter,
        rect,
        &gettext(locale, "Modulation and filters"),
        "",
    );
    let pro = &app.pro_analyser;
    let block = (inner.height() / 3.0).max(40.0);
    let spark = (block - 46.0).clamp(0.0, 30.0);

    // AM: the envelope and its strongest periodicity.
    let mut y = inner.top();
    section(
        painter,
        inner,
        y,
        &gettext(locale, "Envelope (AM)"),
        "x(t) = A·[1 + m·cos(2π·fₘ·t)]·c(t)",
    );
    let am = pro.envelope().am();
    let values = match am {
        Some(am) => {
            let mut text = format!("fₘ = {:.2} Hz   m = {:.2}", am.rate, am.depth);
            if let Some(bpm) = am.bpm() {
                text.push_str(&format!("   ≈ {bpm:.0} BPM"));
            }
            text
        }
        None => "-".to_owned(),
    };
    painter.text(
        pos2(inner.left(), y + 30.0),
        Align2::LEFT_TOP,
        values,
        FontId::monospace(11.5),
        TEXT,
    );
    if spark > 8.0 {
        let points: Vec<f32> = pro.envelope().points().iter().copied().collect();
        sparkline(
            painter,
            Rect::from_min_size(pos2(inner.left(), y + 46.0), vec2(inner.width(), spark)),
            &points,
            ENVELOPE_POINTS,
            TRACE,
        );
    }

    // FM: the held note's vibrato.
    y += block;
    section(
        painter,
        inner,
        y,
        &gettext(locale, "Vibrato (FM)"),
        "f(t) = f₀ + Δf·cos(2π·fₘ·t),  β = Δf/fₘ",
    );
    let values = match pro.vibrato() {
        Some(fm) => format!(
            "fₘ = {:.1} Hz   Δf = ±{:.1} Hz ({:.0}¢)   β = {:.2}",
            fm.rate, fm.deviation_hz, fm.deviation_cents, fm.index
        ),
        None => gettext(locale, "No held note").to_string(),
    };
    painter.text(
        pos2(inner.left(), y + 30.0),
        Align2::LEFT_TOP,
        values,
        FontId::monospace(11.5),
        TEXT,
    );
    if spark > 8.0 {
        let track = pro.note_track();
        let held: Vec<f32> = track.iter().flatten().copied().collect();
        if held.len() == track.len() && !held.is_empty() {
            let mut sorted = held.clone();
            sorted.sort_by(f32::total_cmp);
            let median = sorted[sorted.len() / 2];
            let cents: Vec<f32> = held
                .iter()
                .map(|hz| 1200.0 * (hz / median).log2())
                .collect();
            sparkline(
                painter,
                Rect::from_min_size(pos2(inner.left(), y + 46.0), vec2(inner.width(), spark)),
                &cents,
                cents.len(),
                HARMONIC[0],
            );
        }
    }

    // Filters: where the spectrum's power sits, where it ends, and how
    // flat it is.
    y += block;
    section(
        painter,
        inner,
        y,
        &gettext(locale, "Spectral shape (filters)"),
        "",
    );
    let Some(shape) = pro.shape() else {
        return;
    };
    let sweep = match pro.sweep() {
        Some(rate) if rate > SWEEPING => format!("↑ {}", gettext(locale, "Opening")),
        Some(rate) if rate < -SWEEPING => format!("↓ {}", gettext(locale, "Closing")),
        _ => gettext(locale, "Steady").to_string(),
    };
    let character = if shape.flatness < 0.2 {
        gettext(locale, "Tonal")
    } else {
        gettext(locale, "Noise-like")
    };
    let lines = [
        format!(
            "{}  Σf·P/ΣP = {}",
            gettext(locale, "Brightness"),
            hz_precise(shape.centroid)
        ),
        format!(
            "{}  f({:.0}%) = {}  {sweep}",
            gettext(locale, "Cutoff"),
            ROLLOFF_SHARE * 100.0,
            hz_precise(shape.rolloff)
        ),
        format!(
            "{}  SFM = {:.2}  {character}",
            gettext(locale, "Flatness"),
            shape.flatness
        ),
    ];
    for (row, line) in lines.iter().enumerate() {
        let line_y = y + 18.0 + row as f32 * 15.0;
        if line_y + 12.0 > rect.bottom() {
            break;
        }
        painter.text(
            pos2(inner.left(), line_y),
            Align2::LEFT_TOP,
            line,
            FontId::monospace(11.0),
            TEXT,
        );
    }
}

/// A section's name, and its formula under it.
fn section(painter: &Painter, inner: Rect, y: f32, title: &str, formula: &str) {
    painter.text(
        pos2(inner.left(), y),
        Align2::LEFT_TOP,
        title,
        FontId::proportional(11.5),
        PEAK,
    );
    if !formula.is_empty() {
        painter.text(
            pos2(inner.left(), y + 15.0),
            Align2::LEFT_TOP,
            formula,
            FontId::monospace(10.5),
            LABEL,
        );
    }
}

/// `values` drawn as a line across `rect`, fitted to their range, the
/// newest on the right; `capacity` points span the width.
fn sparkline(painter: &Painter, rect: Rect, values: &[f32], capacity: usize, colour: Color32) {
    painter.rect_filled(rect, 3.0, Color32::from_rgb(9, 13, 11));
    if values.len() < 2 {
        return;
    }
    let (low, high) = values.iter().fold((f32::MAX, f32::MIN), |(low, high), v| {
        (low.min(*v), high.max(*v))
    });
    let range = (high - low).max(1e-6);
    let step = rect.width() / capacity.max(2).saturating_sub(1) as f32;
    let start = rect.right() - step * (values.len() - 1) as f32;
    let points: Vec<Pos2> = values
        .iter()
        .enumerate()
        .map(|(i, value)| {
            pos2(
                start + step * i as f32,
                rect.bottom() - 2.0 - (rect.height() - 4.0) * (value - low) / range,
            )
        })
        .collect();
    painter.add(Shape::line(points, Stroke::new(1.3, colour)));
}

fn lerp_colour(from: Color32, to: Color32, t: f32) -> Color32 {
    super::lerp(from, to, t.clamp(0.0, 1.0))
}
