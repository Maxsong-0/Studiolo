// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation

/// A provider saying the account's usage limit is spent, and when it comes back.
enum UsageLimitSignal {
    /// Whether an error message is the provider refusing to work until a usage limit resets.
    static func matches(_ text: String) -> Bool {
        let lower = text.lowercased()
        return lower.contains("usage limit") || lower.contains("hit your limit") || lower.contains("usage_limit")
            || lower.contains("limit reached") || lower.contains("out of extra usage")
    }

    /// The reset time an error message carries, when it does: a Unix timestamp after a bar
    /// ("usage limit reached|1757865600"), or a clock time such as "resets 3pm" or "resets at
    /// 3:30 pm", taken as the next such time from now.
    static func resetTime(in text: String, now: Date = .now) -> Date? {
        if let match = text.range(of: #"\|\s*(\d{9,})"#, options: .regularExpression) {
            let digits = text[match].drop { !$0.isNumber }
            if let seconds = TimeInterval(digits) {
                // Some providers write milliseconds.
                return Date(timeIntervalSince1970: seconds > 1e11 ? seconds / 1_000 : seconds)
            }
        }
        if let match = text.range(of: #"(?i)resets?(?:\s+at)?\s+(\d{1,2})(?::(\d{2}))?\s*(am|pm)"#, options: .regularExpression) {
            let clock = String(text[match])
            let numbers = clock.components(separatedBy: CharacterSet.decimalDigits.inverted).filter { !$0.isEmpty }
            guard let hourText = numbers.first, var hour = Int(hourText) else { return nil }
            let minute = numbers.count > 1 ? Int(numbers[1]) ?? 0 : 0
            let isPM = clock.lowercased().hasSuffix("pm")
            if isPM, hour < 12 { hour += 12 }
            if !isPM, hour == 12 { hour = 0 }
            var calendar = Calendar.current
            if let zone = text.range(of: #"\(([A-Za-z_]+/[A-Za-z_]+)\)"#, options: .regularExpression),
               let timeZone = TimeZone(identifier: text[zone].trimmingCharacters(in: CharacterSet(charactersIn: "()"))) {
                calendar.timeZone = timeZone
            }
            let components = DateComponents(hour: hour, minute: minute)
            return calendar.nextDate(after: now, matching: components, matchingPolicy: .nextTime)
        }
        return nil
    }

    /// "3:31 PM" today, "tomorrow at 9:00 AM" or "Thursday at 9:00 AM" further out.
    static func format(_ date: Date) -> String {
        let calendar = Calendar.current
        let time = date.formatted(date: .omitted, time: .shortened)
        if calendar.isDateInToday(date) { return time }
        if calendar.isDateInTomorrow(date) { return "tomorrow at \(time)" }
        if let week = calendar.date(byAdding: .day, value: 6, to: .now), date < week {
            return "\(date.formatted(.dateTime.weekday(.wide))) at \(time)"
        }
        return date.formatted(date: .abbreviated, time: .shortened)
    }
}
