//! Payments (Reins Pay, `docs/payments.md`): an AI asks to buy one cart; the phone shows it like a receipt, checks the
//! user's budgets and spend limits, and on approval hands over what pays for exactly that cart: a virtual card made for
//! it, a card from the vault, nothing (the store has the card on file), or the store's checkout opened on the phone.
//! Every approval carries a signed mandate of the cart and lands in the ledger.
//!
//! The engine's side (parking, approving, the activity) is in `flow`; the user's settings and the ledger in
//! `settings`; virtual card providers in `provider`; mandates in `mandate`.

mod flow;
mod manage;
pub mod mandate;
pub mod provider;
pub mod settings;
pub mod views;

use std::sync::Arc;

use reins_proto::connector::ConnectorCall;
use reins_proto::payments::{
    ADDRESSES_LIST_OP, Cart, CartMandate, METHODS_LIST_OP, MandatePayment, MandateShipTo, PAYMENTS,
    PURCHASE_COMPLETE_OP, PURCHASE_REQUEST_OP, ShipTo, display_amount, format_minor, in_domain, normalize_currency,
    normalize_domain, parse_amount,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use zeroize::Zeroizing;

use self::provider::{CardIssuer, CardRequest, PRIVACY, Privacy};
pub use self::settings::LimitPeriod;
use self::settings::{
    Budget, CARD_PREFIX, CardRef, Config, DAY, MAX_LIMIT_SECS, MERCHANT_ACCOUNT, PAY_ON_PHONE, Purchase,
    PurchaseStatus, SpendLimit, VIRTUAL_CARD, VIRTUAL_CARD_DAYS, spent_since,
};
pub use self::views::*;
use super::device::PHONE_ACCOUNT;
use super::vault::{Vault, wallet};
use super::{Connector, Item, Preview};
use crate::store::{Store, unix_now};
use crate::types::GmailStatus;
use crate::{CoreError, text};

/// The one account Payments has: this phone.
pub const PAYMENTS_ACCOUNT: &str = PHONE_ACCOUNT;
/// Where a provider's API key is kept: (`payments.provider`, the provider).
const PROVIDER_SECRET: &str = "payments.provider";
/// Approved by the user (in the ledger and the mandate).
const BY_USER: &str = "you";

fn bad(message: impl Into<String>) -> CoreError {
    CoreError::service(message)
}

/// A payment method as a purchase plan keeps it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MethodOption {
    pub id: String,
    pub kind: String,
    pub name: String,
    #[serde(default)]
    pub brand: Option<String>,
    #[serde(default)]
    pub last4: Option<String>,
    #[serde(default)]
    pub expiry: Option<String>,
    pub detail: String,
    /// Why it cannot pay for this cart.
    #[serde(default)]
    pub unavailable: Option<String>,
}

impl MethodOption {
    fn view(&self, enabled: bool) -> PaymentMethodView {
        PaymentMethodView {
            id: self.id.clone(),
            kind: self.kind.clone(),
            name: self.name.clone(),
            brand: self.brand.clone(),
            last4: self.last4.clone(),
            expiry: self.expiry.clone(),
            detail: self.detail.clone(),
            enabled,
            unavailable: self.unavailable.clone(),
        }
    }

    /// "Visa •• 4242", "Saved at the store".
    fn label(&self) -> String {
        match (&self.brand, &self.last4) {
            (Some(b), Some(l)) => format!("{b} \u{2022}\u{2022} {l}"),
            (None, Some(l)) => format!("Card \u{2022}\u{2022} {l}"),
            _ => self.name.clone(),
        }
    }
}

/// A shipping address as a purchase plan keeps it: whole, for the user's screen and the approved answer.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShipAddress {
    pub id: String,
    pub label: String,
    pub name: String,
    #[serde(default)]
    pub company: String,
    pub line1: String,
    #[serde(default)]
    pub line2: String,
    #[serde(default)]
    pub line3: String,
    pub city: String,
    #[serde(default)]
    pub state: String,
    pub postal_code: String,
    pub country: String,
    #[serde(default)]
    pub phone: String,
    #[serde(default)]
    pub email: String,
}

impl ShipAddress {
    fn from_vault(a: wallet::Address) -> Self {
        Self {
            id: a.id,
            label: a.label,
            name: a.name,
            company: a.company,
            line1: a.line1,
            line2: a.line2,
            line3: a.line3,
            city: a.city,
            state: a.state,
            postal_code: a.postal_code,
            country: a.country,
            phone: a.phone,
            email: a.email,
        }
    }

    /// What an AI is shown before a purchase: the city, the region and the country.
    fn masked(&self) -> String {
        [&self.city, &self.state, &self.country]
            .iter()
            .filter(|s| !s.is_empty())
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn lines(&self) -> Vec<String> {
        let city_line = [&self.postal_code, &self.city, &self.state]
            .iter()
            .filter(|s| !s.is_empty())
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        [&self.name, &self.company, &self.line1, &self.line2, &self.line3, &city_line, &self.country]
            .into_iter()
            .filter(|s| !s.is_empty())
            .cloned()
            .collect()
    }

    fn view(&self) -> AddressView {
        AddressView {
            id: self.id.clone(),
            label: self.label.clone(),
            lines: self.lines(),
            masked: self.masked(),
        }
    }

    /// The address as the AI gets it with an approved purchase.
    fn released(&self) -> Value {
        let mut o = Map::new();
        for (k, v) in [
            ("name", &self.name),
            ("company", &self.company),
            ("line1", &self.line1),
            ("line2", &self.line2),
            ("line3", &self.line3),
            ("city", &self.city),
            ("region", &self.state),
            ("postal_code", &self.postal_code),
            ("country", &self.country),
            ("phone", &self.phone),
            ("email", &self.email),
        ] {
            if !v.is_empty() {
                o.insert(k.to_owned(), json!(v));
            }
        }
        Value::Object(o)
    }
}

/// A purchase request worked out for the user: the cart, what may pay for it and where it may go, and what the user
/// should know. Kept with the parked request, so that what is approved is what was shown.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plan {
    pub cart: Cart,
    pub methods: Vec<MethodOption>,
    pub addresses: Vec<ShipAddress>,
    pub method_id: Option<String>,
    pub address_id: Option<String>,
    #[serde(default)]
    pub warnings: Vec<String>,
    #[serde(default)]
    pub budget_lines: Vec<String>,
    /// How much more than the total a virtual card allows.
    pub tolerance: i64,
}

impl Plan {
    fn ships(&self) -> bool {
        self.cart.ship_to != ShipTo::Nothing
    }

    /// The write preview the approval and the activity show.
    pub fn preview(&self) -> Preview {
        let c = &self.cart;
        let amount = |m: i64| display_amount(m, &c.currency);
        let mut lines = vec![format!("Buy {} from {}", c.summary(), c.merchant)];
        for i in &c.items {
            lines.push(format!("{} \u{d7} {} \u{b7} {}", i.quantity, i.name, amount(i.line_total())));
        }
        for (what, m) in [("Shipping", c.shipping), ("Tax", c.tax), ("Discount", -c.discount)] {
            if m != 0 {
                lines.push(format!("{what}: {}", amount(m)));
            }
        }
        lines.push(format!("Total: {}", amount(c.total)));
        Preview {
            resource: c.domain.clone(),
            resource_label: format!("{} ({})", c.merchant, c.domain),
            lines,
            once_only: true,
            ..Preview::default()
        }
    }

    /// The approval screen's part.
    pub fn view(&self) -> PurchaseView {
        let c = &self.cart;
        let amount = |m: i64| display_amount(m, &c.currency);
        let nonzero = |m: i64| (m != 0).then(|| amount(m));
        PurchaseView {
            merchant: c.merchant.clone(),
            domain: c.domain.clone(),
            merchant_url: c.merchant_url.clone(),
            checkout_url: c.checkout_url.clone(),
            checkout_host: c.checkout_host.clone(),
            items: c
                .items
                .iter()
                .map(|i| PurchaseLineView {
                    name: i.name.clone(),
                    details: i.details.clone(),
                    quantity: i.quantity,
                    unit_price: amount(i.unit_price),
                    line_total: amount(i.line_total()),
                    url: i.url.clone(),
                })
                .collect(),
            subtotal: amount(c.subtotal),
            shipping: nonzero(c.shipping),
            tax: nonzero(c.tax),
            discount: nonzero(c.discount),
            total: amount(c.total),
            total_minor: c.total,
            currency: c.currency.clone(),
            note: c.note.clone(),
            methods: self.methods.iter().map(|m| m.view(true)).collect(),
            method_id: self.method_id.clone(),
            ships: self.ships(),
            addresses: self.addresses.iter().map(ShipAddress::view).collect(),
            address_id: self.address_id.clone(),
            warnings: self.warnings.clone(),
            budget_lines: self.budget_lines.clone(),
            limit_methods: self
                .methods
                .iter()
                .filter(|m| m.unavailable.is_none() && limit_may_use(&m.id))
                .map(|m| m.id.clone())
                .collect(),
        }
    }
}

