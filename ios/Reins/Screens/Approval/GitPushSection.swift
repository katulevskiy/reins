import SwiftUI

// A push from git on the user's computer (the Android app's GitPushSection.kt): each branch or tag, its commits folded
// into a row that opens them (with their lines added and removed), each commit's files, each file's diff. Everything
// shown came from the desktop app.

private extension GitRefView {
    var hiddenChars: Bool { commits.contains(where: \.hiddenChars) || files.contains(where: \.hiddenChars) }
}

struct GitFacts: View {
    var view: ApprovalView
    var git: GitPushView

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            FactGroup {
                ForEach(Array(git.refs.enumerated()), id: \.offset) { i, ref in
                    FactRow(key: ref.kind == "tag" ? "Tag" : "Branch", value: untrusted(ref.shortName), mono: true, valueId: "gitRef:\(i)") {
                        if ref.change != "update" || ref.kind == "tag" {
                            TintTag(text: GitPushFormat.chipLabel(ref), tint: chipTint(ref)).accessibilityIdentifier("gitChip:\(i)")
                        }
                    }
                }
                if let account = view.account, !account.isEmpty {
                    FactRow(key: "Account", value: untrusted(account), mono: true)
                }
                ForEach(Array(git.refs.enumerated()), id: \.offset) { i, ref in
                    if !ref.commits.isEmpty || ref.commitCount > 0 {
                        DrillRow(title: ApprovalWords.commits(Int(ref.commitCount)), id: "gitCommits:\(i)", page: .commits(ref: i)) {
                            RefGlance(ref: ref, index: i)
                        }
                    } else if ref.filesChanged > 0 {
                        DrillRow(title: ApprovalWords.files(Int(ref.filesChanged)), id: "gitFiles:\(i)", page: .files(ref: i)) {
                            RefGlance(ref: ref, index: i)
                        }
                    }
                }
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

    private func chipTint(_ ref: GitRefView) -> Color? {
        if ref.change == "delete" { return Palette.danger }
        if ref.change == "create" { return Palette.success }
        if ref.kind == "tag" { return Palette.violet }
        return nil
    }
}

/// A ref's folded row, beside its title: how many of its items deserve a closer look ("2 of 8 to look at") and its
/// lines added and removed; under them "Hidden" when anything in it has hidden characters and "Rewrites history" when
/// it does, flagged or not.
private struct RefGlance: View {
    var ref: GitRefView
    var index: Int

    var body: some View {
        let summary = AttentionText.summary(ref)
        VStack(alignment: .trailing, spacing: 6) {
            HStack(spacing: 8) {
                if let summary {
                    WarnTag(text: summary, tint: Palette.warning, small: true).accessibilityIdentifier("gitAttention:\(index)")
                }
                Counts(additions: ref.additions, deletions: ref.deletions).accessibilityIdentifier("gitTotals:\(index)")
            }
            if ref.hiddenChars || AttentionText.rewrites(ref) {
                HStack(spacing: 8) {
                    if ref.hiddenChars { WarnTag(text: "Hidden", tint: Palette.danger, small: true).accessibilityIdentifier("gitHidden:\(index)") }
                    if AttentionText.rewrites(ref) {
                        WarnTag(text: "Rewrites history", tint: Palette.danger, small: true).accessibilityIdentifier("gitRewrites:\(index)")
                    }
                }
            }
        }
    }
}

/// The commits a ref gains, newest first, each with its own lines added and removed.
struct CommitsPage: View {
    var view: ApprovalView
    var refIndex: Int
    @Environment(ApprovalNav.self) private var nav

