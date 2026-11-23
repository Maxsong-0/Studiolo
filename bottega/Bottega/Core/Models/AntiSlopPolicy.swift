// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
enum AntiSlopPolicy {
    static let enabledInstructions = """
    [Bottega anti-slop guidance]
    Give the shortest complete answer, leading with the result. Use plain, natural language. Skip filler, hype, stock openings, repeated summaries and unsolicited closers. Preserve requested detail, facts, uncertainty, blockers and required validation.
    Make focused edits using existing patterns. Add comments only for non-obvious reasons, constraints or pitfalls; omit banners, obvious narration and speculative TODOs. Preserve useful documentation. Avoid unrelated cleanup, dependencies and abstractions.
    For UI work, follow the product’s components, typography and colors. Prefer purposeful structure and real content; preserve accessibility, responsive layouts and visible loading, empty and error states.
    Reuse evidence, read relevant sections and batch independent lookups. Avoid repeated scans, polling and echoed logs. Perform required checks; brevity never excuses incomplete work. Apply this guidance to delegated work too. Report changes, affected files and material limits once. User requests and project requirements take precedence.
    """

    // Resumed conversations may still contain the enabled guidance.
    static let disabledInstructions = """
    [Bottega anti-slop guidance: off]
    Disregard earlier app-provided anti-slop guidance. Continue following user requests and project requirements.
    """

    static func instructions(enabled: Bool) -> String {
        enabled ? enabledInstructions : disabledInstructions
    }
}
