//! Payments: an AI buys something for the user. It lists the payment methods and addresses the user allows (masked),
//! asks to buy one cart, and reports how checkout went. The phone shows the purchase like a receipt; see
//! `docs/payments.md`.

use crate::connector::{Effect, Param, ToolSpec, choice_p, json_p, str_p, tool};
use crate::payments::{
    ADDRESSES_LIST_OP, MAX_NOTE, MAX_ORDER_ID, MAX_URL, METHODS_LIST_OP, PAYMENTS, PURCHASE_COMPLETE_OP,
    PURCHASE_REQUEST_OP, check_purchase_complete, check_purchase_request,
};

const AMOUNT_HELP: &str = "An amount as a string in the currency's decimals, like \"4.99\".";

const CLIENT_KEY: Param = str_p(
    "client_key",
    64,
    false,
    "Set by the Reins desktop app's MCP bridge only (its public key): card details are then sealed to it. Leave out.",
);
const NONCE: Param =
    str_p("nonce", 64, false, "Set by the Reins desktop app's MCP bridge only, with client_key. Leave out.");

pub(super) fn tools() -> Vec<ToolSpec> {
    vec![
        tool(
            "payments_methods_list",
            PAYMENTS,
            METHODS_LIST_OP,
            Effect::List,
            "List payment methods",
            "Lists the payment methods the user allows AIs to use: an id, the kind (virtual_card: a new card made for \
             each purchase, locked to the store and capped at the approved total; card: a card from the user's vault; \
             merchant_account: the payment method saved in the user's account at the store; pay_on_phone: the user \
             pays on their phone at the store's checkout), a nickname, and for cards the brand, the last four digits \
             and the expiry. Never a full card number. Use an id as payment_method in payments_purchase_request.",
            vec![],
            None,
        ),
        tool(
            "payments_addresses_list",
            PAYMENTS,
            ADDRESSES_LIST_OP,
            Effect::List,
            "List shipping addresses",
            "Lists the user's shipping addresses: an id, a label (Home, Work), the city, the region and the country. \
             The street is never shown; the full address comes with an approved purchase. Use an id as ship_to in \
             payments_purchase_request.",
            vec![],
            None,
        ),
        tool(
            "payments_purchase_request",
            PAYMENTS,
            PURCHASE_REQUEST_OP,
            Effect::Write,
            "Ask to buy something",
            "Asks the user to approve one purchase, exactly as the store's checkout shows it: the store, its page, \
             every item with its quantity and unit price, shipping, tax, a discount, and the total, which must be \
             exactly the items plus shipping and tax minus the discount. The user sees it on their phone like a \
             receipt and approves with biometrics (or a spend limit they set approves it). When approved, the answer \
             has what to pay with (`payment`: a card's details, or `merchant_account`: use what the store has on \
             file, or `pay_on_phone`: do not place the order, the user pays on their phone), the full shipping \
             address (`ship_to`), and `mandate`, a signed record of the approved cart. Check out right away with \
             exactly this cart, then call payments_purchase_complete. Never reuse card details for anything else.",
            vec![
                str_p("merchant", 100, true, "The store's name, as the store calls itself (Amazon)."),
                str_p(
                    "merchant_url",
                    MAX_URL,
                    true,
                    "The https page of the store for this purchase (the product or the cart). Its domain is shown to the \
                     user and decides which limits apply.",
                ),
                json_p(
                    "items",
                    30_000,
                    true,
                    "The cart: a list of {name, quantity, unit_price, url, details}. quantity defaults to 1; \
                     unit_price is a string (\"9.99\"); details is one line (size, colour, seller).",
                ),
                str_p("currency", 3, true, "The ISO 4217 code of every amount (USD, EUR, GBP, JPY)."),
                str_p("shipping", 20, false, AMOUNT_HELP),
                str_p("tax", 20, false, AMOUNT_HELP),
                str_p("discount", 20, false, AMOUNT_HELP),
                str_p("total", 20, true, "What will be charged in all. An amount as a string, like \"24.97\"."),
                str_p(
                    "ship_to",
                    100,
                    false,
                    "An address id from payments_addresses_list, or `none` for something not shipped. Leave out to let \
                     the user choose on their phone.",
                ),
                str_p(
                    "payment_method",
                    100,
                    false,
                    "A payment method id from payments_methods_list. Leave out to let the user choose on their phone.",
                ),
                str_p(
                    "checkout_url",
                    MAX_URL,
                    false,
                    "For pay_on_phone: the store's https checkout page the phone opens. Default: merchant_url.",
                ),
                str_p("note", MAX_NOTE, false, "Why you are buying this, in one sentence the user will read."),
                CLIENT_KEY,
                NONCE,
            ],
            None,
        )
        .once()
        .checked(check_purchase_request),
        tool(
            "payments_purchase_complete",
            PAYMENTS,
            PURCHASE_COMPLETE_OP,
            Effect::Write,
            "Report how a purchase went",
            "After checking out an approved purchase: says whether the order went through, with the store's order \
             number, the amount actually charged and the receipt page, for the user's spending history. A virtual \
             card made for a purchase that failed or was cancelled is closed. Does not ask the user.",
            vec![
                str_p("purchase_id", 64, true, "The purchase_id of the approved purchase."),
                choice_p(
                    "status",
                    &["completed", "failed", "cancelled"],
                    true,
                    "completed: the order was placed; failed: the store refused it; cancelled: you did not place it.",
                ),
                str_p("order_id", MAX_ORDER_ID, false, "The store's order number."),
                str_p("charged_total", 20, false, "The amount charged, as the store says (with currency)."),
                str_p("currency", 3, false, "The currency of charged_total."),
                str_p("receipt_url", MAX_URL, false, "The https page of the order or receipt."),
                str_p("note", MAX_NOTE, false, "Anything the user should know (a split shipment, a changed price)."),
            ],
            None,
        )
        .checked(check_purchase_complete),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn purchases_are_asked_every_time_and_reports_are_not() {
        let all = tools();
        assert_eq!(all.len(), 4);
        for spec in &all {
            assert!(spec.tool.starts_with("payments_") && spec.tool.trim_start_matches("payments_") == spec.op);
            assert!(!spec.desktop_only, "{} is offered to every AI", spec.tool);
            assert_eq!(spec.class, "");
        }
        let request = all.iter().find(|s| s.op == PURCHASE_REQUEST_OP).unwrap();
        assert!(request.once_only && request.check.is_some());
        let complete = all.iter().find(|s| s.op == PURCHASE_COMPLETE_OP).unwrap();
        assert!(!complete.once_only && complete.check.is_some());
    }
}
