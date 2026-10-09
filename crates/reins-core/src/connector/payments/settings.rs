//! What the user set up for Payments, and the purchases made: kept as two JSON documents in the phone's encrypted
//! store (`payments.config`, `payments.ledger`), which travel with the account state like the rest of it. Neither
//! ever holds a card number or a security code.

use std::collections::BTreeMap;

use reins_proto::payments::in_domain;
use serde::{Deserialize, Serialize};

use crate::CoreError;
use crate::store::Store;

const CONFIG_KEY: &str = "payments.config";
const LEDGER_KEY: &str = "payments.ledger";
/// Purchases kept; older ones are dropped.
pub const LEDGER_RETENTION: usize = 1_000;
pub const DAY: i64 = 86_400;
/// The longest a spend limit may last.
pub const MAX_LIMIT_SECS: i64 = 90 * DAY;
/// How long a virtual card made for a completed purchase stays open (split shipments), unless closed by hand.
pub const VIRTUAL_CARD_DAYS: i64 = 30;
/// How long a virtual card stays open when the AI never says how checkout went.
pub const UNREPORTED_CARD_SECS: i64 = DAY;

pub const VIRTUAL_CARD: &str = "virtual_card";
pub const MERCHANT_ACCOUNT: &str = "merchant_account";
pub const PAY_ON_PHONE: &str = "pay_on_phone";
/// Vault cards are `card:<item id>`.
pub const CARD_PREFIX: &str = "card:";

/// A virtual card provider the user connected (its API key is a secret of its own).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provider {
    /// "privacy".
    pub kind: String,
    #[serde(default)]
    pub sandbox: bool,
    /// Single-use cards close after the first charge; merchant-locked ones (the default) allow split shipments.
    #[serde(default)]
    pub single_use: bool,
    pub added_at: i64,
}

/// Refuses purchases before they reach the user. `connection_id` empty: every AI together.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Budget {
    #[serde(default)]
    pub connection_id: String,
    #[serde(default)]
    pub connection_label: String,
    pub currency: String,
    #[serde(default)]
    pub per_purchase: Option<i64>,
    #[serde(default)]
    pub per_day: Option<i64>,
    #[serde(default)]
    pub per_month: Option<i64>,
    /// When not empty, only these store domains.
    #[serde(default)]
    pub merchants: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "snake_case")]
pub enum LimitPeriod {
    Day,
    Week,
    Month,
}

impl LimitPeriod {
    #[must_use]
    pub fn secs(self) -> i64 {
        match self {
            Self::Day => DAY,
            Self::Week => 7 * DAY,
            Self::Month => 30 * DAY,
        }
    }

    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Self::Day => "day",
            Self::Week => "week",
            Self::Month => "month",
        }
    }
}

/// Lets one AI buy without asking, within these amounts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpendLimit {
    pub id: String,
    pub connection_id: String,
    pub connection_label: String,
    /// Store domains; empty: every store.
    #[serde(default)]
    pub merchants: Vec<String>,
    /// `virtual_card` or `merchant_account`.
    pub method: String,
    pub currency: String,
    pub per_purchase: i64,
    pub per_period: i64,
    pub period: LimitPeriod,
    pub created_at: i64,
    pub expires_at: i64,
}

impl SpendLimit {
    #[must_use]
    pub fn covers_merchant(&self, domain: &str) -> bool {
        self.merchants.is_empty() || self.merchants.iter().any(|m| in_domain(domain, m))
    }
}

/// Everything the user set up.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    /// Vault card item ids switched on.
    #[serde(default)]
    pub cards: Vec<String>,
    #[serde(default = "yes")]
    pub merchant_account: bool,
    #[serde(default = "yes")]
    pub pay_on_phone: bool,
    #[serde(default)]
    pub provider: Option<Provider>,
    /// Method id → the nickname the user gave it.
    #[serde(default)]
    pub nicknames: BTreeMap<String, String>,
    #[serde(default)]
    pub default_method: Option<String>,
    #[serde(default)]
    pub default_address: Option<String>,
    /// How much more than the approved total a virtual card allows, in percent…
    #[serde(default = "ten")]
    pub tolerance_pct: u32,
    #[serde(default)]
    pub budgets: Vec<Budget>,
    #[serde(default)]
    pub limits: Vec<SpendLimit>,
}

