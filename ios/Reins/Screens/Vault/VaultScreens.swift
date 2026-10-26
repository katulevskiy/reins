import SwiftUI
import UIKit

/// What the user picks under "Add": the kinds Reins uses, each with the item kind it makes (the Android app's
/// `NewItem`).
enum NewVaultItem: String, CaseIterable, Hashable, Identifiable {
    case apiKey, sshKey, login, note, card, identity

    var id: String { rawValue }

    var title: String {
        switch self {
        case .apiKey: "API key"
        case .sshKey: "SSH key"
        case .login: "Login"
        case .note: "Secure note"
        case .card: "Card"
        case .identity: "Identity"
        }
    }


    var kind: VaultItemKind {
        switch self {
        case .apiKey, .login: .login
        case .sshKey: .sshKey
        case .note: .note
        case .card: .card
        case .identity: .identity
        }
    }


    /// The form's title: "New API key", "New login".
    var newTitle: String {
        switch self {
        case .apiKey, .sshKey: "New \(title)"
        default: "New \(title.lowercased())"
        }
    }
}

enum VaultText {
    static func kindLabel(_ kind: VaultItemKind) -> String {
        switch kind {
        case .login: "Login"
        case .note: "Secure note"
        case .card: "Card"
        case .identity: "Identity"
        case .sshKey: "SSH key"
        }
    }


    /// The `vault:NAME/...` a new item of this kind gets, for the hint under its name.
    static func referenceHint(_ kind: VaultItemKind, name: String) -> String? {
        let shown = name.trimmingCharacters(in: .whitespaces).isEmpty ? "NAME" : name.trimmingCharacters(in: .whitespaces)
        switch kind {
        case .login: return "Use it as vault:\(shown)/password"
        case .note: return "Use it as vault:\(shown)/notes"
        case .sshKey: return "The desktop app's SSH agent offers it"
        default: return nil
        }
    }

    /// "Delete this key" for the keys, "Delete this item" for the rest.
    static func deleteTitle(_ kind: VaultItemKind) -> String {
        kind == .sshKey ? "Delete this key" : "Delete this item"
    }

    /// The characters a generated password draws from: letters, digits and a few symbols every site takes.
    static let passwordAlphabet = Array("ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789-_!@#%*")

    /// A new password ("generate, don't type"): `length` characters drawn from the system's secure random source.
    static func generatedPassword(length: Int = 24) -> String {
        var rng = SystemRandomNumberGenerator()
        return generatedPassword(length: length, using: &rng)
    }

    static func generatedPassword<G: RandomNumberGenerator>(length: Int, using rng: inout G) -> String {
        String((0..<length).map { _ in passwordAlphabet.randomElement(using: &rng)! })
    }

    static let footer =
        "reins run and the API proxy find an item by its exact name: vault:NAME/password, /username, /notes or a custom field's name. The SSH agent offers every SSH key. From a computer, reins vault add NAME saves a secret here without showing it to an AI."
}

/// One field of the form: its key in the core, its label, and how it is typed (the Android app's `FormField`).
struct VaultFormField: Hashable {
    var key: String
    var label: String
    var secret = false
    var multiline = false
    var keyboard: UIKeyboardType = .default

