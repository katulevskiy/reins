package dev.rewarden.android.platform

/** Whether Rewarden is on screen and focused. Notifications add nothing then, so they are skipped. */
object Foreground {
    @Volatile
    var focused: Boolean = false

    /** New requests pop up by themselves while the app is in front. Tests and screenshots can turn this off. */
    @Volatile
    var autoPopup: Boolean = true
}
