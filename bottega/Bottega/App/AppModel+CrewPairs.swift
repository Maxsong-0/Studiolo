// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation

// A pair set up in Settings is somewhere a chat goes, not a rule it falls under: with Crew
// on, the composer's model picker lists every pair that is ready to run, and one tap puts
// the chat in it. Entering a pair is the same move as choosing a model, with the pair's
// lead model and effort instead of a single model and the pair pinned to the chat; a chat
// made from it carries the pair along. Choosing a model from the rows below the pairs is
// the move back out: the chat is on that model alone, its hands on it too, Crew as it was.

extension AppModel {
    /// The pairs the model picker offers: the ones whose provider is installed and
    /// switched on, in the order they were set up. A provider no check has looked at yet
    /// counts as installed (`countsAsInstalled`): the chat a launch opens is built before
    /// the providers are probed, and it must still find the pair it was last worked in.
    var crewPickerPairs: [CrewPair] {
        settings.crewPairs.filter { providers.countsAsInstalled($0.provider) && settings.isEnabled($0.provider) }
    }

    /// Whether a chat leads with this pair right now: Crew on, and the pair the one picked
    /// for the chat, which is still on its provider. Exactly one pair, or none, at a time.
    func leadsWithCrewPair(_ pair: CrewPair, thread: ChatThread) -> Bool {
        crewIsOn(thread) && thread.crewPairID == pair.id && thread.provider == pair.provider
    }

    /// Puts a chat in a pair from the model picker: it switches to the pair's provider and
    /// lead model and effort, pins the pair so it keeps leading with this one, and Crew
    /// goes on for the app. New chats still start as "New chats lead a team" in Settings ›
    /// Crew has them: entering a pair is this chat's move. The session is left to restart
    /// itself: the runtime launches the next turn with the pair's hand definitions on its own.
    func enterCrewPair(_ pair: CrewPair, for threadID: UUID) {
        guard let thread = thread(threadID) else { return }
        let provider = pair.provider
        let switchesProvider = provider != thread.provider
        let lead = crewLead(of: pair, for: thread)
        updateThread(threadID) { thread in
            if switchesProvider {
                thread.provider = provider
                thread.providerSessionID = nil
            }
            thread.model = lead.model
            thread.effort = lead.effort
            thread.fastMode = lead.fastMode
            thread.crewPairID = pair.id
            thread.crewEnabled = true
        }
        // A chat that changes provider leaves its old session behind, as a model choice does.
        if switchesProvider { existingRuntime(for: threadID)?.stopSession() }
        settings.remember(model: lead.model, effort: lead.effort, for: provider)
        settings.defaultProvider = provider
        settings.lastCrewPairID = pair.id
        settings.crewEnabled = true
    }

    /// The model, effort and fast mode a chat runs on as a pair's lead.
    func crewLead(of pair: CrewPair, for thread: ChatThread) -> (model: String?, effort: String?, fastMode: Bool) {
        let provider = pair.provider
        let switchesProvider = provider != thread.provider
        // A pair with no lead model runs on any model, so the chat keeps the one it is on;
        // arriving from another provider there is nothing to keep and it takes that
        // provider's default.
        let option: ModelOption? = if let lead = pair.orchestratorModel {
            providers.model(lead, for: provider)
        } else if switchesProvider {
            providers.defaultModel(for: provider)
        } else {
            providers.model(thread.model, for: provider)
        }
        // The pair's lead model is the chat's even before the provider's catalog lists it:
        // the chip shows the id until the list arrives, rather than staying on the model
        // the chat had, with the pair's checkmark on a model it is not on.
        let modelID = option?.id ?? pair.orchestratorModel ?? (switchesProvider ? nil : thread.model)
        let preference = settings.preference(for: provider, model: modelID)
        // The pair's own effort wins. Without one the chat picks up what this model was
        // last left on, the way choosing the model by hand does.
        let effort: String? = if let chosen = pair.orchestratorEffort, option?.efforts.contains(chosen) ?? true {
            chosen
        } else {
            preference.effort.flatMap { (option?.efforts.contains($0) ?? false) ? $0 : nil }
        }
        return (modelID, effort, (option?.supportsFast ?? false) && preference.fastMode)
    }

    /// Edits a pair in Settings and moves the chats leading with it along: the pair is
    /// where those chats are, so a new lead model or effort is theirs at once, the way
    /// entering the pair set them, and the chip says so. A pair whose lead provider
    /// changes lets its chats go instead: a chat does not change provider under a
    /// conversation, so it stays on its own model, Crew as it was, out of the pair.
    func updateCrewPair(_ id: UUID, _ change: (inout CrewPair) -> Void) {
        let before = settings.crewPair(id)
        settings.updateCrewPair(id, change)
        guard let pair = settings.crewPair(id), pair != before else { return }
        for thread in threads where thread.crewPairID == id && crewIsOn(thread) {
            guard pair.provider == thread.provider else {
                updateThread(thread.id) { $0.crewPairID = nil }
                continue
            }
            let lead = crewLead(of: pair, for: thread)
            guard lead.model != thread.model || lead.effort != thread.effort || lead.fastMode != thread.fastMode else { continue }
            updateThread(thread.id) {
                $0.model = lead.model
                $0.effort = lead.effort
                $0.fastMode = lead.fastMode
            }
            settings.remember(model: lead.model, effort: lead.effort, for: pair.provider)
        }
    }

    /// Removes a pair and takes every chat out of it, so no chat keeps pointing at a pair
    /// that is gone: each stays on its own model, Crew as it was.
    func removeCrewPair(_ id: UUID) {
        settings.removeCrewPair(id)
        if settings.lastCrewPairID == id { settings.lastCrewPairID = nil }
        for thread in threads where thread.crewPairID == id {
            updateThread(thread.id) { $0.crewPairID = nil }
        }
        // A project rule that names the removed pair is gone too. The project's new chats
        // start on the remembered pair again.
        for project in projects where project.crewPairID == id {
            updateProject(project.id) { $0.crewPairID = nil }
        }
    }

    /// Sets the pair that each new chat in a project starts on. Nil removes the rule.
    /// Open chats do not change: the rule applies to new chats only.
    func setCrewPair(_ pairID: UUID?, forProject projectID: UUID) {
        updateProject(projectID) { $0.crewPairID = pairID }
    }

    /// Takes a chat out of the pair it leads with, when a model is chosen from the picker's
    /// model rows: the reverse of `enterCrewPair`. The chat is on that model alone, with
    /// no pair's mark on its chip; Crew stays as Settings has it, its hands on the chat's
    /// own model and effort until a pair is picked again.
    func leaveCrewPair(for threadID: UUID) {
        updateThread(threadID) { $0.crewPairID = nil }
        settings.lastCrewPairID = nil
    }
}
