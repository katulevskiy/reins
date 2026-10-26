import SwiftUI

// A push from git on the user's computer (the Android app's GitPushSection.kt; Next-Approve 01-03): each branch or
// tag with its riskiest commits first and the rest a tap away, each commit's files, each file's diff. Everything shown
// came from the desktop app.

private extension GitRefView {
    var hiddenChars: Bool { commits.contains(where: \.hiddenChars) || files.contains(where: \.hiddenChars) }
}

struct GitFacts: View {
    var view: ApprovalView
    var git: GitPushView

    /// Commits listed on the main page per ref; the rest are behind "+N more".
    static let shown = 3

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            ForEach(Array(git.refs.enumerated()), id: \.offset) { i, ref in
                RefSection(ref: ref, index: i, named: git.refs.count > 1 || ref.commits.isEmpty)
            }
            if let account = view.account, !account.isEmpty {
                FactGroup { FactLine(glyph: .person, value: untrusted(account), mono: true, valueId: "gitAccount") }
            }
            ForEach(Array(git.notes.enumerated()), id: \.offset) { k, note in
                Text(untrusted(note))
                    .font(RFont.sans(12.5))
                    .foregroundStyle(Palette.inkMeta)
                    .padding(.horizontal, 24)
                    .accessibilityIdentifier("gitNote:\(k)")
            }
        }
        .accessibilityContainer("gitPush")
    }
}

/// One ref of a push: its header (how many commits, lines added and removed, how many deserve a look), what is odd
/// about it (hidden characters, rewritten history), the branch or tag when the field's mono line isn't enough, then
/// its riskiest commits and "+N more".
private struct RefSection: View {
    var ref: GitRefView
    var index: Int
    /// Several refs, or one with no commits (a tag, a delete): each says its name and what happens to it.
    var named: Bool

    var body: some View {
        let i = index
        let hasCommits = !ref.commits.isEmpty || ref.commitCount > 0
        VStack(alignment: .leading, spacing: 8) {
            FactHeader(text: header, glyph: hasCommits ? .gitCommit : .gitBranch) {
                HStack(spacing: 8) {
                    if let summary = AttentionText.summary(ref) {
                        WarnTag(text: summary, tint: Palette.warning).accessibilityIdentifier("gitAttention:\(i)")
                    }
                    Counts(additions: ref.additions, deletions: ref.deletions, size: 13).accessibilityIdentifier("gitTotals:\(i)")
                }
            }
            if ref.hiddenChars || AttentionText.rewrites(ref) {
                HStack(spacing: 8) {
                    if ref.hiddenChars { WarnTag(text: "Hidden characters", tint: Palette.danger).accessibilityIdentifier("gitHidden:\(i)") }
                    if AttentionText.rewrites(ref) {
                        WarnTag(text: "Rewrites history", tint: Palette.danger).accessibilityIdentifier("gitRewrites:\(i)")
                    }
                }
                .padding(.horizontal, 20)
            }
            FactGroup {
                if named {
                    FactRow(key: ref.kind == "tag" ? "Tag" : "Branch", value: untrusted(ref.shortName), mono: true, valueId: "gitRef:\(i)") {
                        if ref.change != "update" || ref.kind == "tag" {
                            WarnTag(text: GitPushFormat.chipLabel(ref), tint: chipTint(ref), glyph: nil).accessibilityIdentifier("gitChip:\(i)")
                        }
                    }
                }
                if !ref.commits.isEmpty {
                    let top = Array(AttentionText.ordered(ref.commits) { $0.attention?.level }.prefix(GitFacts.shown))
                    ForEach(top, id: \.offset) { j, commit in
                        CommitRow(commit: commit, page: .commit(ref: i, index: j)).accessibilityIdentifier("gitTopCommit:\(i):\(j)")
                    }
                    let more = Int(ref.commitCount) - top.count
                    if more > 0 {
                        DrillRow(title: "+\(more) more", id: "gitCommits:\(i)", page: .commits(ref: i), quiet: true) { EmptyView() }
                    }
                } else if ref.commitCount > 0 {
                    DrillRow(title: ApprovalWords.commits(Int(ref.commitCount)), id: "gitCommits:\(i)", page: .commits(ref: i)) { EmptyView() }
                } else if ref.filesChanged > 0 {
                    DrillRow(title: ApprovalWords.files(Int(ref.filesChanged)), id: "gitFiles:\(i)", page: .files(ref: i)) { EmptyView() }
                }
            }
        }
    }

