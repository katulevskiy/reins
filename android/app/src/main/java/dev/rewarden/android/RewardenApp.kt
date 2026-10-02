package dev.rewarden.android

import android.app.Application
import android.os.StrictMode

class RewardenApp : Application() {
    lateinit var container: AppContainer
        private set

    override fun onCreate() {
        super.onCreate()
        if (BuildConfig.DEBUG) {
            StrictMode.setThreadPolicy(
                StrictMode.ThreadPolicy.Builder().detectDiskReads().detectDiskWrites().detectNetwork().penaltyLog().build(),
            )
        }
        dev.rewarden.android.design.Fonts.init(this)
        container = AppContainer(this)
        container.notifier.createChannels()
    }
}
