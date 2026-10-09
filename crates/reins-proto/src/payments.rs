//! Purchases an AI asks the phone to approve (Payments, `docs/payments.md`): the cart as the AI describes it,
//! checked to the cent, the stores it is from, and the mandate the phone signs when the user approves.
//!
//! Amounts are integers in the currency's minor unit (cents for USD, yen for JPY, fils for KWD) and are written as
//! decimal strings ("19.98") on the wire, never as floating point. Nothing here does IO.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::connector::ConnectorCall;

pub const PAYMENTS: &str = "payments";
pub const PURCHASE_REQUEST_OP: &str = "purchase_request";
pub const PURCHASE_COMPLETE_OP: &str = "purchase_complete";
pub const METHODS_LIST_OP: &str = "methods_list";
pub const ADDRESSES_LIST_OP: &str = "addresses_list";

/// What `ship_to` says for something that is not shipped (a download, a booking).
pub const NO_SHIPPING: &str = "none";
/// The `typ` of a cart mandate's payload.
pub const MANDATE_TYPE: &str = "reins-cart-mandate/1";
/// The `typ` of a cart mandate's JWS header.
pub const MANDATE_JWS_TYPE: &str = "reins-cart-mandate+jws";

pub const MAX_ITEMS: usize = 50;
pub const MAX_QUANTITY: u32 = 999;
pub const MAX_NAME: usize = 200;
pub const MAX_URL: usize = 2_000;
pub const MAX_NOTE: usize = 500;
/// The largest amount anything may have, in minor units (a billion cents).
pub const MAX_MINOR: i64 = 100_000_000_000;

/// Hosted payment pages a store may send its checkout to, besides its own site: Pay on phone may open them.
pub const HOSTED_CHECKOUTS: &[&str] =
    &["checkout.stripe.com", "pay.shopify.com", "shop.app", "paypal.com", "checkout.square.site", "pay.amazon.com"];

/// How many decimals a currency's amounts have (ISO 4217). Most have two.
#[must_use]
pub fn currency_exponent(currency: &str) -> u32 {
    match currency {
        "BIF" | "CLP" | "DJF" | "GNF" | "ISK" | "JPY" | "KMF" | "KRW" | "PYG" | "RWF" | "UGX" | "UYI" | "VND"
        | "VUV" | "XAF" | "XOF" | "XPF" => 0,
        "BHD" | "IQD" | "JOD" | "KWD" | "LYD" | "OMR" | "TND" => 3,
        _ => 2,
    }
}

/// A currency code as given: three letters, upper case.
pub fn normalize_currency(raw: &str) -> Result<String, String> {
    let c = raw.trim().to_ascii_uppercase();
    if c.len() == 3 && c.bytes().all(|b| b.is_ascii_uppercase()) {
        Ok(c)
    } else {
        Err("`currency` must be a three-letter ISO 4217 code (USD, EUR, GBP).".to_owned())
    }
}

/// Parses "19.98" (or a JSON number written the same way) into minor units of `currency`. More decimals than the
/// currency has, signs, exponents and anything above [`MAX_MINOR`] are refused.
pub fn parse_amount(value: &Value, currency: &str, name: &str) -> Result<i64, String> {
    let text = match value {
        Value::String(s) => s.trim().to_owned(),
        Value::Number(n) => n.to_string(),
        _ => return Err(format!("`{name}` must be an amount as a string, like \"19.98\".")),
    };
    let exp = currency_exponent(currency);
    let bad = || {
        format!(
            "`{name}` must be an amount like \"{}\" with at most {exp} decimals for {currency}.",
            format_minor(1998, currency)
        )
    };
    let (whole, frac) = text.split_once('.').unwrap_or((&text, ""));
    if whole.is_empty()
        || whole.len() > 12
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || !frac.bytes().all(|b| b.is_ascii_digit())
        || frac.len() > usize::try_from(exp).unwrap_or(0)
        || (text.contains('.') && frac.is_empty())
    {
        return Err(bad());
    }
    let scale = 10_i64.pow(exp);
    let whole: i64 = whole.parse().map_err(|_| bad())?;
    let mut padded = frac.to_owned();
    while padded.len() < usize::try_from(exp).unwrap_or(0) {
        padded.push('0');
    }
    let frac: i64 = if padded.is_empty() {
        0
    } else {
        padded.parse().map_err(|_| bad())?
    };
    let minor = whole.checked_mul(scale).and_then(|w| w.checked_add(frac)).ok_or_else(bad)?;
    if minor > MAX_MINOR {
        return Err(format!("`{name}` is too large."));
    }
    Ok(minor)
}

