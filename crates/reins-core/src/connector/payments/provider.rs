//! Virtual card providers: a new card for each approved purchase, locked to the store and capped at the approved
//! total, closed afterwards. Privacy.com first (every user gets a personal API key); Lithic speaks almost the same API
//! and can follow behind [`CardIssuer`]. The API key is sent to the provider only, and nothing the provider answers is
//! logged: its answer holds the card number.

use serde::Deserialize;
use serde_json::{Value, json};
use zeroize::Zeroizing;

use crate::{CoreError, text};

pub const PRIVACY: &str = "privacy";
pub const PRIVACY_BASE: &str = "https://api.privacy.com/v1";
pub const PRIVACY_SANDBOX_BASE: &str = "https://sandbox.privacy.com/v1";

/// The card to make.
pub struct CardRequest {
    /// The most it may be charged, in cents.
    pub limit_cents: i64,
    /// "Reins · amazon.com · 8c0e7f1a".
    pub memo: String,
    /// Closes after the first charge; else locked to the first store that charges it.
    pub single_use: bool,
}

/// A card the provider made. The number and the code are wiped when dropped.
pub struct IssuedCard {
    pub token: String,
    pub number: Zeroizing<String>,
    pub code: Zeroizing<String>,
    pub exp_month: String,
    pub exp_year: String,
    pub last4: String,
}

#[async_trait::async_trait]
pub trait CardIssuer: Send + Sync {
    /// Checks that the key works (it lists nothing back).
    async fn check(&self, key: &str) -> Result<(), CoreError>;
    async fn create(&self, key: &str, card: &CardRequest) -> Result<IssuedCard, CoreError>;
    /// Closes a card for good.
    async fn close(&self, key: &str, token: &str) -> Result<(), CoreError>;
    /// The approved charges of a card: who made them, as the card network names them ("AMZN Mktp US"), and how much.
    async fn charges(&self, key: &str, token: &str) -> Result<Vec<Charge>, CoreError>;
}

/// One approved charge of a virtual card.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Charge {
    pub descriptor: String,
    /// Cents.
    pub amount: i64,
}

/// A card id as providers make them (a UUID), before it goes into a path or a query.
fn card_token(token: &str) -> Result<(), CoreError> {
    if token.is_empty() || token.len() > 64 || !token.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
        return Err(CoreError::invalid("not a card id"));
    }
    Ok(())
}

/// Privacy.com's API (`https://developers.privacy.com`).
pub struct Privacy {
    http: reqwest::Client,
    base: String,
}

#[derive(Deserialize)]
struct Transactions {
    #[serde(default)]
    data: Vec<Transaction>,
}

#[derive(Deserialize)]
struct Transaction {
    #[serde(default)]
    result: String,
    /// The authorization amount, in cents.
    #[serde(default)]
    amount: i64,
    #[serde(default)]
    merchant: Option<Merchant>,
}

#[derive(Deserialize)]
struct Merchant {
    #[serde(default)]
    descriptor: String,
}

#[derive(Deserialize)]
struct FullCard {
    token: String,
    #[serde(default)]
    pan: Option<String>,
    #[serde(default)]
    cvv: Option<String>,
    #[serde(default)]
    exp_month: Option<String>,
    #[serde(default)]
    exp_year: Option<String>,
    #[serde(default)]
    last_four: Option<String>,
}

impl Privacy {
    pub fn new(http: reqwest::Client, base: &str) -> Self {
        Self {
            http,
            base: base.trim_end_matches('/').to_owned(),
        }
    }

    async fn send(&self, req: reqwest::RequestBuilder, key: &str) -> Result<Zeroizing<Vec<u8>>, CoreError> {
        let response =
            req.header(reqwest::header::AUTHORIZATION, format!("api-key {key}")).send().await.map_err(|_| {
                CoreError::Network {
                    reason: "Privacy.com could not be reached".to_owned(),
                }
            })?;
        let status = response.status().as_u16();
        let body = Zeroizing::new(response.bytes().await.map(|b| b.to_vec()).unwrap_or_default());
        match status {
            200..=299 => Ok(body),
            401 | 403 => Err(CoreError::needs_attention(
                "Privacy.com refused the API key. Paste a new one in Integrations → Payments.",
            )),
            429 => Err(CoreError::service("Privacy.com is busy. Try again in a minute.")),
            _ => {
                let message = serde_json::from_slice::<Value>(&body)
                    .ok()
                    .and_then(|v| v["message"].as_str().map(|m| text::truncate_chars(&text::one_line(m), 200)))
                    .filter(|m| !m.is_empty())
                    .unwrap_or_else(|| format!("error {status}"));
                Err(CoreError::service(format!("Privacy.com refused: {message}")))
            }
        }
    }
}

#[async_trait::async_trait]
impl CardIssuer for Privacy {
    async fn check(&self, key: &str) -> Result<(), CoreError> {
        self.send(self.http.get(format!("{}/cards", self.base)).query(&[("page_size", "1")]), key).await.map(drop)
    }

    async fn create(&self, key: &str, card: &CardRequest) -> Result<IssuedCard, CoreError> {
        let body = json!({
            "type": if card.single_use { "SINGLE_USE" } else { "MERCHANT_LOCKED" },
            "memo": card.memo,
            "spend_limit": card.limit_cents,
            "spend_limit_duration": if card.single_use { "TRANSACTION" } else { "FOREVER" },
            "state": "OPEN",
        });
        let bytes = self.send(self.http.post(format!("{}/cards", self.base)).json(&body), key).await?;
        let made: FullCard = serde_json::from_slice(&bytes)
            .map_err(|_| CoreError::service("Privacy.com sent an answer Reins does not understand."))?;
        let number = Zeroizing::new(made.pan.unwrap_or_default());
        if made.token.is_empty() || number.len() < 12 {
            return Err(CoreError::service("Privacy.com made no usable card."));
        }
        let last4 = made
            .last_four
            .filter(|l| l.len() == 4)
            .or_else(|| crate::connector::vault::wallet::last_four(&number))
            .unwrap_or_default();
        Ok(IssuedCard {
            token: made.token,
            code: Zeroizing::new(made.cvv.unwrap_or_default()),
            number,
            exp_month: made.exp_month.unwrap_or_default(),
            exp_year: made.exp_year.unwrap_or_default(),
            last4,
        })
    }

    async fn charges(&self, key: &str, token: &str) -> Result<Vec<Charge>, CoreError> {
        card_token(token)?;
        let req = self.http.get(format!("{}/transactions", self.base)).query(&[
            ("card_token", token),
            ("result", "APPROVED"),
            ("page_size", "50"),
        ]);
        let bytes = self.send(req, key).await?;
        let list: Transactions = serde_json::from_slice(&bytes)
            .map_err(|_| CoreError::service("Privacy.com sent an answer Reins does not understand."))?;
        Ok(list
            .data
            .into_iter()
            .filter(|t| t.result.eq_ignore_ascii_case("APPROVED"))
            .map(|t| Charge {
                descriptor: text::truncate_chars(
                    &text::one_line(&t.merchant.map(|m| m.descriptor).unwrap_or_default()),
                    80,
                ),
                amount: t.amount.max(0),
            })
            .collect())
    }

    async fn close(&self, key: &str, token: &str) -> Result<(), CoreError> {
        card_token(token)?;
        let url = format!("{}/cards/{token}", self.base);
        self.send(self.http.patch(url).json(&json!({"state": "CLOSED"})), key).await.map(drop)
    }
}
