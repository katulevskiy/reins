//! The films. Each one lists its beats (named scenes), its soundtrack and how to draw a moment.

pub mod hero;
pub mod hooks;
pub mod pay;

use fframes::{AudioMap, AudioTimestamp::*, AudioTrack, FFramesContext, Frame, Svgr};

use crate::Film;
use crate::kit::Format;
use crate::ui::terminal::CPS;

pub fn beats(film: Film) -> &'static [(&'static str, f32)] {
    match film {
        Film::Hero => hero::BEATS,
        Film::HookRmRf => hooks::RMRF_BEATS,
        Film::HookPush => hooks::PUSH_BEATS,
        Film::HookTokens => hooks::TOKENS_BEATS,
        Film::Pay => pay::BEATS,
    }
}

pub fn audio(film: Film) -> AudioMap<'static> {
    AudioMap::from(match film {
        Film::Hero => hero::audio(),
        Film::HookRmRf => hooks::rmrf_audio(),
        Film::HookPush => hooks::push_audio(),
        Film::HookTokens => hooks::tokens_audio(),
        Film::Pay => pay::audio(),
    })
}

pub fn draw<F: Format>(film: Film, frame: &mut Frame, ctx: &FFramesContext, t: f32) -> Svgr<'static> {
    match film {
        Film::Hero => hero::draw::<F>(frame, ctx, t),
        Film::HookRmRf => hooks::draw_rmrf::<F>(frame, ctx, t),
        Film::HookPush => hooks::draw_push::<F>(frame, ctx, t),
        Film::HookTokens => hooks::draw_tokens::<F>(frame, ctx, t),
        Film::Pay => pay::draw::<F>(frame, ctx, t),
    }
}

/// Master gain that brings each mix to about -14 LUFS (measured with `audio analyze`).
pub fn master_gain_db(film: Film) -> f32 {
    match film {
        Film::Hero => 3.8,
        Film::HookRmRf => 3.0,
        Film::HookPush => 4.8,
        Film::HookTokens => 5.6,
        Film::Pay => 5.2,
    }
}

// ---------------------------------------------------------------- sound helpers

/// A sound effect from assets/ (`sfx_<name>.wav`).
pub(crate) fn sfx(name: &'static str, at: f32, gain_db: f32) -> AudioTrack<'static> {
    AudioTrack::new(name, Second(at.max(0.0))..Eof).gain_db(gain_db)
}

/// A voiceover line from media/ (`voNN.mp3`): music ducks under it.
pub(crate) fn vo(file: &'static str, at: f32) -> AudioTrack<'static> {
    AudioTrack::new(file, Second(at)..Eof).voice()
}

/// The film's music bed from assets/.
pub(crate) fn music(file: &'static str, gain_db: f32) -> AudioTrack<'static> {
    AudioTrack::new(file, Second(0.0)..Eof).gain_db(gain_db).duck_under_voice()
}

/// Key clicks while a command is typed: one every three characters.
pub(crate) fn typing(at: f32, cmd: &str, gain_db: f32) -> Vec<AudioTrack<'static>> {
    let n = cmd.chars().count().div_ceil(3);
    (0..n).map(|i| sfx("sfx_tick.wav", at + i as f32 * 3.0 / CPS, gain_db)).collect()
}