/// Minor units as a decimal string: 1998 USD is "19.98", 500 JPY is "500".
#[must_use]
pub fn format_minor(minor: i64, currency: &str) -> String {
    let exp = currency_exponent(currency);
    let sign = if minor < 0 {
        "-"
    } else {
        ""
    };
    let abs = minor.unsigned_abs();
    if exp == 0 {
        return format!("{sign}{abs}");
    }
    let scale = 10_u64.pow(exp);
    format!("{sign}{}.{:0width$}", abs / scale, abs % scale, width = usize::try_from(exp).unwrap_or(2))
}

/// An amount for people: "$19.98", "€5.00", "¥500", else "19.98 CHF".
#[must_use]
pub fn display_amount(minor: i64, currency: &str) -> String {
    let n = format_minor(minor, currency);
    let (sign, n) = n.strip_prefix('-').map_or(("", n.as_str()), |rest| ("-", rest));
    match currency {
        "USD" => format!("{sign}${n}"),
        "EUR" => format!("{sign}€{n}"),
        "GBP" => format!("{sign}£{n}"),
        "JPY" => format!("{sign}¥{n}"),
        "INR" => format!("{sign}₹{n}"),
        "CAD" => format!("{sign}CA${n}"),
        "AUD" => format!("{sign}A${n}"),
        other => format!("{sign}{n} {other}"),
    }
}

/// The host of an `https` address, lower case, without a port. Refused: other schemes, user names in the address
/// (`https://amazon.com@evil.example`), IP addresses and hosts that are not domain names.
pub fn https_host(url: &str) -> Result<String, String> {
    let rest = url
        .trim()
        .strip_prefix("https://")
        .ok_or_else(|| "Store and checkout addresses must start with https://.".to_owned())?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.contains('@') {
        return Err("Store addresses may not contain a user name (`@`).".to_owned());
    }
    let host = authority.rsplit_once(':').map_or(authority, |(h, port)| {
        if port.bytes().all(|b| b.is_ascii_digit()) {
            h
        } else {
            authority
        }
    });
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    let labels: Vec<&str> = host.split('.').collect();
    let label_ok = |l: &&str| {
        !l.is_empty()
            && l.len() <= 63
            && !l.starts_with('-')
            && !l.ends_with('-')
            && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    };
    let top_is_name = labels.last().is_some_and(|t| t.bytes().any(|b| b.is_ascii_alphabetic()));
    if host.len() > 253 || labels.len() < 2 || !labels.iter().all(label_ok) || !top_is_name {
        return Err("Store addresses must name the store's website (https://www.example.com/...).".to_owned());
    }
    Ok(host)
}

/// The domain a store is shown and limited by: the host without a leading `www.`.
#[must_use]
pub fn store_domain(host: &str) -> String {
    host.strip_prefix("www.").filter(|rest| rest.contains('.')).unwrap_or(host).to_owned()
}

/// Whether `host` is `domain` or one of its subdomains (`smile.amazon.com` is in `amazon.com`; `evilamazon.com` is
/// not).
#[must_use]
pub fn in_domain(host: &str, domain: &str) -> bool {
    let (host, domain) = (host.to_ascii_lowercase(), domain.trim().trim_start_matches('.').to_ascii_lowercase());
    !domain.is_empty() && (host == domain || host.ends_with(&format!(".{domain}")))
}

