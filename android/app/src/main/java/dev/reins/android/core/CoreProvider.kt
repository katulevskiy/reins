package dev.rewarden.android.core

import dev.rewarden.android.AppContainer
import dev.rewarden.core.RewardenCoreInterface

/** Lets instrumented tests swap in a fake core before the Application is created. Production returns null. */
fun interface CoreFactory {
    fun create(container: AppContainer): RewardenCoreInterface?
}

object CoreProvider {
    @Volatile
    var factory: CoreFactory = CoreFactory { null }
}
