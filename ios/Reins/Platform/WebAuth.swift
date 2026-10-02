import AuthenticationServices
import UIKit

/// A sign-in page in the system's web sheet (`ASWebAuthenticationSession`, the Custom Tab of the Android app), returning
/// the address it redirected to. The page shares Safari's cookies, so an account already signed in there is offered.
@MainActor
enum WebAuth {
    enum Failure: Error, Equatable {
        case cancelled
        /// No window to show the page over (the app is not in front).
        case noWindow
        case failed(String)
    }

    /// The sessions running now; ASWebAuthenticationSession must be kept alive until it ends.
    private static var running: [ObjectIdentifier: (ASWebAuthenticationSession, Anchor)] = [:]

    /// Opens `url`; returns the redirect to `callbackScheme://...`.
    static func run(_ url: URL, callbackScheme: String, ephemeral: Bool = false) async throws -> URL {
        guard let window = Anchor.window() else { throw Failure.noWindow }
        let anchor = Anchor(window: window)
        return try await withCheckedThrowingContinuation { continuation in
            var key: ObjectIdentifier?
            let session = ASWebAuthenticationSession(url: url, callback: .customScheme(callbackScheme)) { redirect, error in
                MainActor.assumeIsolated {
                    if let key { running[key] = nil }
                    if let redirect {
                        continuation.resume(returning: redirect)
                    } else if let error = error as? ASWebAuthenticationSessionError, error.code == .canceledLogin {
                        continuation.resume(throwing: Failure.cancelled)
                    } else {
                        continuation.resume(throwing: Failure.failed(error?.localizedDescription ?? "The sign-in page closed without an answer."))
                    }
                }
            }
            session.presentationContextProvider = anchor
            session.prefersEphemeralWebBrowserSession = ephemeral
            let id = ObjectIdentifier(session)
            key = id
            running[id] = (session, anchor)
            if !session.start() {
                running[id] = nil
                continuation.resume(throwing: Failure.failed("The sign-in page could not be opened."))
            }
        }
    }

    /// The window the page slides over: the key window of the scene in front.
    final class Anchor: NSObject, ASWebAuthenticationPresentationContextProviding {
        let window: UIWindow

        init(window: UIWindow) {
            self.window = window
        }

        nonisolated func presentationAnchor(for session: ASWebAuthenticationSession) -> ASPresentationAnchor {
            MainActor.assumeIsolated { window }
        }

        static func window() -> UIWindow? {
            let scenes = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }
            let front = scenes.first { $0.activationState == .foregroundActive } ?? scenes.first { $0.activationState == .foregroundInactive }
            return front?.keyWindow ?? front?.windows.first
        }
    }
}
