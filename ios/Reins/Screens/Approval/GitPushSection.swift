import SwiftUI

/// A push from git on the user's computer, ref by ref: what each branch or tag gains or loses, which commits and files,
/// and a warning when history is rewritten (or could not be checked). Everything shown came from the desktop app.
struct GitPushSection: View {
    var push: GitPushView
    var host: String = "GitHub"

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 12) {
                Text(untrusted(push.repo))
                    .font(RFont.mono(15, .semibold))
                    .foregroundStyle(Palette.text)
                    .lineLimit(1)
                    .truncationMode(.middle)
                    .environment(\.layoutDirection, .leftToRight)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .accessibilityIdentifier("gitRepo")
                Text("\(GitPushFormat.packSize(push.packBytes)) sent")
                    .font(RFont.sans(12.5))
                    .foregroundStyle(Palette.tertiary)
                    .lineLimit(1)
                    .accessibilityIdentifier("gitPack")
            }
            .padding(.horizontal, 4)
            ForEach(Array(push.refs.enumerated()), id: \.offset) { i, ref in
                RefCard(index: i, ref: ref, host: host)
            }
            ForEach(Array(push.notes.enumerated()), id: \.offset) { k, note in
                Text(untrusted(note))
                    .font(RFont.sans(13))
                    .foregroundStyle(Palette.tertiary)
                    .padding(.horizontal, 4)
                    .accessibilityIdentifier("gitNote:\(k)")
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 8)
        .accessibilityContainer("gitPush")
    }
}

private struct RefCard: View {
    var index: Int
    var ref: GitRefView
    var host: String

    var body: some View {
        Card(padding: 16) {
            VStack(alignment: .leading, spacing: 0) {
                HStack(spacing: 10) {
                    Text(untrusted(ref.shortName))
                        .font(RFont.mono(17, .semibold))
                        .foregroundStyle(Palette.text)
                        .lineLimit(1)
                        .truncationMode(.middle)
                        .environment(\.layoutDirection, .leftToRight)
                        .accessibilityIdentifier("gitRef:\(index)")
                    TintTag(text: GitPushFormat.chipLabel(ref), tint: chipTint).accessibilityIdentifier("gitChip:\(index)")
                    Spacer(minLength: 0)
                }
                switch GitPushFormat.history(ref) {
                case .rewrites:
                    Banner("Rewrites history (force push)", kind: .error).padding(.top, 12).accessibilityIdentifier("forceWarning:\(index)")
                case .unknown:
                    Banner("Could not check history — treat as a force push", kind: .warning).padding(.top, 12).accessibilityIdentifier("forceUnknown:\(index)")
                case .adds:
                    EmptyView()
                }
                if ref.change == "delete" {
                    Text(ref.kind == "tag" ? "The tag is removed from \(host)." : "The branch is removed from \(host).")
                        .font(RFont.sans(14))
                        .foregroundStyle(Palette.secondary)
                        .padding(.top, 8)
                }
                if !ref.commits.isEmpty || ref.commitCount > 0 { Commits(index: index, ref: ref) }
                if let totals = GitPushFormat.totalsLabel(ref) {
                    Hairline(inset: 0).padding(.top, 14).padding(.bottom, 12)
                    Files(index: index, ref: ref, totals: totals)
                }
            }
        }
    }

    private var chipTint: Color? {
        if ref.change == "delete" { return Palette.danger }
        if ref.change == "create" { return Palette.success }
        if ref.kind == "tag" { return Palette.accent }
        return nil
    }
}

private struct Commits: View {
    var index: Int
    var ref: GitRefView
    @State private var expanded = false
    @Environment(\.feedback) private var feedback

