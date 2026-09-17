import Foundation

/// Renders a short struck tone to 16-bit mono WAV data, so the app's chimes ship as a few
/// numbers rather than sound files and stay tunable right where they are declared.
enum ToneSynth {
    /// A note of a chime: when it starts and how strongly it rings, relative to the others.
    struct Note {
        var frequency: Double
        var onset: Double
        var level: Double
    }

    /// One sine in a note's tone: its frequency as a multiple of the fundamental, how loud it
    /// starts, and how quickly it fades (the seconds it takes to fall by about two thirds).
    struct Partial {
        var multiple: Double
        var level: Double
        var decay: Double
    }

    private static let sampleRate = 44_100.0
    /// A raised-cosine onset: quick enough to feel struck, with no click.
    private static let attack = 0.006
    /// The tail is inaudible by the end; the fade makes sure it stops without a tick.
    private static let release = 0.08

    static func wave(notes: [Note], partials: [Partial], duration: Double) -> Data {
        let count = Int(sampleRate * duration)
        var samples = [Double](repeating: 0, count: count)
        for note in notes {
            let start = Int(note.onset * sampleRate)
            for index in start..<count {
                let time = Double(index - start) / sampleRate
                let onset = time < attack ? 0.5 - 0.5 * cos(.pi * time / attack) : 1
                let tone = partials.reduce(0.0) { sum, partial in
                    sum + partial.level * exp(-time / partial.decay) * sin(2 * .pi * note.frequency * partial.multiple * time)
                }
                samples[index] += note.level * onset * tone
            }
        }
        let fade = Int(release * sampleRate)
        for index in (count - fade)..<count {
            samples[index] *= Double(count - index) / Double(fade)
        }
        let peak = samples.reduce(0) { max($0, abs($1)) }
        let scale = peak > 0 ? 0.9 / peak : 0

        var data = Data(capacity: 44 + count * 2)
        func append<Value: FixedWidthInteger>(_ value: Value) {
            withUnsafeBytes(of: value.littleEndian) { data.append(contentsOf: $0) }
        }
        data.append(contentsOf: Array("RIFF".utf8))
        append(UInt32(36 + count * 2))
        data.append(contentsOf: Array("WAVE".utf8))
        data.append(contentsOf: Array("fmt ".utf8))
        append(UInt32(16))
        append(UInt16(1)) // PCM
        append(UInt16(1)) // mono
        append(UInt32(sampleRate))
        append(UInt32(sampleRate) * 2) // bytes per second
        append(UInt16(2)) // bytes per frame
        append(UInt16(16)) // bits per sample
        data.append(contentsOf: Array("data".utf8))
        append(UInt32(count * 2))
        for sample in samples {
            append(Int16((sample * scale * Double(Int16.max)).rounded()))
        }
        return data
    }
}