    private var header: String {
        let commits = Int(ref.commitCount)
        if !named { return ApprovalWords.commits(commits) }
        return commits > 0 ? "\(untrusted(ref.shortName)) · \(ApprovalWords.commits(commits))" : untrusted(ref.shortName)
    }

    private func chipTint(_ ref: GitRefView) -> Color {
        if ref.change == "delete" { return Palette.danger }
        if ref.change == "create" { return Palette.success }
        if ref.kind == "tag" { return Palette.violet }
        return Palette.ink2
    }
}

/// The commits a ref gains (Next-Approve 02), flagged first, each with its own lines added and removed.
struct CommitsPage: View {
    var view: ApprovalView
    var refIndex: Int
    @Environment(ApprovalNav.self) private var nav

    var body: some View {
        if let refs = view.git?.refs, refs.indices.contains(refIndex) {
            let ref = refs[refIndex]
            VStack(alignment: .leading, spacing: 8) {
                FactHeader(text: untrusted(ref.shortName), glyph: .gitBranch) {
                    Counts(additions: ref.additions, deletions: ref.deletions, size: 13)
                }
                FactGroup {
                    // Flagged first; each row still opens its commit by its place in the push.
                    ForEach(AttentionText.ordered(ref.commits) { $0.attention?.level }, id: \.offset) { j, commit in
                        CommitRow(commit: commit, page: .commit(ref: refIndex, index: j)).accessibilityIdentifier("gitCommit:\(refIndex):\(j)")
                    }
                }
                HStack(spacing: 16) {
                    let unlisted = Int(ref.commitCount) - ref.commits.count
                    if unlisted > 0 {
                        Text("+\(unlisted) more").accessibilityIdentifier("gitCommitsNotListed:\(refIndex)")
                    }
                    if ref.filesChanged > 0 {
                        Button(ApprovalWords.files(Int(ref.filesChanged))) { nav.path.append(.files(ref: refIndex)) }
                            .foregroundStyle(Palette.violet)
                            .frame(minHeight: 44)
                            .accessibilityIdentifier("gitFiles:\(refIndex)")
                    }
                }
                .font(RFont.sans(13, .semibold))
                .foregroundStyle(Palette.inkMeta)
                .frame(maxWidth: .infinity)
            }
            .padding(.top, 10)
        }
    }
}

/// A commit (the kit's `commit_row`): its subject on one line, the short id and lines added and removed under it, why
/// it deserves a look at the right, and the chevron to its files.
private struct CommitRow: View {
    var commit: GitCommitView
    var page: ApprovalPage
    @Environment(ApprovalNav.self) private var nav
    @Environment(\.feedback) private var feedback

