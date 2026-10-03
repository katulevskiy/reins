package dev.rewarden.android.platform.update

/** Lets tests replace the update server and the system installer. Production leaves both null. Read on every use. */
object UpdateProvider {
    @Volatile
    var fetcher: UpdateFetcher? = null

    @Volatile
    var installer: AppInstaller? = null
}