    /// The fields the form offers for a kind; an API key is a login with only its key (in the password) and a website.
    static func fields(_ kind: VaultItemKind, apiKey: Bool) -> [VaultFormField] {
        switch kind {
        case .login:
            if apiKey {
                return [VaultFormField(key: "password", label: "API key", secret: true), VaultFormField(key: "uris", label: "Website (optional)", keyboard: .URL)]
            }
            return [
                VaultFormField(key: "username", label: "Username", keyboard: .emailAddress),
                VaultFormField(key: "password", label: "Password", secret: true),
                VaultFormField(key: "uris", label: "Website", keyboard: .URL),
                VaultFormField(key: "totp", label: "One-time code key (optional)", secret: true),
            ]
        case .note:
            return [VaultFormField(key: "notes", label: "Note", secret: true, multiline: true)]
        case .card:
            return [
                VaultFormField(key: "holder", label: "Name on the card"),
                VaultFormField(key: "number", label: "Number", secret: true, keyboard: .numberPad),
                VaultFormField(key: "exp_month", label: "Expiry month (1-12)", keyboard: .numberPad),
                VaultFormField(key: "exp_year", label: "Expiry year (2030)", keyboard: .numberPad),
                VaultFormField(key: "code", label: "Security code", secret: true, keyboard: .numberPad),
                VaultFormField(key: "brand", label: "Brand (optional)"),
            ]
        case .identity:
            return [
                VaultFormField(key: "first_name", label: "First name"),
                VaultFormField(key: "last_name", label: "Last name"),
                VaultFormField(key: "email", label: "Email", keyboard: .emailAddress),
                VaultFormField(key: "phone", label: "Phone", keyboard: .phonePad),
                VaultFormField(key: "address1", label: "Address"),
                VaultFormField(key: "city", label: "City"),
                VaultFormField(key: "state", label: "State or region"),
                VaultFormField(key: "postal_code", label: "Postal code"),
                VaultFormField(key: "country", label: "Country"),
                VaultFormField(key: "company", label: "Company (optional)"),
            ]
        case .sshKey:
            return []
        }
    }

    /// The fields to send: everything filled in for a new item; for an edit, the plain fields that changed and the
    /// secret fields typed again (an empty secret field means "unchanged").
    static func changed(_ fields: [VaultFormField], values: [String: String], original: [String: String]?) -> [VaultFieldInput] {
        fields.compactMap { f in
            let value = values[f.key] ?? ""
            let send: String?
            if original == nil {
                send = value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? nil : value
            } else if f.secret {
                send = value.isEmpty ? nil : value
            } else {
                send = value != (original?[f.key] ?? "") ? value : nil
            }
            return send.map { VaultFieldInput(key: f.key, value: $0) }
        }
    }
}

/// The vault on this phone (the Android app's `VaultViewModel`): the list and its search, one item (its secrets only
/// after Face ID or the passcode), and adding, changing and deleting items. Every change is encrypted on the phone by
/// the core before it reaches the server.
@Observable
@MainActor
final class VaultModel {
    /// Nil until first read.
    private(set) var items: [VaultItemSummary]?
    var query = ""
    private(set) var item: VaultItemDetail?
    /// Secret values shown on the item screen, by field key; forgotten when the item closes.
    private(set) var revealed: [String: String] = [:]
    /// An SSH key just made on this phone, to show its public half once.
    private(set) var madeKey: VaultSshKey?
    private(set) var phoneKey: String?
    private(set) var busy = false
    var error: String?

    func load(_ model: AppModel) async {
        do {
            items = try await model.core.vaultItems(query: query.trimmingCharacters(in: .whitespaces))
            error = nil
        } catch {
            self.error = error.userMessage
        }
        if phoneKey == nil { phoneKey = try? await model.core.phoneKeyFingerprint() }
    }

    func open(_ id: String, _ model: AppModel) async {
        if item?.id != id {
            item = nil
            revealed = [:]
        }
        do {
            item = try await model.core.vaultItem(id: id)
            error = nil
        } catch {
            self.error = error.userMessage
        }
    }

    func close() {
        revealed = [:]
        madeKey = nil
    }

    /// Another account (or none): nothing of this one stays.
    func reset() {
        items = nil
        query = ""
        item = nil
        revealed = [:]
        madeKey = nil
        phoneKey = nil
        error = nil
    }

    /// A secret field's value after Face ID or the passcode: shown, or copied. A value already shown is copied
    /// without asking again.
    func reveal(_ field: VaultField, copy: Bool, _ model: AppModel) async {
        guard let item else { return }
        if let shown = revealed[field.key] {
            if copy { Self.copySecret(shown) }
            return
        }
        let verb = copy ? "Copy" : "Show"
        guard await model.authenticator.confirm("\(verb) the \(field.label.lowercased()) of \(untrusted(item.name))") else { return }
        do {
            let value = try await model.core.vaultReveal(id: item.id, key: field.key)
            if copy { Self.copySecret(value) } else { revealed[field.key] = value }
        } catch {
            model.feedback.play(.error)
            self.error = error.userMessage
        }
    }