fn yes() -> bool {
    true
}

fn ten() -> u32 {
    10
}

impl Default for Config {
    fn default() -> Self {
        Self {
            cards: Vec::new(),
            merchant_account: true,
            pay_on_phone: true,
            provider: None,
            nicknames: BTreeMap::new(),
            default_method: None,
            default_address: None,
            tolerance_pct: ten(),
            budgets: Vec::new(),
            limits: Vec::new(),
        }
    }
}

impl Config {
    /// …and at least this many minor units (1.00 in most currencies).
    #[must_use]
    pub fn tolerance(&self, total: i64, currency: &str) -> i64 {
        let floor = 10_i64.pow(reins_proto::payments::currency_exponent(currency));
        // Rounded up: the card never allows less than the percentage the user chose.
        (total.saturating_mul(i64::from(self.tolerance_pct)).saturating_add(99) / 100).max(floor)
    }
}

/// How a purchase ended up, as far as the phone knows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PurchaseStatus {
    /// Approved; the AI has not said how checkout went.
    Approved,
    Completed,
    Failed,
    Cancelled,
}

impl PurchaseStatus {
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Self::Approved => "approved",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    /// Whether the money counts against budgets and limits.
    #[must_use]
    pub fn spends(self) -> bool {
        matches!(self, Self::Approved | Self::Completed)
    }
}

/// A virtual card made for one purchase: the provider's id and the last four digits, never the number.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CardRef {
    pub provider: String,
    #[serde(default)]
    pub sandbox: bool,
    pub token: String,
    pub last4: String,
    /// The most it can be charged, in minor units of the purchase's currency.
    pub limit: i64,
    #[serde(default)]
    pub closed_at: Option<i64>,
}

/// One approved purchase.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Purchase {
    /// The request id: the `purchase_id` the AI got.
    pub id: String,
    pub at: i64,
    pub connection_id: String,
    pub connection_label: String,
    pub merchant: String,
    pub domain: String,
    /// "2 × USB-C cable", one per line of the cart.
    pub lines: Vec<String>,
    pub currency: String,
    pub total: i64,
    /// `virtual_card`, `card`, `merchant_account` or `pay_on_phone`.
    pub method_kind: String,
    /// "Visa •• 1111", "Saved at amazon.com".
    pub method_label: String,
    #[serde(default)]
    pub ship_to: Option<String>,
    /// "you", or the spend limit's id.
    pub approved_by: String,
    pub status: PurchaseStatus,
    #[serde(default)]
    pub order_id: Option<String>,
    /// What the store charged, as the AI reported it (same currency).
    #[serde(default)]
    pub charged: Option<i64>,
    #[serde(default)]
    pub receipt_url: Option<String>,
    #[serde(default)]
    pub report_note: Option<String>,
    #[serde(default)]
    pub reported_at: Option<i64>,
    #[serde(default)]
    pub card: Option<CardRef>,
    /// Who charged the virtual card, as the card network names them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub charged_by: Vec<String>,
    /// A charge by someone who does not look like the store that was approved; spend limits for this AI stop until
    /// the user has seen it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mismatch: Option<String>,
    #[serde(default)]
    pub mismatch_seen: bool,
}

impl Purchase {
    /// A charge by another store that the user has not looked at yet.
    #[must_use]
    pub fn unseen_mismatch(&self) -> Option<&str> {
        self.mismatch.as_deref().filter(|_| !self.mismatch_seen)
    }
}

impl Purchase {
    /// What it counts for in budgets and limits.
    #[must_use]
    pub fn spent(&self) -> i64 {
        if self.status.spends() {
            self.charged.unwrap_or(self.total)
        } else {
            0
        }
    }
}

/// What was spent since `since`, in `currency`, by one AI (`Some`) or by all of them, at stores `domain_ok` accepts.
#[must_use]
pub fn spent_since(
    ledger: &[Purchase],
    since: i64,
    currency: &str,
    connection: Option<&str>,
    domain_ok: impl Fn(&str) -> bool,
) -> i64 {
    ledger
        .iter()
        .filter(|p| p.at >= since && p.currency == currency)
        .filter(|p| connection.is_none_or(|c| p.connection_id == c))
        .filter(|p| domain_ok(&p.domain))
        .map(Purchase::spent)
        .sum()
}

