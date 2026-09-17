import AppKit

/// The soft two-note chime that marks a turn finishing: a rising fifth with a celesta-like
/// tone, synthesized on first use so no sound file ships with the app and the tone is
/// tunable right here.
@MainActor
enum FinishChime {
    /// How loud the chime plays relative to the system volume. Well under a stock alert:
    /// it registers without cutting into whatever else is playing.
    private static let volume: Float = 0.35

    private static let sound: NSSound? = {
        let sound = NSSound(data: ToneSynth.wave(notes: notes, partials: partials, duration: 0.7))
        sound?.volume = volume
        return sound
    }()

    static func play() {
        // NSSound declines a play while one is in flight, so several threads finishing in
        // the same moment make one chime, never a pile-up.
        sound?.play()
    }

    /// D5 up to A5, the second note a beat later and a touch stronger, the way "done" lands.
    private static let notes = [
        ToneSynth.Note(frequency: 587.33, onset: 0, level: 0.8),
        ToneSynth.Note(frequency: 880, onset: 0.14, level: 1),
    ]

    /// The fundamental carries the tone; the octave and double octave add a brief glint at
    /// the strike and are gone within a tenth of a second, which keeps it soft rather than pingy.
    private static let partials = [
        ToneSynth.Partial(multiple: 1, level: 1, decay: 0.14),
        ToneSynth.Partial(multiple: 2, level: 0.22, decay: 0.06),
        ToneSynth.Partial(multiple: 4, level: 0.08, decay: 0.04),
    ]
}