    var body: some View {
        let opens = commit.filesChanged != nil
        Button {
            nav.path.append(page)
            feedback.play(.open)
        } label: {
            HStack(alignment: .center, spacing: 10) {
                VStack(alignment: .leading, spacing: 2) {
                    let subject = untrusted(commit.subject)
                    Text(subject.isEmpty ? "(no message)" : subject).font(RFont.sans(16, .medium)).foregroundStyle(Palette.ink).lineLimit(1)
                    HStack(spacing: 8) {
                        Text(untrusted(commit.shortSha)).font(RFont.mono(12.5)).foregroundStyle(Palette.inkMeta)
                        Counts(additions: commit.additions, deletions: commit.deletions, size: 12.5)
                        if AttentionText.hiddenApart(commit.hiddenChars, commit.attention) { WarnTag(text: "Hidden", tint: Palette.danger, small: true) }
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                if let mark = AttentionText.mark(commit.attention) {
                    AttentionTag(mark: mark, id: "commitAttention:\(commit.shortSha)")
                        .frame(maxWidth: 150, alignment: .trailing)
                }
                if opens { GlyphIcon(.chevronRight, size: 14).foregroundStyle(Palette.inkFaint) }
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 9)
            .frame(minHeight: 58)
            .contentShape(Rectangle())
        }
        .buttonStyle(RowButtonStyle())
        .disabled(!opens)
        .accessibilityElement(children: .combine)
    }
}

/// One commit: its message, who and when, and each file it changes, which opens to its lines.
struct CommitPage: View {
    var view: ApprovalView
    var refIndex: Int
    var index: Int
    /// In Activity, a push's changes not kept on this iPhone: loaded from GitHub here. Nil on the waiting sheet.
    @Environment(PushDiffs.self) private var diffs: PushDiffs?

    var body: some View {
        if let refs = view.git?.refs, refs.indices.contains(refIndex), refs[refIndex].commits.indices.contains(index) {
            let kept = refs[refIndex].commits[index]
            let commit = diffs?.commit(kept) ?? kept
            VStack(alignment: .leading, spacing: 14) {
                VStack(alignment: .leading, spacing: 8) {
                    let subject = untrusted(commit.subject)
                    Text(subject.isEmpty ? "(no message)" : subject)
                        .font(RFont.display(20.5))
                        .tracking(RFont.displayTracking(20.5))
                        .foregroundStyle(Palette.ink)
                        .fixedSize(horizontal: false, vertical: true)
                        .accessibilityIdentifier("commitSubject")
                    FlowRow {
                        Text(untrusted(commit.shortSha)).font(RFont.mono(12.5)).foregroundStyle(Palette.inkMeta)
                        if commit.additions != nil || commit.deletions != nil {
                            Counts(additions: commit.additions, deletions: commit.deletions, size: 12.5)
                        }
                        // Why this commit is worth a look; hidden characters keep their own tag and (?) below.
                        if let mark = AttentionText.mark(commit.attention, shown: commit.hiddenChars ? [.hiddenChars] : []) {
                            AttentionTag(mark: mark, id: "commitAttention")
                        }
                    }
                    let who = untrusted(ApprovalWords.displayName(commit.author))
                    if !who.isEmpty || commit.date != 0 {
                        Text(commit.date != 0 ? "\(who) · \(TimeText.relative(commit.date))" : who)
                            .font(RFont.sans(13))
                            .foregroundStyle(Palette.inkMeta)
                            .lineLimit(1)
                    }
                    if commit.hiddenChars {
                        HStack(spacing: 8) {
                            WarnTag(text: "Hidden characters", tint: Palette.danger).accessibilityIdentifier("hiddenChars")
                            HelpDot(id: "hiddenCommit", text: ApprovalWords.hiddenHelp, size: 28)
                        }
                    }
                }
                .padding(.horizontal, 20)
                .padding(.top, 18)
                if let diffs, diffs.missing(kept) {
                    DiffNotKept(diffs: diffs, shortSha: kept.shortSha)
                }
                if let changed = commit.filesChanged {
                    FactGroup {
                        ForEach(AttentionText.ordered(commit.files) { $0.attention?.level }, id: \.offset) { j, file in
                            FileRow(file: file, id: "commitFile:\(j)", expandable: true)
                        }
                        let unlisted = Int(changed) - commit.files.count
                        if unlisted > 0 { NotListed(text: "\(unlisted) more not listed").accessibilityIdentifier("commitFilesNotListed") }
                    }
                } else {
                    FactGroup { NotListed(text: "Files not compared").accessibilityIdentifier("commitFilesUnknown") }
                }
            }
        }
    }
}

/// Every file a ref changes, with its lines added and removed.
struct FilesPage: View {
    var view: ApprovalView
    var refIndex: Int

    var body: some View {
        if let refs = view.git?.refs, refs.indices.contains(refIndex) {
            let ref = refs[refIndex]
            FactGroup {
                ForEach(AttentionText.ordered(ref.files) { $0.attention?.level }, id: \.offset) { j, file in
                    FileRow(file: file, id: "gitFile:\(refIndex):\(j)", expandable: false)
                }
                let unlisted = Int(ref.filesChanged) - ref.files.count
                if unlisted > 0 { NotListed(text: "\(unlisted) more not listed").accessibilityIdentifier("gitFilesNotListed:\(refIndex)") }
            }
            .padding(.top, 6)
        }
    }
}

private struct NotListed: View {
    var text: String

    var body: some View {
        Text(text)
            .font(RFont.sans(13.5))
            .foregroundStyle(Palette.inkMeta)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 16)
            .padding(.vertical, 14)
    }
}

/// A file (Next-Approve 03): its glyph, its path in mono (the folder dimmed), lines added and removed; a commit's file
/// opens to its diff in place.
private struct FileRow: View {
    var file: GitFileView
    var id: String
    var expandable: Bool
    @State private var open = false
    @Environment(\.feedback) private var feedback

    init(file: GitFileView, id: String, expandable: Bool) {
        self.file = file
        self.id = id
        self.expandable = expandable
        #if DEBUG
        // Screenshots: `-approvalOpenFile` opens a commit's first file.
        _open = State(initialValue: expandable && id == "commitFile:0" && ProcessInfo.processInfo.arguments.contains("-approvalOpenFile"))
        #endif
    }