/// A domain as the user types it for a limit or a budget ("Amazon.com", "https://www.amazon.com/"): the store domain.
pub fn normalize_domain(raw: &str) -> Result<String, String> {
    let raw = raw.trim();
    let url = if raw.contains("://") {
        raw.to_owned()
    } else {
        format!("https://{raw}")
    };
    https_host(&url).map(|h| store_domain(&h))
}

/// One line of a cart.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineItem {
    pub name: String,
    pub quantity: u32,
    /// Minor units.
    pub unit_price: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Size, colour, seller: one line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
}

impl LineItem {
    #[must_use]
    pub fn line_total(&self) -> i64 {
        self.unit_price.saturating_mul(i64::from(self.quantity))
    }
}

/// Where a purchase goes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "id")]
pub enum ShipTo {
    /// The AI left it out: the user picks on the phone.
    Choose,
    /// Not shipped.
    Nothing,
    /// An address id from `payments_addresses_list`.
    Address(String),
}

/// A purchase as the AI asked for it, checked: amounts add up, addresses are `https` on the store's site.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cart {
    pub merchant: String,
    pub merchant_url: String,
    /// The host of `merchant_url` without `www.`.
    pub domain: String,
    /// The page Pay on phone opens: `checkout_url`, else `merchant_url`.
    pub checkout_url: String,
    /// The host of `checkout_url`.
    pub checkout_host: String,
    pub items: Vec<LineItem>,
    pub currency: String,
    pub subtotal: i64,
    pub shipping: i64,
    pub tax: i64,
    pub discount: i64,
    pub total: i64,
    pub ship_to: ShipTo,
    pub payment_method: Option<String>,
    pub note: Option<String>,
}

fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn text_field(v: Option<&Value>, name: &str, max: usize, required: bool) -> Result<Option<String>, String> {
    match v {
        None | Some(Value::Null) if !required => Ok(None),
        None | Some(Value::Null) => Err(format!("`{name}` is required.")),
        Some(Value::String(s)) => {
            let s = one_line(s);
            if s.chars().any(char::is_control) || s.chars().count() > max || (required && s.is_empty()) {
                return Err(format!("`{name}` must be 1..={max} characters on one line."));
            }
            Ok((!s.is_empty()).then_some(s))
        }
        Some(_) => Err(format!("`{name}` must be a string.")),
    }
}

fn url_field(v: Option<&str>, name: &str) -> Result<Option<(String, String)>, String> {
    match v.map(str::trim).filter(|s| !s.is_empty()) {
        None => Ok(None),
        Some(u) if u.len() > MAX_URL || u.chars().any(|c| c.is_whitespace() || c.is_control()) => {
            Err(format!("`{name}` must be one https address, at most {MAX_URL} characters."))
        }
        Some(u) => Ok(Some((u.to_owned(), https_host(u)?))),
    }
}

fn parse_items(value: Option<&Value>, currency: &str) -> Result<Vec<LineItem>, String> {
    let list = value.and_then(Value::as_array).ok_or_else(|| {
        "`items` must be a list of {name, quantity, unit_price} objects (unit_price as a string, like \"9.99\")."
            .to_owned()
    })?;
    if list.is_empty() || list.len() > MAX_ITEMS {
        return Err(format!("`items` must have 1 to {MAX_ITEMS} lines."));
    }
    let mut items = Vec::with_capacity(list.len());
    for (i, raw) in list.iter().enumerate() {
        let n = i + 1;
        let obj = raw.as_object().ok_or_else(|| format!("Item {n} must be an object."))?;
        if let Some(k) =
            obj.keys().find(|k| !matches!(k.as_str(), "name" | "quantity" | "unit_price" | "url" | "details"))
        {
            return Err(format!(
                "Item {n} has an unknown property `{k}` (allowed: name, quantity, unit_price, url, details)."
            ));
        }
        let name = text_field(obj.get("name"), &format!("items[{i}].name"), MAX_NAME, true)?.unwrap_or_default();
        let quantity = match obj.get("quantity") {
            None | Some(Value::Null) => 1,
            Some(q) => q
                .as_u64()
                .and_then(|q| u32::try_from(q).ok())
                .filter(|q| (1..=MAX_QUANTITY).contains(q))
                .ok_or_else(|| format!("`items[{i}].quantity` must be a whole number from 1 to {MAX_QUANTITY}."))?,
        };
        let unit_price = parse_amount(
            obj.get("unit_price").ok_or_else(|| format!("`items[{i}].unit_price` is required."))?,
            currency,
            &format!("items[{i}].unit_price"),
        )?;
        let url = url_field(obj.get("url").and_then(Value::as_str), &format!("items[{i}].url"))?.map(|(u, _)| u);
        let details = text_field(obj.get("details"), &format!("items[{i}].details"), MAX_NAME, false)?;
        items.push(LineItem {
            name,
            quantity,
            unit_price,
            url,
            details,
        });
    }
    Ok(items)
}

