// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation
import Synchronization

extension CrewRoster {
    /// The roster as the app reads it: the built-in table (`builtInPersonas`) in the colours
    /// the theme gives the hands, where it gives any. Names, flags and order are always the
    /// roster's own, since the prompts and the hands' branches go by the name. Behind a lock:
    /// personas are made on whatever thread builds a prompt.
    static var personas: [CrewPersona] {
        themedRoster.withLock { $0.personas } ?? builtInPersonas
    }

    /// Goes up whenever the colours change, so a list of personas kept for later can tell it
    /// is out of date (see `TimelineBlockCache.mentionPersonas`).
    static var colorRevision: Int {
        themedRoster.withLock { $0.revision }
    }

    /// Recolours the roster: colours as 0xRRGGBB by lowercased roster name, the built-in one
    /// wherever a name has none, so an empty table puts the roster's own back. The theme layer
    /// hands them down on every theme change (see `ThemeManager.refresh`).
    static func setColors(_ colors: [String: UInt32]) {
        themedRoster.withLock { state in
            guard colors != state.colors else { return }
            state.colors = colors
            state.personas = colors.isEmpty ? nil : builtInPersonas.map { persona in
                colors[persona.name.lowercased()].map { CrewPersona(persona.name, persona.asset, $0) } ?? persona
            }
            state.revision += 1
        }
    }

    private struct ThemedRoster {
        var colors: [String: UInt32] = [:]
        /// Nil while the theme gives no colours of its own.
        var personas: [CrewPersona]?
        var revision = 0
    }

    private static let themedRoster = Mutex(ThemedRoster())
}
