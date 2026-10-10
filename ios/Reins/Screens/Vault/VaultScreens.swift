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

    var detail: String {
        switch self {
        case .apiKey: "For reins run and the API proxy: vault:NAME/password"
        case .sshKey: "Made on this phone; the private key never leaves the vault"
        case .login: "A username and password for a website"
        case .note: "Any text: vault:NAME/notes"
        case .card: "A payment card"
        case .identity: "Your name, address and documents"
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

    var symbol: String {
        switch self {
        case .apiKey: "key.horizontal"
        case .sshKey: "lock"
        case .login: "link"
        case .note: "note.text"
        case .card: "creditcard"
        case .identity: "person.text.rectangle"
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

    static func symbol(_ kind: VaultItemKind) -> String {
        switch kind {
        case .login: "key.horizontal"
        case .note: "note.text"
        case .card: "creditcard"
        case .identity: "person.text.rectangle"
        case .sshKey: "lock"
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

/// Integrations > Password vault > Open the vault: every item, a search, and "Add".
struct VaultScreen: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        @Bindable var vm = model.vault
        List {
            Section {
                switch vm.items {
                case nil:
                    InfoRow(title: "Loading…") { EmptyView() }.cardRow()
                case let list? where list.isEmpty && vm.query.isEmpty:
                    InfoRow(
                        title: "Nothing in the vault yet",
                        subtitle: "Add an API key, a login or an SSH key. The desktop app finds them by name, like vault:OpenAI/password.",
                        symbol: "key.horizontal"
                    ) { EmptyView() }
                    .accessibilityIdentifier("vaultEmpty")
                    .cardRow()
                case let list? where list.isEmpty:
                    InfoRow(title: "Nothing matches") { EmptyView() }.accessibilityIdentifier("vaultNoMatch").cardRow()
                case let list?:
                    ForEach(list, id: \.id) { item in
                        Button {
                            model.push(.vaultItem(item.id))
                        } label: {
                            InfoRow(
                                title: untrusted(item.name),
                                subtitle: untrusted(item.subtitle).isEmpty ? VaultText.kindLabel(item.kind) : untrusted(item.subtitle),
                                symbol: VaultText.symbol(item.kind),
                                tint: Palette.accent
                            ) {
                                if item.favorite { Image(systemName: "star.fill").foregroundStyle(Palette.warning) }
                                Chevron()
                            }
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                        .accessibilityIdentifier("vaultItem:\(item.id)")
                        .cardRow()
                    }
                }
            } header: {
                if let n = vm.items?.count, n > 0 { GroupHeader(n == 1 ? "1 item" : "\(n) items") }
            } footer: {
                VStack(alignment: .leading, spacing: 8) {
                    GroupFooter(VaultText.footer)
                    if let key = vm.phoneKey {
                        GroupFooter("This phone's key for reins vault add: \(key)").accessibilityIdentifier("phoneKey")
                    }
                }
            }
            if let error = vm.error {
                Section { FormBanner(text: untrusted(error)).accessibilityIdentifier("vaultError") }
                    .listRowBackground(Color.clear)
            }
        }
        .reinsGrouped()
        .searchable(text: $vm.query, prompt: "Search names, usernames, websites")
        .onChange(of: vm.query) { Task { await vm.load(model) } }
        .navigationTitle("Vault")
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                Button { model.push(.vaultAdd) } label: { Image(systemName: "plus") }
                    .accessibilityLabel("Add")
                    .accessibilityIdentifier("vaultAdd")
            }
        }
        .task { await vm.load(model) }
        .refreshable { await vm.load(model) }
    }
}

/// Vault > Add: what kind of item.
struct VaultAddScreen: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        List {
            Section {
                ForEach(NewVaultItem.allCases) { kind in
                    SettingsLinkRow(title: kind.title, subtitle: kind.detail, symbol: kind.symbol, tint: Palette.accent, id: "newItem:\(kind.rawValue)") {
                        model.push(.vaultEdit(id: nil, newItem: kind))
                    }
                }
            } header: {
                GroupHeader("What to add")
            } footer: {
                GroupFooter("Everything is encrypted on this phone with your vault's key before it reaches the server.")
            }
        }
        .reinsGrouped()
        .navigationTitle("Add to the vault")
    }
}

/// One vault item: how the desktop app refers to it, its fields (secrets after Face ID or the passcode), edit and
/// delete.
struct VaultItemScreen: View {
    var id: String
    @Environment(AppModel.self) private var model
    @State private var deleting = false
    private var vm: VaultModel { model.vault }
    @State private var copied: String?

    var body: some View {
        List {
            if let item = vm.item, item.id == id {
                if vm.madeKey?.id == id {
                    Section {
                        FormBanner(
                            text: "Made on this phone. Put the public key below on the servers and Git hosts you sign in to (GitHub: Settings, SSH and GPG keys). The private key stays in your vault.",
                            kind: .info
                        )
                        .accessibilityIdentifier("sshMade")
                    }
                    .listRowBackground(Color.clear)
                }
                if let warning = item.warning {
                    Section { FormBanner(text: untrusted(warning)).accessibilityIdentifier("vaultWarning") }
                        .listRowBackground(Color.clear)
                }
                if !item.uses.isEmpty {
                    Section {
                        ForEach(item.uses, id: \.reference) { use in
                            Button {
                                guard item.kind != .sshKey else { return }
                                UIPasteboard.general.string = use.reference
                                copied = use.reference
                            } label: {
                                VStack(alignment: .leading, spacing: 2) {
                                    Text(untrusted(use.reference)).font(RFont.mono(14.5)).foregroundStyle(Palette.text)
                                    Text(use.hint).font(RFont.sans(12.5)).foregroundStyle(Palette.secondary)
                                }
                                .frame(maxWidth: .infinity, alignment: .leading)
                                .contentShape(Rectangle())
                            }
                            .buttonStyle(.plain)
                            .accessibilityIdentifier("vaultUse:\(use.reference)")
                            .cardRow()
                        }
                    } header: {
                        GroupHeader("Use it from the computer")
                    } footer: {
                        if item.kind != .sshKey { GroupFooter("Tap to copy. Your phone asks you each time one is used.") }
                    }
                }
                Section {
                    if item.fields.isEmpty {
                        InfoRow(title: "Nothing is filled in.") { EmptyView() }.cardRow()
                    }
                    ForEach(item.fields, id: \.key) { field in
                        FieldRow(field: field, shown: vm.revealed[field.key]) {
                            if vm.revealed[field.key] != nil { vm.hide(field.key) } else { Task { await vm.reveal(field, copy: false, model) } }
                        } onCopy: {
                            if let plain = field.value {
                                UIPasteboard.general.string = plain
                            } else {
                                Task { await vm.reveal(field, copy: true, model) }
                            }
                            copied = field.label
                        }
                        .cardRow()
                    }
                } header: {
                    GroupHeader("Fields")
                } footer: {
                    if let copied { GroupFooter("Copied \(untrusted(copied)).").accessibilityIdentifier("vaultCopied") }
                }
                Section {
                    Button("Delete", role: .destructive) { deleting = true }
                        .buttonStyle(CapsuleButtonStyle(kind: .danger, height: 50))
                        .disabled(vm.busy)
                        .accessibilityIdentifier("vaultDelete")
                }
                .listRowBackground(Color.clear)
                .listRowInsets(EdgeInsets())
            } else {
                InfoRow(title: "Loading…") { EmptyView() }.cardRow()
            }
            if let error = vm.error {
                Section { FormBanner(text: untrusted(error)).accessibilityIdentifier("vaultItemError") }
                    .listRowBackground(Color.clear)
            }
        }
        .reinsGrouped()
        .navigationTitle(vm.item.map { untrusted($0.name) } ?? "Vault item")
        .navigationSubtitle(vm.item.map { VaultText.kindLabel($0.kind) } ?? "")
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                Button { model.push(.vaultEdit(id: id, newItem: nil)) } label: { Image(systemName: "pencil") }
                    .accessibilityLabel("Edit")
                    .accessibilityIdentifier("vaultEdit")
                    .disabled(vm.item?.id != id)
            }
        }
        .privacySensitive()
        .task(id: id) { await vm.open(id, model) }
        .onDisappear { vm.close() }
        .confirmationDialog(
            "Delete \(untrusted(vm.item?.name ?? ""))?",
            isPresented: $deleting,
            titleVisibility: .visible
        ) {
            Button("Delete", role: .destructive) {
                Task { if await vm.delete(id, model) { model.back() } }
            }
        } message: {
            Text("It is deleted from your vault for good, on every device. Anything that uses it from the computer stops working.")
        }
    }
}

