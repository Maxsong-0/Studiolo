// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation

/// A prompt sent with one command instead of typed out: the user's concurrency SOP, as its
/// opening and closing checklists, for the lead to work through. Typed as `/sop` or
/// `/sop-close` in the chat box (the `/` list offers them on every provider), or picked in
/// the command palette. Either way the message goes out as the prompt it stands for, once
/// its turn starts, and stays with the lead: no hand is ever sent out with one.
enum PromptMacro: String, CaseIterable, Identifiable, Sendable {
    /// Read the coordination state, check for duplicates, claim, reserve numbers, verify.
    case sopOpening = "sop"
    /// Move the row, write back what was reserved, archive, write the log, refresh INDEX.
    case sopClosing = "sop-close"

    var id: String { rawValue }

    /// The command that sends it, slash and all.
    var command: String { "/" + rawValue }

    var title: String {
        switch self {
        case .sopOpening: "SOP: opening checklist"
        case .sopClosing: "SOP: closing checklist"
        }
    }

    /// One line on what it sends, for the `/` list and the palette.
    var detail: String {
        switch self {
        case .sopOpening: "Sends the lead the SOP's opening checklist"
        case .sopClosing: "Sends the lead the SOP's closing checklist"
        }
    }

    var symbol: String {
        switch self {
        case .sopOpening: "checklist"
        case .sopClosing: "checklist.checked"
        }
    }

    /// The prompt itself, in Chinese, the language the user works in.
    var prompt: String {
        switch self {
        case .sopOpening: Self.openingPrompt
        case .sopClosing: Self.closingPrompt
        }
    }

    /// A message that leads with a macro's command, read back into the prompt it stands for.
    struct Invocation: Sendable {
        let macro: PromptMacro
        /// What the user wrote after the command, trimmed; empty when nothing was.
        let note: String

        /// Nil unless the message's first word is a macro's command.
        init?(_ text: String) {
            let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
            guard trimmed.hasPrefix("/"), let name = SlashCommand.name(in: trimmed),
                  let macro = PromptMacro(rawValue: name) else { return nil }
            self.macro = macro
            note = String(trimmed.dropFirst(1 + name.count)).trimmingCharacters(in: .whitespacesAndNewlines)
        }

        /// What goes to the agent in the message's place: the macro's prompt, with the words
        /// the user added after the command under it.
        var prompt: String {
            note.isEmpty ? macro.prompt : macro.prompt + "\n\n补充说明：\n" + note
        }

        /// What a chat started this way is named after: the user's own words, else the macro.
        var titleSource: String {
            note.isEmpty ? macro.title : note
        }
    }

    private static let openingPrompt = """
    按并发协作 SOP 开工。下面五步做完之前，不要改任何代码或共享文件：

    1. 读协调状态：有 Studiolo 路牌就先取路牌（handshake），以它为权威状态；没有就读项目的 00-总览/INDEX.md、最新一篇日志和 04-协调/TASKS.md。
    2. 查重：用任务关键词（功能名、编号、路径、表名、实体名）搜「进行中」和「已完成」。命中就三选一：接续、换一个未认领项、协商划界，绝不重复做。
    3. 细粒度认领：在「进行中」加一行（线名 | 任务 | 要碰的文件 | 状态 | 日期），每行细到可独立验收，不写「剩余项」「相关优化」这类模糊认领。
    4. 占号：要用全局递增编号（迁移版本号、T 编号等）就先在「占用与编号登记」取 max+1 并登记，写入那一刻再扫一遍防撞号，不填历史空号。
    5. 核实现状：动手前用 grep、ls 或查询核对当前状态，不信几小时前的计划或审计快照；要动的模块先搜 06-BUG/ 有无前科。

    五步做完后，用几行告诉我查重结论、认领的行和占的号，再开始干活。项目没有这些档案就说明一声并跳过对应步骤，不要凭空新建。
    """

    private static let closingPrompt = """
    按并发协作 SOP 收尾，逐项做完再回复：

    1. 移行：把自己在 TASKS「进行中」的行移到「已完成」，写上关键文件和一句验证结论。
    2. 回写占用：在「占用与编号登记」回写自己占的编号、文件和资源的状态，释放锁。
    3. 归档：计划和待办文件用同一文件改名归档进 04-协调/归档/YYYY-MM-DD-任务名.md，不另存一份；改名断了链接就同步改引用方。
    4. 写日志：在 02-日志/ 写当日日志（做了什么、为什么、改动文件、踩坑、下一步），一天多次就追加小节，日期一律写绝对日期 YYYY-MM-DD。
    5. 刷 INDEX：刷新 00-总览/INDEX.md 的最近日志、未关闭 BUG 和当前重点。

    做完用几行告诉我每一步的结果。项目没有这些档案就跳过对应步骤，不要凭空新建。
    """
}
