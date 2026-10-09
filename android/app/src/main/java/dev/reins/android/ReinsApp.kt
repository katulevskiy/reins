package dev.reins.android

import android.app.Application
import android.os.StrictMode
import androidx.work.Configuration
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch

/**
 * Starts nothing on the main thread that the first frame does not need: the core opens in the background right away
 * (the first frame shows the last settled session meanwhile), notification channels are made off the main thread, and
 * WorkManager starts when something first uses it, not at process start.
 */
class ReinsApp : Application(), Configuration.Provider {
    lateinit var container: AppContainer
        private set

    override val workManagerConfiguration: Configuration get() = Configuration.Builder().build()

    override fun onCreate() {
        super.onCreate()
        if (BuildConfig.DEBUG) {
            StrictMode.setThreadPolicy(
                StrictMode.ThreadPolicy.Builder().detectDiskReads().detectDiskWrites().detectNetwork().penaltyLog().build(),
            )
        }
        dev.reins.android.design.Fonts.init(this)
        container = AppContainer(this)
        container.appScope.launch(Dispatchers.IO) {
            // Opens the native core (library, store, Keystore) while the UI draws; its first real use finds it ready.
            runCatching { container.core.session() }
        }
        container.appScope.launch(Dispatchers.IO) { container.notifier.createChannels() }
        container.appScope.launch(Dispatchers.Default) { dev.reins.android.design.Fonts.warm() }
    }
}