    func hide(_ key: String) {
        revealed[key] = nil
    }

    /// Creates the item (`id` nil) or changes it, after Face ID or the passcode; the item's id.
    func save(id: String?, input: VaultItemInput, _ model: AppModel) async -> String? {
        await run(model, reason: id == nil ? "Save \(untrusted(input.name)) in your vault" : "Change \(untrusted(input.name))") {
            if let id {
                try await model.core.vaultUpdate(id: id, input: input)
                return id
            }
            return try await model.core.vaultCreate(input: input)
        }
    }

    func generateSshKey(name: String, _ model: AppModel) async -> String? {
        await run(model, reason: "Make the SSH key \(untrusted(name))") {
            let key = try await model.core.vaultGenerateSshKey(name: name)
            self.madeKey = key
            return key.id
        }
    }

    func delete(_ id: String, _ model: AppModel) async -> Bool {
        let done = await run(model, reason: "Delete \(untrusted(item?.name ?? "the item")) for good") {
            try await model.core.vaultDelete(id: id)
            self.item = nil
            return id
        }
        return done != nil
    }

    /// A change to the vault: only after Face ID or the passcode, like approving one from the computer.
    private func run(_ model: AppModel, reason: String, _ block: () async throws -> String) async -> String? {
        guard !busy else { return nil }
        busy = true
        error = nil
        defer { busy = false }
        guard await model.authenticator.confirm(reason) else {
            if !ScreenLock.isSet() { error = "Set a passcode on this iPhone to change your vault." }
            return nil
        }
        do {
            let id = try await block()
            model.feedback.play(.grantCreated)
            await load(model)
            return id
        } catch {
            model.feedback.play(.error)
            self.error = error.userMessage
            return nil
        }
    }

    /// A secret: kept on this phone only, and gone from the clipboard after a minute.
    static func copySecret(_ value: String) {
        UIPasteboard.general.setItems([[UIPasteboard.typeAutomatic: value]], options: [.localOnly: true, .expirationDate: Date().addingTimeInterval(60)])
    }
}

/// The vault (the Vault board, 01): a standard page under Connections > Password vault. A search, the items (each by
/// its kind's glyph, its name, its short detail, a star for a favourite), "Add" on the items' own header, and how a
/// computer refers to them: the `vault:NAME/password` form and this phone's key for `reins vault add`.
struct VaultScreen: View {
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback

