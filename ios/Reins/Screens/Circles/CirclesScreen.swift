import Observation
import SwiftUI

/// Settings › Circles (decisions v2, 04 §6.1): the people who help with this account ("Helping you"), and those this
/// account helps ("You help"), each with their role, scope and class. The owner pauses, resumes or ends what they set
/// up; a helper leaves. Add invites someone, Join opens a link someone sent. Circles need this phone to be the
/// account's primary phone: without it, one line and the existing setup.
///
/// Guardianship (P7, 04 §4): a ward's row opens what the guardian sees and decided, and Leave with its notice (never
/// Pause); a guardian sees a minor's request to leave as "Sam asked to leave. Allow?", and what it was told about its
/// wards.
@Observable
@MainActor
final class CirclesModel {
    /// Nil until first read (and while Circles are not possible here).
    private(set) var people: [CirclePerson]?
    /// Nil until known.
    private(set) var available: Bool?
    private(set) var busy = false
    var error: String?

    func load(_ app: AppModel, fromServer: Bool) async {
        let epoch = app.accountEpoch
        do {
            let ok = try await app.core.circlesAvailable()
            guard epoch == app.accountEpoch else { return }
            available = ok
            guard ok else { return }
            let list = try await app.core.circlePeople(refresh: fromServer)
            guard epoch == app.accountEpoch else { return }
            people = list
            error = nil
        } catch {
            guard epoch == app.accountEpoch else { return }
            if available == nil { available = true }
            self.error = error.userMessage
        }
    }

    /// Pause or resume (the owner's), End (the owner's) or Leave (a helper's): the core's word, then the list again.
    func change(_ person: CirclePerson, _ change: Change, _ app: AppModel) async {
        guard !busy else { return }
        busy = true
        error = nil
        defer { busy = false }
        do {
            switch change {
            case .pause: try await app.core.circlePause(linkId: person.linkId, paused: true)
            case .resume: try await app.core.circlePause(linkId: person.linkId, paused: false)
            case .end: try await app.core.circleEnd(linkId: person.linkId)
            }
            app.feedback.play(change == .resume ? .toggleOn : .revoked)
            await load(app, fromServer: false)
            await app.refreshCircles()
        } catch {
            app.feedback.play(.error)
            self.error = error.userMessage
        }
    }

    enum Change { case pause, resume, end }
}

struct CirclesScreen: View {
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @State private var vm = CirclesModel()
    @State private var guardian = GuardianNoticesModel()
    @State private var ward: CirclePerson?
    @State private var adding = false
    @State private var joining = false
    @State private var ending: CirclePerson?
    @State private var setup = false

