//! Payments for the apps (`docs/payments.md`): the purchase screen's approval, the settings and the spending history.

use std::sync::Arc;

use super::ReinsCore;
use crate::connector::payments::{
    BudgetView, PaymentsOverview, PurchaseChoice, SpendLimitInput, SpendLimitView, SpendingView,
};
use crate::{CoreError, rt};

/// Runs `$body` on the signed-in account's engine, like every other call of [`ReinsCore`].
macro_rules! on_engine {
    ($core:ident, |$engine:ident| $body:expr) => {{
        let runtime = Arc::clone(&$core.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let $engine = runtime.prepare(generation).await?;
            $engine.ensure_active()?;
            let result = $engine.run_account(async { $body }).await;
            runtime.finish(&$engine, result)
        })
        .await
    }};
}

#[uniffi::export]
impl ReinsCore {
    /// Approves a purchase with what the user picked on the purchase screen (and maybe a spend limit for more like it).
    pub async fn approve_purchase(&self, request_id: String, choice: PurchaseChoice) -> Result<(), CoreError> {
        on_engine!(self, |engine| engine.approve_purchase(&request_id, choice).await)
    }

    pub async fn payments_overview(&self) -> Result<PaymentsOverview, CoreError> {
        on_engine!(self, |engine| engine.payments_overview().await)
    }

    pub async fn payments_set_method(&self, method_id: String, enabled: bool) -> Result<(), CoreError> {
        on_engine!(self, |engine| engine.payments_set_method(&method_id, enabled))
    }

    pub async fn payments_set_nickname(&self, method_id: String, nickname: Option<String>) -> Result<(), CoreError> {
        on_engine!(self, |engine| engine.payments_set_nickname(&method_id, nickname))
    }

    pub async fn payments_set_defaults(
        &self,
        method_id: Option<String>,
        address_id: Option<String>,
    ) -> Result<(), CoreError> {
        on_engine!(self, |engine| engine.payments_set_defaults(method_id, address_id))
    }

    pub async fn payments_connect_provider(
        &self,
        kind: String,
        api_key: String,
        sandbox: bool,
        single_use: bool,
    ) -> Result<(), CoreError> {
        let api_key = zeroize::Zeroizing::new(api_key);
        on_engine!(self, |engine| engine.payments_connect_provider(&kind, &api_key, sandbox, single_use).await)
    }

    pub async fn payments_disconnect_provider(&self) -> Result<(), CoreError> {
        on_engine!(self, |engine| engine.payments_disconnect_provider().await)
    }

    pub async fn payments_set_card_options(&self, tolerance_pct: u32, single_use: bool) -> Result<(), CoreError> {
        on_engine!(self, |engine| engine.payments_set_card_options(tolerance_pct, single_use))
    }

    pub async fn payments_set_budget(&self, budget: BudgetView) -> Result<(), CoreError> {
        on_engine!(self, |engine| engine.payments_set_budget(budget).await)
    }

    pub async fn payments_add_limit(&self, limit: SpendLimitInput) -> Result<SpendLimitView, CoreError> {
        on_engine!(self, |engine| engine.payments_add_limit(limit).await)
    }

    pub async fn payments_remove_limit(&self, limit_id: String) -> Result<(), CoreError> {
        on_engine!(self, |engine| engine.payments_remove_limit(&limit_id))
    }

    pub async fn payments_spending(&self, since: i64) -> Result<SpendingView, CoreError> {
        on_engine!(self, |engine| engine.payments_spending(since).await)
    }

    pub async fn payments_clear_purchase(&self, purchase_id: String) -> Result<(), CoreError> {
        on_engine!(self, |engine| engine.payments_clear_purchase(&purchase_id).await)
    }

    pub async fn payments_acknowledge_charge(&self, purchase_id: String) -> Result<(), CoreError> {
        on_engine!(self, |engine| engine.payments_acknowledge_charge(&purchase_id).await)
    }

    pub async fn payments_close_card(&self, purchase_id: String) -> Result<(), CoreError> {
        on_engine!(self, |engine| engine.payments_close_card(&purchase_id).await)
    }
}

/// "19.98" in `currency` as minor units (1998), for amounts the user types; an error says what is wrong.
#[uniffi::export]
pub fn payments_parse_amount(text: &str, currency: &str) -> Result<i64, CoreError> {
    let currency = reins_proto::payments::normalize_currency(currency).map_err(CoreError::invalid)?;
    reins_proto::payments::parse_amount(&serde_json::Value::String(text.to_owned()), &currency, "amount")
        .map_err(|_| CoreError::invalid("Enter an amount like 25 or 25.00."))
}

/// Minor units as the user reads them ("$19.98").
#[uniffi::export]
pub fn payments_format_amount(minor: i64, currency: &str) -> String {
    reins_proto::payments::display_amount(minor, &currency.trim().to_ascii_uppercase())
}
