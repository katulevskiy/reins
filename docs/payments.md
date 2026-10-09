# Payments (Reins Pay)

An agent can buy things for you. It finds what you want, fills the cart at the store, and asks Reins:
"buy these items from this store, shipped to this address, paid with this payment method". Your phone shows the
purchase like a receipt. You approve it with your screen lock or biometrics, and only then does the agent get what
it needs to pay. Every purchase goes into a ledger on the phone, with budgets and limits you set.

Turn it on under **Integrations → Payments**.

## What exists today, and what Reins builds on

Agent payments are new, and the big standards need the store to take part:

| Standard | What it is | Can Reins use it today? |
|---|---|---|
| Google's [AP2](https://ap2-protocol.org) (Agent Payments Protocol) | Signed "mandates": the user's intent, and the exact cart the user approved, as verifiable credentials. | The idea, yes. Reins signs a mandate of every cart you approve. Stores that verify AP2 mandates are still rare. |
| Stripe and OpenAI's [Agentic Commerce Protocol](https://www.agenticcommerce.dev) | The agent gets a Shared Payment Token, scoped to one store and one amount, and the store charges it through Stripe. | No. The store has to support ACP and the token is issued by Stripe for a platform it works with. |
| Visa Intelligent Commerce, Mastercard Agent Pay | Card networks issue agent-specific tokens bound to spend rules. | No. They are issued through partner programs, not to individuals. |
| Virtual cards: [Privacy.com](https://privacy.com), [Lithic](https://lithic.com) | An API creates a card number with a spending limit, locked to the first store that charges it, and closes it afterwards. | Yes. Privacy.com gives every user a personal API key. |
| Your own cards, Google Pay, Apple Pay | The card in your vault, or the wallet on your phone, used at the store's own checkout. | Yes. |

So Reins does what works without a partnership with the store: it decides on the phone, hands over a payment method
only for the one cart you approved, prefers capped virtual cards over real card numbers, and lets you pay on the
phone yourself when you would rather not hand over anything. It attaches an AP2-style signed mandate to every
approval so that the agent, the store or you can later prove exactly what was approved.

## Payment methods

Choose which of these an agent may use (Integrations → Payments):

- **Virtual card (Privacy.com).** Paste your Privacy.com API key; it stays on the phone, encrypted like your other
  tokens. For each approved purchase the phone creates a new card, locked to the first store that charges it, with a
  spending limit of the approved total plus a small tolerance you set (default 10%, at least 1.00, for shipping or
  tax that changes at checkout). The agent gets that card's number. It cannot be charged more than the limit, and
  after its first charge it works only at that store. The phone closes it when the agent reports that the purchase
  failed or was cancelled, when you close it from Spending, or 30 days after the purchase (later charges for split
  shipments still go through until then). In Payments you can choose single-use cards instead, which close after the
  first charge. This is the safest way to let an agent pay, and the only card that a spend limit can approve on its
  own.
- **A card from your vault.** Cards you saved in the vault (type Card), switched on one by one. When you approve a
  purchase, the agent gets the card's number, expiry, security code and holder name, for that purchase. A real card
  number cannot be taken back once handed over, so a card from the vault is always asked for; no spend limit and no
  Autopilot mode approves it.
- **Saved at the store.** You are signed in at the store and it has your card (Amazon, for example). The agent gets
  your approval and the mandate, nothing else, and places the order with what the store has on file. Nothing caps
  what it then checks out, so a spend limit never approves this either.
- **Pay on this phone.** The agent prepares the cart and gives its checkout page. When you approve, the phone opens
  that page in the browser and you pay there yourself, with Google Pay, Apple Pay or anything else the store offers.
  The agent is told the purchase was handed to you, and gets nothing to pay with. The page must be on the store's own
  site (the domain the approval shows) or a well-known hosted payment page (Stripe Checkout, Shopify, PayPal, Square,
  Amazon Pay), which the approval names.

## Shipping addresses

Addresses come from the identities in your vault (type Identity) that have an address. Listing them shows the agent
the label, the city, the region and the country, never the street. The full address is handed over with an approved
purchase, for that purchase. Purchases that are not shipped (a download, a booking) use `ship_to: "none"`.

## What the agent can call

| Tool | What it does | Asks you? |
|---|---|---|
| `payments_methods_list` | The payment methods you switched on: an id, the kind, a nickname, the card brand, the last four digits and the expiry. Never a full number. | Like any list: you tick what to show, or a standing permission covers it. |
| `payments_addresses_list` | Your shipping addresses, masked as above. | The same. |
| `payments_purchase_request` | The store's name and page, the items (name, quantity, unit price), shipping, tax, discount, the total and its currency, the address and the payment method (both optional: left out, you pick them on the phone), and a note. | Always, unless a spend limit you set covers it. |
| `payments_purchase_complete` | After checkout: the order number, the amount charged and the receipt link, or that it failed. Recorded in the ledger; a virtual card is closed. | No. It only records what happened. |

The phone checks a purchase request before you see it: the total must be exactly the items plus shipping and tax
minus the discount, amounts are in the currency's own decimals, at most 50 lines, every page address is `https`, and
the checkout page is on the store's domain, one of its subdomains, or a hosted payment page.

### What an approved purchase returns

```json
{
  "status": "approved",
  "purchase_id": "8c0e7f1a-...",
  "approved_by": "you",
  "merchant": {"name": "Amazon", "domain": "amazon.com"},
  "total": "34.97",
  "currency": "USD",
  "payment": {"kind": "virtual_card", "provider": "privacy", "number": "4111...", "exp_month": "10",
              "exp_year": "2031", "code": "123", "holder": "Ada Lovelace", "last4": "1111", "limit": "38.47"},
  "ship_to": {"name": "Ada Lovelace", "line1": "...", "city": "London", "postal_code": "...", "country": "GB"},
  "mandate": "eyJhbGciOiJFZERTQSIs...",
  "expires_at": "2026-10-09T13:00:00Z",
  "next": "Check out now, then call payments_purchase_complete with the order number and the amount charged."
}
```

`payment` is one of `virtual_card`, `card`, `merchant_account` (nothing to pay with: use what the store has on file)
or `pay_on_phone` (`"status": "handed_off"`: do not place the order; the user pays on their phone).

## The approval on the phone

A purchase gets its own screen, laid out like a receipt: the store and the domain it is on, each item with its
quantity and price, shipping, tax, discount and the total, then the address, the payment method and which AI asked,
with its note. You can change the address and the payment method there. Approving needs the phone's screen lock or
biometrics, as every approval does. Warnings are shown above the total: the store's name does not match its domain,
the total is over a budget, the agent has not bought here before.

From the approval you can also create a spend limit for purchases like this one (below).

## Spend limits

A spend limit lets one AI buy without asking you, within amounts you choose: for example "Claude may spend up to 25.00
a day at amazon.com, with the Privacy.com card, for 7 days". It always pays with a virtual card, the only method whose
cap the card itself enforces. A limit names:

- the AI connection it is for (one connection, never every AI);
- the stores it covers (domains; a subdomain counts as its domain), or every store;
- the most per purchase, and the most per day, week or month (the last 24 hours, 7 days or 30 days);
- the currency, and when it ends (at most 90 days).

A purchase is approved on its own only when a limit covers all of it: the AI, the store, the virtual card, the
currency, the per-purchase amount, and what this AI already spent at those stores in the period plus this purchase.
The request must name the address (or say nothing is shipped) and the virtual card. Anything else, or anything above a
limit, asks you. Lockdown stops limits too.

What counts as spent is the approved total, or what the agent reports was charged when that is more. A report never
lowers it: a purchase the agent calls failed stops counting only when Privacy.com says its card was never charged.

The store a limit names is the one the agent says it buys from. A virtual card locks to whichever store charges it
first, so before a limit approves anything the phone reads who charged that AI's earlier cards. A charge by a name
that does not look like the approved store's domain ("CHEAP WATCHES LTD" on a card made for amazon.com; Amazon's own
charges read "AMZN") closes the card at once, shows on the next approval and in Spending, and stops that AI's limits
until you have looked at it. When the charges cannot be read, limits approve nothing. The amounts are what the card
itself enforces.