    var body: some View {
        QuietPage("Circles", lead: AnyView(SymbolTile(symbol: "person.2", tint: Palette.violet, side: 36))) {
            if let error = vm.error ?? guardian.error { QuietBanner(text: error).accessibilityIdentifier("circlesError") }
            switch vm.available {
            case .none:
                ProgressView().frame(maxWidth: .infinity).padding(.vertical, 40)
            case .some(false):
                unavailable
            case .some(true):
                list
            }
        } footer: {
            if vm.available == true {
                QuietFooter {
                    // "Add someone" is the longer label: the wider button, as in the join and enrol sheets.
                    RatioRow(spacing: 10, ratio: 1 / 1.35) {
                        Button {
                            joining = true
                            feedback.play(.open)
                        } label: {
                            Label("Join", systemImage: "link")
                        }
                        .buttonStyle(SignalButtonStyle(.outline))
                        .accessibilityIdentifier("circleJoin")
                        Button {
                            adding = true
                            feedback.play(.open)
                        } label: {
                            Label("Add someone", systemImage: "person.badge.plus")
                        }
                        .buttonStyle(SignalButtonStyle(.inverse))
                        .accessibilityIdentifier("circleAdd")
                    }
                    .frame(height: 54)
                }
            }
        }
        // The first read asks the server; the core's news (`circleTick`) is read from the phone.
        .task(id: model.circleTick) {
            let first = vm.people == nil
            await vm.load(model, fromServer: first)
            if vm.available == true { await guardian.load(model, fromServer: first) }
        }
        .refreshable {
            await vm.load(model, fromServer: true)
            if vm.available == true { await guardian.load(model, fromServer: true) }
        }
        .animation(.smooth(duration: 0.25), value: vm.people)
        .sheet(isPresented: $adding, onDismiss: { Task { await vm.load(model, fromServer: false) } }) {
            AddCircleSheet { adding = false }
                .environment(model)
                .environment(\.feedback, feedback)
        }
        .sheet(isPresented: $joining, onDismiss: { Task { await vm.load(model, fromServer: false) } }) {
            JoinCircleSheet { joining = false }
                .environment(model)
                .environment(\.feedback, feedback)
        }
        .sheet(item: Binding(get: { ward.map(WardSheet.init) }, set: { ward = $0?.person }), onDismiss: {
            Task { await vm.load(model, fromServer: false) }
        }) { sheet in
            GuardianWardScreen(person: sheet.person) { ward = nil }
                .environment(model)
                .environment(\.feedback, feedback)
        }
        .sheet(isPresented: $setup, onDismiss: { Task { await vm.load(model, fromServer: true) } }) {
            PrimaryPhoneSetupSheet(onClose: { setup = false })
                .environment(model)
        }
        .confirmationDialog(
            ending.map { "\(CircleLogic.endTitle($0)) with \(CircleLogic.name($0.peer))?" } ?? "",
            isPresented: Binding(get: { ending != nil }, set: { if !$0 { ending = nil } }),
            titleVisibility: .visible,
            presenting: ending
        ) { person in
            Button(CircleLogic.endTitle(person), role: .destructive) {
                ending = nil
                Task { await vm.change(person, .end, model) }
            }
            .accessibilityIdentifier("circleConfirmEnd")
            Button("Cancel", role: .cancel) { ending = nil }
        } message: { person in
            Text(person.mine ? "Nothing goes to them any more." : "You no longer decide for them.")
        }
        .presentationFeedback(ending != nil)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("circlesScreen")
    }

    /// Circles need this phone to be the account's primary phone (a roster, so there is an owner key to pin).
    private var unavailable: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("Circles need this phone as your primary phone.")
                .textRole(.body)
                .foregroundStyle(Palette.ink2)
                .fixedSize(horizontal: false, vertical: true)
            Button {
                setup = true
                feedback.play(.open)
            } label: {
                Label("Set up this phone", systemImage: "person.badge.shield.checkmark")
            }
            .buttonStyle(SignalButtonStyle(.inverse))
            .accessibilityIdentifier("circlesSetUpPrimary")
        }
        .padding(.horizontal, 20)
        .padding(.top, 18)
        .accessibilityIdentifier("circlesUnavailable")
    }

    @ViewBuilder private var list: some View {
        let split = CircleLogic.split(vm.people ?? [])
        ForEach(GuardianLogic.consents(vm.people ?? []), id: \.linkId) { person in
            GuardianConsentCard(ward: person, busy: guardian.busy) {
                Task {
                    await guardian.allow(person, model)
                    await vm.load(model, fromServer: false)
                }
            }
        }
        section("Helping you", people: split.helpingYou, empty: "Nobody yet", id: "helpingYou")
        section("You help", people: split.youHelp, empty: "Nobody yet", id: "youHelp")
        if !guardian.notices.isEmpty { GuardianNoticeList(notices: guardian.notices) }
    }

    @ViewBuilder private func section(_ title: String, people: [CirclePerson], empty: String, id: String) -> some View {
        SectionHeader(title, quiet: true, trailing: people.isEmpty ? nil : "\(people.count)")
        FlatGroup(inset: 0) {
            if vm.people == nil {
                TileRow("Loading…", chevron: false) { SymbolTile(symbol: "person.2") } trailing: { EmptyView() }.flatRow()
            } else if people.isEmpty {
                TileRow(empty, chevron: false) { SymbolTile(symbol: "person.2") } trailing: { EmptyView() }
                    .flatRow()
                    .accessibilityIdentifier("\(id)Empty")
            }
            ForEach(people, id: \.linkId) { person in
                CirclePersonRow(person: person, busy: vm.busy || guardian.busy) { change in
                    switch change {
                    case .end where GuardianLogic.isWard(person):
                        // A ward leaves from its guardianship's page, where the notice is.
                        ward = person
                        feedback.play(.open)
                    case .end:
                        ending = person
                    case .open:
                        ward = person
                        feedback.play(.open)
                    case .proposeMinor:
                        Task { await guardian.proposeMinor(person, model) }
                    case .pause, .resume:
                        Task { await vm.change(person, change == .pause ? .pause : .resume, model) }
                    }
                }
            }
        }
        // A container first: an identifier on the group alone replaces every row's own ("circlePerson:l6").
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier(id)
    }
}

