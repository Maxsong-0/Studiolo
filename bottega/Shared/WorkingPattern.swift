// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation

/// How bright each dot of the "AI is working" matrix is (Bottega-5 §7): each lights up fully, sinks back to a dim
/// brown and waits, on its own delay and one of three periods, so the pattern does not visibly repeat.
enum WorkingPattern {
    /// The dim the dots rest at.
    static let rest = 0.16

    static func opacity(of index: Int, count: Int, at time: TimeInterval) -> Double {
        let spread = (index * 37 + 11) % count
        let period = [2.0, 2.6, 3.3][spread % 3]
        let delay = Double(spread % 14) * 0.23
        let phase = (time + delay).truncatingRemainder(dividingBy: period) / period
        return flash(phase)
    }

    /// 0 → 1, 10 % → 0.75, 26 % → 0.3, 45 % and after → 0.16, straight lines between.
    static func flash(_ phase: Double) -> Double {
        let stops: [(Double, Double)] = [(0, 1), (0.1, 0.75), (0.26, 0.3), (0.45, rest)]
        for (start, end) in zip(stops, stops.dropFirst()) where phase < end.0 {
            let t = (phase - start.0) / (end.0 - start.0)
            return start.1 + (end.1 - start.1) * t
        }
        return rest
    }

    /// The one frame held with less motion: a few lit, a few half lit, as the design shows it.
    static func still(of index: Int, side: Int) -> Double {
        let lit: [Int: Double] = side == 3
            ? [1: 1, 4: 1, 6: 1]
            : [5: 1, 10: 0.45, 17: 1, 21: 1, 23: 0.45, 24: 1, 25: 0.45, 28: 1, 31: 0.45, 32: 1, 33: 0.45, 35: 0.45]
        return lit[index] ?? rest
    }
}
