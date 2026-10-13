import Foundation

enum SpeechText {
    static func clean(_ markdown: String) -> String {
        var lines: [String] = []
        var inFence = false
        var insertedCodeMarker = false
        for line in markdown.components(separatedBy: .newlines) {
            let trimmed = line.trimmingCharacters(in: .whitespaces)
            if trimmed.hasPrefix("```") {
                if !inFence { insertedCodeMarker = false }
                inFence.toggle()
                if !insertedCodeMarker {
                    lines.append("(code omitted)")
                    insertedCodeMarker = true
                }
                continue
            }
            if inFence { continue }
            if trimmed.hasPrefix("|"), trimmed.hasSuffix("|") { continue }
            lines.append(line)
        }

        var text = lines.joined(separator: "\n")
        text = replacing(text, pattern: #"\[([^\]]+)\]\([^)]+\)"#, with: "$1")
        text = replacing(text, pattern: #"`([^`]+)`"#, with: "$1")
        text = replacing(text, pattern: #"https?://[^\s)\]]+"#, with: "link")
        text = replacing(text, pattern: #"(?m)^\s{0,3}(?:#{1,6}\s+|[-*+]\s+|\d+[.)]\s+|>\s?)"#, with: "")
        text = text.replacingOccurrences(of: "**", with: "")
        text = text.replacingOccurrences(of: "__", with: "")
        text = text.replacingOccurrences(of: "~~", with: "")
        text = text.replacingOccurrences(of: "*", with: "")
        text = text.replacingOccurrences(of: "_", with: "")
        text = text.split(whereSeparator: \.isWhitespace).joined(separator: " ")
        return truncate(text)
    }

    private static func replacing(_ text: String, pattern: String, with replacement: String) -> String {
        guard let regex = try? NSRegularExpression(pattern: pattern) else { return text }
        let range = NSRange(text.startIndex..., in: text)
        return regex.stringByReplacingMatches(in: text, range: range, withTemplate: replacement)
    }

    private static func truncate(_ text: String) -> String {
        let limit = 1200
        guard text.count > limit else { return text }
        let characters = Array(text)
        var cut = limit
        if let boundary = characters[..<limit].lastIndex(where: { ".!?".contains($0) }) {
            cut = boundary + 1
        } else if let boundary = characters[..<limit].lastIndex(where: \.isWhitespace) {
            cut = boundary
        }
        let prefix = String(characters[..<cut]).trimmingCharacters(in: .whitespacesAndNewlines)
        return prefix + " …and more in the chat."
    }
}