/// One field: its label, its value or dots, and show, share (a public key) and copy.
private struct FieldRow: View {
    var field: VaultField
    var shown: String?
    var onReveal: () -> Void
    var onCopy: () -> Void

    var body: some View {
        // The SSH agent signs on the phone: the private key is neither shown nor copied, so it never leaves the vault.
        let kept = field.key == "private_key"
        let value = field.value ?? shown
        HStack(spacing: 6) {
            VStack(alignment: .leading, spacing: 2) {
                Text(untrusted(field.label)).font(RFont.sans(12.5, .medium)).foregroundStyle(Palette.secondary)
                Text(kept ? "Kept in the vault. The phone signs with it." : value.map(untrusted) ?? "••••••••••")
                    .font(!kept && (field.multiline || field.secret) ? RFont.mono(14.5) : RFont.sans(16))
                    .foregroundStyle(Palette.text)
                    .lineLimit(field.multiline ? 12 : 3)
                    .accessibilityIdentifier("vaultValue:\(field.key)")
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            if !kept {
                if field.secret {
                    iconButton(shown != nil ? "eye.slash" : "eye", shown != nil ? "Hide" : "Show", "vaultReveal:\(field.key)", onReveal)
                }
                if field.key == "public_key", let text = field.value {
                    ShareLink(item: text) { Image(systemName: "square.and.arrow.up").foregroundStyle(Palette.accent) }
                        .buttonStyle(.borderless)
                        .accessibilityIdentifier("vaultShare:\(field.key)")
                }
                iconButton("doc.on.doc", "Copy", "vaultCopy:\(field.key)", onCopy)
            }
        }
        .accessibilityIdentifier("vaultField:\(field.key)")
    }

    private func iconButton(_ symbol: String, _ label: String, _ id: String, _ action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Image(systemName: symbol)
                .font(.system(size: 17, weight: .medium))
                .foregroundStyle(Palette.accent)
                .frame(width: 40, height: 40)
                .contentShape(Circle())
        }
        .buttonStyle(.borderless)
        .accessibilityLabel(label)
        .accessibilityIdentifier(id)
    }
}

