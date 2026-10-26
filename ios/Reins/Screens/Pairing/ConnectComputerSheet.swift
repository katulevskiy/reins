import SwiftUI
import VisionKit

/// Connecting a computer (the Connect board, 02): scan the QR code `reins login` (or the Reins desktop app) shows, or
/// type the eight letters under it into the code cells, and the pairing it stands for takes this sheet's place, to be
/// answered like any other (the number the computer shows, its key, Face ID). Without a usable camera (the simulator,
/// no permission) the code is typed.
struct ConnectComputerSheet: View {
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @State private var pairing: PairingModel?
    @State private var typing: Bool
    /// The letters typed so far (no dash), at most eight.
    @State private var code = ""
    @State private var busy = false
    @State private var error: String?
    @State private var copied = false
    /// The last code the camera read, so one QR code held in view is redeemed once.
    @State private var scanned: String?
    @FocusState private var focused: Bool

    /// `typing`: open on the code cells (Settings' "Type code"); without a usable camera it always does.
    init(typing: Bool = false) {
        startTyped = typing
        _typing = State(initialValue: typing || !QRScanner.available)
    }

    private let startTyped: Bool

    var body: some View {
        if let pairing {
            PairingSheet(model: pairing)
                .presentationDetents([.large])
        } else {
            ScrollView { content.padding(.top, 14).padding(.bottom, 24) }
                .scrollDismissesKeyboard(.interactively)
                .scrollBounceBehavior(.basedOnSize)
                .background(Palette.paper.ignoresSafeArea())
                .presentationDetents(typing ? [.height(error == nil ? 452 : 520), .large] : [.height(600), .large])
                .presentationDragIndicator(.visible)
                .presentationCornerRadius(32)
                .accessibilityContainer("connectComputerSheet")
                .onAppear {
                    model.connectStartsTyped = false
                    if typing { focused = true }
                }
        }
    }

    private var content: some View {
        VStack(alignment: .leading, spacing: 0) {
            SheetHeading("Computer") { model.closeSheet() }
            if typing { typed } else { camera }
            if let error {
                FormBanner(text: error)
                    .padding(.horizontal, 16)
                    .padding(.top, 12)
                    .accessibilityIdentifier("pairingCodeError")
            }
            if QRScanner.available {
                Button {
                    feedback.play(.tap)
                    error = nil
                    typing.toggle()
                    focused = typing
                } label: {
                    Label { Text(typing ? "Scan instead" : "Type instead") } icon: { GlyphIcon(typing ? .qrCode : .pencil, size: 18) }
                        .labelStyle(SignalLabelStyle())
                        .font(RFont.sans(16, .semibold))
                        .foregroundStyle(Palette.violet)
                        .frame(maxWidth: .infinity, minHeight: 46)
                }
                .buttonStyle(PressDimsStyle())
                .padding(.horizontal, 16)
                .padding(.top, 6)
                .disabled(busy)
                .accessibilityIdentifier("switchEntry")
            }
        }
        .animation(.smooth(duration: 0.25), value: typing)
        .animation(.smooth(duration: 0.25), value: error)
    }

    /// "Run reins login" and Copy: the command, in code type.
    private var command: some View {
        RaisedGroup {
            HStack(spacing: 8) {
                Text("Run").font(RFont.sans(15)).foregroundStyle(Palette.ink2)
                Text(verbatim: "reins login")
                    .font(RFont.mono(15))
                    .foregroundStyle(Palette.ink)
                    .frame(maxWidth: .infinity, alignment: .leading)
                RoundHeaderButton(glyph: copied ? .check : .copy, label: "Copy reins login") {
                    copyPlain("reins login", feedback: feedback)
                    copied = true
                }
                .accessibilityIdentifier("copyCommand")
            }
            .padding(.leading, 14)
            .padding(.trailing, 12)
            .frame(minHeight: 56)
        }
        .padding(.top, 12)
    }

    private var camera: some View {
        VStack(spacing: 0) {
            command
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
            .padding(.horizontal, 16)
            .padding(.top, 18)
            .accessibilityLabel("Camera, looking for the QR code")
        }
    }

    @ViewBuilder private var typed: some View {
        command
        // The real field sits invisible under the cells, so typing, pasting and the keyboard's suggestions work as usual.
        ZStack {
            TextField("BCDF-GHJK", text: Binding(get: { code }, set: { enter($0) }))
                .textInputAutocapitalization(.characters)
                .autocorrectionDisabled()
                .keyboardType(.asciiCapable)
                .textContentType(.oneTimeCode)
                .submitLabel(.go)
                .focused($focused)
                .onSubmit(submitTyped)
                .disabled(busy)
                .foregroundStyle(.clear)
                .tint(.clear)
                .opacity(0.02)
                .accessibilityLabel("Pairing code")
                .accessibilityValue(code)
                .accessibilityIdentifier("pairingCode")
            HStack(spacing: 6) {
                let letters = Array(code)
                ForEach(0..<8, id: \.self) { i in
                    if i == 4 {
                        Text("-").font(RFont.mono(22)).foregroundStyle(Palette.inkFaint)
                    }
                    CodeCell(character: i < letters.count ? String(letters[i]) : "", next: focused && i == letters.count)
                }
            }
            .allowsHitTesting(false)
            .accessibilityHidden(true)
        }
        .contentShape(Rectangle())
        .onTapGesture { focused = true }
        .padding(.horizontal, 16)
        .padding(.top, 18)
        let valid = PairingCode.parse(code) != nil
        Button(action: submitTyped) {
            HStack(spacing: 10) {
                if busy { ProgressView().tint(Palette.onInverse) }
                Text("Continue")
            }
        }
        .buttonStyle(SignalButtonStyle(.inverse))
        .disabled(!valid || busy)
        .padding(.horizontal, 16)
        .padding(.top, 18)
        .accessibilityIdentifier("pairWithCode")
    }

    /// What was typed or pasted, as the code's letters: a pasted link or code becomes its eight letters.
    private func enter(_ text: String) {
        if let found = PairingCode.parse(text) {
            code = found.replacingOccurrences(of: "-", with: "")
        } else {
            code = String(text.uppercased().filter { $0.isLetter || $0.isNumber }.prefix(PairingCode.length))
        }
        error = nil
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
