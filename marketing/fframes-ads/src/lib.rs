//! Reins ads, written in Rust and SVG with fframes and rendered on the GPU.
//!
//! One `Ad` video type plays every film; the film decides what is drawn and heard, the format
//! (`Wide` 1920x1080 or `Tall` 1080x1920) decides the canvas. Each film's beats are named
//! scenes, so the CLI can address them (`strip Threat -n 12`, `frame Keys@1s`), while the picture
//! is drawn from one continuous clock so the phone and the terminal can live across cuts.

pub mod films;
pub mod kit;
pub mod ui;

use std::marker::PhantomData;

use fframes::{AudioMap, Color, Duration, FFramesContext, Frame, Scene, Scenes, Svgr, Video, include_media_dir};

pub use kit::{Format, Tall, Wide};

// Fonts (Geist, linked from the Android app) and the voiceover lines, embedded into the binary.
include_media_dir!(pub struct AdsMedia, "media");

pub const FPS: usize = 60;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Film {
    Hero,
    HookRmRf,
    HookPush,
    HookTokens,
    Pay,
}

/// A named stretch of a film. Draws nothing itself: the film draws from the global clock.
#[derive(Debug)]
pub struct Beat {
    pub name: &'static str,
    pub secs: f32,
}

impl Scene for Beat {
    fn duration(&self) -> Duration<'_> {
        Duration::Seconds(self.secs)
    }

    fn render_frame<'a>(&'a self, _frame: Frame, _ctx: &FFramesContext<'a, '_>) -> Svgr<'a> {
        Svgr::empty()
    }

    fn name(&self) -> &'static str {
        self.name
    }
}

pub struct Ad<F: Format> {
    pub film: Film,
    beats: Vec<Beat>,
    _format: PhantomData<F>,
}

impl<F: Format> Ad<F> {
    pub fn new(film: Film) -> Self {
        let beats = films::beats(film)
            .iter()
            .map(|&(name, secs)| Beat {
                name,
                secs,
            })
            .collect();
        Ad {
            film,
            beats,
            _format: PhantomData,
        }
    }
}

impl<F: Format> std::fmt::Debug for Ad<F> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Ad").field("film", &self.film).finish()
    }
}

impl<F: Format> Video for Ad<F> {
    const FPS: usize = FPS;
    const WIDTH: usize = F::W;
    const HEIGHT: usize = F::H;
    const BACKGROUND_COLOR: Color = Color::BLACK;

    fn duration(&self) -> Duration<'_> {
        Duration::Auto
    }

    fn audio(&self) -> AudioMap<'_> {
        films::audio(self.film)
    }

    fn define_scenes(&self) -> Scenes<'_> {
        Scenes::from(self.beats.iter().map(|b| b as &dyn Scene).collect::<Vec<_>>())
    }

    fn render_frame<'a>(&'a self, mut frame: Frame, ctx: &FFramesContext<'a, '_>) -> Svgr<'a> {
        let t = frame.global_index as f32 / FPS as f32;
        let body = films::draw::<F>(self.film, &mut frame, ctx, t);
        let (w, h) = (F::W, F::H);
        // Films draw around the centre of the frame.
        let centre = format!("translate({} {})", w / 2, h / 2);
        fframes::svgr!(
            <svg xmlns="http://www.w3.org/2000/svg" width={w} height={h}>
                <g transform={centre}>{body}</g>
            </svg>
        )
    }
}
