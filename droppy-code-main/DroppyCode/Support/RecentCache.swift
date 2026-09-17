import Foundation

struct RecentCache<Key: Hashable, Value> {
    private struct Entry {
        var value: Value
        var cost: Int
    }

    private var current: [Key: Entry] = [:]
    private var previous: [Key: Entry] = [:]
    private var currentCost = 0
    private var previousCost = 0
    private let limit: Int
    private let costLimit: Int

    init(limit: Int, costLimit: Int = .max) {
        self.limit = max(1, limit)
        self.costLimit = max(0, costLimit)
        current.reserveCapacity(self.limit)
    }

    mutating func value(for key: Key) -> Value? {
        if let hit = current[key] { return hit.value }
        guard let hit = previous.removeValue(forKey: key) else { return nil }
        previousCost -= hit.cost
        insert(hit.value, for: key, cost: hit.cost)
        return hit.value
    }

    func contains(_ key: Key) -> Bool {
        current[key] != nil || previous[key] != nil
    }

    @discardableResult
    mutating func removeValue(for key: Key) -> Value? {
        let currentEntry = current.removeValue(forKey: key)
        let previousEntry = previous.removeValue(forKey: key)
        currentCost -= currentEntry?.cost ?? 0
        previousCost -= previousEntry?.cost ?? 0
        return currentEntry?.value ?? previousEntry?.value
    }

    mutating func insert(_ value: Value, for key: Key, cost: Int = 1) {
        removeValue(for: key)
        let cost = max(0, cost)
        guard cost <= costLimit else { return }
        if current.count >= limit || currentCost > costLimit - cost {
            previous = current
            previousCost = currentCost
            current = [:]
            current.reserveCapacity(limit)
            currentCost = 0
        }
        current[key] = Entry(value: value, cost: cost)
        currentCost += cost
    }
}
