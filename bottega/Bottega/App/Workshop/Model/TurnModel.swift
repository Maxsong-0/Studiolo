// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import BottegaKit
import Observation

/// What is known of the inside of one turn. Each its own object: they change up to thirty times a second, and only
/// what shows this one should notice.
@MainActor @Observable
final class TurnModel: Identifiable {
    let id: String
    var details = TurnDetails()
    var isFetching = false
    /// Why the details could not be had.
    var problem: Why?

    init(id: String) { self.id = id }
}