/// A ward's guardianship, as the sheet it opens.
private struct WardSheet: Identifiable {
    var person: CirclePerson
    var id: String { person.linkId }
}

/// One arrangement: the person, their role, the scope and class, its state; Pause or Resume (the owner's) and End or
/// Leave behind the menu. A ward's guardianship opens its page (and never pauses); a guardian may ask to mark the
/// ward's account a minor's.
private struct CirclePersonRow: View {
    enum Action { case pause, resume, end, open, proposeMinor }

    var person: CirclePerson
    var busy: Bool
    var onChange: (Action) -> Void

    /// A ward's running guardianship: the row opens its page (Leave is there, with the notice).
    private var opens: Bool { GuardianLogic.isWard(person) && person.state != .ended }

    var body: some View {
        if opens {
            Button {
                onChange(.open)
            } label: {
                row
            }
            .buttonStyle(.plain)
            .disabled(busy)
            .accessibilityIdentifier("circlePerson:\(person.linkId)")
        } else {
            row.accessibilityIdentifier("circlePerson:\(person.linkId)")
        }
    }

    private var row: some View {
        HStack(alignment: .top, spacing: 12) {
            BlobAvatar(seed: untrusted(person.peer), size: 36)
            VStack(alignment: .leading, spacing: 4) {
                HStack(spacing: 6) {
                    Text(verbatim: CircleLogic.name(person.peer))
                        .textRole(.row)
                        .foregroundStyle(Palette.ink)
                        .lineLimit(1)
                    CircleRoleChip(role: person.role)
                    if let badge = CircleLogic.badge(person.state) {
                        Pill(badge, tone: person.state == .suspended ? .danger : person.state == .paused ? .warning : .neutral)
                    }
                    TimelineView(.periodic(from: .now, by: 60)) { context in
                        if let badge = GuardianLogic.badge(person, now: Int64(context.date.timeIntervalSince1970)) {
                            Pill(badge, tone: .warning).accessibilityIdentifier("guardianBadge:\(person.linkId)")
                        }
                    }
                }
                Text(verbatim: untrustedName(person.peer, limit: 64))
                    .font(RFont.sans(12.5))
                    .foregroundStyle(Palette.inkMeta)
                    .lineLimit(1)
                    .truncationMode(.middle)
                    .environment(\.layoutDirection, .leftToRight)
                let scope = untrusted(person.scopeLine)
                Text(verbatim: [scope, CircleLogic.classLine(deferred: person.deferred, windowSecs: person.windowSecs)]
                    .filter { !$0.isEmpty }.joined(separator: " · "))
                    .font(RFont.sans(13))
                    .foregroundStyle(Palette.ink2)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            if opens {
                RowChevron().frame(height: 44)
            } else if CircleLogic.canEnd(person) {
                Menu {
                    if CircleLogic.canPause(person) {
                        if person.state == .paused {
                            Button("Resume", systemImage: "play") { onChange(.resume) }
                        } else {
                            Button("Pause", systemImage: "pause") { onChange(.pause) }
                        }
                    }
                    if GuardianLogic.canProposeMinor(person) {
                        Button("Mark as a minor's", systemImage: "figure.child") { onChange(.proposeMinor) }
                    }
                    Button(CircleLogic.endTitle(person), systemImage: "xmark.circle", role: .destructive) { onChange(.end) }
                } label: {
                    Image(systemName: "ellipsis")
                        .font(.system(size: 17, weight: .semibold))
                        .foregroundStyle(Palette.ink2)
                        .frame(width: 44, height: 44)
                        .contentShape(Rectangle())
                }
                .disabled(busy)
                .accessibilityLabel("More for \(CircleLogic.name(person.peer))")
                .accessibilityIdentifier("circleMore:\(person.linkId)")
            }
        }
        .padding(.vertical, 12)
        .opacity(person.state == .ended ? 0.5 : 1)
        .contentShape(Rectangle())
        .accessibilityElement(children: .contain)
    }
}
