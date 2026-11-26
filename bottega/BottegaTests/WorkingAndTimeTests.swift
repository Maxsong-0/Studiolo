// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation
import Testing

/// The amber dots of an AI at work (Bottega-5 §7), and the short times in the sidebar.
struct WorkingAndTimeTests {
    @Test func aDotLightsAndSinksBack() {
        #expect(WorkingPattern.flash(0) == 1)
        #expect(abs(WorkingPattern.flash(0.1) - 0.75) < 1e-9)
        #expect(abs(WorkingPattern.flash(0.26) - 0.3) < 1e-9)
        #expect(WorkingPattern.flash(0.45) == WorkingPattern.rest)
        #expect(WorkingPattern.flash(0.99) == WorkingPattern.rest)
        // Halfway between two stops is halfway between their brightness.
        #expect(abs(WorkingPattern.flash(0.05) - 0.875) < 1e-9)
    }

    @Test func dotsAreNotInStep() {
        let at: TimeInterval = 1000
        let values = Set((0..<49).map { (WorkingPattern.opacity(of: $0, count: 49, at: at) * 1000).rounded() })
        #expect(values.count > 5)
        for index in 0..<49 {
            let value = WorkingPattern.opacity(of: index, count: 49, at: at)
            #expect(value >= WorkingPattern.rest && value <= 1)
        }
    }

    @Test func lessMotionHoldsAFewLit() {
        let seven = (0..<49).map { WorkingPattern.still(of: $0, side: 7) }
        #expect(seven.filter { $0 == 1 }.count == 6)
        #expect(seven.filter { $0 == 0.45 }.count == 6)
        let three = (0..<9).map { WorkingPattern.still(of: $0, side: 3) }
        #expect(three.filter { $0 == 1 }.count == 3)
        #expect(three.filter { $0 == WorkingPattern.rest }.count == 6)
    }

    @Test func shortTimes() {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = TimeZone(identifier: "UTC")!
        let now = Date(timeIntervalSince1970: 1_800_000_000) // a Friday
        func ago(_ seconds: Double) -> String {
            ShortTime.ago(UInt64((now.timeIntervalSince1970 - seconds) * 1000), now: now, calendar: calendar, locale: Locale(identifier: "en_US"))
        }
        #expect(ago(10) == "now")
        #expect(ago(59) == "now")
        #expect(ago(60) == "1m")
        #expect(ago(3599) == "59m")
        #expect(ago(3600) == "1h")
        #expect(ago(86_399) == "23h")
        #expect(ago(2 * 86_400) == "Wed")
        #expect(ago(30 * 86_400).hasPrefix("Dec"))
    }
}