    var body: some View {
        @Bindable var vm = model.vault
        NextPage {
            StandardTitle("Vault").padding(.top, -2)
            HStack(spacing: 10) {
                GlyphIcon(.search, size: 18).foregroundStyle(Palette.inkMeta)
                TextField("Search", text: $vm.query)
                    .font(RFont.sans(16))
                    .foregroundStyle(Palette.ink)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .submitLabel(.search)
                    .accessibilityLabel("Search names, usernames, websites")
                    .accessibilityIdentifier("vaultSearch")
            }
            .padding(.horizontal, 12)
            .frame(minHeight: 44)
            .background(Palette.paperSunken, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
            .padding(.horizontal, 16)
            .padding(.top, 14)
            PlusSectionHeader(title: "Items", glyph: .vault, count: vm.items?.count, addLabel: "Add an item", addId: "vaultAdd") {
                model.push(.vaultAdd)
            }
            RaisedGroup {
                switch vm.items {
                case nil:
                    VaultNote(text: "Loading…")
                case let list? where list.isEmpty && vm.query.isEmpty:
                    VaultNote(text: "Nothing in the vault yet").accessibilityIdentifier("vaultEmpty")
                case let list? where list.isEmpty:
                    VaultNote(text: "Nothing matches").accessibilityIdentifier("vaultNoMatch")
                case let list?:
                    ForEach(list, id: \.id) { item in
                        Button {
                            model.push(.vaultItem(item.id))
                            feedback.defaultTap()
                        } label: {
                            let detail = untrustedName(item.subtitle, limit: 32)
                            TileRow(untrustedName(item.name, limit: 40), glyph: VaultText.glyph(item.kind), value: detail.isEmpty ? nil : detail, mono: item.kind == .sshKey) {
                                if item.favorite {
                                    // The glyph set has no filled star.
                                    Image(systemName: "star.fill").font(.system(size: 13)).foregroundStyle(Palette.warning).accessibilityLabel("Favourite")
                                }
                            }
                        }
                        .buttonStyle(RowPressStyle())
                        .accessibilityIdentifier("vaultItem:\(item.id)")
                    }
                }
            }
            SectionHeader("From a computer", glyph: .laptop)
            RaisedGroup {
                HStack(spacing: 12) {
                    GlyphTile(.terminal)
                    Text(verbatim: "vault:NAME/password")
                        .font(RFont.mono(14))
                        .foregroundStyle(Palette.ink)
                        .frame(maxWidth: .infinity, alignment: .leading)
                    QuietIconButton(.copy, label: "Copy vault:NAME/password") { copyPlain("vault:NAME/password", feedback: feedback) }
                }
                .padding(.leading, 14)
                .padding(.trailing, 8)
                .frame(minHeight: 56)
                if let key = vm.phoneKey {
                    HStack(spacing: 12) {
                        GlyphTile(.phone)
                        Text("Phone key").textRole(.row).foregroundStyle(Palette.ink)
                        Text(verbatim: key)
                            .font(RFont.mono(13))
                            .foregroundStyle(Palette.ink2)
                            .lineLimit(1)
                            .truncationMode(.middle)
                            .frame(maxWidth: .infinity, alignment: .trailing)
                        QuietIconButton(.copy, label: "Copy this phone's key") { copyPlain(key, feedback: feedback) }
                    }
                    .padding(.leading, 14)
                    .padding(.trailing, 8)
                    .frame(minHeight: 56)
                    .accessibilityIdentifier("phoneKey")
                }
            }
            GroupFooter(VaultText.footer + " Add a secret with reins vault add NAME and this phone's key.")
                .padding(.horizontal, 20)
            if let error = vm.error {
                FormBanner(text: untrusted(error)).padding(.horizontal, 16).padding(.top, 12).accessibilityIdentifier("vaultError")
            }
        }
        .onChange(of: vm.query) { Task { await vm.load(model) } }
        .quietPageBar("Vault")
        .task { await vm.load(model) }
        .refreshable { await vm.load(model) }
    }
}

/// A line in a group with nothing else in it ("Loading…", "Nothing matches").
private struct VaultNote: View {
    var text: String

    var body: some View {
        Text(text)
            .font(RFont.sans(15))
            .foregroundStyle(Palette.ink2)
            .frame(maxWidth: .infinity, minHeight: 56, alignment: .leading)
            .padding(.horizontal, 14)
    }
}

/// Vault > Add (the Vault board, 02): a quiet page, one flat list of kinds. A kind opens its form.
struct VaultAddScreen: View {
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback

    var body: some View {
        NextPage {
            MarkedTitle("Add")
            SectionHeader("Kind", quiet: true)
            FlatGroup {
                ForEach(NewVaultItem.allCases) { kind in
                    Button {
                        feedback.play(.tap)
                        model.push(.vaultEdit(id: nil, newItem: kind))
                    } label: {
                        TileRow(kind.title, glyph: kind.glyph).padding(.horizontal, -14)
                    }
                    .buttonStyle(RowPressStyle())
                    .accessibilityHint(kind.tileLine)
                    .accessibilityIdentifier("newItem:\(kind.rawValue)")
                }
            }
            GroupFooter("Everything is encrypted on this phone with your vault's key before it reaches the server.")
                .padding(.horizontal, 20)
        }
        .quietPageBar("Add")
    }
}

extension NewVaultItem {
    /// The kind's glyph in the list.
    var glyph: Glyph {
        switch self {
        case .apiKey: .token
        case .sshKey: .terminal
        case .login: .person
        case .note: .fileLock
        case .card: .card
        case .identity: .idCard
        }
    }

    /// One short line about the kind (VoiceOver's hint).
    var tileLine: String {
        switch self {
        case .apiKey: "For reins run and the API proxy"
        case .sshKey: "Made here, never leaves"
        case .login: "Username and password"
        case .note: "Any text"
        case .card: "For approved purchases"
        case .identity: "Name and address"
        }
    }
}

extension VaultText {
    /// An item's glyph in the list, by its kind.
    static func glyph(_ kind: VaultItemKind) -> Glyph {
        switch kind {
        case .login: .person
        case .note: .fileLock
        case .card: .card
        case .identity: .idCard
        case .sshKey: .terminal
        }
    }

