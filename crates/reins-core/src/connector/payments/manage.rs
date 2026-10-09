//! What the Payments screen does: switch methods on and off, connect a virtual card provider, set budgets and spend
//! limits, and read the spending history. Budgets and limits name AI connections by the names the server has.

use reins_proto::connector::PAYMENTS;

use super::{BudgetView, PaymentsOverview, SpendLimitInput, SpendLimitView, SpendingView};
use crate::CoreError;
use crate::engine::Engine;
use crate::store::unix_now;

impl Engine {
    /// The name of an AI connection, from the server's list.
    async fn connection_label(&self, connection_id: &str) -> Result<String, CoreError> {
        self.connections()
            .await?
            .into_iter()
            .find(|c| c.id == connection_id)
            .map(|c| c.label)
            .ok_or_else(|| CoreError::invalid("That AI connection is not known."))
    }

    pub async fn payments_overview(&self) -> Result<PaymentsOverview, CoreError> {
        let enabled = !self.accounts_of(PAYMENTS).is_empty();
        self.payments.overview(enabled, unix_now()).await
    }

    /// Switches a method on or off for AIs: a vault card (`card:<item id>`), `merchant_account` or `pay_on_phone`.
    pub fn payments_set_method(&self, method_id: &str, enabled: bool) -> Result<(), CoreError> {
        self.payments.set_method_enabled(method_id, enabled)
    }

    pub fn payments_set_nickname(&self, method_id: &str, nickname: Option<String>) -> Result<(), CoreError> {
        self.payments.set_nickname(method_id, nickname)
    }

    pub fn payments_set_defaults(
        &self,
        method_id: Option<String>,
        address_id: Option<String>,
    ) -> Result<(), CoreError> {
        self.payments.set_defaults(method_id, address_id)
    }

    /// Checks the API key with the provider and keeps it, encrypted, on this phone.
    pub async fn payments_connect_provider(
        &self,
        kind: &str,
        api_key: &str,
        sandbox: bool,
        single_use: bool,
    ) -> Result<(), CoreError> {
        self.payments.connect_provider(kind, api_key, sandbox, single_use).await
    }

    /// Closes the provider's open cards, then forgets its key and the limits that paid with its cards.
    pub async fn payments_disconnect_provider(&self) -> Result<(), CoreError> {
        let one = self.payments.purchases.lock().await;
        self.payments.disconnect_provider(&one).await
    }

    /// The user says a purchase that did not pay with a virtual card went through for nothing: it stops counting
    /// against budgets and limits.
    pub async fn payments_clear_purchase(&self, purchase_id: &str) -> Result<(), CoreError> {
        let one = self.payments.purchases.lock().await;
        self.payments.clear_purchase(&one, purchase_id)
    }

    pub fn payments_set_card_options(&self, tolerance_pct: u32, single_use: bool) -> Result<(), CoreError> {
        self.payments.set_card_options(tolerance_pct, single_use)
    }

    /// Sets (or, with nothing in it, removes) the budget of every AI (`connection_id` empty) or of one.
    pub async fn payments_set_budget(&self, mut budget: BudgetView) -> Result<(), CoreError> {
        budget.connection_label = if budget.connection_id.is_empty() {
            String::new()
        } else {
            self.connection_label(&budget.connection_id).await?
        };
        self.payments.set_budget(&budget)
    }

    pub async fn payments_add_limit(&self, limit: SpendLimitInput) -> Result<SpendLimitView, CoreError> {
        let label = self.connection_label(&limit.connection_id).await?;
        self.payments.add_limit(&limit, &limit.connection_id, &label, unix_now())
    }

    pub fn payments_remove_limit(&self, limit_id: &str) -> Result<(), CoreError> {
        self.payments.remove_limit(limit_id)
    }

    /// Every purchase kept, with what was spent since `since` (the app's start of the month, in its time zone).
    pub async fn payments_spending(&self, since: i64) -> Result<SpendingView, CoreError> {
        self.payments.spending(since, unix_now()).await
    }

    /// The user has seen that a virtual card was charged by another store: spend limits for that AI work again.
    pub async fn payments_acknowledge_charge(&self, purchase_id: &str) -> Result<(), CoreError> {
        let one = self.payments.purchases.lock().await;
        self.payments.acknowledge_charge(&one, purchase_id)
    }

    /// Closes the virtual card of a purchase now.
    pub async fn payments_close_card(&self, purchase_id: &str) -> Result<(), CoreError> {
        let one = self.payments.purchases.lock().await;
        self.payments.close_card(&one, purchase_id).await
    }
}
