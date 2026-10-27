import Observation
import SwiftUI

/// Settings › Circles (decisions v2, 04 §6.1): the people who help with this account ("Helping you"), and those this
/// account helps ("You help"), each with their role, scope and class. The owner pauses, resumes or ends what they set
/// up; a helper leaves. Add invites someone, Join opens a link someone sent. Circles need this phone to be the
/// account's primary phone: without it, one line and the existing setup.
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
    @State private var adding = false
    @State private var joining = false
    @State private var ending: CirclePerson?
    @State private var setup = false

    var body: some View {
        QuietPage("Circles", lead: AnyView(SymbolTile(symbol: "person.2", tint: Palette.violet, side: 36))) {
            if let error = vm.error { QuietBanner(text: error).accessibilityIdentifier("circlesError") }
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
        .task(id: model.circleTick) { await vm.load(model, fromServer: vm.people == nil) }
        .refreshable { await vm.load(model, fromServer: true) }
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
        section("Helping you", people: split.helpingYou, empty: "Nobody yet", id: "helpingYou")
        section("You help", people: split.youHelp, empty: "Nobody yet", id: "youHelp")
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
                CirclePersonRow(person: person, busy: vm.busy) { change in
                    if change == .end {
                        ending = person
                    } else {
                        Task { await vm.change(person, change, model) }
                    }
                }
            }
        }
        .accessibilityIdentifier(id)
    }
}

/// One arrangement: the person, their role, the scope and class, its state; Pause or Resume (the owner's) and End or
/// Leave behind the menu.
private struct CirclePersonRow: View {
    var person: CirclePerson
    var busy: Bool
    var onChange: (CirclesModel.Change) -> Void

    var body: some View {
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
            if CircleLogic.canEnd(person) {
                Menu {
                    if CircleLogic.canPause(person) {
                        if person.state == .paused {
                            Button("Resume", systemImage: "play") { onChange(.resume) }
                        } else {
                            Button("Pause", systemImage: "pause") { onChange(.pause) }
                        }
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
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("circlePerson:\(person.linkId)")
    }
}