/// Whether a spend limit may approve purchases paid this way: never with a card from the vault, whose number cannot
/// be taken back or capped once handed over, nor on the phone, which needs the user anyway.
#[must_use]
pub fn limit_may_use(method: &str) -> bool {
    matches!(method, VIRTUAL_CARD | MERCHANT_ACCOUNT)
}

/// Proof that the caller holds [`Payments::purchases`]: what changes the ledger takes one.
pub(crate) type Held<'a> = tokio::sync::MutexGuard<'a, ()>;

/// Who approved a purchase, for [`Payments::perform`].
pub(crate) struct Approval<'a> {
    pub purchase_id: &'a str,
    pub connection_id: &'a str,
    pub connection_label: &'a str,
    /// [`BY_USER`] or the spend limit's id.
    pub approved_by: &'a str,
    /// The desktop app's key, when it asked for card details sealed to it, and its nonce.
    pub seal_to: Option<([u8; 32], String)>,
}

pub struct Payments {
    vault: Arc<Vault>,
    store: Arc<Store>,
    issuer: Arc<dyn CardIssuer>,
    sandbox: Arc<dyn CardIssuer>,
    /// One purchase at a time, so that budgets and limits are checked against a ledger nobody else is changing.
    pub(crate) purchases: tokio::sync::Mutex<()>,
    /// One change of the settings at a time (each reads them, changes them and writes them back).
    config_edit: std::sync::Mutex<()>,
}

/// The settings answer: the methods with their switch, and the addresses.
struct Wallet {
    cards: Vec<wallet::CardInfo>,
    addresses: Vec<ShipAddress>,
    ready: bool,
    error: Option<String>,
}

fn expiry(month: &str, year: &str) -> Option<String> {
    let m: u32 = month.trim().parse().ok().filter(|m| (1..=12).contains(m))?;
    let y = year.trim();
    let yy = y.get(y.len().saturating_sub(2)..)?;
    Some(format!("{m:02}/{yy}"))
}

/// Whether a card's expiry (month, year) is before `now`.
fn expired(month: &str, year: &str, now: i64) -> bool {
    let (Ok(m), Ok(mut y)) = (month.trim().parse::<i64>(), year.trim().parse::<i64>()) else {
        return false;
    };
    if y < 100 {
        y += 2000;
    }
    // The first day after the expiry month, as days since 1970 (good enough: a card expires at the end of a month).
    let (y2, m2) = if m >= 12 {
        (y + 1, 1)
    } else {
        (y, m + 1)
    };
    let days = days_from_civil(y2, m2);
    now >= days * DAY
}

