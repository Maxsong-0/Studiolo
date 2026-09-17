import Foundation

/// A provider asking for permission before it acts.
struct ApprovalRequest: Identifiable, Sendable {
    enum Kind: Sendable {
        case command
        case fileChange
        case tool
        case permissions
        case plan
    }

    struct Option: Identifiable, Hashable, Sendable {
        enum Role: Sendable {
            case approve
            case approveAlways
            case decline
            case cancel
        }

        var id: String
        var title: String
        var role: Role
    }

    var id: String
    var kind: Kind
    var title: String
    var detail: String?
    var reason: String?
    var options: [Option]
    var toolItemID: String?

    var headline: String {
        switch kind {
        case .command: "Run this command?"
        case .fileChange: "Apply these changes?"
        case .tool: "Allow this action?"
        case .permissions: "Grant extra access?"
        case .plan: "Ready to build this plan?"
        }
    }

    var symbol: String {
        switch kind {
        case .command: "terminal"
        case .fileChange: "doc.badge.plus"
        case .tool: "wrench.and.screwdriver"
        case .permissions: "lock.shield"
        case .plan: "checklist"
        }
    }
}

/// A provider asking the user to choose or type an answer.
struct QuestionRequest: Identifiable, Sendable {
    struct Question: Identifiable, Hashable, Sendable {
        var id: String
        var header: String
        var prompt: String
        var choices: [Choice]
        var allowsMultiple: Bool
        var allowsOther: Bool
        var isSecret: Bool
    }

    struct Choice: Hashable, Sendable {
        var label: String
        var detail: String?
    }

    var id: String
    var questions: [Question]
}
