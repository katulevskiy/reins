package dev.reins.android

import org.robolectric.annotation.Config

/**
 * The Google Play listing's phone screenshots: the `play` build (no text messages, no updater) at 1215 x 2160 (9:16,
 * 405 x 720 dp at xxhdpi), dark. android/play/graphics/render.sh renders them and copies the listed screens into
 * android/play/graphics/phone-screenshots (android/PLAY_STORE.md, "Store listing").
 */
@Config(sdk = [35], qualifiers = "w405dp-h720dp-normal-night-xxhdpi")
class PlayStoreScreenshots : ScreenshotsBase("play")
