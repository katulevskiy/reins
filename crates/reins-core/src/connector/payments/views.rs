//! What the apps show of Payments: the purchase on the approval screen, the settings, the spending history.
//! Amounts come both as minor units (for arithmetic) and as text ("$24.97").

use super::settings::LimitPeriod;

/// A payment method, as the approval and the settings show it.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct PaymentMethodView {
    /// `virtual_card`, `card:<vault item id>`, `merchant_account` or `pay_on_phone`.
    pub id: String,
    /// `virtual_card`, `card`, `merchant_account` or `pay_on_phone`.
    pub kind: String,
    /// The user's nickname, else a name ("Visa •• 4242", "Privacy.com card").
    pub name: String,
    pub brand: Option<String>,
    pub last4: Option<String>,
    /// "04/29".
    pub expiry: Option<String>,
    /// One line on what happens with it ("A new card for this purchase, capped at $27.47").
    pub detail: String,
    /// Settings: switched on for AIs. Approval: always true.
    pub enabled: bool,
    /// Approval: why it cannot pay for this cart (a virtual card for another currency), else `None`.
    pub unavailable: Option<String>,
}

/// A shipping address.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct AddressView {
    pub id: String,
    pub label: String,
    /// The whole address, line by line, for the user's own screen.
    pub lines: Vec<String>,
    /// What an AI is shown when it lists addresses ("London, GB").
    pub masked: String,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct PurchaseLineView {
    pub name: String,
    pub details: Option<String>,
    pub quantity: u32,
    pub unit_price: String,
    pub line_total: String,
    pub url: Option<String>,
}

/// A purchase waiting for the user (`ApprovalView.purchase`).
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct PurchaseView {
    pub merchant: String,
    /// The store's domain (`amazon.com`), from the page it named.
    pub domain: String,
    pub merchant_url: String,
    /// The page Pay on phone opens, and its host.
    pub checkout_url: String,
    pub checkout_host: String,
    pub items: Vec<PurchaseLineView>,
    pub subtotal: String,
    pub shipping: Option<String>,
    pub tax: Option<String>,
    pub discount: Option<String>,
    pub total: String,
    pub total_minor: i64,
    pub currency: String,
    /// Why, in the AI's words.
    pub note: Option<String>,
    /// The methods the user may pay with, and the one picked (the AI's choice, else the default).
    pub methods: Vec<PaymentMethodView>,
    pub method_id: Option<String>,
    /// Whether it ships; the addresses, and the one picked.
    pub ships: bool,
    pub addresses: Vec<AddressView>,
    pub address_id: Option<String>,
    /// Shown above the total ("amazon.com is new for this AI").
    pub warnings: Vec<String>,
    /// "Claude spent $40.00 of $200.00 in the last 30 days".
    pub budget_lines: Vec<String>,
    /// The methods a spend limit made from this approval may use.
    pub limit_methods: Vec<String>,
}

/// A spend limit to create alongside an approval, or from the settings.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct SpendLimitInput {
    /// From the settings; ignored alongside an approval (it is the request's).
    pub connection_id: String,
    /// Store domains; empty: every store.
    pub merchants: Vec<String>,
    pub method: String,
    pub currency: String,
    pub per_purchase: i64,
    pub per_period: i64,
    pub period: LimitPeriod,
    pub duration_secs: i64,
}

/// What the user picked on the purchase screen.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct PurchaseChoice {
    /// `None`: the one the view picked.
    pub method_id: Option<String>,
    pub address_id: Option<String>,
    pub limit: Option<SpendLimitInput>,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct SpendLimitView {
    pub id: String,
    pub connection_id: String,
    pub connection_label: String,
    pub merchants: Vec<String>,
    pub method: String,
    pub method_name: String,
    pub currency: String,
    pub per_purchase: i64,
    pub per_period: i64,
    pub period: LimitPeriod,
    /// "Up to $25.00 a purchase and $50.00 a day at amazon.com".
    pub summary: String,
    /// Spent within this limit's period so far.
    pub spent: i64,
    pub created_at: i64,
    pub expires_at: i64,
    pub active: bool,
}

/// A budget: every AI together (`connection_id` empty) or one.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct BudgetView {
    pub connection_id: String,
    pub connection_label: String,
    pub currency: String,
    pub per_purchase: Option<i64>,
    pub per_day: Option<i64>,
    pub per_month: Option<i64>,
    pub merchants: Vec<String>,
    /// One line per rule ("At most $500.00 a purchase").
    pub lines: Vec<String>,
}

/// The connected virtual card provider.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct CardProviderView {
    /// "privacy".
    pub kind: String,
    pub name: String,
    pub sandbox: bool,
    pub single_use: bool,
    pub added_at: i64,
}

/// Integrations → Payments.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct PaymentsOverview {
    /// Payments is on (the integration has its account).
    pub enabled: bool,
    /// The vault can be read (cards and addresses come from it).
    pub vault_ready: bool,
    pub methods: Vec<PaymentMethodView>,
    pub addresses: Vec<AddressView>,
    pub provider: Option<CardProviderView>,
    pub default_method: Option<String>,
    pub default_address: Option<String>,
    pub tolerance_pct: u32,
    pub budgets: Vec<BudgetView>,
    pub limits: Vec<SpendLimitView>,
    /// The mandate key's RFC 7638 thumbprint.
    pub mandate_key: String,
}

/// One purchase in the spending history.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct PurchaseRecordView {
    pub id: String,
    pub at: i64,
    pub connection_id: String,
    pub connection_label: String,
    pub merchant: String,
    pub domain: String,
    pub lines: Vec<String>,
    pub currency: String,
    pub total: i64,
    pub total_text: String,
    pub method_kind: String,
    pub method_label: String,
    pub ship_to: Option<String>,
    /// Approved by the user, or by a spend limit.
    pub by_limit: bool,
    /// "approved" | "completed" | "failed" | "cancelled".
    pub status: String,
    pub order_id: Option<String>,
    pub charged_text: Option<String>,
    pub receipt_url: Option<String>,
    pub report_note: Option<String>,
    /// A virtual card that is still open and can be closed.
    pub card_open: bool,
    pub card_last4: Option<String>,
    /// Who charged the virtual card, as the card network names them.
    pub charged_by: Vec<String>,
    /// A charge by someone who does not look like the approved store: shown until the user has seen it, and spend
    /// limits for this AI wait until then.
    pub mismatch: Option<String>,
    /// What it counts for in budgets and limits now ("$27.47": an open card counts its cap).
    pub counted_text: String,
    /// It did not pay with a virtual card and still counts: the user can say nothing was charged.
    pub clearable: bool,
}

/// What was spent in one currency.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct SpendTotal {
    pub currency: String,
    pub minor: i64,
    pub text: String,
}

/// What one AI spent.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct SpendByAi {
    pub connection_id: String,
    pub connection_label: String,
    pub totals: Vec<SpendTotal>,
}

/// Payments → Spending.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct SpendingView {
    pub since: i64,
    pub totals: Vec<SpendTotal>,
    pub by_ai: Vec<SpendByAi>,
    /// Newest first, every purchase kept.
    pub purchases: Vec<PurchaseRecordView>,
}