    var body: some View {
        let count = Int(ref.commitCount)
        Text(count == 1 ? "1 commit" : "\(count) commits")
            .font(RFont.sans(12.5, .medium))
            .foregroundStyle(Palette.tertiary)
            .padding(.top, 14)
            .padding(.bottom, 4)
        let shown = expanded ? ref.commits : Array(ref.commits.prefix(GitPushFormat.commitsShown))
        ForEach(Array(shown.enumerated()), id: \.offset) { j, commit in
            HStack(alignment: .firstTextBaseline, spacing: 0) {
                Text(commit.shortSha)
                    .font(RFont.mono(13))
                    .foregroundStyle(Palette.secondary)
                    .frame(width: 66, alignment: .leading)
                    .environment(\.layoutDirection, .leftToRight)
                VStack(alignment: .leading, spacing: 1) {
                    let subject = untrusted(commit.subject)
                    Text(subject.isEmpty ? "(no message)" : subject).font(RFont.sans(14.5)).foregroundStyle(Palette.text).lineLimit(2)
                    Text(untrusted(commit.author)).font(RFont.sans(12.5)).foregroundStyle(Palette.tertiary).lineLimit(1)
                }
            }
            .padding(.vertical, 6)
            .accessibilityElement(children: .combine)
            .accessibilityIdentifier("gitCommit:\(index):\(j)")
        }
        let hidden = ref.commits.count - shown.count
        if hidden > 0 {
            MoreLink(text: "and \(hidden) more") {
                withAnimation(.smooth) { expanded = true }
                feedback.play(.open)
            }
            .accessibilityIdentifier("gitMoreCommits:\(index)")
        }
        let unlisted = count - ref.commits.count
        if unlisted > 0 { NotListed(n: unlisted).accessibilityIdentifier("gitCommitsNotListed:\(index)") }
    }
}

private struct Files: View {
    var index: Int
    var ref: GitRefView
    var totals: String
    @State private var expanded = false
    @Environment(\.feedback) private var feedback

    var body: some View {
        Counts(text: totals, font: RFont.sans(14, .medium), color: Palette.text)
            .padding(.bottom, 4)
            .accessibilityIdentifier("gitTotals:\(index)")
        let shown = expanded ? ref.files : Array(ref.files.prefix(GitPushFormat.filesShown))
        ForEach(Array(shown.enumerated()), id: \.offset) { j, file in
            FileRow(file: file).accessibilityIdentifier("gitFile:\(index):\(j)")
        }
        let hidden = ref.files.count - shown.count
        if hidden > 0 {
            MoreLink(text: "and \(hidden) more") {
                withAnimation(.smooth) { expanded = true }
                feedback.play(.open)
            }
            .accessibilityIdentifier("gitMoreFiles:\(index)")
        }
        let unlisted = Int(ref.filesChanged) - ref.files.count
        if unlisted > 0 { NotListed(n: unlisted).accessibilityIdentifier("gitFilesNotListed:\(index)") }
    }
}

private struct FileRow: View {
    var file: GitFileView

    var body: some View {
        let letter = GitPushFormat.fileLetter(file.status)
        let tone: Color = switch letter {
        case "A": Palette.success
        case "D": Palette.danger
        case "M": Palette.search
        case "T": Palette.accent
        default: Palette.secondary
        }
        HStack(spacing: 0) {
            Text(letter).font(RFont.mono(13, .bold)).foregroundStyle(tone).frame(width: 22, alignment: .leading)
            Text(untrusted(file.path))
                .font(RFont.mono(13))
                .foregroundStyle(Palette.text)
                .lineLimit(1)
                .truncationMode(.middle)
                .frame(maxWidth: .infinity, alignment: .leading)
            let counts = GitPushFormat.fileCounts(file)
            if !counts.isEmpty {
                Group {
                    if file.binary {
                        Text(counts).font(RFont.sans(12.5)).foregroundStyle(Palette.tertiary)
                    } else {
                        Counts(text: counts, font: RFont.mono(12.5), color: Palette.secondary)
                    }
                }
                .padding(.leading, 10)
            }
        }
        .environment(\.layoutDirection, .leftToRight)
        .padding(.vertical, 5)
        .accessibilityElement(children: .combine)
    }
}

/// A line such as "+120 −14 in 12 files" with the additions in green and the deletions in red.
private struct Counts: View {
    var text: String
    var font: Font
    var color: Color

    var body: some View {
        var out = AttributedString()
        for (k, word) in text.split(separator: " ", omittingEmptySubsequences: false).enumerated() {
            if k > 0 { out += AttributedString(" ") }
            var part = AttributedString(String(word))
            if word.hasPrefix("+") { part.foregroundColor = Palette.success } else if word.hasPrefix("−") { part.foregroundColor = Palette.danger }
            out += part
        }
        return Text(out).font(font).foregroundStyle(color).lineLimit(1)
    }
}

private struct MoreLink: View {
    var text: String
    var action: () -> Void

    var body: some View {
        Button(action: action) {
            Text(text).font(RFont.sans(14, .medium)).foregroundStyle(Palette.accent).padding(.vertical, 8)
        }
        .buttonStyle(.plain)
    }
}

private struct NotListed: View {
    var n: Int

    var body: some View {
        Text("\(n) more not listed").font(RFont.sans(12.5)).foregroundStyle(Palette.tertiary).padding(.top, 4)
    }
}
