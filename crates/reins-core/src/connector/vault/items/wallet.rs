//! The vault's cards and addresses, for Payments: cards with their number masked (and, for one approved purchase,
//! their details), and identities that hold an address. Items in the trash or the archive are left out.

use zeroize::Zeroizing;

use super::model::{Entry, Kind, Snapshot, State};
use crate::CoreError;
use crate::connector::vault::Vault;

/// A card as lists show it: never the number or the code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CardInfo {
    pub id: String,
    pub name: String,
    pub brand: String,
    pub last4: Option<String>,
    pub exp_month: String,
    pub exp_year: String,
    pub holder: String,
}

/// A card's details, for one approved purchase. Wiped when dropped.
pub(crate) struct CardDetails {
    pub number: Zeroizing<String>,
    pub code: Zeroizing<String>,
    pub holder: String,
    pub brand: String,
    pub exp_month: String,
    pub exp_year: String,
}

/// An identity with an address.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Address {
    pub id: String,
    /// The item's name (Home, Work).
    pub label: String,
    pub name: String,
    pub company: String,
    pub line1: String,
    pub line2: String,
    pub line3: String,
    pub city: String,
    pub state: String,
    pub postal_code: String,
    pub country: String,
    pub phone: String,
    pub email: String,
}

/// The last four digits of a card number, when it has that many.
pub(crate) fn last_four(number: &str) -> Option<String> {
    let digits: Vec<char> = number.chars().filter(char::is_ascii_digit).collect();
    (digits.len() >= 4).then(|| digits[digits.len() - 4..].iter().collect())
}

/// The brand of a card number by its first digits, for cards saved without one.
pub(crate) fn brand_of(number: &str) -> &'static str {
    let digits: String = number.chars().filter(char::is_ascii_digit).take(6).collect();
    let prefix = |n: usize| digits.get(..n).and_then(|p| p.parse::<u32>().ok()).unwrap_or(0);
    match () {
        () if digits.starts_with('4') => "Visa",
        () if (51..=55).contains(&prefix(2)) || (2221..=2720).contains(&prefix(4)) => "Mastercard",
        () if matches!(prefix(2), 34 | 37) => "American Express",
        () if digits.starts_with("6011") || digits.starts_with("65") || (644..=649).contains(&prefix(3)) => "Discover",
        () if (3528..=3589).contains(&prefix(4)) => "JCB",
        () => "Card",
    }
}

fn usable(e: &Entry, kind: Kind) -> bool {
    e.kind == kind && e.state() == State::Active
}

fn card_info(e: &Entry) -> CardInfo {
    let number = e.dec("/card/number");
    let brand = e.line("/card/brand");
    CardInfo {
        id: e.id.clone(),
        name: e.name(),
        brand: if brand.is_empty() {
            number.as_deref().map(|n| brand_of(n).to_owned()).unwrap_or_default()
        } else {
            brand
        },
        last4: number.as_deref().and_then(|n| last_four(n)),
        exp_month: e.line("/card/expMonth"),
        exp_year: e.line("/card/expYear"),
        holder: e.line("/card/cardholderName"),
    }
}

/// The vault's cards in use, by name.
pub(crate) async fn cards(vault: &Vault, account: &str) -> Result<Vec<CardInfo>, CoreError> {
    let snap = Snapshot::load(vault, account).await?;
    let mut cards: Vec<CardInfo> = snap.entries.iter().filter(|e| usable(e, Kind::Card)).map(card_info).collect();
    cards.sort_by_key(|c| c.name.to_lowercase());
    Ok(cards)
}

/// One card's details, read fresh from the vault. A card without a number cannot pay.
pub(crate) async fn card_details(vault: &Vault, account: &str, id: &str) -> Result<CardDetails, CoreError> {
    let snap = Snapshot::load(vault, account).await?;
    let entry = snap
        .entries
        .iter()
        .find(|e| e.id == id && usable(e, Kind::Card))
        .ok_or_else(|| CoreError::service("That card is no longer in the vault."))?;
    let number = entry
        .dec("/card/number")
        .filter(|n| n.chars().filter(char::is_ascii_digit).count() >= 12)
        .ok_or_else(|| CoreError::service(format!("{} has no card number in the vault.", entry.name())))?;
    let info = card_info(entry);
    Ok(CardDetails {
        code: entry.dec("/card/code").unwrap_or_default(),
        number,
        holder: info.holder,
        brand: info.brand,
        exp_month: info.exp_month,
        exp_year: info.exp_year,
    })
}

fn address_of(e: &Entry) -> Option<Address> {
    let f = |k: &str| e.line(&format!("/identity/{k}"));
    let a = Address {
        id: e.id.clone(),
        label: e.name(),
        name: e.holder(),
        company: f("company"),
        line1: f("address1"),
        line2: f("address2"),
        line3: f("address3"),
        city: f("city"),
        state: f("state"),
        postal_code: f("postalCode"),
        country: f("country"),
        phone: f("phone"),
        email: f("email"),
    };
    (!a.line1.is_empty() && (!a.city.is_empty() || !a.postal_code.is_empty())).then_some(a)
}

/// The identities in use that hold an address, by label.
pub(crate) async fn addresses(vault: &Vault, account: &str) -> Result<Vec<Address>, CoreError> {
    let snap = Snapshot::load(vault, account).await?;
    let mut found: Vec<Address> =
        snap.entries.iter().filter(|e| usable(e, Kind::Identity)).filter_map(address_of).collect();
    found.sort_by_key(|a| a.label.to_lowercase());
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brands_are_told_by_their_first_digits() {
        assert_eq!(brand_of("4111 1111 1111 1111"), "Visa");
        assert_eq!(brand_of("5555555555554444"), "Mastercard");
        assert_eq!(brand_of("2223003122003222"), "Mastercard");
        assert_eq!(brand_of("378282246310005"), "American Express");
        assert_eq!(brand_of("6011111111111117"), "Discover");
        assert_eq!(brand_of("3530111333300000"), "JCB");
        assert_eq!(brand_of("9999"), "Card");
        assert_eq!(last_four("4111-1111-1111-1234").as_deref(), Some("1234"));
        assert_eq!(last_four("123"), None);
    }
}
