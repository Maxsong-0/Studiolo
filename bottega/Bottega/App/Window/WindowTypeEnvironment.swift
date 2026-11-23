// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import SwiftUI

/// The main window's type from Settings: the text size every view scales its fonts by
/// (`chatZoom`), the faces the conversation's prose and code are set in (`chatTypefaces`)
/// and the terminal's own size (`terminalFontSize`). Applied at the root of the main window
/// and at the root of each chat column, which is hosted on its own (see `ThreadColumns`)
/// and so starts from an environment of its own. Never in the Settings window: its layout
/// is a fixed size, and it shows the choice on a sample of its own.
struct WindowTypeEnvironment: ViewModifier {
    let settings: AppSettings

    func body(content: Content) -> some View {
        content
            .environment(\.chatZoom, ChatZoom.scale(at: settings.chatZoom))
            .environment(\.chatTypefaces, settings.chatTypefaces)
            .environment(\.terminalFontSize, CGFloat(settings.terminalFontSize))
    }
}
