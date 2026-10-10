import Foundation

// Payments is not in the iOS app yet: the demo core answers its calls as a phone that has not turned Payments on, so
// that the app builds against the core. The screens come with the iOS payments work.
extension DemoReinsCore {
    private static var paymentsUnavailable: CoreError { .Invalid(reason: "Payments is not available on iOS yet.") }

    func approvePurchase(requestId: String, choice: PurchaseChoice) async throws { throw Self.paymentsUnavailable }

    func paymentsAcknowledgeCharge(purchaseId: String) async throws { throw Self.paymentsUnavailable }

    func paymentsAddLimit(limit: SpendLimitInput) async throws -> SpendLimitView { throw Self.paymentsUnavailable }

    func paymentsClearPurchase(purchaseId: String) async throws { throw Self.paymentsUnavailable }

    func paymentsCloseCard(purchaseId: String) async throws { throw Self.paymentsUnavailable }

    func paymentsConnectProvider(kind: String, apiKey: String, sandbox: Bool, singleUse: Bool) async throws {
        throw Self.paymentsUnavailable
    }

    func paymentsDisconnectProvider() async throws { throw Self.paymentsUnavailable }

    func paymentsOverview() async throws -> PaymentsOverview {
        PaymentsOverview(
            enabled: false,
            vaultReady: false,
            methods: [],
            addresses: [],
            provider: nil,
            defaultMethod: nil,
            defaultAddress: nil,
            tolerancePct: 10,
            budgets: [],
            limits: [],
            mandateKey: ""
        )
    }

    func paymentsRemoveLimit(limitId: String) async throws { throw Self.paymentsUnavailable }

    func paymentsSetBudget(budget: BudgetView) async throws { throw Self.paymentsUnavailable }

    func paymentsSetCardOptions(tolerancePct: UInt32, singleUse: Bool) async throws { throw Self.paymentsUnavailable }

    func paymentsSetDefaults(methodId: String?, addressId: String?) async throws { throw Self.paymentsUnavailable }

    func paymentsSetMethod(methodId: String, enabled: Bool) async throws { throw Self.paymentsUnavailable }

    func paymentsSetNickname(methodId: String, nickname: String?) async throws { throw Self.paymentsUnavailable }

    func paymentsSpending(since: Int64) async throws -> SpendingView {
        SpendingView(since: since, totals: [], byAi: [], purchases: [])
    }
}