    var body: some View {
        if let refs = view.git?.refs, refs.indices.contains(refIndex) {
            let ref = refs[refIndex]
            VStack(alignment: .leading, spacing: 10) {
                HStack(spacing: 0) {
                    Text(untrusted(ref.shortName)).font(RFont.mono(13)).foregroundStyle(Palette.ink2)
                    if ref.filesChanged > 0 {
                        Button(" · \(ApprovalWords.files(Int(ref.filesChanged)))") { nav.path.append(.files(ref: refIndex)) }
                            .font(RFont.sans(13))
                            .foregroundStyle(Palette.inkMeta)
                            .accessibilityIdentifier("gitFiles:\(refIndex)")
                    }
                    Spacer(minLength: 8)
                    Counts(additions: ref.additions, deletions: ref.deletions, size: 13)
                }
                .padding(.horizontal, 22)
                FactGroup {
                    // Flagged first; each row still opens its commit by its place in the push.
                    ForEach(AttentionText.ordered(ref.commits) { $0.attention?.level }, id: \.offset) { j, commit in
                        CommitRow(commit: commit, page: .commit(ref: refIndex, index: j)).accessibilityIdentifier("gitCommit:\(refIndex):\(j)")
                    }
                    let unlisted = Int(ref.commitCount) - ref.commits.count
                    if unlisted > 0 { NotListed(text: "\(unlisted) more not listed").accessibilityIdentifier("gitCommitsNotListed:\(refIndex)") }
                }
            }
            .padding(.top, 6)
        }
    }
}

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
            HStack(alignment: .center, spacing: 12) {
                Text(untrusted(commit.shortSha))
                    .font(RFont.mono(12.5))
                    .foregroundStyle(Palette.inkMeta)
                    .frame(width: 62, alignment: .leading)
                    .frame(maxHeight: .infinity, alignment: .top)
                    .padding(.top, 2)
                VStack(alignment: .leading, spacing: 3) {
                    let subject = untrusted(commit.subject)
                    Text(subject.isEmpty ? "(no message)" : subject).font(RFont.sans(15)).foregroundStyle(Palette.ink).lineLimit(2)
                    HStack(spacing: 8) {
                        Text(untrusted(ApprovalWords.displayName(commit.author))).font(RFont.sans(12.5)).foregroundStyle(Palette.inkMeta).lineLimit(1)
                        Counts(additions: commit.additions, deletions: commit.deletions, size: 12)
                        if AttentionText.hiddenApart(commit.hiddenChars, commit.attention) { WarnTag(text: "Hidden", tint: Palette.danger, small: true) }
                    }
                    if let mark = AttentionText.mark(commit.attention) {
                        AttentionTag(mark: mark, id: "commitAttention:\(commit.shortSha)").padding(.top, 3)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                if opens {
                    Image(systemName: "chevron.right").font(.system(size: 13, weight: .semibold)).foregroundStyle(Palette.inkMeta)
                }
            }
            .fixedSize(horizontal: false, vertical: true)
            .padding(.leading, 16)
            .padding(.trailing, 14)
            .padding(.vertical, 12)
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
                VStack(alignment: .leading, spacing: 6) {
                    let subject = untrusted(commit.subject)
                    Text(subject.isEmpty ? "(no message)" : subject)
                        .font(RFont.sans(21, .semibold))
                        .foregroundStyle(Palette.ink)
                        .accessibilityIdentifier("commitSubject")
                    HStack(spacing: 6) {
                        let who = untrusted(ApprovalWords.displayName(commit.author))
                        Text(commit.date != 0 ? "\(who) · \(TimeText.relative(commit.date))" : who)
                            .font(RFont.sans(13))
                            .foregroundStyle(Palette.inkMeta)
                            .lineLimit(1)
                        if commit.additions != nil || commit.deletions != nil {
                            Text("·").font(RFont.sans(13)).foregroundStyle(Palette.inkMeta)
                            Counts(additions: commit.additions, deletions: commit.deletions, size: 12.5)
                        }
                    }
                    // Why this commit is worth a look; hidden characters keep their own tag and (?) below.
                    if let mark = AttentionText.mark(commit.attention, shown: commit.hiddenChars ? [.hiddenChars] : []) {
                        AttentionTag(mark: mark, id: "commitAttention").padding(.top, 6)
                    }
                    if commit.hiddenChars {
                        HStack(spacing: 8) {
                            WarnTag(text: "Hidden characters", tint: Palette.danger).accessibilityIdentifier("hiddenChars")
                            HelpDot(id: "hiddenCommit", text: ApprovalWords.hiddenHelp)
                        }
                        .padding(.top, 6)
                    }
                }
                .padding(.horizontal, 22)
                .padding(.top, 6)
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
            .padding(.horizontal, 18)
            .padding(.vertical, 13)
    }
}

/// A file: its path (the folder dimmed), lines added and removed; a commit's file opens to its diff.
private struct FileRow: View {
    var file: GitFileView
    var id: String
    var expandable: Bool
    @State private var open = false
    @Environment(\.feedback) private var feedback

