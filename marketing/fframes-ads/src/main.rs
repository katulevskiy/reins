use std::path::Path;
use std::process::ExitCode;

use fframes::cli::clap;
use fframes::{
    AudioMixOptions, CombinedMediaProvider, EncoderOptions, LimiterOptions, MediaDirectory, MediaProvider,
    RenderOptions, StaticMediaProvider, cli,
};
use fframes_skia_renderer::{
    SkiaFFramesRenderer, SkiaPipelineConcurrencyPolicy, SkiaPipelineConfig, vulkan::SkiaVulkanCtx,
};
use reins_ads::{Ad, AdsMedia, Film, Format, Tall, Wide};

/// Which film to work on, next to the standard commands of `fframes::cli` (render, frame, strip,
/// inspect, audio, preview...). `cargo run --release -- --film hook-rmrf strip -n 12`.
#[derive(Debug, clap::Args)]
struct AdArgs {
    #[arg(long, global = true, default_value = "hero-wide")]
    film: String,
}

fn main() -> ExitCode {
    let args = cli::parse::<AdArgs>();
    let film = args.app.film.clone();
    let media = AdsMedia::prepare().expect("media");
    // Music and sound effects, rendered by `bun sound/build.ts`, loaded at runtime to keep stereo.
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets");
    let Ok(dir) = MediaDirectory::read_folder(&folder) else {
        eprintln!("No sound in {}: run `bun sound/build.ts` first.", folder.display());
        return ExitCode::FAILURE;
    };
    let sounds = dir.process_media_source().expect("sounds");
    let all = CombinedMediaProvider::from([&media as &dyn MediaProvider, &sounds]);

    match film.as_str() {
        "hero-wide" => run::<Wide>(Film::Hero, &all, args, "out/reins-hero-16x9.mp4"),
        "hero-tall" => run::<Tall>(Film::Hero, &all, args, "out/reins-hero-9x16.mp4"),
        "hook-rmrf" => run::<Tall>(Film::HookRmRf, &all, args, "out/reins-hook-rmrf-9x16.mp4"),
        "hook-push" => run::<Tall>(Film::HookPush, &all, args, "out/reins-hook-push-9x16.mp4"),
        "hook-tokens" => run::<Tall>(Film::HookTokens, &all, args, "out/reins-hook-tokens-9x16.mp4"),
        "pay-wide" => run::<Wide>(Film::Pay, &all, args, "out/reins-pay-16x9.mp4"),
        "pay-tall" => run::<Tall>(Film::Pay, &all, args, "out/reins-pay-9x16.mp4"),
        other => {
            eprintln!(
                "Unknown film {other:?}. One of: hero-wide, hero-tall, hook-rmrf, hook-push, hook-tokens, pay-wide, pay-tall"
            );
            ExitCode::FAILURE
        }
    }
}

fn run<'m, F: Format>(
    film: Film,
    media: &'m dyn MediaProvider<'m>,
    args: cli::Cli<AdArgs>,
    output: &'static str,
) -> ExitCode {
    let video = Ad::<F>::new(film);
    let gpu = SkiaVulkanCtx::new(F::W, F::H).expect("GPU context");
    cli::new(
        &video,
        RenderOptions {
            media: Some(media),
            video_encoder_options: EncoderOptions {
                preferred_encoder: Some("libx264"),
                codec_params: Some(&[("crf", "16"), ("preset", "slow"), ("tune", "animation"), ("profile", "high")]),
                ..Default::default()
            },
            audio_mix: AudioMixOptions {
                master_gain_db: reins_ads::films::master_gain_db(film),
                // Room for inter-sample peaks: the true peak stays under -1 dBTP.
                limiter: Some(LimiterOptions {
                    ceiling_db: -1.5,
                    ..Default::default()
                }),
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .args(args)
    .backend(
        SkiaFFramesRenderer::new_vulkan(
            &gpu,
            SkiaPipelineConfig {
                concurrency_policy: SkiaPipelineConcurrencyPolicy::MaxPerformance,
                ..Default::default()
            },
        )
        .expect("skia renderer"),
    )
    .preview(fframes_native_player::cli_preview)
    .default_output(output)
    .run()
}