## Budgets

Budgets refuse a purchase before it reaches you. In Payments → Budgets:

- **Most per purchase**: a request above it is refused, and the agent is told why.
- **Most in 30 days** (and in 24 hours), for every AI together and for each AI: what you approved plus the request.
- **Only these stores**: when set, a request from any other domain is refused.

They count purchases in their own currency only. A purchase in another currency is not refused by the amounts; its
approval says that your budget does not count it.

## Spending and the ledger

**Payments → Spending** lists every approved purchase: when, which AI, the store, the total, the method (masked),
the address label, what the agent reported afterwards (the order number, the amount charged and the receipt link), and
for virtual cards who actually charged them. It shows what was spent this month, overall and per AI, and lets you
close a virtual card that is still open.
Each purchase is also in **Activity**, like every other request.

The ledger stays on the phone, in its encrypted store, and travels with the account to your other phones like the
rest of the account state. It never holds a card number or a security code: a virtual card is kept as the
provider's card id and its last four digits only.

## Autopilot and standing permissions

Purchases are part of the [hard floor](autopilot.md#the-hard-floor): Autopilot never approves one in any mode
(Bypass included), one-tap approvals and "Approve all" skip them, and a standing permission never covers one. The only
approvals that do not come from you are those of a spend limit you set. Listing payment methods and addresses is an
ordinary list.

## The signed mandate

Every approved purchase carries a mandate: a compact JWS (RFC 7515, `EdDSA` with Ed25519) signed by a key the phone
makes once for your account. The header holds the public key (`jwk`), so anyone can check the signature; the key's
thumbprint is shown in Payments so you can recognize it. The payload is the cart as approved:

```json
{
  "typ": "reins-cart-mandate/1",
  "purchase_id": "8c0e7f1a-...",
  "merchant": {"name": "Amazon", "domain": "amazon.com", "url": "https://www.amazon.com/..."},
  "items": [{"name": "USB-C cable", "quantity": 2, "unit_price": "9.99"}],
  "amounts": {"subtotal": "19.98", "shipping": "4.99", "tax": "0.00", "discount": "0.00", "total": "24.97"},
  "currency": "USD",
  "ship_to": {"label": "Home", "country": "GB"},
  "payment": {"kind": "virtual_card", "brand": "Visa", "last4": "1111"},
  "agent": "Claude",
  "approved_by": "you",
  "iat": 1791547200,
  "exp": 1791550800
}
```

It follows the idea of AP2's cart mandate (the exact cart, signed when approved) without its credential format,
which needs a wallet the store trusts. It proves what you approved; it does not move money.

## What leaves the phone, and when

| What | When | To whom |
|---|---|---|
| Masked methods (kind, nickname, brand, last four, expiry) | A list you allowed | The AI, through the server |
| Masked addresses (label, city, region, country) | A list you allowed | The AI, through the server |
| The full address | An approved purchase that ships | The AI, through the server |
| A virtual card's number, expiry and code | An approved purchase paid with it | The AI, through the server; Privacy.com made it |
| A vault card's number, expiry, code and holder | An approved purchase paid with it | The AI, through the server, or sealed to the desktop app (below) |
| Your Privacy.com API key | Creating and closing cards | Privacy.com only |
| The mandate | Every approved purchase | The AI, through the server |

The server relays answers in memory only and forgets them after 10 minutes at most; it never writes them to disk
or logs them. When the purchase comes through the Reins desktop app's MCP bridge (`reins mcp`), the bridge sends its
key and the phone seals the card details to it, so the server cannot read them; the bridge opens them for the local
agent. Card numbers and codes are never written to the phone's activity log, the ledger or any log.

## Limits of this design

- Without a store that verifies mandates, a store cannot tell an agent's order from yours. The protection is on your
  side: you approve each cart, and a virtual card cannot be charged more than you approved.
- The store is the one the agent names. A virtual card enforces the amount, not the store: the phone checks the
  charges afterwards (above), but cannot stop the first one.
- Spend limits act without you, on the requests the Reins server relays. A server that forged requests could spend
  within your limits, as it could use any standing permission you gave; it still cannot exceed them.
- A card number from the vault, once handed over, can be used again by whoever has it. Prefer virtual cards.
- With "saved at the store", the store's own account settings decide what the agent can buy once it is signed in;
  Reins only records and signs what it asked for.
- Lithic and other virtual card providers can be added behind the same interface; Privacy.com is the first.