/// Adding an item (`newItem`) or changing one (`id`). An SSH key is made on the phone, or pasted.
struct VaultEditScreen: View {
    var id: String?
    var newItem: NewVaultItem?
    @Environment(AppModel.self) private var model
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

    var body: some View {
        let sshNew = kind == .sshKey && existing == nil
        Form {
            Section {
                TextField(newItem == .apiKey ? "OpenAI" : "Name", text: $name)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .accessibilityIdentifier("vaultName")
            } header: {
                GroupHeader("Name")
            } footer: {
                if let hint = VaultText.referenceHint(kind, name: name) {
                    Text(hint).font(RFont.mono(13)).foregroundStyle(Palette.secondary).accessibilityIdentifier("vaultHint")
                }
            }
            ForEach(fields, id: \.key) { f in
                Section { input(f) } header: { GroupHeader(f.label) }
            }
            if sshNew && pasteKey {
                Section {
                    input(VaultFormField(key: "private_key", label: "Private key", secret: true, multiline: true))
                } header: {
                    GroupHeader("Private key (OpenSSH, without a passphrase)")
                }
            }
            if let error = vm.error {
                Section { FormBanner(text: untrusted(error)).accessibilityIdentifier("vaultFormError") }
                    .listRowBackground(Color.clear)
            }
            Section {
                if sshNew && !pasteKey {
                    Button {
                        Task {
                            if let made = await vm.generateSshKey(name: name.trimmingCharacters(in: .whitespaces), model) { saved(made) }
                        }
                    } label: {
                        HStack(spacing: 10) {
                            if vm.busy { ProgressView().tint(Palette.background) } else { Image(systemName: "key.horizontal") }
                            Text("Make a new key on this phone")
                        }
                    }
                    .buttonStyle(CapsuleButtonStyle(kind: .primary, height: 50))
                    .disabled(name.trimmingCharacters(in: .whitespaces).isEmpty || vm.busy)
                    .accessibilityIdentifier("vaultGenerate")
                    Button("I have a key: paste it") { pasteKey = true }
                        .buttonStyle(CapsuleButtonStyle(kind: .secondary, height: 44))
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
                        HStack(spacing: 10) {
                            if vm.busy { ProgressView().tint(Palette.background) }
                            Text("Save")
                        }
                    }
                    .buttonStyle(CapsuleButtonStyle(kind: .primary, height: 50))
                    .disabled(!ready || vm.busy)
                    .accessibilityIdentifier("vaultSave")
                }
            } footer: {
                GroupFooter("Saved encrypted with your vault's key, on this phone, before it reaches the server.")
            }
            .listRowBackground(Color.clear)
            .listRowInsets(EdgeInsets())
        }
        .reinsGrouped()
        .navigationTitle(existing != nil ? "Edit" : newItem?.newTitle ?? "New item")
        .privacySensitive()
        .onAppear {
            guard !started else { return }
            started = true
            vm.error = nil
            name = existing?.name ?? ""
            values = original ?? [:]
        }
    }

    @ViewBuilder
    private func input(_ f: VaultFormField) -> some View {
        let binding = Binding(get: { values[f.key] ?? "" }, set: { values[f.key] = $0 })
        let placeholder = f.secret && existing != nil ? "Unchanged" : ""
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
        .accessibilityIdentifier("vaultInput:\(f.key)")
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
