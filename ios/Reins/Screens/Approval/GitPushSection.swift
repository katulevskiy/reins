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
                            HStack(spacing: 8) {
                                if ref.hiddenChars { WarnTag(text: "Hidden", tint: Palette.danger, small: true) }
                                Counts(additions: ref.additions, deletions: ref.deletions).accessibilityIdentifier("gitTotals:\(i)")
                            }
                        }
                    } else if ref.filesChanged > 0 {
                        DrillRow(title: ApprovalWords.files(Int(ref.filesChanged)), id: "gitFiles:\(i)", page: .files(ref: i)) {
                            Counts(additions: ref.additions, deletions: ref.deletions).accessibilityIdentifier("gitTotals:\(i)")
                        }
                    }
                }
            }
            ForEach(Array(git.notes.enumerated()), id: \.offset) { k, note in
                Text(untrusted(note))
                    .font(RFont.sans(12.5))
                    .foregroundStyle(Palette.tertiary)
                    .padding(.horizontal, 24)
                    .accessibilityIdentifier("gitNote:\(k)")
            }
        }
        .accessibilityContainer("gitPush")
    }

    private func chipTint(_ ref: GitRefView) -> Color? {
        if ref.change == "delete" { return Palette.danger }
        if ref.change == "create" { return Palette.success }
        if ref.kind == "tag" { return Palette.accent }
        return nil
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
                    Text(untrusted(ref.shortName)).font(RFont.mono(13)).foregroundStyle(Palette.secondary).lineLimit(1)
                    if ref.filesChanged > 0 {
                        Button(" · \(ApprovalWords.files(Int(ref.filesChanged)))") { nav.path.append(.files(ref: refIndex)) }
                            .font(RFont.sans(13))
                            .foregroundStyle(Palette.tertiary)
                            .accessibilityIdentifier("gitFiles:\(refIndex)")
                    }
                    Spacer(minLength: 8)
                    Counts(additions: ref.additions, deletions: ref.deletions, size: 13)
                }
                .padding(.horizontal, 22)
                FactGroup {
                    ForEach(Array(ref.commits.enumerated()), id: \.offset) { j, commit in
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
                Text(commit.shortSha)
                    .font(RFont.mono(12.5))
                    .foregroundStyle(Palette.tertiary)
                    .frame(width: 62, alignment: .leading)
                    .frame(maxHeight: .infinity, alignment: .top)
                    .padding(.top, 2)
                VStack(alignment: .leading, spacing: 3) {
                    let subject = untrusted(commit.subject)
                    Text(subject.isEmpty ? "(no message)" : subject).font(RFont.sans(15)).foregroundStyle(Palette.text).lineLimit(2)
                    HStack(spacing: 8) {
                        Text(untrusted(ApprovalWords.displayName(commit.author))).font(RFont.sans(12.5)).foregroundStyle(Palette.tertiary).lineLimit(1)
                        Counts(additions: commit.additions, deletions: commit.deletions, size: 12)
                        if commit.hiddenChars { WarnTag(text: "Hidden", tint: Palette.danger, small: true) }
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                if opens {
                    Image(systemName: "chevron.right").font(.system(size: 13, weight: .semibold)).foregroundStyle(Palette.tertiary)
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

    var body: some View {
        if let refs = view.git?.refs, refs.indices.contains(refIndex), refs[refIndex].commits.indices.contains(index) {
            let commit = refs[refIndex].commits[index]
            VStack(alignment: .leading, spacing: 14) {
                VStack(alignment: .leading, spacing: 6) {
                    let subject = untrusted(commit.subject)
                    Text(subject.isEmpty ? "(no message)" : subject)
                        .font(RFont.sans(21, .semibold))
                        .foregroundStyle(Palette.text)
                        .accessibilityIdentifier("commitSubject")
                    HStack(spacing: 6) {
                        let who = untrusted(ApprovalWords.displayName(commit.author))
                        Text(commit.date != 0 ? "\(who) · \(TimeText.relative(commit.date))" : who)
                            .font(RFont.sans(13))
                            .foregroundStyle(Palette.tertiary)
                            .lineLimit(1)
                        if commit.additions != nil || commit.deletions != nil {
                            Text("·").font(RFont.sans(13)).foregroundStyle(Palette.tertiary)
                            Counts(additions: commit.additions, deletions: commit.deletions, size: 12.5)
                        }
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
                if let changed = commit.filesChanged {
                    FactGroup {
                        ForEach(Array(commit.files.enumerated()), id: \.offset) { j, file in
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
                ForEach(Array(ref.files.enumerated()), id: \.offset) { j, file in
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
            .foregroundStyle(Palette.tertiary)
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
                    Image(systemName: "doc.text").font(.system(size: 14)).foregroundStyle(Palette.secondary)
                    (Text(path.0).foregroundStyle(Palette.tertiary) + Text(path.1).foregroundStyle(Palette.text))
                        .font(RFont.mono(13))
                        .lineLimit(1)
                        .truncationMode(.middle)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .environment(\.layoutDirection, .leftToRight)
                    if file.hiddenChars { WarnTag(text: "Hidden", tint: Palette.danger, small: true) }
                    if file.binary {
                        Text("binary").font(RFont.sans(12.5)).foregroundStyle(Palette.tertiary)
                    } else {
                        Counts(additions: file.additions.map { UInt64($0) }, deletions: file.deletions.map { UInt64($0) }, size: 12.5)
                    }
                    if opens {
                        Image(systemName: open ? "chevron.down" : "chevron.right").font(.system(size: 12, weight: .semibold)).foregroundStyle(Palette.tertiary)
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
            ScrollView(.horizontal, showsIndicators: false) {
                VStack(alignment: .leading, spacing: 0) {
                    ForEach(Array(lines.enumerated()), id: \.offset) { _, line in row(line) }
                }
                .padding(.vertical, 2)
            }
            if cut {
                Text("More changes not shown")
                    .font(RFont.sans(12.5))
                    .foregroundStyle(Palette.tertiary)
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
                .background(Palette.search.opacity(0.06))
        case .note:
            Text(untrustedLine(line.text)).font(font).foregroundStyle(Palette.tertiary).padding(.leading, 46).frame(minHeight: 20)
        case .add, .delete, .context:
            let add = line.kind == .add
            let del = line.kind == .delete
            HStack(spacing: 8) {
                Text((line.newLine ?? line.oldLine).map { String($0) } ?? "")
                    .foregroundStyle(add ? Palette.dynamic(0x15803D, 0x2F7F5F) : del ? Palette.dynamic(0xDC2626, 0x8A3A3A) : Palette.dynamic(0x97979F, 0x4A4A52))
                    .frame(width: 30, alignment: .trailing)
                Text("\(add ? "+" : del ? "−" : " ") \(untrustedLine(line.text))")
                    .foregroundStyle(add ? Palette.dynamic(0x0F6A3F, 0x9FF0CF) : del ? Palette.dynamic(0x9B1C1C, 0xFDB4B4) : Palette.secondary)
                    .padding(.trailing, 14)
            }
            .font(font)
            .lineLimit(1)
            .frame(maxWidth: .infinity, minHeight: 20, alignment: .leading)
            .background(add ? Palette.success.opacity(0.10) : del ? Palette.danger.opacity(0.10) : Color.clear)
        }
    }
}