impl Cart {
    /// The cart of a `payments_purchase_request` call, checked.
    pub fn from_call(call: &ConnectorCall) -> Result<Self, String> {
        let a = &call.args;
        let currency = normalize_currency(call.str_arg("currency").unwrap_or_default())?;
        let merchant = text_field(a.get("merchant"), "merchant", 100, true)?.unwrap_or_default();
        let (merchant_url, host) = url_field(call.str_arg("merchant_url"), "merchant_url")?
            .ok_or_else(|| "`merchant_url` is required: the store's page for this purchase.".to_owned())?;
        let domain = store_domain(&host);
        let (checkout_url, checkout_host) =
            url_field(call.str_arg("checkout_url"), "checkout_url")?.unwrap_or_else(|| (merchant_url.clone(), host));
        if !in_domain(&checkout_host, &domain) && !HOSTED_CHECKOUTS.iter().any(|h| in_domain(&checkout_host, h)) {
            return Err(format!(
                "`checkout_url` must be on {domain} (or a hosted payment page such as checkout.stripe.com), not \
                 {checkout_host}."
            ));
        }
        let items = parse_items(a.get("items"), &currency)?;
        let amount = |name: &str| -> Result<i64, String> {
            match a.get(name) {
                None | Some(Value::Null) => Ok(0),
                Some(v) => parse_amount(v, &currency, name),
            }
        };
        let (shipping, tax, discount) = (amount("shipping")?, amount("tax")?, amount("discount")?);
        let total = parse_amount(a.get("total").unwrap_or(&Value::Null), &currency, "total")?;
        let subtotal = items.iter().try_fold(0_i64, |sum, i| sum.checked_add(i.line_total()));
        let subtotal = subtotal.filter(|s| *s <= MAX_MINOR).ok_or_else(|| "The cart is too large.".to_owned())?;
        let expected = subtotal + shipping + tax - discount;
        if expected != total {
            return Err(format!(
                "`total` is {} but the items ({}) plus shipping ({}) and tax ({}) minus the discount ({}) make {}. \
                 Send the amounts exactly as the store's checkout shows them.",
                format_minor(total, &currency),
                format_minor(subtotal, &currency),
                format_minor(shipping, &currency),
                format_minor(tax, &currency),
                format_minor(discount, &currency),
                format_minor(expected, &currency)
            ));
        }
        if total <= 0 {
            return Err("`total` must be more than zero.".to_owned());
        }
        let ship_to = match call.str_arg("ship_to").map(str::trim).filter(|s| !s.is_empty()) {
            None => ShipTo::Choose,
            Some(s) if s.eq_ignore_ascii_case(NO_SHIPPING) => ShipTo::Nothing,
            Some(s) => ShipTo::Address(s.to_owned()),
        };
        Ok(Self {
            merchant,
            merchant_url,
            domain,
            checkout_url,
            checkout_host,
            items,
            currency,
            subtotal,
            shipping,
            tax,
            discount,
            total,
            ship_to,
            payment_method: call.str_arg("payment_method").map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned),
            note: text_field(a.get("note"), "note", MAX_NOTE, false)?,
        })
    }

    /// "2 items at amazon.com for $24.97".
    #[must_use]
    pub fn summary(&self) -> String {
        let n: u32 = self.items.iter().map(|i| i.quantity).sum();
        format!(
            "{n} item{} at {} for {}",
            if n == 1 {
                ""
            } else {
                "s"
            },
            self.domain,
            display_amount(self.total, &self.currency)
        )
    }

    /// The items as the mandate and the ledger keep them.
    #[must_use]
    pub fn items_json(&self) -> Vec<Value> {
        self.items
            .iter()
            .map(|i| {
                let mut o = Map::new();
                o.insert("name".to_owned(), json!(i.name));
                o.insert("quantity".to_owned(), json!(i.quantity));
                o.insert("unit_price".to_owned(), json!(format_minor(i.unit_price, &self.currency)));
                if let Some(d) = &i.details {
                    o.insert("details".to_owned(), json!(d));
                }
                if let Some(u) = &i.url {
                    o.insert("url".to_owned(), json!(u));
                }
                Value::Object(o)
            })
            .collect()
    }
}

