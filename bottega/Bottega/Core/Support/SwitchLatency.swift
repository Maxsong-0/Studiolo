// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation
import os

/// Logs how long the main thread stays busy after a thread is selected, in the dev app:
/// the span from the selection to the run loop's next sleep (what the click waits on), then
/// the busy total over the following second. Read with
/// `log stream --predicate 'subsystem == "<the app's bundle id>" && category == "switch"'`.
/// Each switch is also a point of interest, so an Instruments trace can be cut at it.
@MainActor
enum SwitchLatency {
    private static let log = Logger(subsystem: AppInfo.logSubsystem, category: "switch")
    private static let pointsOfInterest = OSLog(subsystem: AppInfo.logSubsystem, category: .pointsOfInterest)
    private static var switchStart: CFAbsoluteTime = 0
    private static var busySince: CFAbsoluteTime = 0
    private static var firstSpan: Double?
    private static var busyTotal: Double = 0
    private static var label = ""
    private static var observer: CFRunLoopObserver?

    static func start() {
        guard AppInfo.isDevelopment, observer == nil else { return }
        let activities = CFRunLoopActivity.afterWaiting.rawValue | CFRunLoopActivity.beforeWaiting.rawValue
        observer = CFRunLoopObserverCreateWithHandler(nil, activities, true, 0) { _, activity in
            let now = CFAbsoluteTimeGetCurrent()
            if activity == .afterWaiting {
                busySince = now
                return
            }
            guard switchStart > 0 else { return }
            let span = (now - max(busySince, switchStart)) * 1000
            if firstSpan == nil { firstSpan = span }
            busyTotal += span
            if now - switchStart >= 1 {
                log.notice("switch to \(label, privacy: .public): blocked \(Int(firstSpan ?? 0)) ms, busy \(Int(busyTotal)) ms in 1 s")
                switchStart = 0
            }
        }
        CFRunLoopAddObserver(CFRunLoopGetMain(), observer, .commonModes)
    }

    /// A line beside the switch timings, in the dev app only.
    static func note(_ message: String) {
        guard observer != nil else { return }
        log.notice("\(message, privacy: .public)")
    }

    /// The selection just changed: the next spans are its cost.
    static func noteSwitch(to title: String) {
        guard observer != nil else { return }
        if switchStart > 0 { log.notice("switch to \(label, privacy: .public): blocked \(Int(firstSpan ?? 0)) ms, busy \(Int(busyTotal)) ms before the next switch") }
        os_signpost(.event, log: pointsOfInterest, name: "switch", "%{public}s", title)
        switchStart = CFAbsoluteTimeGetCurrent()
        busySince = switchStart
        firstSpan = nil
        busyTotal = 0
        label = String(title.prefix(24))
    }
}
