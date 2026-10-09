package dev.reins.android

import dev.reins.android.platform.PasskeyPrompt
import dev.reins.android.platform.PasskeyResult
import dev.reins.core.VaultPasskeyOptions
import java.util.concurrent.CopyOnWriteArrayList

/** The platform's passkey UI in tests: answers what the test set, records what it was asked. */
class FakePasskeys : PasskeyPrompt {
    /** What "make a passkey" answers: by default a new passkey whose PRF output opens the fake vault. */
    @Volatile var createResult: PasskeyResult = made()
    /** What "use a passkey" answers: by default the account's existing passkey. */
    @Volatile var getResult: PasskeyResult = PasskeyResult.Passkey(FakeCore.EXISTING_PASSKEY_ID, FakeCore.PASSKEY_PRF)
    val creates = CopyOnWriteArrayList<VaultPasskeyOptions>()
    /** The allowed credential ids of each "use a passkey". */
    val gets = CopyOnWriteArrayList<List<ByteArray>>()

    override suspend fun create(options: VaultPasskeyOptions): PasskeyResult {
        creates += options
        return createResult
    }

    override suspend fun get(options: VaultPasskeyOptions, allowed: List<ByteArray>): PasskeyResult {
        gets += allowed
        return getResult
    }

    fun reset() {
        createResult = made()
        getResult = PasskeyResult.Passkey(FakeCore.EXISTING_PASSKEY_ID, FakeCore.PASSKEY_PRF)
        creates.clear()
        gets.clear()
    }

    companion object {
        val NEW_PASSKEY_ID = byteArrayOf(9, 8, 7, 6)

        /** A passkey just made; [prf] null: the provider gives it only when the passkey is used. */
        fun made(prf: ByteArray? = FakeCore.PASSKEY_PRF) = PasskeyResult.Passkey(NEW_PASSKEY_ID, prf)
    }
}
