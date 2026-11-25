// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.

// Arranging lines, projects, lines straight to one agent, models, stopping a turn (LMAS `docs/bottega-protocol.md`,
// section 11).

extension Event {
    /// A line was arranged: what is written here is what it became.
    public struct LineArranged: Decodable, Equatable, Sendable {
        /// The number of its first letter, or a session's name for all there is with that session.
        public var line: String
        /// Empty when the title was taken away.
        public var title: String?
        public var pinned, archived: Bool?
        public var atMs: UInt64
    }
    /// How far a line has been read. Only ever further.
    public struct LineRead: Decodable, Equatable, Sendable { public var line: String; public var through, atMs: UInt64 }

    public struct ProjectAdded: Decodable, Equatable, Sendable { public var project, path: String; public var atMs: UInt64 }
    public struct ProjectRemoved: Decodable, Equatable, Sendable { public var project: String; public var atMs: UInt64 }

    /// A line straight to one agent is being set up. `LineReady` or `LineFailed` follows.
    public struct LineOpened: Decodable, Equatable, Sendable {
        /// The line's number, and its agent's session name.
        public var line, project, vendor, role: String
        public var model, effort: String?
        /// The first thing the user says on it. Becomes a letter once the agent is up.
        public var text: String?
        public var files: [TextFile]?
        public var atMs: UInt64
    }
    public struct LineReady: Decodable, Equatable, Sendable {
        public var line, worktree, gitDir, branch, base: String
        public var atMs: UInt64
    }
    /// It will never be ready.
    public struct LineFailed: Decodable, Equatable, Sendable { public var line: String; public var why: Why; public var atMs: UInt64 }
    /// A line's agent came up or went down.
    public struct LineWorker: Decodable, Equatable, Sendable {
        /// `state` is `up` or `down`.
        public var line, state: String
        /// Which start of it, from 1. A later one does not remember; it was shown what was said on the line.
        public var run: Int
        public var why: Why?
        public var atMs: UInt64
    }

    /// What a vendor's CLI offers now. Comes when it differs from before.
    public struct ModelsListed: Decodable, Equatable, Sendable {
        public var vendor: String
        /// What its agents can be on a line: `coder`, `reviewer`.
        public var roles: [String]
        public var cliVersion: String?
        public var models: [Model]
        public var defaultModel, defaultEffort: String?
        public var atMs: UInt64

        public struct Model: Decodable, Equatable, Sendable, Identifiable {
            /// How it is written in an intent.
            public var model: String
            public var name: String
            public var about: String?
            /// None when its effort cannot be chosen.
            public var efforts: [String]?
            public var defaultEffort: String?

            public var id: String { model }
        }
    }
    public struct ModelsUnavailable: Decodable, Equatable, Sendable { public var vendor: String; public var why: Why; public var atMs: UInt64 }

    /// From its next turn on, a session is to use this. `ModelChanged` or `ModelNotChanged` tells what came of it.
    public struct ModelRequested: Decodable, Equatable, Sendable { public var session, model: String; public var effort: String?; public var atMs: UInt64 }
    public struct ModelChanged: Decodable, Equatable, Sendable { public var session, model: String; public var effort: String?; public var atMs: UInt64 }
    /// It goes on with what it used.
    public struct ModelNotChanged: Decodable, Equatable, Sendable {
        public var session, model: String
        public var effort: String?
        public var why: Why
        public var atMs: UInt64
    }

    public struct TurnStopRequested: Decodable, Equatable, Sendable { public var turn, session: String; public var atMs: UInt64 }
    /// It did not stop in time and goes on.
    public struct TurnNotStopped: Decodable, Equatable, Sendable { public var turn, session: String; public var atMs: UInt64 }
}