    /// A field's glyph in a form or on an item's page.
    static func glyph(field key: String) -> Glyph {
        switch key {
        case "username", "holder", "first_name", "last_name": .person
        case "password", "code": .vault
        case "uris": .globe
        case "totp": .timer
        case "notes": .fileText
        case "number", "brand": .card
        case "exp_month", "exp_year": .calendar
        case "email": .mail
        case "phone": .phone
        case "address1", "city", "state", "postal_code", "country": .home
        case "company": .briefcase
        case "private_key": .key
        case "public_key": .fileCode
        case "fingerprint": .fingerprint
        default: .pencil
        }
    }

    /// An SSH key's type from its public half ("ssh-ed25519 AAAA…" is "ed25519").
    static func keyType(_ publicKey: String?) -> String {
        guard let first = publicKey?.split(separator: " ").first else { return "SSH key" }
        let type = first.hasPrefix("ssh-") ? String(first.dropFirst(4)) : String(first)
        return type.isEmpty ? "SSH key" : untrustedName(type, limit: 24)
    }
}

/// One vault item (the Vault board, 03): a quiet page. Its focal card (its kind, and for an SSH key its type, that it
/// signs on this phone and its fingerprint), its fields (secrets after Face ID or the passcode; an SSH key's private
/// half never shows), how a computer asks for it, and deleting it, held.
struct VaultItemScreen: View {
    var id: String
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    private var vm: VaultModel { model.vault }
    @State private var copied: String?

