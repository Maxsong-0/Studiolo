import Foundation

// A pair set up in Settings is somewhere a chat goes, not a rule it falls under: with Hydra
// on, the composer's model picker lists every pair that is ready to run, and one tap puts
// the chat in it. Entering a pair is the same move as choosing a model, with the pair's
// lead model and effort instead of a single model and the pair pinned to the chat; a chat
// made from it carries the pair along. Choosing a model from the rows below the pairs is
// the move back out: the chat is on that model alone, its heads on it too, Hydra as it was.

extension AppModel {
    /// The pairs the model picker offers: the ones whose provider is installed and
    /// switched on, in the order they were set up.
    var hydraPickerPairs: [HydraPair] {
        settings.hydraPairs.filter { providers.status($0.provider).isInstalled && settings.isEnabled($0.provider) }
    }

    /// Whether a chat leads with this pair right now: Hydra on, and the pair the one picked
    /// for the chat, which is still on its provider. Exactly one pair, or none, at a time.
    func leadsWithHydraPair(_ pair: HydraPair, thread: ChatThread) -> Bool {
        hydraIsOn(thread) && thread.hydraPairID == pair.id && thread.provider == pair.provider
    }

    /// Puts a chat in a pair from the model picker: it switches to the pair's provider and
    /// lead model and effort, pins the pair so it keeps leading with this one, and Hydra
    /// goes on for the app. The session is left to restart itself: the runtime launches the
    /// next turn with the pair's head definitions on its own.
    func enterHydraPair(_ pair: HydraPair, for threadID: UUID) {
        guard let thread = thread(threadID) else { return }
        let provider = pair.provider
        let switchesProvider = provider != thread.provider
        let lead = hydraLead(of: pair, for: thread)
        updateThread(threadID) { thread in
            if switchesProvider {
                thread.provider = provider
                thread.providerSessionID = nil
            }
            thread.model = lead.model
            thread.effort = lead.effort
            thread.fastMode = lead.fastMode
            thread.hydraPairID = pair.id
            thread.hydraEnabled = true
        }
        // A chat that changes provider leaves its old session behind, as a model choice does.
        if switchesProvider { existingRuntime(for: threadID)?.stopSession() }
        settings.remember(model: lead.model, effort: lead.effort, for: provider)
        settings.defaultProvider = provider
        settings.lastHydraPairID = pair.id
        settings.hydraEnabled = true
        settings.hydraDefaultEnabled = true
    }

    /// The model, effort and fast mode a chat runs on as a pair's lead.
    func hydraLead(of pair: HydraPair, for thread: ChatThread) -> (model: String?, effort: String?, fastMode: Bool) {
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
    /// conversation, so it stays on its own model, Hydra as it was, out of the pair.
    func updateHydraPair(_ id: UUID, _ change: (inout HydraPair) -> Void) {
        let before = settings.hydraPair(id)
        settings.updateHydraPair(id, change)
        guard let pair = settings.hydraPair(id), pair != before else { return }
        for thread in threads where thread.hydraPairID == id && hydraIsOn(thread) {
            guard pair.provider == thread.provider else {
                updateThread(thread.id) { $0.hydraPairID = nil }
                continue
            }
            let lead = hydraLead(of: pair, for: thread)
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
    /// that is gone: each stays on its own model, Hydra as it was.
    func removeHydraPair(_ id: UUID) {
        settings.removeHydraPair(id)
        if settings.lastHydraPairID == id { settings.lastHydraPairID = nil }
        for thread in threads where thread.hydraPairID == id {
            updateThread(thread.id) { $0.hydraPairID = nil }
        }
    }

    /// Takes a chat out of the pair it leads with, when a model is chosen from the picker's
    /// model rows: the reverse of `enterHydraPair`. The chat is on that model alone, with
    /// no pair's mark on its chip; Hydra stays as Settings has it, its heads on the chat's
    /// own model and effort until a pair is picked again.
    func leaveHydraPair(for threadID: UUID) {
        updateThread(threadID) { $0.hydraPairID = nil }
        settings.lastHydraPairID = nil
    }
}