/// The check `payments_purchase_request` runs on its arguments, on the server and again on the phone.
pub fn check_purchase_request(call: &ConnectorCall) -> Result<(), String> {
    Cart::from_call(call).map(drop)
}

/// The check `payments_purchase_complete` runs: a charged amount needs its currency, and the other way round.
pub fn check_purchase_complete(call: &ConnectorCall) -> Result<(), String> {
    match (call.args.get("charged_total"), call.str_arg("currency")) {
        (Some(v), Some(c)) => parse_amount(v, &normalize_currency(c)?, "charged_total").map(drop),
        (None, None) => Ok(()),
        _ => Err("Give `charged_total` and `currency` together.".to_owned()),
    }?;
    url_field(call.str_arg("receipt_url"), "receipt_url").map(drop)
}

/// What the payment of an approved purchase is, as the mandate and the ledger name it (never a number).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MandatePayment {
    /// `virtual_card`, `card`, `merchant_account` or `pay_on_phone`.
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brand: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last4: Option<String>,
}

/// Where an approved purchase goes, as the mandate names it: the label and the country, not the street.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MandateShipTo {
    pub label: String,
    pub country: String,
}

/// The payload of a cart mandate: exactly what the user (or their spend limit) approved. Field order is fixed, so the
/// same cart always serializes to the same bytes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CartMandate {
    pub typ: String,
    pub purchase_id: String,
    pub merchant: Value,
    pub items: Vec<Value>,
    pub amounts: Value,
    pub currency: String,
    #[serde(default)]
    pub ship_to: Option<MandateShipTo>,
    pub payment: MandatePayment,
    /// The AI connection that asked, by its name.
    pub agent: String,
    /// "you" or "spend limit".
    pub approved_by: String,
    pub iat: i64,
    pub exp: i64,
}

