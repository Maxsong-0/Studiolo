import AppKit

/// The small sound of a thread settling: two soft wooden notes stepping down, the way a
/// checkbox ticks and the row comes to rest. Quieter and shorter than the finish chime,
/// and the setting can silence it.
@MainActor
enum SettleChime {
    /// Barely there: a tick under the sidebar's motion, not a notification.
    private static let volume: Float = 0.18

    private static let sound: NSSound? = {
        let sound = NSSound(data: ToneSynth.wave(notes: notes, partials: partials, duration: 0.32))
        sound?.volume = volume
        return sound
    }()

    static func play() {
        // A replay from the top, so settling two threads in a row ticks twice.
        sound?.stop()
        sound?.play()
    }

    /// E5 down to C5, the second note right behind the first: a small landing.
    private static let notes = [
        ToneSynth.Note(frequency: 659.26, onset: 0, level: 0.7),
        ToneSynth.Note(frequency: 523.25, onset: 0.075, level: 1),
    ]

    /// A woody tone: the fundamental fades fast, a quiet third harmonic gives it a knock
    /// at the strike, and the octave only glints.
    private static let partials = [
        ToneSynth.Partial(multiple: 1, level: 1, decay: 0.075),
        ToneSynth.Partial(multiple: 3, level: 0.16, decay: 0.02),
        ToneSynth.Partial(multiple: 2, level: 0.08, decay: 0.03),
    ]
}