pub(super) fn load_config(store: &Store) -> Result<Config, CoreError> {
    match store.meta_get(CONFIG_KEY)? {
        Some(json) => serde_json::from_str(&json).map_err(|_| CoreError::storage("unreadable payment settings")),
        None => Ok(Config::default()),
    }
}

pub(super) fn save_config(store: &Store, config: &Config) -> Result<(), CoreError> {
    let json = serde_json::to_string(config).map_err(|_| CoreError::storage("cannot encode payment settings"))?;
    store.meta_set(CONFIG_KEY, &json)
}

/// Newest first.
pub(super) fn load_ledger(store: &Store) -> Result<Vec<Purchase>, CoreError> {
    match store.meta_get(LEDGER_KEY)? {
        Some(json) => serde_json::from_str(&json).map_err(|_| CoreError::storage("unreadable purchase history")),
        None => Ok(Vec::new()),
    }
}

pub(super) fn save_ledger(store: &Store, ledger: &mut Vec<Purchase>) -> Result<(), CoreError> {
    ledger.sort_by_key(|p| std::cmp::Reverse(p.at));
    ledger.truncate(LEDGER_RETENTION);
    let json = serde_json::to_string(ledger).map_err(|_| CoreError::storage("cannot encode purchase history"))?;
    store.meta_set(LEDGER_KEY, &json)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn purchase(id: &str, at: i64, connection: &str, domain: &str, total: i64, status: PurchaseStatus) -> Purchase {
        Purchase {
            id: id.to_owned(),
            at,
            connection_id: connection.to_owned(),
            connection_label: connection.to_owned(),
            merchant: domain.to_owned(),
            domain: domain.to_owned(),
            lines: Vec::new(),
            currency: "USD".to_owned(),
            total,
            method_kind: VIRTUAL_CARD.to_owned(),
            method_label: String::new(),
            ship_to: None,
            approved_by: "you".to_owned(),
            status,
            order_id: None,
            charged: None,
            receipt_url: None,
            report_note: None,
            reported_at: None,
            card: None,
            charged_by: Vec::new(),
            mismatch: None,
            mismatch_seen: false,
        }
    }

    #[test]
    fn spending_counts_what_was_approved_or_charged_in_the_window() {
        let mut charged = purchase("c", 900, "claude", "amazon.com", 1_000, PurchaseStatus::Completed);
        charged.charged = Some(1_200);
        let ledger = vec![
            purchase("a", 1_000, "claude", "smile.amazon.com", 500, PurchaseStatus::Approved),
            charged,
            purchase("f", 950, "claude", "amazon.com", 9_999, PurchaseStatus::Failed),
            purchase("o", 990, "gpt", "amazon.com", 300, PurchaseStatus::Approved),
            purchase("old", 10, "claude", "amazon.com", 7_000, PurchaseStatus::Completed),
        ];
        assert_eq!(spent_since(&ledger, 100, "USD", Some("claude"), |d| in_domain(d, "amazon.com")), 1_700);
        assert_eq!(spent_since(&ledger, 100, "USD", None, |_| true), 2_000);
        assert_eq!(spent_since(&ledger, 100, "EUR", None, |_| true), 0);
        assert_eq!(spent_since(&ledger, 0, "USD", Some("claude"), |_| true), 8_700);
    }

    #[test]
    fn defaults_allow_the_store_and_the_phone_and_a_tolerance_of_ten_percent() {
        let config: Config = serde_json::from_str("{}").unwrap();
        assert_eq!(config, Config::default());
        assert!(config.merchant_account && config.pay_on_phone && config.provider.is_none());
        assert_eq!(config.tolerance(10_000, "USD"), 1_000);
        assert_eq!(config.tolerance(500, "USD"), 100, "at least 1.00");
        assert_eq!(config.tolerance(2_497, "USD"), 250, "rounded up");
        assert_eq!(config.tolerance(500, "JPY"), 50);
        assert_eq!(config.tolerance(5, "JPY"), 1);
    }
}