impl CartMandate {
    #[must_use]
    #[allow(clippy::too_many_arguments, reason = "a mandate is described by this many independent facts")]
    pub fn new(
        cart: &Cart,
        purchase_id: &str,
        ship_to: Option<MandateShipTo>,
        payment: MandatePayment,
        agent: &str,
        approved_by: &str,
        iat: i64,
        lifetime_secs: i64,
    ) -> Self {
        let c = &cart.currency;
        Self {
            typ: MANDATE_TYPE.to_owned(),
            purchase_id: purchase_id.to_owned(),
            merchant: json!({"name": cart.merchant, "domain": cart.domain, "url": cart.merchant_url}),
            items: cart.items_json(),
            amounts: json!({
                "subtotal": format_minor(cart.subtotal, c),
                "shipping": format_minor(cart.shipping, c),
                "tax": format_minor(cart.tax, c),
                "discount": format_minor(cart.discount, c),
                "total": format_minor(cart.total, c),
            }),
            currency: c.clone(),
            ship_to,
            payment,
            agent: agent.to_owned(),
            approved_by: approved_by.to_owned(),
            iat,
            exp: iat.saturating_add(lifetime_secs),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connector::spec_for_tool;

    fn call(args: &Value) -> ConnectorCall {
        ConnectorCall {
            service: PAYMENTS.to_owned(),
            op: PURCHASE_REQUEST_OP.to_owned(),
            args: args.as_object().unwrap().clone(),
        }
    }

    fn cart_args() -> Value {
        json!({
            "merchant": "Amazon",
            "merchant_url": "https://www.amazon.com/dp/B0EXAMPLE",
            "items": [{"name": "USB-C cable", "quantity": 2, "unit_price": "9.99"}],
            "shipping": "4.99",
            "currency": "usd",
            "total": "24.97"
        })
    }

    #[test]
    fn amounts_parse_exactly_in_the_currencys_decimals() {
        let v = |s: &str| json!(s);
        assert_eq!(parse_amount(&v("19.98"), "USD", "x"), Ok(1998));
        assert_eq!(parse_amount(&v("19.9"), "USD", "x"), Ok(1990));
        assert_eq!(parse_amount(&v("19"), "USD", "x"), Ok(1900));
        assert_eq!(parse_amount(&json!(19.98), "USD", "x"), Ok(1998));
        assert_eq!(parse_amount(&v("500"), "JPY", "x"), Ok(500));
        assert_eq!(parse_amount(&v("1.234"), "KWD", "x"), Ok(1234));
        for bad in ["19.999", "-1", "1e3", "", ".5", "5.", "1,5", " 1 2", "0x10"] {
            assert!(parse_amount(&v(bad), "USD", "x").is_err(), "{bad}");
        }
        assert!(parse_amount(&v("5.5"), "JPY", "x").is_err(), "yen have no decimals");
        assert!(parse_amount(&json!(0.1 + 0.2), "USD", "x").is_err(), "a float that is not a price");
        assert!(parse_amount(&v("2000000000"), "USD", "x").is_err(), "above the cap");
        assert_eq!(format_minor(1998, "USD"), "19.98");
        assert_eq!(format_minor(5, "USD"), "0.05");
        assert_eq!(format_minor(500, "JPY"), "500");
        assert_eq!(format_minor(1234, "KWD"), "1.234");
        assert_eq!(display_amount(1998, "USD"), "$19.98");
        assert_eq!(display_amount(-100, "EUR"), "-€1.00");
        assert_eq!(display_amount(100, "CHF"), "1.00 CHF");
    }

    #[test]
    fn hosts_are_https_domain_names_without_user_names() {
        assert_eq!(https_host("https://www.Amazon.com/dp/X?a=b").unwrap(), "www.amazon.com");
        assert_eq!(https_host("https://shop.example.co.uk:443/").unwrap(), "shop.example.co.uk");
        for bad in [
            "http://amazon.com",
            "https://amazon.com@evil.example/",
            "https://192.168.1.1/",
            "https://localhost/",
            "https://exa mple.com",
            "https://-bad.com",
            "javascript:alert(1)",
        ] {
            assert!(https_host(bad).is_err(), "{bad}");
        }
        assert_eq!(store_domain("www.amazon.com"), "amazon.com");
        assert_eq!(store_domain("smile.amazon.com"), "smile.amazon.com");
        assert!(in_domain("smile.amazon.com", "amazon.com"));
        assert!(in_domain("amazon.com", "Amazon.com"));
        assert!(!in_domain("evilamazon.com", "amazon.com"));
        assert!(!in_domain("amazon.com.evil.example", "amazon.com"));
        assert_eq!(normalize_domain(" https://www.Amazon.com/x ").unwrap(), "amazon.com");
        assert_eq!(normalize_domain("ebay.co.uk").unwrap(), "ebay.co.uk");
    }

    #[test]
    fn a_cart_adds_up_or_is_refused() {
        let cart = Cart::from_call(&call(&cart_args())).unwrap();
        assert_eq!(
            (cart.domain.as_str(), cart.currency.as_str(), cart.subtotal, cart.total),
            ("amazon.com", "USD", 1998, 2497)
        );
        assert_eq!(cart.ship_to, ShipTo::Choose);
        assert_eq!(cart.checkout_url, "https://www.amazon.com/dp/B0EXAMPLE");
        assert_eq!(cart.summary(), "2 items at amazon.com for $24.97");

        let mut wrong = cart_args();
        wrong["total"] = json!("20.00");
        let e = Cart::from_call(&call(&wrong)).unwrap_err();
        assert!(e.contains("make 24.97"), "{e}");

        let mut discounted = cart_args();
        discounted["discount"] = json!("5.00");
        discounted["tax"] = json!("1.50");
        discounted["total"] = json!("21.47");
        discounted["ship_to"] = json!("none");
        let cart = Cart::from_call(&call(&discounted)).unwrap();
        assert_eq!((cart.total, cart.ship_to), (2147, ShipTo::Nothing));

        let mut elsewhere = cart_args();
        elsewhere["checkout_url"] = json!("https://amazon.com.evil.example/pay");
        assert!(Cart::from_call(&call(&elsewhere)).unwrap_err().contains("must be on amazon.com"));
        let mut stripe = cart_args();
        stripe["checkout_url"] = json!("https://checkout.stripe.com/c/pay/cs_test_1");
        assert_eq!(Cart::from_call(&call(&stripe)).unwrap().checkout_host, "checkout.stripe.com");

        let mut extra = cart_args();
        extra["items"][0]["price"] = json!("1");
        assert!(Cart::from_call(&call(&extra)).unwrap_err().contains("unknown property `price`"));
        let mut many = cart_args();
        many["items"] = json!((0..51).map(|_| json!({"name": "x", "unit_price": "1"})).collect::<Vec<_>>());
        assert!(Cart::from_call(&call(&many)).unwrap_err().contains("1 to 50"));
        let mut zero = cart_args();
        zero["items"][0]["quantity"] = json!(0);
        assert!(Cart::from_call(&call(&zero)).is_err());
        let mut free = cart_args();
        free["items"][0]["unit_price"] = json!("0");
        free["shipping"] = json!("0");
        free["total"] = json!("0");
        assert!(Cart::from_call(&call(&free)).unwrap_err().contains("more than zero"));
    }

    #[test]
    fn the_server_checks_the_cart_with_the_tool_description() {
        let spec = spec_for_tool("payments_purchase_request").unwrap();
        assert!(spec.parse(&cart_args()).is_ok());
        let mut wrong = cart_args();
        wrong["total"] = json!("1.00");
        assert!(spec.parse(&wrong).unwrap_err().contains("make 24.97"));
        let complete = spec_for_tool("payments_purchase_complete").unwrap();
        assert!(complete.parse(&json!({"purchase_id": "p", "status": "completed"})).is_ok());
        assert!(
            complete
                .parse(&json!({"purchase_id": "p", "status": "completed", "charged_total": "1.00"}))
                .unwrap_err()
                .contains("together")
        );
        assert!(
            complete.parse(&json!({"purchase_id": "p", "status": "completed", "receipt_url": "http://x.com"})).is_err()
        );
    }

    #[test]
    fn a_mandate_serializes_the_same_cart_to_the_same_bytes() {
        let cart = Cart::from_call(&call(&cart_args())).unwrap();
        let payment = MandatePayment {
            kind: "virtual_card".to_owned(),
            brand: Some("Visa".to_owned()),
            last4: Some("1111".to_owned()),
        };
        let a = CartMandate::new(&cart, "p1", None, payment.clone(), "Claude", "you", 100, 3600);
        let b = CartMandate::new(&cart, "p1", None, payment, "Claude", "you", 100, 3600);
        assert_eq!(serde_json::to_vec(&a).unwrap(), serde_json::to_vec(&b).unwrap());
        let v = serde_json::to_value(&a).unwrap();
        assert_eq!(v["amounts"]["total"], "24.97");
        assert_eq!(v["items"][0], json!({"name": "USB-C cable", "quantity": 2, "unit_price": "9.99"}));
        assert_eq!((v["iat"].as_i64(), v["exp"].as_i64()), (Some(100), Some(3700)));
    }
}
