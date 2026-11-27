// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.

// What the "work finished" notification counts (vault Bottega-6, D10). It says how many and nothing else: no line,
// card or word of anyone's (Bottega-4, section 3).

extension WorkshopState {
    /// How many things finished between `earlier` and now that the user would want to hear of: cards done or failed,
    /// and turns that took in something the user said and ended other than by the user stopping them. A card's own
    /// agents are left out: their work ends with the card, which is counted already.
    public func finished(since earlier: WorkshopState) -> Int {
        let cards = cards.values.count { card in card.end != nil && earlier.cards[card.id]?.end == nil }
        let turns = turns.values.count { turn in
            guard let end = turn.end, earlier.turns[turn.id]?.end == nil else { return false }
            // Asked to stop, whatever came of it: the user knows.
            guard turn.stop == nil, end.outcome.code != "stopped_by_user" else { return false }
            return !isCardAgent(turn.session) && tookFromTheUser(turn)
        }
        return cards + turns
    }

    /// Pushed into it, or handed over while it ran (`turns(with:)` the other way round).
    private func tookFromTheUser(_ turn: Turn) -> Bool {
        messages.values.contains { letter in
            letter.isFromUser && letter.to == turn.session
                && (turn.messages.contains(letter.id) || letter.handedAt.map(turn.wasRunning(at:)) ?? false)
        }
    }
}