    var body: some View {
        NextPage {
            if let item = vm.item, item.id == id {
                content(item)
            } else {
                MarkedTitle("Vault item")
                Text("Loading…").font(RFont.sans(15)).foregroundStyle(Palette.ink2).padding(20)
            }
            if let error = vm.error {
                FormBanner(text: untrusted(error)).padding(.horizontal, 20).padding(.top, 12).accessibilityIdentifier("vaultItemError")
            }
        }
        .quietPageBar(vm.item.map { untrustedName($0.name, limit: 40) } ?? "Vault item")
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                Button { model.push(.vaultEdit(id: id, newItem: nil)) } label: { GlyphIcon(.pencil, size: 20).foregroundStyle(Palette.ink) }
                    .accessibilityLabel("Edit")
                    .accessibilityIdentifier("vaultEdit")
                    .disabled(vm.item?.id != id)
            }
        }
        .privacySensitive()
        .task(id: id) { await vm.open(id, model) }
        .onDisappear { vm.close() }
    }

    @ViewBuilder
    private func content(_ item: VaultItemDetail) -> some View {
        let ssh = item.kind == .sshKey
        let publicKey = item.fields.first { $0.key == "public_key" }?.value
        let fingerprint = item.fields.first { $0.key == "fingerprint" }?.value
        MarkedTitle(untrustedName(item.name, limit: 40), titleId: "vaultName")
        FocalCard(glyph: VaultText.glyph(item.kind), title: ssh ? VaultText.keyType(publicKey) : VaultText.kindLabel(item.kind), sub: ssh ? "SSH key" : "In your vault") {
            if ssh {
                FocalFact(glyph: .phone, value: "On this phone")
                if let fingerprint {
                    FocalFact(glyph: .fingerprint, value: fingerprint, mono: true) {
                        QuietIconButton(.copy, label: "Copy fingerprint") { copy(fingerprint, label: "fingerprint") }
                            .accessibilityIdentifier("vaultCopy:fingerprint")
                    }
                }
            } else {
                FocalFact(glyph: .lock, value: "Encrypted on this phone")
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("vaultKind")
        if let warning = item.warning {
            FormBanner(text: untrusted(warning)).padding(.horizontal, 20).padding(.top, 12).accessibilityIdentifier("vaultWarning")
        }
        if ssh {
            if let publicKey {
                SectionHeader("Public key", quiet: true)
                FlatGroup(inset: 32) {
                    MachineRow(glyph: .fileCode, value: untrusted(publicKey), valueId: "vaultValue:public_key") {
                        ShareLink(item: publicKey) {
                            GlyphIcon(.share, size: 18).foregroundStyle(Palette.ink2).frame(width: 40, height: 40)
                        }
                        .accessibilityLabel("Share")
                        .accessibilityIdentifier("vaultShare:public_key")
                        QuietIconButton(.copy, label: "Copy the public key") { copy(publicKey, label: "public key") }
                            .accessibilityIdentifier("vaultCopy:public_key")
                    }
                }
            }
        } else {
            SectionHeader("Fields", quiet: true)
            FlatGroup(inset: 32) {
                if item.fields.isEmpty {
                    Text("Nothing is filled in.").font(RFont.sans(15)).foregroundStyle(Palette.ink2).frame(maxWidth: .infinity, minHeight: 54, alignment: .leading)
                }
                ForEach(item.fields, id: \.key) { field in
                    FieldRow(field: field, shown: vm.revealed[field.key]) {
                        if vm.revealed[field.key] != nil { vm.hide(field.key) } else { Task { await vm.reveal(field, copy: false, model) } }
                    } onCopy: {
                        if let plain = field.value {
                            UIPasteboard.general.string = plain
                            feedback.play(.copied)
                        } else {
                            Task { await vm.reveal(field, copy: true, model) }
                        }
                        copied = field.label
                    }
                }
            }
        }
        if !item.uses.isEmpty {
            SectionHeader("For a computer", quiet: true)
            FlatGroup(inset: 32) {
                ForEach(item.uses, id: \.reference) { use in
                    // Copying a reference, not a secret.
                    MachineRow(glyph: .terminal, value: untrusted(use.reference)) {
                        QuietIconButton(.copy, label: "Copy \(untrusted(use.reference))") { copy(use.reference, label: use.reference) }
                    }
                    .accessibilityIdentifier("vaultUse:\(use.reference)")
                }
            }
            GroupFooter(item.uses.map(\.hint).joined(separator: "\n") + "\n\nYour phone asks you each time one is used.")
                .padding(.horizontal, 20)
        }
        if let copied {
            Text("Copied \(untrustedName(copied, limit: 40)).")
                .textRole(.sub)
                .foregroundStyle(Palette.inkMeta)
                .padding(.horizontal, 20)
                .padding(.top, 8)
                .accessibilityIdentifier("vaultCopied")
        }
        QuietDangerRow(
            title: ssh ? "Delete key" : "Delete item",
            note: ssh ? "Hold to delete. AIs using it start asking." : "Hold to delete. What uses it stops working.",
            busy: vm.busy
        ) {
            Task { if await vm.delete(id, model) { model.back() } }
        }
        .accessibilityIdentifier("vaultDelete")
    }

    private func copy(_ text: String, label: String) {
        copyPlain(text, feedback: feedback)
        copied = label
    }
}

/// One field on an item's page: its glyph, its label over its value (or dots), then show and copy.
private struct FieldRow: View {
    var field: VaultField
    var shown: String?
    var onReveal: () -> Void
    var onCopy: () -> Void

    var body: some View {
        // The SSH agent signs on the phone: the private key is neither shown nor copied, so it never leaves the vault.
        let kept = field.key == "private_key"
        let value = field.value ?? shown
        HStack(alignment: .center, spacing: 12) {
            GlyphIcon(VaultText.glyph(field: field.key), size: 18).foregroundStyle(Palette.inkMeta).frame(width: 20)
            VStack(alignment: .leading, spacing: 2) {
                Text(untrustedName(field.label, limit: 40)).textRole(.sub).foregroundStyle(Palette.inkMeta)
                Text(verbatim: kept ? "Kept in the vault. The phone signs with it." : value.map(untrusted) ?? "••••••••••")
                    .font(!kept && (field.multiline || field.secret) ? RFont.mono(14.5) : RFont.sans(16))
                    .foregroundStyle(kept ? Palette.ink2 : Palette.ink)
                    .lineLimit(field.multiline ? 12 : 3)
                    .accessibilityIdentifier("vaultValue:\(field.key)")
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            if !kept {
                if field.secret {
                    QuietIconButton(.eye, label: shown != nil ? "Hide" : "Show", action: onReveal)
                        .opacity(shown != nil ? 0.5 : 1)
                        .accessibilityIdentifier("vaultReveal:\(field.key)")
                }
                QuietIconButton(.copy, label: "Copy", action: onCopy)
                    .accessibilityIdentifier("vaultCopy:\(field.key)")
            }
        }
        .padding(.vertical, 10)
        .frame(minHeight: 56)
        .accessibilityIdentifier("vaultField:\(field.key)")
    }
}

/// Adding an item (`newItem`) or changing one (`id`): a quiet form (the Vault board, 04 and 05). Its fields as rows,
/// how AIs will ask for it, Save with Face ID at the bottom. An SSH key is made on the phone (the key-to-be is the focal
/// card), or pasted.
struct VaultEditScreen: View {
    var id: String?
    var newItem: NewVaultItem?
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @State private var name = ""
    @State private var values: [String: String] = [:]
    @State private var pasteKey = false
    @State private var started = false

    private var vm: VaultModel { model.vault }
    private var existing: VaultItemDetail? { id.flatMap { id in vm.item?.id == id ? vm.item : nil } }
    private var kind: VaultItemKind { existing?.kind ?? newItem?.kind ?? .login }
    private var fields: [VaultFormField] { VaultFormField.fields(kind, apiKey: newItem == .apiKey) }
    private var original: [String: String]? {
        existing.map { Dictionary(uniqueKeysWithValues: $0.fields.compactMap { f in f.value.map { (f.key, $0) } }) }
    }
    private var sshNew: Bool { kind == .sshKey && existing == nil }
    private var title: String { existing != nil ? "Edit" : newItem?.newTitle ?? "New item" }

    var body: some View {
        NextPage(bottom: 24) {
            MarkedTitle(title)
            if sshNew {
                FocalCard(glyph: .terminal, title: "ed25519", sub: "SSH key", dashed: true) {
                    FocalFact(glyph: .phone, value: "On this phone")
                    HStack(spacing: 12) {
                        GlyphIcon(.fingerprint, size: 18).foregroundStyle(Palette.inkMeta).frame(width: 20)
                        Capsule().fill(Palette.paperSunken).frame(width: 168, height: 10)
                        Capsule().fill(Palette.paperSunken).frame(width: 52, height: 10)
                        Spacer(minLength: 0)
                    }
                    .frame(minHeight: 48)
                    .accessibilityElement()
                    .accessibilityLabel("Fingerprint, made with the key")
                }
            }
            SectionHeader(sshNew ? "Name" : newItem?.title ?? VaultText.kindLabel(kind), quiet: true)
            FlatFields {
                GlyphInputRow(glyph: .pencil) {
                    TextField(newItem == .apiKey ? "OpenAI" : "Name", text: $name)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .accessibilityIdentifier("vaultName")
                }
                ForEach(fields, id: \.key) { f in input(f) }
                if sshNew && pasteKey {
                    input(VaultFormField(key: "private_key", label: "Private key (OpenSSH, no passphrase)", secret: true, multiline: true))
                }
            }
            if let reference = reference {
                SectionHeader("AIs ask for it as", quiet: true)
                FlatGroup(inset: 32) {
                    MachineRow(glyph: .terminal, value: reference, valueId: "vaultHint") {
                        QuietIconButton(.copy, label: "Copy \(reference)") { copyPlain(reference, feedback: feedback) }
                    }
                }
            } else if let hint = VaultText.referenceHint(kind, name: name) {
                Text(hint).textRole(.sub).foregroundStyle(Palette.inkMeta).padding(.horizontal, 20).padding(.top, 10).accessibilityIdentifier("vaultHint")
            }
            if let error = vm.error {
                FormBanner(text: untrusted(error)).padding(.horizontal, 20).padding(.top, 12).accessibilityIdentifier("vaultFormError")
            }
            GroupFooter("Saved encrypted with your vault's key, on this phone, before it reaches the server.")
                .padding(.horizontal, 20)
        }
        .safeAreaInset(edge: .bottom, spacing: 0) { footer }
        .quietPageBar(title)
        .privacySensitive()
        .onAppear {
            guard !started else { return }
            started = true
            vm.error = nil
            name = existing?.name ?? ""
            values = original ?? [:]
        }
    }

    /// `vault:NAME/password` for a login or an API key, `vault:NAME/notes` for a note: the name as typed, or NAME.
    private var reference: String? {
        let shown = name.trimmingCharacters(in: .whitespaces).isEmpty ? "NAME" : name.trimmingCharacters(in: .whitespaces)
        switch kind {
        case .login: return "vault:\(shown)/password"
        case .note: return "vault:\(shown)/notes"
        default: return nil
        }
    }

    @ViewBuilder private var footer: some View {
        VStack(spacing: 8) {
            if sshNew && !pasteKey {
                Button {
                    Task {
                        if let made = await vm.generateSshKey(name: name.trimmingCharacters(in: .whitespaces), model) { saved(made) }
                    }
                } label: {
                    BusyLabel(title: "Make on this phone", glyph: .faceId, busy: vm.busy)
                }
                .buttonStyle(SignalButtonStyle(.inverse))
                .disabled(name.trimmingCharacters(in: .whitespaces).isEmpty || vm.busy)
                .accessibilityIdentifier("vaultGenerate")
                Button { pasteKey = true } label: {
                    Label { Text("Paste a key") } icon: { GlyphIcon(.clipboard, size: 18) }
                }
                .buttonStyle(SignalButtonStyle(.soft))
                .accessibilityIdentifier("vaultPasteKey")
            } else {
                let send = sshNew
                    ? [values["private_key"]].compactMap { $0 }.filter { !$0.isEmpty }.map { VaultFieldInput(key: "private_key", value: $0) }
                    : VaultFormField.changed(fields, values: values, original: original)
                let ready = !name.trimmingCharacters(in: .whitespaces).isEmpty && (existing != nil || !send.isEmpty || kind == .identity)
                Button {
                    Task {
                        let input = VaultItemInput(kind: kind, name: name.trimmingCharacters(in: .whitespaces), fields: send)
                        if let saved = await vm.save(id: existing?.id, input: input, model) { self.saved(saved) }
                    }
                } label: {
                    BusyLabel(title: "Save", glyph: .faceId, busy: vm.busy)
                }
                .buttonStyle(SignalButtonStyle(.inverse))
                .disabled(!ready || vm.busy)
                .accessibilityIdentifier("vaultSave")
            }
        }
        .padding(.horizontal, 16)
        .padding(.top, 10)
        .padding(.bottom, 8)
        .background(Palette.paper.ignoresSafeArea())
    }

    @ViewBuilder
    private func input(_ f: VaultFormField) -> some View {
        let binding = Binding(get: { values[f.key] ?? "" }, set: { values[f.key] = $0 })
        let placeholder = f.secret && existing != nil ? "\(f.label): unchanged" : f.label
        GlyphInputRow(glyph: VaultText.glyph(field: f.key)) {
            Group {
                if f.multiline {
                    TextField(placeholder, text: binding, axis: .vertical).lineLimit(3...12).font(RFont.mono(14.5))
                } else if f.secret {
                    SecureField(placeholder, text: binding).font(RFont.mono(14.5))
                } else {
                    TextField(placeholder, text: binding).keyboardType(f.keyboard)
                }
            }
            .textInputAutocapitalization(.never)
            .autocorrectionDisabled()
            .accessibilityLabel(f.label)
            .accessibilityIdentifier("vaultInput:\(f.key)")
        } trailing: {
            if f.key == "password" && kind == .login && newItem != .apiKey {
                // Generate, don't type: a new 24-character password at a tap.
                QuietIconButton(.refresh, label: "Generate a password") {
                    values[f.key] = VaultText.generatedPassword()
                    feedback.play(.selection)
                }
                .accessibilityIdentifier("vaultGeneratePassword")
            }
        }
    }

    /// An edited item goes back to its page; a new one opens in place of "Add" and the form.
    private func saved(_ savedId: String) {
        model.back()
        if id == nil {
            model.back()
            model.push(.vaultItem(savedId))
        }
    }
}
