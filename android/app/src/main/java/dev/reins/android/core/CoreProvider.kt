package dev.reins.android.core

import dev.reins.android.AppContainer
import dev.reins.core.ReinsCoreInterface

/** Lets instrumented tests swap in a fake core before the Application is created. Production returns null. */
fun interface CoreFactory {
    fun create(container: AppContainer): ReinsCoreInterface?
}

object CoreProvider {
    @Volatile
    var factory: CoreFactory = CoreFactory { null }
}
