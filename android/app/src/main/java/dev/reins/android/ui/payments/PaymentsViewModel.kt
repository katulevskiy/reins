package dev.reins.android.ui.payments

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import dev.reins.android.AppContainer
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.play
import dev.reins.android.ui.common.userMessage
import dev.reins.core.BudgetView
import dev.reins.core.PaymentsOverview
import dev.reins.core.SpendLimitInput
import dev.reins.core.SpendingView
import kotlin.coroutines.cancellation.CancellationException
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

/** The account Payments has: this phone (the core's `PAYMENTS_ACCOUNT`). */
const val PAYMENTS_ACCOUNT = "this phone"

data class PaymentsUi(
    val loading: Boolean = true,
    val overview: PaymentsOverview? = null,
    val spending: SpendingView? = null,
    val busy: Boolean = false,
    val error: String? = null,
    /** What just happened, shown once ("Privacy.com is connected"). */
    val notice: String? = null,
)

/** Integrations > Payments and Payments > Spending. Every change goes to the core, then the screen reloads. */
class PaymentsViewModel(private val container: AppContainer) : ViewModel() {
    private val _ui = MutableStateFlow(PaymentsUi())
    val ui: StateFlow<PaymentsUi> = _ui.asStateFlow()

    fun refresh() {
        viewModelScope.launch { load() }
    }

    private suspend fun load() {
        try {
            val overview = container.core.paymentsOverview()
            _ui.update { it.copy(loading = false, overview = overview) }
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            _ui.update { it.copy(loading = false, error = e.userMessage()) }
        }
    }

    fun loadSpending() {
        viewModelScope.launch {
            try {
                val spending = container.core.paymentsSpending(Money.monthStart())
                _ui.update { it.copy(spending = spending) }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                _ui.update { it.copy(error = e.userMessage()) }
            }
        }
    }

    fun clearNotice() = _ui.update { it.copy(notice = null, error = null) }

    /** Runs one change; failures are shown and nothing else starts meanwhile. */
    private fun change(success: Event? = null, notice: String? = null, spending: Boolean = false, block: suspend () -> Unit) {
        if (_ui.value.busy) return
        _ui.update { it.copy(busy = true, error = null, notice = null) }
        viewModelScope.launch {
            try {
                block()
                success?.let { container.feedback.play(it) }
                load()
                if (spending) loadSpending()
                _ui.update { it.copy(busy = false, notice = notice) }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                container.feedback.play(Event.Error)
                _ui.update { it.copy(busy = false, error = e.userMessage()) }
            }
        }
    }

    /** Switches Payments on: the integration gets its one account, and the AIs see its tools. */
    fun enable() = change(Event.Connected) {
        container.core.addServiceAccount("payments", "")
        container.state.setServices(container.core.services())
    }

    fun disable() = change {
        container.core.removeServiceAccount("payments", PAYMENTS_ACCOUNT)
        container.state.setServices(container.core.services())
    }

    fun setMethod(id: String, enabled: Boolean) = change { container.core.paymentsSetMethod(id, enabled) }

    fun setDefaultMethod(id: String?) = change {
        container.core.paymentsSetDefaults(id, _ui.value.overview?.defaultAddress)
    }

    fun setDefaultAddress(id: String?) = change {
        container.core.paymentsSetDefaults(_ui.value.overview?.defaultMethod, id)
    }

    fun connectProvider(apiKey: String, sandbox: Boolean, singleUse: Boolean) =
        change(Event.Connected, notice = "Privacy.com is connected.") {
            container.core.paymentsConnectProvider("privacy", apiKey, sandbox, singleUse)
        }

    fun disconnectProvider() = change { container.core.paymentsDisconnectProvider() }

    fun setCardOptions(tolerancePct: Int, singleUse: Boolean) = change {
        container.core.paymentsSetCardOptions(tolerancePct.coerceIn(0, 50).toUInt(), singleUse)
    }

    fun addLimit(limit: SpendLimitInput) = change(Event.GrantCreated, notice = "Spend limit added.") {
        container.core.paymentsAddLimit(limit)
    }

    fun removeLimit(id: String) = change { container.core.paymentsRemoveLimit(id) }

    fun setBudget(budget: BudgetView) = change(notice = "Budget saved.") { container.core.paymentsSetBudget(budget) }

    fun closeCard(purchaseId: String) = change(spending = true) { container.core.paymentsCloseCard(purchaseId) }

    fun acknowledge(purchaseId: String) = change(spending = true) { container.core.paymentsAcknowledgeCharge(purchaseId) }

    /** A purchase not paid with a virtual card went through for nothing: it stops counting against budgets. */
    fun clear(purchaseId: String) = change(spending = true) { container.core.paymentsClearPurchase(purchaseId) }
}
