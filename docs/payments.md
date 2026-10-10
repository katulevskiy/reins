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
approval, so that you can later prove exactly what you approved.

## Payment methods

Choose which of these an agent may use (Integrations → Payments):

- **Virtual card (Privacy.com).** Paste your Privacy.com API key. It is kept encrypted on this phone only: it is not
  copied to your other phones with the rest of the account (connect it there too if you want to use it there). For
  each approved purchase the phone creates a new card, locked to the first store that charges it, with a spending
  limit of the approved total plus a small tolerance you set (default 10%, at least 1.00, for shipping or tax that
  changes at checkout). The agent gets that card's number. It cannot be charged more than that cap, and after its
  first charge it works only at that store. The phone closes it when the agent reports that the purchase failed or
  was cancelled, when you close it from Spending, or 30 days after the purchase (later charges for split shipments
  still go through until then). In Payments you can choose single-use cards instead, which close after the first
  charge. This is the safest way to let an agent pay, and the only one a spend limit can approve on its own.
- **A card from your vault.** Cards you saved in the vault (type Card), switched on one by one. When you approve a
  purchase, the agent gets the card's number, expiry, security code and holder name, for that purchase. A real card
  number cannot be taken back once handed over, so a card from the vault is always asked for; no spend limit and no
  Autopilot mode approves it. While a Reins desktop app is paired with your phone, a card from the vault goes only to
  that app, sealed ([below](#what-leaves-the-phone-and-when)); without one, it goes to the agent through the server.
- **Saved at the store.** You are signed in at the store and it has your card (Amazon, for example). The agent gets
  your approval and the mandate, nothing else, and places the order with what the store has on file. Nothing caps
  what it then checks out, so a spend limit never approves this either.
- **Pay on this phone.** The agent prepares the cart and gives its checkout page. When you approve, the phone opens
  that page in the browser and you pay there yourself, with Google Pay, Apple Pay or anything else the store offers.
  The agent is told the purchase was handed to you, and gets nothing to pay with. The page must be on the store's own
  site: its registrable domain, by the [public suffix list](https://publicsuffix.org) (`pay.amazon.com` for
  `www.amazon.com`, never `checkout.stripe.com` or `paypal.com`). Stores whose checkout is elsewhere need another
  method.

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
| `payments_purchase_complete` | After checkout: the order number, the amount charged and the receipt page (on the store's own site), or that it failed. Recorded in the ledger; a virtual card is closed when it failed. | No. It only records what happened, and never lowers what a purchase counts for. |

The server checks a purchase request before relaying it, and the phone again: the total must be exactly the items
plus shipping and tax minus the discount, amounts are in the currency's own decimals, at most 50 lines, every page is
`https`, and no text the user reads (the store's name, the items, the note, and the order number and note of a
report) may hold control or format characters: none of Unicode's format category (direction marks, zero-width
characters, tags, the soft hyphen), and no variation selectors, Hangul fillers or combining grapheme joiner. The phone
also reads every page as a browser reads it, and refuses a checkout or an item page that is not on the store's own
site.

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

`purchase_id` is made by the phone. `payment` is one of `virtual_card`, `card`, `merchant_account` (nothing to pay
with: use what the store has on file) or `pay_on_phone` (`"status": "handed_off"`: do not place the order; the user
pays on their phone). For the desktop app, the answer also carries `signed`, the whole answer signed by the phone
with the request's nonce, and card details arrive sealed; its bridge checks and opens them
([below](#what-leaves-the-phone-and-when)).

## The approval on the phone

A purchase gets its own screen, laid out like a receipt: the store and the domain it is on, each item with its
quantity and price, shipping, tax, discount and the total, then the address, the payment method and which AI asked,
with its note. You can change the address and the payment method there. Approving needs the phone's screen lock or
biometrics, as every approval does, and happens on that screen only: a plain approval (an older app, a notification
button) is refused rather than paying with defaults. Warnings are shown above the total: the store's name does not
match its registrable domain (`Amazon` on `amazon.com.evil.shop`, a Cyrillic `А` in `Аmazon`), the agent has not
bought there before, an earlier card of this agent was charged by someone else, the purchase is in a currency your
budget does not count, the request carries the name of your paired desktop app but did not come from it. Under the
payment method, a line says where card details go: sealed to your desktop app, by the name it had when you paired it
(the phone's own record, not the server's), or through the Reins server, which can read them.

From the approval you can also create a spend limit for purchases like this one (below).

## Spend limits

A spend limit lets one AI buy without asking you, within amounts you choose: for example "Claude may spend up to 25.00
a day at amazon.com, with the Privacy.com card, for 7 days". It always pays with a virtual card, the only method whose
cap the card itself enforces. A limit names:

- the AI connection it is for (one connection, never every AI);
- the stores it covers (domains; a subdomain counts as its domain), or every store. A public suffix (`co.uk`, or a
  shared host like `github.io` or `myshopify.com`) is refused: it would cover every site under it;
- the most per purchase, and the most per day, week or month (the last 24 hours, 7 days or 30 days);
- the currency, and when it ends (at most 90 days).

A purchase is approved on its own only when a limit covers all of it:

- the request names the virtual card and the address (or that nothing is shipped), and it is fresh: a request older
  than 10 minutes, or relayed again after it was paid for, never is;
- what the new card can be charged (the total plus the tolerance) fits the amount per purchase, and fits what is
  left of the period after everything this AI already spent at those stores;
- this AI had fewer than 3 purchases approved by a limit in the last hour, and fewer than 10 in the last day;
- Lockdown is off, checked again right before paying.

Anything else, or anything above a limit, asks you. Every purchase a limit approves is shown in a notification, like
Autopilot's own decisions.

Before a limit approves anything, the phone reads from Privacy.com what that AI's earlier cards were charged, and by
whom (up to 8 cards at a time). If that cannot be read, or more cards are waiting, the limit approves nothing and the
purchase asks you. A charge whose name does not look like the approved store's domain ("CHEAP WATCHES LTD" on a card
made for amazon.com; Amazon's own read "AMZN") closes the card at once, shows on the next approval and in Spending,
and stops that AI's limits until you have looked at it. The name is a hint only, since the store chooses it; the
amounts are what the card enforces.

## Budgets

Budgets refuse a purchase before it reaches you. In Payments → Budget:

- **Most per purchase**: a request that can cost more (its total, plus a virtual card's tolerance) is refused, and
  the agent is told why.
- **Most in 30 days** (and in 24 hours), for every AI together and for each AI: what is counted already plus what the
  request can cost.
- **Only these stores**: when set, a request from any other domain is refused (a public suffix is refused here
  too).

They count purchases in their own currency only. A purchase in another currency is not refused by the amounts; its
approval says that your budget does not count it.

## What counts as spent

Nothing the agent reports lowers it:

- A virtual card counts its cap (the most it can be charged) while it is open, and once closed until its charges have
  been read after the closing; after that, what Privacy.com says was charged (every page of its charges). A card is
  always closed first and read after (when the agent reports a failure, when you close it, 30 days on, when the
  provider is disconnected), so that a charge made meanwhile still counts.
- Anything else counts the approved total, or a higher amount the agent reports, whatever the agent says happened,
  until you clear it in Spending ("Nothing was charged"): only you know nothing was.

## Spending and the ledger

**Payments → Spending** lists every approved purchase: when, which AI, the store, the total, the method (masked),
the address label, what the agent reported afterwards (the order number, the amount charged and the receipt link),
what it counts for now, and for virtual cards who actually charged them. It shows what was spent this month, overall
and per AI, and lets you close a virtual card that is still open and clear a purchase that went through for nothing.
Each purchase is also in **Activity**, like every other request.

The ledger stays in the phone's encrypted store and travels with the account to your other phones like the rest of
the account state. It never drops a purchase from the last 31 days, one whose card is still open, or a charge you have
not looked at; older settled purchases are kept up to 1,000. It never holds a card number or a security code: a
virtual card is kept as the provider's card id and its last four digits only.

Disconnecting Privacy.com closes its open cards first. If the provider does not answer, the key stays until it does,
so that no card is left open that nothing could close; the spend limits that paid with its cards go with the key.
Turning Payments off removes every spend limit with a virtual card and closes the open cards; the key goes once
they are all closed, and stays until then so that Spending can still close them.

## Autopilot and standing permissions

Purchases are part of the [hard floor](autopilot.md#the-hard-floor): Autopilot never approves one in any mode
(Bypass included), one-tap approvals and "Approve all" skip them, and a standing permission never covers one. The only
approvals that do not come from you are those of a spend limit you set. Listing payment methods and addresses is an
ordinary list.

## The signed mandate

Every approved purchase carries a mandate: a compact JWS (RFC 7515, `EdDSA` with Ed25519) signed by a key the phone
makes once for your account. The key is kept with the account's encrypted state, so your phones sign with the same
key; the server holds it only encrypted with your account key. The header holds the public key (`jwk`) and its
thumbprint (`kid`). The key in the header only says which key signed: to rely on a mandate, compare the thumbprint with
the one shown under Integrations → Payments (the desktop app does, once you type it into `reins payments-trust`).
The payload is the cart as approved:

```json
{
  "typ": "reins-cart-mandate/1",
  "purchase_id": "8c0e7f1a-...",
  "merchant": {"name": "Amazon", "domain": "amazon.com", "url": "https://www.amazon.com/..."},
  "items": [{"name": "USB-C cable", "quantity": 2, "unit_price": "9.99"}],
  "amounts": {"subtotal": "19.98", "shipping": "4.99", "tax": "0.00", "discount": "0.00", "total": "24.97"},
  "currency": "USD",
  "ship_to": {"label": "Home", "country": "GB", "address_sha256": "Qm9...", "salt": "x3F..."},
  "payment": {"kind": "virtual_card", "brand": "Visa", "last4": "1111"},
  "agent": "Claude",
  "approved_by": "you",
  "iat": 1791547200,
  "exp": 1791550800
}
```

`address_sha256` binds the whole address without spelling out the street: SHA-256 (base64url) of the salt, a
newline, and the address's lines (name, company, street lines, postal code with city and region, country) joined by
newlines. The mandate follows the idea of AP2's cart mandate (the exact cart, signed when approved) without its
credential format, which needs a wallet the store trusts. It proves what you approved; it does not move money.

## What leaves the phone, and when

| What | When | To whom |
|---|---|---|
| Masked methods (kind, nickname, brand, last four, expiry) | A list you allowed | The AI, through the server |
| Masked addresses (label, city, region, country) | A list you allowed | The AI, through the server |
| The full address | An approved purchase that ships | The AI, through the server |
| A virtual card's number, expiry and code | An approved purchase paid with it | Sealed to the desktop app that asked (below), or the AI through the server; Privacy.com made it |
| A vault card's number, expiry, code and holder | An approved purchase paid with it | Sealed to the desktop app that asked (below); only when no desktop app is paired, the AI through the server |
| Your Privacy.com API key | Creating, reading and closing cards | Privacy.com only; never to the server or your other phones |
| The mandate | Every approved purchase | The AI, through the server |

The server relays answers in memory only and forgets them after 10 minutes at most; it never writes them to disk or
logs them. For an AI connected directly (Claude.ai, ChatGPT) it does see card details as they pass, like any answer.

The Reins desktop app's MCP bridge (`reins mcp`) keeps them from the server. Its connection carries the app's key,
which you pinned on the phone when you paired it:

- the bridge adds that key and a fresh nonce to every purchase request (one alone or in a batch); without its key it
  sends no purchase;
- which connection a request comes from is the server's to say, so the phone decides from what it knows itself. A
  request with the pinned key, on its desktop app's connection, gets card details sealed to that key and signed with
  the mandate key, and the whole answer signed with the request's nonce. The desktop app's connection without its key
  gets no card details (paying at the store, or on the phone, still works), so a server that strips the key gets
  nothing. While a desktop app is paired, a request from any other connection never gets a card from the vault: the
  server could have moved the app's purchase there, under the app's name; a virtual card, capped and locked to the
  store, still goes to it through the server, as its purchase screen says. A key other than the pinned one, or a key
  on a connection without a desktop app, is refused, and so is a request whose desktop app was unpaired or paired
  again while it waited;
- the bridge passes an approval to the agent only when the phone signed the whole answer, with a nonce the bridge
  issued for that purchase (each nonce once), by the key you confirmed, and the mandate in it is signed by the same key
  for the same purchase. This holds for every way of paying: an approval the server makes up ("use the card saved at
  the store") is withheld. The agent gets what the phone signed and nothing the server added; card details are opened
  from the sealed part only, and card details that come back in the clear are withheld;
- any other answer to a purchase (a denial, still waiting, an error) is the server's word: it is passed on labelled as
  such, and withheld when it holds something like a card number or talks about trusting a payment key. Answers
  fetched later with `reins_get_result` are checked the same way.

The bridge opens purchases only once you have confirmed the phone's key on that computer: until then they are
withheld, and the agent is told to have you run `reins payments-trust`. Run it yourself in a terminal and type the
key shown under Integrations → Payments → Payment key on the phone (at least its first 16 characters). Read it from
the phone's screen only: never type a key that came in a message, an email, a web page or an AI's answer. It needs a
terminal, the bridge never prints the key it was offered, and harness hooks send the command to your phone when an AI
tool tries it. From then on, answers signed by any other key are withheld. Card numbers and codes are never written to
the phone's activity log, the ledger or any log.

## Limits of this design

- Without a store that verifies mandates, a store cannot tell an agent's order from yours. The protection is on your
  side: you approve each cart, and a virtual card cannot be charged more than its cap.
- The store is the one the agent names. A virtual card enforces the amount, not the store: the phone checks the
  charges afterwards (above), but cannot stop the first one.
- A card number from the vault, once handed over, can be used again by whoever has it. Prefer virtual cards.
- With "saved at the store", the store's own account decides what the agent can buy once it is signed in; Reins
  only records and signs what it asked for.
- Spend limits act without you, on the requests the Reins server relays. A server that forged requests could spend
  within your limits (at most 3 purchases an hour per AI, within their amounts), as it could use any standing
  permission you gave; it cannot exceed them.
- The desktop bridge relies on you typing the phone's key in once (`reins payments-trust`); a key typed from anywhere
  but the phone's screen could be anyone's. A trusted key of the server's own would let it make up approvals and cards
  for the agent; it would still not get your cards.
- For an AI connected directly (Claude.ai, ChatGPT), card details pass through the server, and the connection's name
  is the server's to say. Without a desktop app paired, that includes cards from the vault: pair the desktop app, or
  use virtual cards, to keep them from the server.
- Sealing keeps card details from a dishonest server, for an honest agent on your computer. It does not protect them
  from a program that runs as you and ignores the harness hooks: such a program can read the desktop app's own key,
  and trust a key of its own. The hooks ask your phone about `reins payments-trust` and about `*.key` files, but a
  program gets around them by driving a terminal of its own (`script`, a pseudo-terminal from Python) or by writing
  the trusted key's file itself. As everywhere in Reins, [hooks are guard rails, not a sandbox](security-model.md).
- The bridge checks answers to purchases. What other tools return (an email, a web page) can say anything, including
  that a purchase was approved.
- Lithic and other virtual card providers can be added behind the same interface; Privacy.com is the first.
