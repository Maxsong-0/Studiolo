// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import AppKit
import SwiftUI
import Testing

/// What comes over the top of a page must not move the buttons under it (Studiolo T-240).
@MainActor
struct FloatingTopTests {
    /// Where a marker in the page is, from the top, before something comes over it, while it is there, and after.
    private func positions(makesRoom: Bool) -> [CGFloat] {
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 400, height: 300), styleMask: [.borderless], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        let marker = NSView()
        let hosting = NSHostingView(rootView: Sample(marker: marker, makesRoom: makesRoom, isCovered: false))
        window.contentView = hosting
        return [false, true, false].map { isCovered in
            hosting.rootView = Sample(marker: marker, makesRoom: makesRoom, isCovered: isCovered)
            // Twice and a turn of the run loop: making room follows from a measurement.
            for _ in 0..<2 {
                hosting.layoutSubtreeIfNeeded()
                RunLoop.main.run(until: Date().addingTimeInterval(0.05))
            }
            return 300 - marker.convert(marker.bounds, to: nil).maxY
        }
    }

    @Test func pageStaysWhereItIs() {
        let at = positions(makesRoom: false)
        #expect(at == [at[0], at[0], at[0]])
    }

    @Test func readOnlyPageMakesRoom() {
        let at = positions(makesRoom: true)
        #expect(at == [at[0], at[0] + 40, at[0]])
    }
}

private struct Sample: View {
    let marker: NSView
    let makesRoom: Bool
    let isCovered: Bool

    var body: some View {
        FloatingTop(makesRoom: makesRoom) {
            ScrollView {
                VStack(spacing: 0) {
                    Marker(view: marker).frame(width: 100, height: 20)
                    Color.clear.frame(height: 900)
                }
                .padding(.top, FloatingTopRoom.reserved)
            }
        } over: {
            if isCovered { Color.gray.frame(height: 40) }
        }
    }
}

private struct Marker: NSViewRepresentable {
    let view: NSView
    func makeNSView(context: Context) -> NSView { view }
    func updateNSView(_ view: NSView, context: Context) {}
}
