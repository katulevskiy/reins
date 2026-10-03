package dev.reins.android.platform.update

/** Why an update check or download failed. [message] is written for the user. */
sealed class UpdateException(message: String, cause: Throwable? = null) : Exception(message, cause) {
    /** The manifest is not one this app can use; [detail] says which check failed (for logs). */
    class BadManifest(val detail: String) : UpdateException("The update server sent a release this app can't read.")

    class Network(cause: Throwable? = null) :
        UpdateException("Couldn't reach the update server. Check your connection and try again.", cause)

    class Http(val status: Int) : UpdateException("The update server answered with error $status. Try again later.")

    /** The download does not match the release (size or SHA-256); it has been deleted. */
    class Corrupt(val detail: String) : UpdateException("The download didn't match the release and was deleted. Try again.")

    class Storage(cause: Throwable? = null) : UpdateException("Not enough free space to download the update.", cause)
}