    var body: some View {
        let opens = expandable && !file.diff.isEmpty
        let path = ApprovalWords.splitPath(untrusted(file.path))
        VStack(spacing: 0) {
            Button {
                withAnimation(.smooth(duration: 0.25)) { open.toggle() }
                feedback.play(.expand(open))
            } label: {
                HStack(spacing: 10) {
                    Image(systemName: "doc.text").font(.system(size: 14)).foregroundStyle(Palette.ink2)
                    VStack(alignment: .leading, spacing: 5) {
                        (Text(path.0).foregroundStyle(Palette.inkMeta) + Text(path.1).foregroundStyle(Palette.ink))
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
                        Image(systemName: open ? "chevron.down" : "chevron.right").font(.system(size: 12, weight: .semibold)).foregroundStyle(Palette.inkMeta)
                    }
                }
                .padding(.leading, 16)
                .padding(.trailing, 14)
                .padding(.vertical, 12)
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

/// A file's change line by line: hunk headers, numbers, additions in green and deletions in red.
private struct DiffBlock: View {
    var lines: [DiffLineView]
    var cut: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            ScrollView(.horizontal, showsIndicators: true) {
                VStack(alignment: .leading, spacing: 0) {
                    ForEach(Array(lines.enumerated()), id: \.offset) { _, line in row(line) }
                }
                .padding(.vertical, 2)
            }
            if cut {
                Text("More changes not shown")
                    .font(RFont.sans(12.5))
                    .foregroundStyle(Palette.inkMeta)
                    .padding(.horizontal, 14)
                    .padding(.vertical, 8)
                    .accessibilityIdentifier("diffCut")
            }
        }
        .background(Palette.dynamic(0xF6F6F8, 0x0B0B0D))
        .environment(\.layoutDirection, .leftToRight)
    }

    @ViewBuilder private func row(_ line: DiffLineView) -> some View {
        let font = RFont.fixedMono(11.5)
        switch line.kind {
        case .hunk:
            Text(untrustedLine(line.text))
                .font(font)
                .foregroundStyle(Palette.dynamic(0x4A5A99, 0x7C88B8))
                .lineLimit(1)
                .padding(.horizontal, 12)
                .frame(maxWidth: .infinity, minHeight: 20, alignment: .leading)
                .background(Palette.kindRead.opacity(0.06))
        case .note:
            Text(untrustedLine(line.text)).font(font).foregroundStyle(Palette.inkMeta).padding(.leading, 46).frame(minHeight: 20)
        case .add, .delete, .context:
            let add = line.kind == .add
            let del = line.kind == .delete
            HStack(spacing: 8) {
                Text((line.newLine ?? line.oldLine).map { String($0) } ?? "")
                    .foregroundStyle(add ? Palette.dynamic(0x15803D, 0x2F7F5F) : del ? Palette.dynamic(0xDC2626, 0x8A3A3A) : Palette.dynamic(0x97979F, 0x4A4A52))
                    .frame(width: 30, alignment: .trailing)
                Text("\(add ? "+" : del ? "−" : " ") \(untrustedLine(line.text))")
                    .foregroundStyle(add ? Palette.dynamic(0x0F6A3F, 0x9FF0CF) : del ? Palette.dynamic(0x9B1C1C, 0xFDB4B4) : Palette.ink2)
                    .padding(.trailing, 14)
            }
            .font(font)
            .lineLimit(1)
            .frame(maxWidth: .infinity, minHeight: 20, alignment: .leading)
            .background(add ? Palette.success.opacity(0.10) : del ? Palette.danger.opacity(0.10) : Color.clear)
        }
    }
}
