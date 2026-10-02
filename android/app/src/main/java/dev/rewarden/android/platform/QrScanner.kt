package dev.rewarden.android.platform

import android.content.Context
import com.google.mlkit.vision.barcode.common.Barcode
import com.google.mlkit.vision.codescanner.GmsBarcodeScannerOptions
import com.google.mlkit.vision.codescanner.GmsBarcodeScanning
import kotlin.coroutines.cancellation.CancellationException
import kotlin.coroutines.resume
import kotlinx.coroutines.suspendCancellableCoroutine

/** How a scan ended. */
sealed interface ScanResult {
    /** The text in the QR code, untrusted. */
    data class Scanned(val text: String) : ScanResult

    /** The user closed the scanner. */
    data object Cancelled : ScanResult

    /** The scanner could not run (no Google Play services, or its module did not download). */
    data class Unavailable(val reason: String?) : ScanResult
}

/** Scans one QR code. */
fun interface QrScanner {
    suspend fun scan(): ScanResult
}

/**
 * Google's code scanner: Play services shows its own camera screen and hands back only the result, so this app needs
 * no camera permission and never sees a camera frame.
 */
class GmsQrScanner(private val context: Context) : QrScanner {
    override suspend fun scan(): ScanResult = suspendCancellableCoroutine { continuation ->
        val task = try {
            val options = GmsBarcodeScannerOptions.Builder().setBarcodeFormats(Barcode.FORMAT_QR_CODE).build()
            GmsBarcodeScanning.getClient(context, options).startScan()
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            continuation.resume(ScanResult.Unavailable(e.message))
            return@suspendCancellableCoroutine
        }
        task
            .addOnSuccessListener { barcode ->
                val text = barcode.rawValue
                continuation.resume(if (text.isNullOrEmpty()) ScanResult.Cancelled else ScanResult.Scanned(text))
            }
            .addOnCanceledListener { continuation.resume(ScanResult.Cancelled) }
            .addOnFailureListener { continuation.resume(ScanResult.Unavailable(it.message)) }
    }
}

/** Lets tests scan without Play services. */
object QrScannerProvider {
    @Volatile
    var factory: (Context) -> QrScanner = { GmsQrScanner(it) }
}
