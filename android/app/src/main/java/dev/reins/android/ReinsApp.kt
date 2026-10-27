package dev.reins.android

import android.app.Application
import android.os.StrictMode
import androidx.work.Configuration
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.distinctUntilChanged
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
            // This profile's push tag, so a push for another profile on this phone is never handled by this core.
            runCatching { container.core.pushTag() }.getOrNull()?.let {
                dev.reins.android.push.PushRouting.remember(this@ReinsApp, container.profile.id, it)
            }
        }
        container.appScope.launch(Dispatchers.IO) { container.notifier.createChannels() }
        container.appScope.launch(Dispatchers.Default) { dev.reins.android.design.Fonts.warm() }
        // The tiles and widgets outside the app follow what waits and Autopilot's mode.
        container.appScope.launch {
            combine(container.state.pending, container.state.autopilot, container.state.activity) { pending, autopilot, activity -> Triple(pending.map { it.id }, autopilot?.mode, activity.firstOrNull()?.id) }
                .distinctUntilChanged()
                .collect {
                    dev.reins.android.platform.tiles.Tiles.refresh(this@ReinsApp)
                    dev.reins.android.platform.widgets.Widgets.refresh(this@ReinsApp)
                }
        }
    }
}