/// Days since 1970-01-01 of the first day of a month (Howard Hinnant's algorithm).
fn days_from_civil(y: i64, m: i64) -> i64 {
    let y = if m <= 2 {
        y - 1
    } else {
        y
    };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Whether the store's name fits its domain ("Best Buy" on bestbuy.com does, "Apple" on amazon.com does not).
fn name_fits_domain(merchant: &str, domain: &str) -> bool {
    let squash = |s: &str| s.chars().filter(char::is_ascii_alphanumeric).collect::<String>().to_ascii_lowercase();
    let host = squash(domain);
    let whole = squash(merchant);
    (!whole.is_empty() && host.contains(&whole))
        || merchant.split(|c: char| !c.is_alphanumeric()).map(squash).any(|w| w.len() >= 4 && host.contains(&w))
}

/// The store's page and its checkout as a browser reads them (WHATWG URL parsing, as the `url` crate does): their hosts
/// must be the ones the cart was checked with, so that the domain the user approves is the one the phone opens.
fn check_urls(cart: &Cart) -> Result<(), CoreError> {
    let host = |u: &str| {
        url::Url::parse(u)
            .ok()
            .filter(|u| u.scheme() == "https" && u.username().is_empty() && u.password().is_none())
            .and_then(|u| u.host_str().map(|h| h.trim_end_matches('.').to_ascii_lowercase()))
    };
    let merchant = host(&cart.merchant_url).is_some_and(|h| reins_proto::payments::store_domain(&h) == cart.domain);
    let checkout = host(&cart.checkout_url).is_some_and(|h| h == cart.checkout_host);
    let items = cart.items.iter().filter_map(|i| i.url.as_deref()).all(|u| host(u).is_some());
    if merchant && checkout && items {
        Ok(())
    } else {
        Err(bad(
            "A store address in this purchase is not a plain https address. Send each page as the browser shows it.",
        ))
    }
}

/// Whether a charge's merchant name ("AMZN Mktp US*2K3", "SQ *BLUE BOTTLE") plausibly is the approved store: a word of
/// it is in the store's domain or in the name it gave. Card networks shorten names; Amazon's are "AMZN".
fn charge_fits(descriptor: &str, merchant: &str, domain: &str) -> bool {
    let lower = descriptor.to_ascii_lowercase();
    if domain.split('.').any(|label| label == "amazon") && (lower.contains("amzn") || lower.contains("amazon")) {
        return true;
    }
    name_fits_domain(descriptor, domain) || name_fits_domain(descriptor, merchant)
}

impl Payments {
    pub fn new(
        vault: Arc<Vault>,
        store: Arc<Store>,
        http: reqwest::Client,
        privacy_base: &str,
        sandbox_base: &str,
    ) -> Self {
        Self {
            vault,
            store,
            issuer: Arc::new(Privacy::new(http.clone(), privacy_base)),
            sandbox: Arc::new(Privacy::new(http, sandbox_base)),
            purchases: tokio::sync::Mutex::new(()),
            config_edit: std::sync::Mutex::new(()),
        }
    }

    /// Held while the settings are read, changed and written back.
    fn edit_config(&self) -> std::sync::MutexGuard<'_, ()> {
        self.config_edit.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub(crate) fn config(&self) -> Result<Config, CoreError> {
        settings::load_config(&self.store)
    }

    pub(crate) fn save(&self, config: &Config) -> Result<(), CoreError> {
        settings::save_config(&self.store, config)
    }

    pub(crate) fn ledger(&self) -> Result<Vec<Purchase>, CoreError> {
        settings::load_ledger(&self.store)
    }

    fn issuer(&self, sandbox: bool) -> &dyn CardIssuer {
        if sandbox {
            self.sandbox.as_ref()
        } else {
            self.issuer.as_ref()
        }
    }

    fn provider_key(&self, kind: &str) -> Result<Zeroizing<String>, CoreError> {
        let raw = self
            .store
            .secret_get(PROVIDER_SECRET, kind)?
            .ok_or_else(|| CoreError::needs_attention("Connect the virtual card provider again in Payments"))?;
        String::from_utf8(raw).map(Zeroizing::new).map_err(|_| CoreError::storage("unreadable provider key"))
    }

    /// The vault account the cards and addresses come from.
    fn vault_account(&self) -> Option<String> {
        self.store.accounts().ok()?.into_iter().find(|a| a.service == reins_proto::connector::VAULT).map(|a| a.account)
    }

    async fn wallet(&self) -> Wallet {
        let Some(account) = self.vault_account() else {
            return Wallet {
                cards: Vec::new(),
                addresses: Vec::new(),
                ready: false,
                error: Some("The password vault is not connected (Integrations → Password vault).".to_owned()),
            };
        };
        let (cards, addresses) =
            (wallet::cards(&self.vault, &account).await, wallet::addresses(&self.vault, &account).await);
        match (cards, addresses) {
            (Ok(cards), Ok(addresses)) => Wallet {
                cards,
                addresses: addresses.into_iter().map(ShipAddress::from_vault).collect(),
                ready: true,
                error: None,
            },
            (Err(e), _) | (_, Err(e)) => Wallet {
                cards: Vec::new(),
                addresses: Vec::new(),
                ready: false,
                error: Some(format!("The password vault could not be read: {e}")),
            },
        }
    }

    /// Every method there is (`all`) or only those switched on, for a cart (`cart`) or in general.
    fn methods(
        config: &Config,
        cards: &[wallet::CardInfo],
        cart: Option<&Cart>,
        all: bool,
    ) -> Vec<(MethodOption, bool)> {
        let now = unix_now();
        let nick =
            |id: &str, default: String| config.nicknames.get(id).cloned().filter(|n| !n.is_empty()).unwrap_or(default);
        let mut out = Vec::new();
        if let Some(p) = &config.provider {
            let detail = match cart {
                Some(c) => format!(
                    "A new card for this purchase only, locked to the store, at most {}",
                    display_amount(c.total + config.tolerance(c.total, &c.currency), &c.currency)
                ),
                None => "A new card for each purchase, locked to the store and capped at the approved total".to_owned(),
            };
            let unavailable =
                cart.filter(|c| c.currency != "USD").map(|_| "Privacy.com cards pay in US dollars only".to_owned());
            out.push((
                MethodOption {
                    id: VIRTUAL_CARD.to_owned(),
                    kind: VIRTUAL_CARD.to_owned(),
                    name: nick(
                        VIRTUAL_CARD,
                        format!(
                            "{} card{}",
                            provider_name(&p.kind),
                            if p.sandbox {
                                " (sandbox)"
                            } else {
                                ""
                            }
                        ),
                    ),
                    brand: None,
                    last4: None,
                    expiry: None,
                    detail,
                    unavailable,
                },
                true,
            ));
        }
        for card in cards {
            let id = format!("{CARD_PREFIX}{}", card.id);
            let enabled = config.cards.contains(&card.id);
            if !enabled && !all {
                continue;
            }
            let unavailable = if expired(&card.exp_month, &card.exp_year, now) {
                Some("This card has expired".to_owned())
            } else if card.last4.is_none() {
                Some("This card has no number in the vault".to_owned())
            } else {
                None
            };
            out.push((
                MethodOption {
                    name: nick(
                        &id,
                        if card.name.is_empty() {
                            "Card".to_owned()
                        } else {
                            card.name.clone()
                        },
                    ),
                    id,
                    kind: "card".to_owned(),
                    brand: (!card.brand.is_empty()).then(|| card.brand.clone()),
                    last4: card.last4.clone(),
                    expiry: expiry(&card.exp_month, &card.exp_year),
                    detail: "Its number, expiry and security code are handed over for this purchase".to_owned(),
                    unavailable,
                },
                enabled,
            ));
        }
        for (id, on, name, detail) in [
            (
                MERCHANT_ACCOUNT,
                config.merchant_account,
                cart.map_or_else(|| "Saved at the store".to_owned(), |c| format!("Saved at {}", c.domain)),
                "The store's own saved payment method; nothing is handed over",
            ),
            (
                PAY_ON_PHONE,
                config.pay_on_phone,
                "Pay on this phone".to_owned(),
                "The checkout opens on this phone and you pay there",
            ),
        ] {
            if on || all {
                out.push((
                    MethodOption {
                        id: id.to_owned(),
                        kind: id.to_owned(),
                        name: nick(id, name),
                        brand: None,
                        last4: None,
                        expiry: None,
                        detail: detail.to_owned(),
                        unavailable: None,
                    },
                    on,
                ));
            }
        }
        out
    }

    // ---- what an AI lists ---------------------------------------------------------------------------------------

    async fn list_methods(&self) -> Result<Vec<Item>, CoreError> {
        let config = self.config()?;
        let wallet = if config.cards.is_empty() {
            Vec::new()
        } else {
            self.wallet().await.cards
        };
        Ok(Self::methods(&config, &wallet, None, false)
            .into_iter()
            .map(|(m, _)| {
                let mut extra = Map::new();
                extra.insert("kind".to_owned(), json!(m.kind));
                extra.insert("name".to_owned(), json!(m.name));
                for (k, v) in [("brand", &m.brand), ("last4", &m.last4), ("expiry", &m.expiry)] {
                    if let Some(v) = v {
                        extra.insert(k.to_owned(), json!(v));
                    }
                }
                if let Some(why) = &m.unavailable {
                    extra.insert("unavailable".to_owned(), json!(why));
                }
                Item {
                    snippet: [Some(m.label()), m.expiry.as_ref().map(|e| format!("expires {e}"))]
                        .into_iter()
                        .flatten()
                        .collect::<Vec<_>>()
                        .join(" \u{b7} "),
                    id: m.id,
                    resource: "methods".to_owned(),
                    resource_label: "Payment methods".to_owned(),
                    title: m.name,
                    extra,
                    ..Item::default()
                }
            })
            .collect())
    }

    async fn list_addresses(&self) -> Result<Vec<Item>, CoreError> {
        let wallet = self.wallet().await;
        if let Some(e) = wallet.error {
            return Err(bad(e));
        }
        Ok(wallet
            .addresses
            .into_iter()
            .map(|a| {
                let mut extra = Map::new();
                extra.insert("label".to_owned(), json!(a.label));
                for (k, v) in [("city", &a.city), ("region", &a.state), ("country", &a.country)] {
                    if !v.is_empty() {
                        extra.insert(k.to_owned(), json!(v));
                    }
                }
                Item {
                    snippet: a.masked(),
                    id: a.id.clone(),
                    resource: "addresses".to_owned(),
                    resource_label: "Shipping addresses".to_owned(),
                    title: a.label,
                    extra,
                    ..Item::default()
                }
            })
            .collect())
    }

    // ---- a purchase -----------------------------------------------------------------------------------------------

    /// Works out a purchase request: the cart checked again, the methods and addresses, the user's defaults, warnings
    /// and where the budgets stand. An address or a method the AI named that does not exist is an error for the AI.
    pub(crate) async fn plan(
        &self,
        call: &ConnectorCall,
        connection_id: &str,
        connection_label: &str,
    ) -> Result<Plan, CoreError> {
        let cart = Cart::from_call(call).map_err(bad)?;
        check_urls(&cart)?;
        let config = self.config()?;
        let needs_vault = cart.ship_to != ShipTo::Nothing || !config.cards.is_empty();
        let wallet = if needs_vault {
            self.wallet().await
        } else {
            Wallet {
                cards: Vec::new(),
                addresses: Vec::new(),
                ready: false,
                error: None,
            }
        };
        let methods: Vec<MethodOption> =
            Self::methods(&config, &wallet.cards, Some(&cart), false).into_iter().map(|(m, _)| m).collect();
        if methods.is_empty() {
            return Err(bad("The user has not set up a payment method in Reins (Integrations → Payments)."));
        }
        let usable = |id: &str| methods.iter().any(|m| m.id == id && m.unavailable.is_none());
        let method_id = match &cart.payment_method {
            Some(id) => match methods.iter().find(|m| &m.id == id) {
                Some(m) if m.unavailable.is_some() => {
                    return Err(bad(format!(
                        "{} cannot pay for this: {}.",
                        m.name,
                        m.unavailable.as_deref().unwrap_or_default()
                    )));
                }
                Some(m) => Some(m.id.clone()),
                None => {
                    return Err(bad(
                        "That payment_method is not one the user allows. Call payments_methods_list, or leave it out to let the user choose.",
                    ));
                }
            },
            None => config
                .default_method
                .clone()
                .filter(|d| usable(d))
                .or_else(|| methods.iter().find(|m| m.unavailable.is_none()).map(|m| m.id.clone())),
        };
        let mut warnings = Vec::new();
        let address_id = match &cart.ship_to {
            ShipTo::Nothing => None,
            ShipTo::Address(id) => {
                if let Some(e) = &wallet.error {
                    return Err(bad(e.clone()));
                }
                if !wallet.addresses.iter().any(|a| &a.id == id) {
                    return Err(bad(
                        "That ship_to is not one of the user's addresses. Call payments_addresses_list, or leave it out to let the user choose.",
                    ));
                }
                Some(id.clone())
            }
            ShipTo::Choose => {
                if let Some(e) = &wallet.error {
                    warnings.push(e.clone());
                } else if wallet.addresses.is_empty() {
                    warnings.push(
                        "There is no address in the vault: add an identity with an address to ship this.".to_owned(),
                    );
                }
                config
                    .default_address
                    .clone()
                    .filter(|d| wallet.addresses.iter().any(|a| &a.id == d))
                    .or_else(|| wallet.addresses.first().map(|a| a.id.clone()))
            }
        };
        if !name_fits_domain(&cart.merchant, &cart.domain) {
            warnings.push(format!("The store calls itself {}, but the page is on {}.", cart.merchant, cart.domain));
        }
        if cart.checkout_host != cart.domain && !in_domain(&cart.checkout_host, &cart.domain) {
            warnings.push(format!("Its checkout page is on {}.", cart.checkout_host));
        }
        let ledger = self.ledger()?;
        if let Some(p) = ledger.iter().find(|p| p.connection_id == connection_id && p.unseen_mismatch().is_some()) {
            warnings.push(format!(
                "A card made for an earlier purchase at {} by {} was charged by {}. Its spend limits wait until you \
                 look at it in Spending.",
                p.domain,
                text::one_line(connection_label),
                p.unseen_mismatch().unwrap_or_default()
            ));
        }
        let budget_currencies: Vec<&str> = applicable(&config.budgets, connection_id)
            .filter(|b| b.per_purchase.is_some() || b.per_day.is_some() || b.per_month.is_some())
            .map(|b| b.currency.as_str())
            .collect();
        if !budget_currencies.is_empty() && !budget_currencies.contains(&cart.currency.as_str()) {
            warnings.push(format!(
                "This is in {}: your budget, in {}, does not count it.",
                cart.currency,
                budget_currencies.join(", ")
            ));
        }
        if !ledger.iter().any(|p| p.connection_id == connection_id && in_domain(&p.domain, &cart.domain)) {
            warnings.push(format!("The first purchase at {} by {}.", cart.domain, text::one_line(connection_label)));
        }
        let now = unix_now();
        let budget_lines = applicable(&config.budgets, connection_id)
            .filter(|b| b.currency == cart.currency)
            .flat_map(|b| budget_status(b, &ledger, now))
            .collect();
        Ok(Plan {
            tolerance: config.tolerance(cart.total, &cart.currency),
            cart,
            methods,
            addresses: wallet.addresses,
            method_id,
            address_id,
            warnings,
            budget_lines,
        })
    }

    /// Why the user's budgets refuse this cart, if they do.
    pub(crate) fn refusal(&self, cart: &Cart, connection_id: &str, now: i64) -> Result<Option<String>, CoreError> {
        let config = self.config()?;
        let ledger = self.ledger()?;
        for b in applicable(&config.budgets, connection_id) {
            let who = if b.connection_id.is_empty() {
                "every AI together".to_owned()
            } else {
                text::one_line(&b.connection_label)
            };
            if !b.merchants.is_empty() && !b.merchants.iter().any(|m| in_domain(&cart.domain, m)) {
                return Ok(Some(format!(
                    "The user only allows purchases at {} in Reins, not at {}.",
                    b.merchants.join(", "),
                    cart.domain
                )));
            }
            if b.currency != cart.currency {
                continue;
            }
            let amount = |m: i64| display_amount(m, &b.currency);
            if let Some(max) = b.per_purchase.filter(|max| cart.total > *max) {
                return Ok(Some(format!(
                    "The total {} is over the {} per purchase the user allows ({who}).",
                    amount(cart.total),
                    amount(max)
                )));
            }
            let by = (!b.connection_id.is_empty()).then_some(b.connection_id.as_str());
            for (max, window, word) in [(b.per_day, DAY, "a day"), (b.per_month, 30 * DAY, "in 30 days")] {
                let Some(max) = max else {
                    continue;
                };
                let spent = spent_since(&ledger, now - window, &b.currency, by, |_| true);
                if spent + cart.total > max {
                    return Ok(Some(format!(
                        "This would go over the {} {word} the user allows ({who}): {} spent already.",
                        amount(max),
                        amount(spent)
                    )));
                }
            }
        }
        Ok(None)
    }

    /// The spend limit that approves this purchase without asking, if one does. The request must name the method and
    /// the address itself.
    pub(crate) fn covering_limit(
        &self,
        plan: &Plan,
        connection_id: &str,
        now: i64,
    ) -> Result<Option<SpendLimit>, CoreError> {
        let cart = &plan.cart;
        let (Some(method), true) = (&cart.payment_method, cart.ship_to != ShipTo::Choose) else {
            return Ok(None);
        };
        if !limit_may_use(method) || !plan.methods.iter().any(|m| &m.id == method && m.unavailable.is_none()) {
            return Ok(None);
        }
        let config = self.config()?;
        let ledger = self.ledger()?;
        if ledger.iter().any(|p| p.connection_id == connection_id && p.unseen_mismatch().is_some()) {
            return Ok(None);
        }
        Ok(config
            .limits
            .iter()
            .find(|l| {
                l.connection_id == connection_id
                    && now < l.expires_at
                    && &l.method == method
                    && l.currency == cart.currency
                    && l.covers_merchant(&cart.domain)
                    && cart.total <= l.per_purchase
                    && spent_since(&ledger, now - l.period.secs(), &l.currency, Some(connection_id), |d| {
                        l.covers_merchant(d)
                    }) + cart.total
                        <= l.per_period
            })
            .cloned())
    }

    /// Pays for an approved purchase: makes or reads what pays, signs the mandate, records it, and returns the answer
    /// for the AI. `method` and `address` override the plan's picks (the user changed them).
    pub(crate) async fn perform(
        &self,
        _one: &Held<'_>,
        plan: &Plan,
        method: Option<&str>,
        address: Option<&str>,
        approval: &Approval<'_>,
    ) -> Result<Value, CoreError> {
        let cart = &plan.cart;
        let config = self.config()?;
        let method_id = method
            .map(str::to_owned)
            .or_else(|| plan.method_id.clone())
            .ok_or_else(|| CoreError::invalid("Choose how to pay."))?;
        let chosen = plan
            .methods
            .iter()
            .find(|m| m.id == method_id)
            .ok_or_else(|| CoreError::invalid("That payment method was not offered for this purchase."))?;
        if let Some(why) = &chosen.unavailable {
            return Err(CoreError::invalid(why.clone()));
        }
        // The user may have switched it off while the request waited.
        let still_on = match chosen.kind.as_str() {
            VIRTUAL_CARD => config.provider.is_some(),
            MERCHANT_ACCOUNT => config.merchant_account,
            PAY_ON_PHONE => config.pay_on_phone,
            _ => method_id.strip_prefix(CARD_PREFIX).is_some_and(|id| config.cards.iter().any(|c| c == id)),
        };
        if !still_on {
            return Err(CoreError::invalid("That payment method is switched off in Payments."));
        }
        let ship_to = if plan.ships() {
            let id = address
                .map(str::to_owned)
                .or_else(|| plan.address_id.clone())
                .ok_or_else(|| CoreError::invalid("Choose where to ship it."))?;
            Some(
                plan.addresses
                    .iter()
                    .find(|a| a.id == id)
                    .ok_or_else(|| CoreError::invalid("That address was not offered for this purchase."))?
                    .clone(),
            )
        } else {
            None
        };
        let now = unix_now();
        let mut ledger = self.ledger()?;
        // Approved again after an answer that did not reach the server: the card made then is closed first.
        if let Some(old) = ledger.iter().find(|p| p.id == approval.purchase_id).and_then(|p| p.card.clone()) {
            self.close_card_ref(&old).await.ok();
        }
        ledger.retain(|p| p.id != approval.purchase_id);

        let mut card_ref = None;
        let mut payment_info = MandatePayment {
            kind: chosen.kind.clone(),
            brand: chosen.brand.clone(),
            last4: chosen.last4.clone(),
        };
        let holder = ship_to.as_ref().map(|a| a.name.clone()).unwrap_or_default();
        let mut payment = match chosen.kind.as_str() {
            VIRTUAL_CARD => {
                let provider = config
                    .provider
                    .clone()
                    .ok_or_else(|| CoreError::invalid("No virtual card provider is connected."))?;
                let key = self.provider_key(&provider.kind)?;
                let limit = cart.total + plan.tolerance;
                let short_id: String = approval.purchase_id.chars().take(8).collect();
                let made = self
                    .issuer(provider.sandbox)
                    .create(
                        &key,
                        &CardRequest {
                            limit_cents: limit,
                            memo: format!("Reins \u{b7} {} \u{b7} {short_id}", cart.domain),
                            single_use: provider.single_use,
                        },
                    )
                    .await?;
                let brand = wallet::brand_of(&made.number).to_owned();
                payment_info.brand = Some(brand.clone());
                payment_info.last4 = Some(made.last4.clone());
                card_ref = Some(CardRef {
                    provider: provider.kind.clone(),
                    sandbox: provider.sandbox,
                    token: made.token.clone(),
                    last4: made.last4.clone(),
                    limit,
                    closed_at: None,
                });
                json!({
                    "kind": VIRTUAL_CARD, "provider": provider.kind, "brand": brand, "number": made.number.as_str(),
                    "exp_month": made.exp_month, "exp_year": made.exp_year, "code": made.code.as_str(), "holder": holder,
                    "last4": made.last4, "limit": format_minor(limit, &cart.currency),
                    "single_use": provider.single_use,
                })
            }
            "card" => {
                let id = method_id.strip_prefix(CARD_PREFIX).unwrap_or_default();
                let account =
                    self.vault_account().ok_or_else(|| CoreError::invalid("The password vault is not connected."))?;
                let details = wallet::card_details(&self.vault, &account, id).await?;
                let last4 = wallet::last_four(&details.number).unwrap_or_default();
                payment_info.last4 = Some(last4.clone());
                json!({
                    "kind": "card", "brand": details.brand, "number": details.number.as_str(),
                    "exp_month": details.exp_month, "exp_year": details.exp_year, "code": details.code.as_str(),
                    "holder": details.holder, "last4": last4,
                })
            }
            MERCHANT_ACCOUNT => json!({
                "kind": MERCHANT_ACCOUNT,
                "instructions": format!("Pay with the payment method saved in the user's account at {}. Nothing else is handed over.", cart.domain),
            }),
            _ => json!({
                "kind": PAY_ON_PHONE, "status": "handed_off", "checkout_url": cart.checkout_url,
                "instructions": "The checkout page is open on the user's phone and they pay there. Do not place the order yourself.",
            }),
        };
        let undo = |made: &Option<CardRef>| {
            let made = made.clone();
            async move {
                if let Some(made) = made {
                    self.close_card_ref(&made).await.ok();
                }
            }
        };
        if let Some((key, nonce)) = &approval.seal_to
            && matches!(chosen.kind.as_str(), VIRTUAL_CARD | "card")
        {
            let opened = json!({"v": 1, "nonce": nonce, "purchase_id": approval.purchase_id, "payment": payment});
            match super::sealed::seal(*key, &opened) {
                Ok(sealed) => payment = json!({"kind": chosen.kind, "sealed": sealed}),
                Err(e) => {
                    undo(&card_ref).await;
                    return Err(e);
                }
            }
        }
        let approved_by = if approval.approved_by == BY_USER {
            BY_USER
        } else {
            "spend limit"
        };
        let mandate = CartMandate::new(
            cart,
            approval.purchase_id,
            ship_to.as_ref().map(|a| MandateShipTo {
                label: a.label.clone(),
                country: a.country.clone(),
            }),
            payment_info,
            &text::one_line(approval.connection_label),
            approved_by,
            now,
            mandate::MANDATE_SECS,
        );
        let signed = match mandate::sign(&self.store, &mandate) {
            Ok(s) => s,
            Err(e) => {
                undo(&card_ref).await;
                return Err(e);
            }
        };
        ledger.push(Purchase {
            id: approval.purchase_id.to_owned(),
            at: now,
            connection_id: approval.connection_id.to_owned(),
            connection_label: text::one_line(approval.connection_label),
            merchant: cart.merchant.clone(),
            domain: cart.domain.clone(),
            lines: cart.items.iter().map(|i| format!("{} \u{d7} {}", i.quantity, i.name)).collect(),
            currency: cart.currency.clone(),
            total: cart.total,
            method_kind: chosen.kind.clone(),
            method_label: match &card_ref {
                Some(c) => format!("{} \u{2022}\u{2022} {}", provider_name(&c.provider), c.last4),
                None => chosen.label(),
            },
            ship_to: ship_to.as_ref().map(|a| a.label.clone()),
            approved_by: approval.approved_by.to_owned(),
            status: PurchaseStatus::Approved,
            order_id: None,
            charged: None,
            receipt_url: None,
            report_note: None,
            reported_at: None,
            card: card_ref.clone(),
            charged_by: Vec::new(),
            mismatch: None,
            mismatch_seen: false,
        });
        if let Err(e) = settings::save_ledger(&self.store, &mut ledger) {
            undo(&card_ref).await;
            return Err(e);
        }
        let next = if chosen.kind == PAY_ON_PHONE {
            "The user pays on their phone. When they tell you it is done, call payments_purchase_complete."
        } else {
            "Check out now with exactly this cart, then call payments_purchase_complete with the order number and the amount charged."
        };
        Ok(json!({
            "status": "approved",
            "purchase_id": approval.purchase_id,
            "approved_by": approved_by,
            "merchant": {"name": cart.merchant, "domain": cart.domain},
            "total": format_minor(cart.total, &cart.currency),
            "currency": cart.currency,
            "payment": payment,
            "ship_to": ship_to.as_ref().map(ShipAddress::released),
            "mandate": signed,
            "expires_at": text::iso_utc(mandate.exp),
            "next": next,
        }))
    }

    /// The AI reports how checkout went. Only for its own purchases.
    pub(crate) async fn complete(
        &self,
        _one: &Held<'_>,
        connection_id: &str,
        call: &ConnectorCall,
    ) -> Result<(Value, String), CoreError> {
        let id = call.str_arg("purchase_id").unwrap_or_default();
        let mut ledger = self.ledger()?;
        let purchase = ledger
            .iter_mut()
            .find(|p| p.id == id && p.connection_id == connection_id)
            .ok_or_else(|| bad("No approved purchase of yours has that purchase_id."))?;
        let status = match call.str_arg("status") {
            Some("completed") => PurchaseStatus::Completed,
            Some("failed") => PurchaseStatus::Failed,
            _ => PurchaseStatus::Cancelled,
        };
        let charged = match (call.args.get("charged_total"), call.str_arg("currency")) {
            (Some(v), Some(c)) => {
                let c = normalize_currency(c).map_err(bad)?;
                if c != purchase.currency {
                    return Err(bad(format!("The purchase was approved in {}, not {c}.", purchase.currency)));
                }
                Some(parse_amount(v, &c, "charged_total").map_err(bad)?)
            }
            _ => None,
        };
        purchase.status = status;
        purchase.charged = charged.or(purchase.charged);
        purchase.order_id =
            call.str_arg("order_id").map(text::one_line).filter(|s| !s.is_empty()).or(purchase.order_id.take());
        purchase.receipt_url = call.str_arg("receipt_url").map(str::to_owned).or(purchase.receipt_url.take());
        purchase.report_note = call.str_arg("note").map(text::one_line).filter(|s| !s.is_empty());
        purchase.reported_at = Some(unix_now());
        if purchase.card.is_some() && self.check_card(purchase, unix_now()).await.is_err() {
            log::warn!("the charges of a virtual card could not be read yet");
        }
        let mut card_state = None;
        if let Some(card) = purchase.card.as_mut().filter(|c| c.closed_at.is_none()) {
            if status.spends() {
                card_state = Some(format!(
                    "open for {VIRTUAL_CARD_DAYS} days (later charges for split shipments), up to its limit"
                ));
            } else {
                self.close_card_ref(card).await?;
                card.closed_at = Some(unix_now());
                card_state = Some("closed".to_owned());
            }
        }
        let over = purchase.charged.filter(|c| *c > purchase.total).map(|c| {
            format!(
                "Charged {}, more than the {} approved.",
                display_amount(c, &purchase.currency),
                display_amount(purchase.total, &purchase.currency)
            )
        });
        let detail = format!(
            "{} at {}{}{}",
            status.word(),
            purchase.domain,
            purchase.order_id.as_ref().map(|o| format!(", order {o}")).unwrap_or_default(),
            purchase
                .charged
                .map(|c| format!(", {} charged", display_amount(c, &purchase.currency)))
                .unwrap_or_default()
        );
        let mut answer = json!({"recorded": true, "purchase_id": id, "status": status.word()});
        if let Some(state) = card_state {
            answer["virtual_card"] = json!(state);
        }
        if let Some(over) = &over {
            answer["warning"] = json!(over);
        }
        if let Some(other) = purchase.unseen_mismatch() {
            answer["warning"] = json!(format!(
                "The virtual card was charged by {other}, which does not look like {}. It was closed and the user is told.",
                purchase.domain
            ));
        }
        settings::save_ledger(&self.store, &mut ledger)?;
        Ok((answer, over.map_or(detail.clone(), |o| format!("{detail}. {o}"))))
    }

    async fn close_card_ref(&self, card: &CardRef) -> Result<(), CoreError> {
        if card.closed_at.is_some() {
            return Ok(());
        }
        let key = self.provider_key(&card.provider)?;
        self.issuer(card.sandbox).close(&key, &card.token).await
    }

    /// Reads who charged a purchase's virtual card. A charge by someone who does not look like the approved store is
    /// flagged (spend limits for that AI wait until the user has seen it) and the card is closed at once. `Ok(true)`
    /// when the purchase changed; an error when the charges could not be read.
    async fn check_card(&self, p: &mut Purchase, now: i64) -> Result<bool, CoreError> {
        let Some(card) = p.card.clone() else {
            return Ok(false);
        };
        let key = self.provider_key(&card.provider)?;
        let names = self.issuer(card.sandbox).charged_by(&key, &card.token).await?;
        let other = names.iter().find(|n| !charge_fits(n, &p.merchant, &p.domain)).filter(|_| p.mismatch.is_none());
        let changed = names != p.charged_by || other.is_some();
        if let Some(other) = other {
            p.mismatch = Some(other.clone());
            p.mismatch_seen = false;
        }
        let closed = p.mismatch.is_some()
            && match p.card.as_mut().filter(|c| c.closed_at.is_none()) {
                Some(open) if self.close_card_ref(open).await.is_ok() => {
                    open.closed_at = Some(now);
                    true
                }
                _ => false,
            };
        p.charged_by = names;
        Ok(changed || closed)
    }

    /// Checks the charges of the virtual cards of the last [`VIRTUAL_CARD_DAYS`] days (one AI's, or everyone's) and
    /// closes the cards whose time is up: those of purchases that failed or were cancelled, and older ones. Failures
    /// wait for the next time. `true` when every card's charges could be read.
    pub(crate) async fn tidy(&self, _one: &Held<'_>, connection: Option<&str>, now: i64) -> bool {
        let Ok(mut ledger) = self.ledger() else {
            return false;
        };
        let mut changed = false;
        let mut all_read = true;
        for p in &mut ledger {
            let recent = p.at + (VIRTUAL_CARD_DAYS + 1) * DAY > now;
            if p.card.is_none() || !recent || connection.is_some_and(|c| c != p.connection_id) {
                continue;
            }
            if let Ok(c) = self.check_card(p, now).await {
                changed |= c;
            } else {
                log::warn!("the charges of a virtual card could not be read yet");
                all_read = false;
            }
            let due = !p.status.spends() || p.at + VIRTUAL_CARD_DAYS * DAY <= now;
            let Some(card) = p.card.as_mut().filter(|c| c.closed_at.is_none() && due) else {
                continue;
            };
            match self.close_card_ref(card).await {
                Ok(()) => {
                    card.closed_at = Some(now);
                    changed = true;
                }
                Err(_) => log::warn!("a virtual card could not be closed yet; trying again later"),
            }
        }
        if changed && settings::save_ledger(&self.store, &mut ledger).is_err() {
            log::warn!("the purchase history could not be saved");
            return false;
        }
        all_read
    }

    /// Whether spend limits may approve for an AI now: what its earlier virtual cards were charged with is read again
    /// first, and when that cannot be done, nothing is approved without the user.
    pub(crate) async fn limits_may_apply(
        &self,
        one: &Held<'_>,
        connection_id: &str,
        now: i64,
    ) -> Result<bool, CoreError> {
        let config = self.config()?;
        if !config.limits.iter().any(|l| l.connection_id == connection_id && now < l.expires_at) {
            return Ok(false);
        }
        Ok(self.tidy(one, Some(connection_id), now).await)
    }

    /// The user has seen a charge by another store.
    pub(crate) fn acknowledge_charge(&self, _one: &Held<'_>, purchase_id: &str) -> Result<(), CoreError> {
        let mut ledger = self.ledger()?;
        let p = ledger.iter_mut().find(|p| p.id == purchase_id).ok_or(CoreError::NotFound)?;
        p.mismatch_seen = true;
        settings::save_ledger(&self.store, &mut ledger)
    }

    // ---- the user's settings --------------------------------------------------------------------------------------

    pub(crate) async fn overview(&self, enabled: bool, now: i64) -> Result<PaymentsOverview, CoreError> {
        let config = self.config()?;
        let wallet = self.wallet().await;
        let ledger = self.ledger()?;
        Ok(PaymentsOverview {
            enabled,
            vault_ready: wallet.ready,
            methods: Self::methods(&config, &wallet.cards, None, true).into_iter().map(|(m, on)| m.view(on)).collect(),
            addresses: wallet.addresses.iter().map(ShipAddress::view).collect(),
            provider: config.provider.as_ref().map(|p| CardProviderView {
                kind: p.kind.clone(),
                name: provider_name(&p.kind).to_owned(),
                sandbox: p.sandbox,
                single_use: p.single_use,
                added_at: p.added_at,
            }),
            default_method: config.default_method.clone(),
            default_address: config.default_address.clone(),
            tolerance_pct: config.tolerance_pct,
            budgets: config.budgets.iter().map(|b| budget_view(b, &ledger, now)).collect(),
            limits: config.limits.iter().map(|l| limit_view(l, &config, &ledger, now)).collect(),
            mandate_key: mandate::key_thumbprint(&self.store)?,
        })
    }

    pub(crate) fn set_method_enabled(&self, method: &str, enabled: bool) -> Result<(), CoreError> {
        let _edit = self.edit_config();
        let mut config = self.config()?;
        match method {
            MERCHANT_ACCOUNT => config.merchant_account = enabled,
            PAY_ON_PHONE => config.pay_on_phone = enabled,
            VIRTUAL_CARD => return Err(CoreError::invalid("Connect or disconnect the provider instead.")),
            other => {
                let id = other
                    .strip_prefix(CARD_PREFIX)
                    .filter(|id| !id.is_empty() && id.len() <= 100)
                    .ok_or_else(|| CoreError::invalid("not a payment method"))?;
                config.cards.retain(|c| c != id);
                if enabled {
                    config.cards.push(id.to_owned());
                }
            }
        }
        if !enabled && config.default_method.as_deref() == Some(method) {
            config.default_method = None;
        }
        self.save(&config)
    }

    pub(crate) fn set_nickname(&self, method: &str, nickname: Option<String>) -> Result<(), CoreError> {
        let _edit = self.edit_config();
        let mut config = self.config()?;
        match nickname.map(|n| text::truncate_chars(&text::one_line(&n), 40)).filter(|n| !n.is_empty()) {
            Some(n) => config.nicknames.insert(method.to_owned(), n),
            None => config.nicknames.remove(method),
        };
        self.save(&config)
    }

    pub(crate) fn set_defaults(&self, method: Option<String>, address: Option<String>) -> Result<(), CoreError> {
        let _edit = self.edit_config();
        let mut config = self.config()?;
        config.default_method = method.filter(|m| !m.is_empty());
        config.default_address = address.filter(|a| !a.is_empty());
        self.save(&config)
    }

    pub(crate) async fn connect_provider(
        &self,
        kind: &str,
        key: &str,
        sandbox: bool,
        single_use: bool,
    ) -> Result<(), CoreError> {
        if kind != PRIVACY {
            return Err(CoreError::invalid("Reins knows Privacy.com cards only for now."));
        }
        let key = Zeroizing::new(key.trim().to_owned());
        if key.is_empty() || key.len() > 200 || !key.bytes().all(|b| b.is_ascii_graphic()) {
            return Err(CoreError::invalid("Paste the API key from Privacy.com (Account → API)."));
        }
        self.issuer(sandbox).check(&key).await?;
        self.store.secret_put(PROVIDER_SECRET, kind, key.as_bytes())?;
        let _edit = self.edit_config();
        let mut config = self.config()?;
        config.provider = Some(settings::Provider {
            kind: kind.to_owned(),
            sandbox,
            single_use,
            added_at: unix_now(),
        });
        self.save(&config)
    }

    pub(crate) fn disconnect_provider(&self) -> Result<(), CoreError> {
        let _edit = self.edit_config();
        let mut config = self.config()?;
        if let Some(p) = config.provider.take() {
            self.store.secret_delete(PROVIDER_SECRET, &p.kind)?;
        }
        config.limits.retain(|l| l.method != VIRTUAL_CARD);
        if config.default_method.as_deref() == Some(VIRTUAL_CARD) {
            config.default_method = None;
        }
        self.save(&config)
    }

    pub(crate) fn set_card_options(&self, tolerance_pct: u32, single_use: bool) -> Result<(), CoreError> {
        if tolerance_pct > 50 {
            return Err(CoreError::invalid("The tolerance is at most 50%."));
        }
        let _edit = self.edit_config();
        let mut config = self.config()?;
        config.tolerance_pct = tolerance_pct;
        if let Some(p) = config.provider.as_mut() {
            p.single_use = single_use;
        }
        self.save(&config)
    }

    pub(crate) fn set_budget(&self, budget: &BudgetView) -> Result<(), CoreError> {
        let currency = normalize_currency(&budget.currency).map_err(CoreError::invalid)?;
        let positive = |v: Option<i64>| match v {
            Some(n) if n <= 0 || n > reins_proto::payments::MAX_MINOR => {
                Err(CoreError::invalid("Amounts must be more than zero."))
            }
            v => Ok(v),
        };
        let merchants = budget
            .merchants
            .iter()
            .map(|m| normalize_domain(m).map_err(CoreError::invalid))
            .collect::<Result<Vec<_>, _>>()?;
        let _edit = self.edit_config();
        let mut config = self.config()?;
        config.budgets.retain(|b| b.connection_id != budget.connection_id);
        let kept = Budget {
            connection_id: budget.connection_id.clone(),
            connection_label: text::one_line(&budget.connection_label),
            currency,
            per_purchase: positive(budget.per_purchase)?,
            per_day: positive(budget.per_day)?,
            per_month: positive(budget.per_month)?,
            merchants,
        };
        if kept.per_purchase.is_some()
            || kept.per_day.is_some()
            || kept.per_month.is_some()
            || !kept.merchants.is_empty()
        {
            config.budgets.push(kept);
        }
        self.save(&config)
    }

    /// Checks a new spend limit and adds it.
    pub(crate) fn add_limit(
        &self,
        input: &SpendLimitInput,
        connection_id: &str,
        connection_label: &str,
        now: i64,
    ) -> Result<SpendLimitView, CoreError> {
        if !limit_may_use(&input.method) {
            return Err(CoreError::invalid(
                "A spend limit can pay with a virtual card or the store's saved payment method only.",
            ));
        }
        let currency = normalize_currency(&input.currency).map_err(CoreError::invalid)?;
        if input.per_purchase <= 0
            || input.per_period < input.per_purchase
            || input.per_period > reins_proto::payments::MAX_MINOR
        {
            return Err(CoreError::invalid("A limit needs an amount per purchase, and at least as much per period."));
        }
        if !(DAY / 24..=MAX_LIMIT_SECS).contains(&input.duration_secs) {
            return Err(CoreError::invalid("A spend limit lasts from an hour to 90 days."));
        }
        if connection_id.is_empty() {
            return Err(CoreError::invalid("A spend limit is for one AI."));
        }
        let merchants = input
            .merchants
            .iter()
            .map(|m| normalize_domain(m).map_err(CoreError::invalid))
            .collect::<Result<Vec<_>, _>>()?;
        let _edit = self.edit_config();
        let mut config = self.config()?;
        if input.method == VIRTUAL_CARD && config.provider.is_none() {
            return Err(CoreError::invalid("Connect a virtual card provider first."));
        }
        let limit = SpendLimit {
            id: uuid::Uuid::new_v4().to_string(),
            connection_id: connection_id.to_owned(),
            connection_label: text::one_line(connection_label),
            merchants,
            method: input.method.clone(),
            currency,
            per_purchase: input.per_purchase,
            per_period: input.per_period,
            period: input.period,
            created_at: now,
            expires_at: now + input.duration_secs,
        };
        config.limits.retain(|l| l.expires_at > now);
        config.limits.push(limit.clone());
        self.save(&config)?;
        Ok(limit_view(&limit, &config, &self.ledger()?, now))
    }

    pub(crate) fn remove_limit(&self, id: &str) -> Result<(), CoreError> {
        let _edit = self.edit_config();
        let mut config = self.config()?;
        let before = config.limits.len();
        config.limits.retain(|l| l.id != id);
        if config.limits.len() == before {
            return Err(CoreError::NotFound);
        }
        self.save(&config)
    }

    pub(crate) async fn spending(&self, since: i64, now: i64) -> Result<SpendingView, CoreError> {
        {
            let one = self.purchases.lock().await;
            self.tidy(&one, None, now).await;
        }
        let ledger = self.ledger()?;
        let totals = |filter: &dyn Fn(&Purchase) -> bool| -> Vec<SpendTotal> {
            let mut by: std::collections::BTreeMap<String, i64> = std::collections::BTreeMap::new();
            for p in ledger.iter().filter(|p| p.at >= since && filter(p)) {
                *by.entry(p.currency.clone()).or_default() += p.spent();
            }
            by.into_iter()
                .map(|(currency, minor)| SpendTotal {
                    text: display_amount(minor, &currency),
                    currency,
                    minor,
                })
                .collect()
        };
        let mut ais: Vec<(String, String)> = Vec::new();
        for p in ledger.iter().filter(|p| p.at >= since) {
            if !ais.iter().any(|(id, _)| id == &p.connection_id) {
                ais.push((p.connection_id.clone(), p.connection_label.clone()));
            }
        }
        Ok(SpendingView {
            since,
            totals: totals(&|_| true),
            by_ai: ais
                .into_iter()
                .map(|(id, label)| SpendByAi {
                    totals: totals(&|p| p.connection_id == id),
                    connection_id: id,
                    connection_label: label,
                })
                .collect(),
            purchases: ledger.iter().map(record_view).collect(),
        })
    }

    pub(crate) async fn close_card(&self, _one: &Held<'_>, purchase_id: &str) -> Result<(), CoreError> {
        let mut ledger = self.ledger()?;
        let card =
            ledger.iter_mut().find(|p| p.id == purchase_id).and_then(|p| p.card.as_mut()).ok_or(CoreError::NotFound)?;
        self.close_card_ref(card).await?;
        card.closed_at = Some(unix_now());
        settings::save_ledger(&self.store, &mut ledger)
    }
}

fn provider_name(kind: &str) -> &str {
    match kind {
        PRIVACY => "Privacy.com",
        other => other,
    }
}

/// The budgets that apply to a connection: every AI's and its own.
fn applicable<'a>(budgets: &'a [Budget], connection_id: &'a str) -> impl Iterator<Item = &'a Budget> {
    budgets.iter().filter(move |b| b.connection_id.is_empty() || b.connection_id == connection_id)
}

fn budget_status(b: &Budget, ledger: &[Purchase], now: i64) -> Vec<String> {
    let who = if b.connection_id.is_empty() {
        "every AI".to_owned()
    } else {
        text::one_line(&b.connection_label)
    };
    let by = (!b.connection_id.is_empty()).then_some(b.connection_id.as_str());
    let amount = |m: i64| display_amount(m, &b.currency);
    [(b.per_day, DAY, "today"), (b.per_month, 30 * DAY, "in the last 30 days")]
        .into_iter()
        .filter_map(|(max, window, word)| {
            let max = max?;
            let spent = spent_since(ledger, now - window, &b.currency, by, |_| true);
            Some(format!("{who}: {} of {} spent {word}", amount(spent), amount(max)))
        })
        .collect()
}

fn budget_view(b: &Budget, ledger: &[Purchase], now: i64) -> BudgetView {
    let amount = |m: i64| display_amount(m, &b.currency);
    let mut lines: Vec<String> = Vec::new();
    if let Some(m) = b.per_purchase {
        lines.push(format!("At most {} a purchase", amount(m)));
    }
    if let Some(m) = b.per_day {
        lines.push(format!("At most {} a day", amount(m)));
    }
    if let Some(m) = b.per_month {
        lines.push(format!("At most {} in 30 days", amount(m)));
    }
    if !b.merchants.is_empty() {
        lines.push(format!("Only at {}", b.merchants.join(", ")));
    }
    lines.extend(budget_status(b, ledger, now));
    BudgetView {
        connection_id: b.connection_id.clone(),
        connection_label: b.connection_label.clone(),
        currency: b.currency.clone(),
        per_purchase: b.per_purchase,
        per_day: b.per_day,
        per_month: b.per_month,
        merchants: b.merchants.clone(),
        lines,
    }
}

fn limit_view(l: &SpendLimit, config: &Config, ledger: &[Purchase], now: i64) -> SpendLimitView {
    let amount = |m: i64| display_amount(m, &l.currency);
    let at = if l.merchants.is_empty() {
        "any store".to_owned()
    } else {
        l.merchants.join(", ")
    };
    let method_name = match l.method.as_str() {
        VIRTUAL_CARD => {
            config.provider.as_ref().map_or("Virtual card", |p| provider_name(&p.kind)).to_owned() + " card"
        }
        _ => "Saved at the store".to_owned(),
    };
    SpendLimitView {
        id: l.id.clone(),
        connection_id: l.connection_id.clone(),
        connection_label: l.connection_label.clone(),
        merchants: l.merchants.clone(),
        method: l.method.clone(),
        method_name,
        currency: l.currency.clone(),
        per_purchase: l.per_purchase,
        per_period: l.per_period,
        period: l.period,
        summary: format!(
            "Up to {} a purchase and {} a {} at {at}",
            amount(l.per_purchase),
            amount(l.per_period),
            l.period.word()
        ),
        spent: spent_since(ledger, now - l.period.secs(), &l.currency, Some(&l.connection_id), |d| {
            l.covers_merchant(d)
        }),
        created_at: l.created_at,
        expires_at: l.expires_at,
        active: now < l.expires_at,
    }
}

fn record_view(p: &Purchase) -> PurchaseRecordView {
    PurchaseRecordView {
        id: p.id.clone(),
        at: p.at,
        connection_id: p.connection_id.clone(),
        connection_label: p.connection_label.clone(),
        merchant: p.merchant.clone(),
        domain: p.domain.clone(),
        lines: p.lines.clone(),
        currency: p.currency.clone(),
        total: p.total,
        total_text: display_amount(p.total, &p.currency),
        method_kind: p.method_kind.clone(),
        method_label: p.method_label.clone(),
        ship_to: p.ship_to.clone(),
        by_limit: p.approved_by != BY_USER,
        status: p.status.word().to_owned(),
        order_id: p.order_id.clone(),
        charged_text: p.charged.map(|c| display_amount(c, &p.currency)),
        receipt_url: p.receipt_url.clone(),
        report_note: p.report_note.clone(),
        card_open: p.card.as_ref().is_some_and(|c| c.closed_at.is_none()),
        card_last4: p.card.as_ref().map(|c| c.last4.clone()),
        charged_by: p.charged_by.clone(),
        mismatch: p.unseen_mismatch().map(str::to_owned),
    }
}

#[async_trait::async_trait]
impl Connector for Payments {
    fn service(&self) -> &'static str {
        PAYMENTS
    }

    async fn fetch(&self, _account: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
        match call.op.as_str() {
            METHODS_LIST_OP => self.list_methods().await,
            ADDRESSES_LIST_OP => self.list_addresses().await,
            other => Err(bad(format!("Payments cannot {other}"))),
        }
    }

    /// A purchase is worked out by the engine with the connection it is for ([`Payments::plan`]); this is what any
    /// other caller sees.
    async fn preview(&self, _account: &str, call: &ConnectorCall) -> Result<Preview, CoreError> {
        match call.op.as_str() {
            PURCHASE_REQUEST_OP => Ok(self.plan(call, "", "").await?.preview()),
            PURCHASE_COMPLETE_OP => Ok(Preview {
                resource: "purchases".to_owned(),
                resource_label: "Purchases".to_owned(),
                lines: vec![format!("Record how purchase {} went", call.str_arg("purchase_id").unwrap_or_default())],
                ..Preview::default()
            }),
            other => Err(bad(format!("Payments cannot {other}"))),
        }
    }

    async fn status(&self, _account: &str) -> GmailStatus {
        GmailStatus::Ready
    }

    /// Payments is switched off: the provider's key goes; the settings and the history stay.
    async fn forget(&self, _account: &str) {
        if let Ok(Some(p)) = self.config().map(|c| c.provider) {
            self.store.secret_delete(PROVIDER_SECRET, &p.kind).ok();
            let _edit = self.edit_config();
            let mut config = self.config().unwrap_or_default();
            config.provider = None;
            self.save(&config).ok();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expiries_are_shown_short_and_checked_against_now() {
        assert_eq!(expiry("4", "2029").as_deref(), Some("04/29"));
        assert_eq!(expiry("12", "31").as_deref(), Some("12/31"));
        assert_eq!(expiry("13", "2029"), None);
        // 2026-10-09 is 20_735 days after 1970-01-01.
        let now = 20_735 * DAY;
        assert_eq!(days_from_civil(2026, 10), 20_727);
        assert!(expired("9", "2026", now));
        assert!(!expired("10", "2026", now), "valid until the end of the month");
        assert!(!expired("1", "27", now));
        assert!(!expired("", "", now), "unknown is not expired");
    }

    #[test]
    fn store_names_are_checked_against_their_domains() {
        assert!(name_fits_domain("Amazon", "amazon.com"));
        assert!(name_fits_domain("Best Buy", "bestbuy.com"));
        assert!(name_fits_domain("Amazon.com, Inc.", "smile.amazon.com"));
        assert!(name_fits_domain("The Home Depot", "homedepot.com"));
        assert!(!name_fits_domain("Apple", "amazon.com"));
        assert!(!name_fits_domain("Amazon", "amaz0n-deals.shop"));
    }

    #[test]
    fn store_addresses_are_read_the_way_a_browser_reads_them() {
        let cart = |merchant: &str, checkout: &str| {
            let args = json!({"merchant": "Amazon", "merchant_url": merchant, "checkout_url": checkout,
                "items": [{"name": "x", "unit_price": "1.00"}], "currency": "USD", "total": "1.00"});
            Cart::from_call(&ConnectorCall {
                service: PAYMENTS.to_owned(),
                op: PURCHASE_REQUEST_OP.to_owned(),
                args: args.as_object().unwrap().clone(),
            })
            .unwrap()
        };
        assert!(check_urls(&cart("https://www.amazon.com/dp/1", "https://www.amazon.com/checkout")).is_ok());
        assert!(check_urls(&cart("https://www.Amazon.com./dp/1", "https://checkout.stripe.com/c/1")).is_ok());
        assert!(check_urls(&cart("https://www.amazon.com/dp/1", "https://www.amazon.com:99999/pay")).is_err());
    }

    #[test]
    fn only_capped_methods_may_be_approved_by_a_limit() {
        assert!(limit_may_use(VIRTUAL_CARD) && limit_may_use(MERCHANT_ACCOUNT));
        assert!(!limit_may_use("card:abc") && !limit_may_use(PAY_ON_PHONE));
    }

    #[test]
    fn addresses_are_masked_for_ais_and_whole_for_the_user() {
        let a = ShipAddress {
            id: "i".to_owned(),
            label: "Home".to_owned(),
            name: "Ada Lovelace".to_owned(),
            line1: "12 St James's Square".to_owned(),
            city: "London".to_owned(),
            postal_code: "SW1Y 4JH".to_owned(),
            country: "GB".to_owned(),
            ..ShipAddress::default()
        };
        assert_eq!(a.masked(), "London, GB");
        assert_eq!(a.lines(), ["Ada Lovelace", "12 St James's Square", "SW1Y 4JH London", "GB"]);
        assert_eq!(a.released()["line1"], "12 St James's Square");
        assert!(a.released().get("company").is_none(), "empty fields are left out");
    }
}