    var body: some View {
        let opens = expandable && !file.diff.isEmpty
        let path = ApprovalWords.splitPath(untrusted(file.path))
        VStack(spacing: 0) {
            Button {
                withAnimation(.smooth(duration: 0.25)) { open.toggle() }
                feedback.play(.expand(open))
            } label: {
                HStack(spacing: 10) {
                    GlyphIcon(.file, size: 16).foregroundStyle(Palette.inkMeta)
                    VStack(alignment: .leading, spacing: 5) {
                        (Text(path.0).foregroundStyle(Palette.inkMeta) + Text(path.1).foregroundStyle(Palette.ink2))
                            .font(RFont.mono(13))
                            .fixedSize(horizontal: false, vertical: true)
                            .environment(\.layoutDirection, .leftToRight)
                        if let mark = AttentionText.mark(file.attention) {
                            AttentionTag(mark: mark, id: "\(id):attention")
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    if AttentionText.hiddenApart(file.hiddenChars, file.attention) { WarnTag(text: "Hidden", tint: Palette.danger, small: true) }
                    if file.binary {
                        Text("binary").font(RFont.sans(12.5)).foregroundStyle(Palette.inkMeta)
                    } else {
                        Counts(additions: file.additions.map { UInt64($0) }, deletions: file.deletions.map { UInt64($0) }, size: 12.5)
                    }
                    if opens {
                        GlyphIcon(open ? .chevronDown : .chevronRight, size: 14).foregroundStyle(Palette.inkFaint)
                    }
                }
                .padding(.horizontal, 14)
                .padding(.vertical, 9)
                .frame(minHeight: 48)
                .contentShape(Rectangle())
            }
            .buttonStyle(RowButtonStyle())
            .disabled(!opens)
            .accessibilityElement(children: .combine)
            .accessibilityIdentifier(id)
            if open && opens {
                DiffBlock(lines: file.diff, cut: file.diffCut).accessibilityIdentifier("\(id):diff")
            }
        }
    }
}

/// A file's change line by line (the kit's code: mono 11.5 on 19): hunk headers, line numbers, additions on green
/// and deletions on red at 14 %, nothing around it.
private struct DiffBlock: View {
    var lines: [DiffLineView]
    var cut: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            ScrollView(.horizontal, showsIndicators: true) {
                VStack(alignment: .leading, spacing: 0) {
                    ForEach(Array(lines.enumerated()), id: \.offset) { _, line in row(line) }
                }
                .padding(.top, 4)
                .padding(.bottom, 10)
            }
            if cut {
                Text("More changes not shown")
                    .font(RFont.sans(12.5))
                    .foregroundStyle(Palette.inkMeta)
                    .padding(.horizontal, 14)
                    .padding(.bottom, 10)
                    .accessibilityIdentifier("diffCut")
            }
        }
        .environment(\.layoutDirection, .leftToRight)
    }

    @ViewBuilder private func row(_ line: DiffLineView) -> some View {
        let font = RFont.fixedMono(11.5)
        switch line.kind {
        case .hunk:
            Text(untrustedLine(line.text))
                .font(font)
                .foregroundStyle(Palette.inkMeta)
                .lineLimit(1)
                .padding(.horizontal, 14)
                .frame(maxWidth: .infinity, minHeight: 19, alignment: .leading)
        case .note:
            Text(untrustedLine(line.text)).font(font).foregroundStyle(Palette.inkMeta).padding(.leading, 42).frame(minHeight: 19)
        case .add, .delete, .context:
            let add = line.kind == .add
            let del = line.kind == .delete
            HStack(spacing: 10) {
                Text((line.newLine ?? line.oldLine).map { String($0) } ?? "")
                    .foregroundStyle(Palette.inkMeta)
                    .frame(width: 18, alignment: .trailing)
                Text(untrustedLine(line.text))
                    .foregroundStyle(Palette.ink)
                    .padding(.trailing, 14)
            }
            .padding(.leading, 14)
            .font(font)
            .lineLimit(1)
            .frame(maxWidth: .infinity, minHeight: 19, alignment: .leading)
            .background(add ? Palette.success.opacity(0.14) : del ? Palette.danger.opacity(0.14) : Color.clear)
            .accessibilityLabel("\(add ? "Added" : del ? "Removed" : "Unchanged"): \(untrustedLine(line.text))")
        }
    }

}
