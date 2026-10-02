import SwiftUI
import VisionKit

/// Connecting a computer: scan the QR code `rewarden login` (or the Reins desktop app) shows, or type the code under
/// it, and the pairing it stands for takes this sheet's place, to be answered like any other (the number the computer
/// shows, its key, Face ID). Without a usable camera (the simulator, no permission) the code is typed.
struct ConnectComputerSheet: View {
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @State private var pairing: PairingModel?
    @State private var typing = !QRScanner.available
    @State private var code = ""
    @State private var busy = false
    @State private var error: String?
    /// The last code the camera read, so one QR code held in view is redeemed once.
    @State private var scanned: String?
    @FocusState private var focused: Bool

    var body: some View {
        if let pairing {
            PairingSheet(model: pairing)
        } else {
            ScrollView { content.padding(20).padding(.top, 8) }
                .scrollDismissesKeyboard(.interactively)
                .background(Palette.background.ignoresSafeArea())
                .overlay(alignment: .topTrailing) {
                    GlassIconButton(symbol: "xmark", size: 36, label: "Close") { model.closeSheet() }
                        .padding(.top, 14)
                        .padding(.trailing, 14)
                        .accessibilityIdentifier("closeSheet")
                }
                .accessibilityContainer("connectComputerSheet")
        }
    }

    private var content: some View {
        VStack(alignment: .leading, spacing: 14) {
            Image(systemName: "desktopcomputer")
                .font(.system(size: 24, weight: .semibold))
                .foregroundStyle(Palette.pair)
                .frame(width: 52, height: 52)
                .background(Palette.pair.opacity(0.12), in: RoundedRectangle(cornerRadius: 16, style: .continuous))
                .accessibilityHidden(true)
            Text("Connect a computer")
                .font(RFont.sans(26, .semibold))
                .foregroundStyle(Palette.text)
                .accessibilityAddTraits(.isHeader)
            // A key, not a String, so the command shows in code type.
            Text(LocalizedStringKey(typing
                ? "Run `rewarden login` on your computer, or open the Reins desktop app, and type the code it shows."
                : "Run `rewarden login` on your computer, or open the Reins desktop app, and point the camera at the QR code it shows."))
                .font(RFont.sans(15.5))
                .foregroundStyle(Palette.secondary)
                .fixedSize(horizontal: false, vertical: true)
            if typing { typed } else { camera }
            if let error {
                FormBanner(text: error).accessibilityIdentifier("pairingCodeError")
            }
            if QRScanner.available {
                Button(typing ? "Scan the QR code instead" : "Type the code instead") {
                    feedback.play(.tap)
                    error = nil
                    typing.toggle()
                    focused = typing
                }
                .font(RFont.sans(15, .medium))
                .foregroundStyle(Palette.accent)
                .frame(maxWidth: .infinity)
                .padding(.vertical, 6)
                .disabled(busy)
                .accessibilityIdentifier("switchEntry")
            }
        }
        .animation(.smooth(duration: 0.25), value: typing)
        .animation(.smooth(duration: 0.25), value: error)
    }

    private var camera: some View {
        QRScanner { text in
            guard let found = PairingCode.parse(text), found != scanned else { return }
            scanned = found
            redeem(found)
        }
        .frame(height: 320)
        .clipShape(RoundedRectangle(cornerRadius: 24, style: .continuous))
        .overlay {
            if busy {
                ProgressView().controlSize(.large).tint(.white)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .background(.black.opacity(0.35), in: RoundedRectangle(cornerRadius: 24, style: .continuous))
            }
        }
        .accessibilityLabel("Camera, looking for the QR code")
    }

    @ViewBuilder private var typed: some View {
        TextField("BCDF-GHJK", text: $code)
            .textInputAutocapitalization(.characters)
            .autocorrectionDisabled()
            .keyboardType(.asciiCapable)
            .textContentType(.oneTimeCode)
            .submitLabel(.go)
            .focused($focused)
            .onSubmit(submitTyped)
            .disabled(busy)
            .fieldWell(mono: true)
            .accessibilityLabel("Pairing code")
            .accessibilityIdentifier("pairingCode")
        let valid = PairingCode.parse(code) != nil
        Button(action: submitTyped) {
            HStack(spacing: 10) {
                if busy { ProgressView().tint(Palette.background) }
                Text("Continue")
            }
        }
        .buttonStyle(CapsuleButtonStyle(kind: .primary))
        .disabled(!valid || busy)
        .opacity(valid ? 1 : 0.45)
        .accessibilityIdentifier("pairWithCode")
    }

    private func submitTyped() {
        guard let found = PairingCode.parse(code) else { return }
        focused = false
        redeem(found)
    }

    private func redeem(_ found: String) {
        guard !busy else { return }
        busy = true
        error = nil
        Task {
            do {
                let view = try await model.redeemPairingCode(found)
                feedback.play(.selection)
                withAnimation(.smooth(duration: 0.3)) { pairing = PairingModel(view: view) }
            } catch {
                feedback.play(.error)
                self.error = AppModel.pairingCodeMessage(error)
                // The same QR code may be scanned again after the error.
                scanned = nil
            }
            busy = false
        }
    }
}

/// VisionKit's live scanner, reading QR codes only, wrapped for SwiftUI. `available` is false on the simulator, on a
/// device without a camera, and when camera access was turned off for Reins.
struct QRScanner: UIViewControllerRepresentable {
    var onCode: (String) -> Void

    static var available: Bool { DataScannerViewController.isSupported && DataScannerViewController.isAvailable }

    func makeUIViewController(context: Context) -> DataScannerViewController {
        let scanner = DataScannerViewController(
            recognizedDataTypes: [.barcode(symbologies: [.qr])],
            qualityLevel: .balanced,
            recognizesMultipleItems: false,
            isHighFrameRateTrackingEnabled: false,
            isPinchToZoomEnabled: true,
            isGuidanceEnabled: true,
            isHighlightingEnabled: true
        )
        scanner.delegate = context.coordinator
        // Scanning starts once the controller is on screen.
        DispatchQueue.main.async { try? scanner.startScanning() }
        return scanner
    }

    func updateUIViewController(_ scanner: DataScannerViewController, context: Context) {
        context.coordinator.onCode = onCode
    }

    static func dismantleUIViewController(_ scanner: DataScannerViewController, coordinator: Coordinator) {
        scanner.stopScanning()
    }

    func makeCoordinator() -> Coordinator { Coordinator(onCode: onCode) }

    @MainActor
    final class Coordinator: NSObject, DataScannerViewControllerDelegate {
        var onCode: (String) -> Void

        init(onCode: @escaping (String) -> Void) { self.onCode = onCode }

        func dataScanner(_ dataScanner: DataScannerViewController, didAdd addedItems: [RecognizedItem], allItems: [RecognizedItem]) {
            for item in addedItems {
                if case let .barcode(barcode) = item, let text = barcode.payloadStringValue {
                    onCode(text)
                    return
                }
            }
        }
    }
}
