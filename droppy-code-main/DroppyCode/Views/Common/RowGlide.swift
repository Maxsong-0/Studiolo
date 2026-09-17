import AppKit
import SwiftUI

/// A thread row's glide to its new place in the sidebar: down to the settled rows as it
/// settles, back up among the open ones as it reopens.
///
/// The row itself is swapped for its new form the moment the thread changes; what travels
/// is a ghost. It leaves the old row's frame looking like the old row and lands on the new
/// row's frame looking like the new one, crossfading on the way, with the green check
/// gliding from one end of the row to the other. When the new place is out of view the
/// ghost heads for the list's edge instead and slips out of it.
@MainActor
@Observable
final class RowGlideAnimator {
    static let shared = RowGlideAnimator()

    /// The motion of the glide and of the rows making room for it, so they land together.
    nonisolated static let spring = Spring(response: 0.6, dampingRatio: 0.86)

    struct Glide: Identifiable {
        enum Direction {
            case settle
            case reopen
        }

        let id = UUID()
        let threadID: UUID
        let direction: Direction
        /// The row's face as it looked where it left, and as it looks where it lands.
        let departure: NSImage
        let arrival: NSImage
        /// The fill under the face at each end: the hover or selection the row had, and
        /// the one its new row has.
        let departureFill: Double
        let arrivalFill: Double
        let from: CGRect
        var to: CGRect
        /// The list's visible frame, which the flight is clipped to.
        let bounds: CGRect
        let launchedAt = Date.now
        /// Landing is close: the real row shows again under the ghost's last frames.
        var hasArrived = false
        /// The new row reported its frame, so the ghost lands on a real row; otherwise
        /// it is headed for the fallback and fades out on the way.
        var hasLanded = false
    }

    private(set) var glides: [Glide] = []
    /// Whether each thread's row is out of sight under its ghost, one cell per thread: a
    /// row reads its own cell (see `isHiding`), never `glides`, so a launch, landing or
    /// finish re-renders the two rows of that glide and the layer, not every row in the
    /// list. Cells are kept once made: a row observes the cell it read, so a fresh one
    /// would flip out of its sight.
    @ObservationIgnored private var hidingCells: [UUID: ObservedValue<Bool>] = [:]

    /// The `Settled` header's frame in the window, reported by the header while it is
    /// on screen; a settle with the section folded lands on it.
    @ObservationIgnored private(set) var settledHeaderFrame: CGRect?

    func noteSettledHeader(_ frame: CGRect?) { settledHeaderFrame = frame }

    /// A landing frame counts through the first half second of the flight, from the
    /// thread's arriving row only; the new row lays out in the pass after the change,
    /// but a busy pass can be later, and its position converges as the list closes up,
    /// so the latest report inside the window wins; the interpolation below absorbs
    /// the re-targeting.
    private static let landingWindow: TimeInterval = 0.5

    /// Starts a glide from `from`, headed for `fallback` (just past the list's edge) until
    /// the new row reports where it really is. Nothing flies with Reduce Motion on.
    func launch(
        threadID: UUID,
        direction: Glide.Direction,
        from: CGRect,
        fallback: CGRect,
        bounds: CGRect,
        departure: NSImage,
        arrival: NSImage,
        departureFill: Double,
        arrivalFill: Double
    ) {
        guard from.width > 1, from.height > 1, !NSWorkspace.shared.accessibilityDisplayShouldReduceMotion else { return }
        glides.removeAll { $0.threadID == threadID }
        glides.append(Glide(
            threadID: threadID, direction: direction, departure: departure, arrival: arrival,
            departureFill: departureFill, arrivalFill: arrivalFill, from: from, to: fallback, bounds: bounds
        ))
        setHiding(threadID, true)
    }

    /// The new row is on screen at `frame`: the ghost lands there.
    func land(threadID: UUID, settled: Bool, at frame: CGRect) {
        guard let index = glides.firstIndex(where: { $0.threadID == threadID }),
              Date.now.timeIntervalSince(glides[index].launchedAt) < Self.landingWindow,
              glides[index].direction == (settled ? .settle : .reopen) else { return }
        glides[index].to = frame
        glides[index].hasLanded = true
    }

    /// Whether the thread's row should stay out of sight: its ghost is in the air. Read
    /// through the thread's own cell, made on the first ask; making it observes nothing.
    func isHiding(_ threadID: UUID) -> Bool {
        hidingCell(threadID).value
    }

    private func hidingCell(_ threadID: UUID) -> ObservedValue<Bool> {
        if let cell = hidingCells[threadID] { return cell }
        let cell = ObservedValue(false)
        hidingCells[threadID] = cell
        return cell
    }

    /// Flips a thread's cell only when the answer changes, so a row is never re-rendered
    /// for a write that said what it already showed.
    private func setHiding(_ threadID: UUID, _ hiding: Bool) {
        let cell = hidingCell(threadID)
        if cell.value != hiding { cell.value = hiding }
    }

    /// Landing is close (settlingDuration − 0.12 s): the row shows again under the ghost's
    /// last frames, and the ghost goes 0.12 s later in `finish`.
    func arrive(_ id: UUID) {
        guard let index = glides.firstIndex(where: { $0.id == id }) else { return }
        glides[index].hasArrived = true
        setHiding(glides[index].threadID, false)
    }

    func finish(_ id: UUID) {
        guard let index = glides.firstIndex(where: { $0.id == id }) else { return }
        let threadID = glides[index].threadID
        glides.remove(at: index)
        // A thread with no ghost left in the air shows; one relaunched meanwhile stays hidden.
        setHiding(threadID, glides.contains { $0.threadID == threadID && !$0.hasArrived })
    }

    /// A row's face as a flat image at `size`, for a ghost.
    func render<Face: View>(_ face: Face, size: CGSize, colorScheme: ColorScheme) -> NSImage? {
        let renderer = ImageRenderer(content: face
            .frame(width: size.width, height: size.height, alignment: .topLeading)
            .environment(\.colorScheme, colorScheme))
        renderer.scale = NSScreen.main?.backingScaleFactor ?? 2
        return renderer.nsImage
    }
}

/// The window-wide layer the glides draw in. It never takes clicks.
struct RowGlideLayer: View {
    var body: some View {
        GeometryReader { proxy in
            ZStack(alignment: .topLeading) {
                ForEach(RowGlideAnimator.shared.glides) { glide in
                    RowGlideView(glide: glide, canvas: proxy.size)
                }
            }
            .frame(width: proxy.size.width, height: proxy.size.height, alignment: .topLeading)
        }
        .allowsHitTesting(false)
        .accessibilityHidden(true)
    }
}

private struct RowGlideView: View {
    let glide: RowGlideAnimator.Glide
    let canvas: CGSize

    @State private var hasStarted = false

    /// Where the check sits in a row, measured from the row's edge to the check's centre:
    /// beside the ellipsis at the trailing end while the thread is open, at the front once
    /// it has settled (see SidebarThreadRow).
    /// Read from the keyframe closure, which runs off the main actor.
    private nonisolated static let trailingCheckInset: CGFloat = 36
    private nonisolated static let leadingCheckInset: CGFloat = 16

    var body: some View {
        let from = glide.from
        let to = glide.to
        let shape = RoundedRectangle(cornerRadius: Chrome.rowCornerRadius, style: .continuous)
        Color.clear
            .keyframeAnimator(initialValue: 0.0, trigger: hasStarted) { _, progress in
                let p = progress
                let frame = CGRect(
                    x: from.minX + (to.minX - from.minX) * p,
                    y: from.minY + (to.minY - from.minY) * p,
                    width: from.width + (to.width - from.width) * p,
                    height: from.height + (to.height - from.height) * p
                )
                // The lift comes up as the row takes off and is gone by the time it lands.
                let lift = min(Self.smoothstep(0, 0.12, p), 1 - Self.smoothstep(0.7, 1, p))
                let fill = max(glide.departureFill + (glide.arrivalFill - glide.departureFill) * p, 0.12 * lift)
                let settles = glide.direction == .settle
                let checkTravel = Self.smoothstep(0.05, 0.95, p)
                let checkStart = settles ? frame.maxX - Self.trailingCheckInset : frame.minX + Self.leadingCheckInset
                let checkEnd = settles ? frame.minX + Self.leadingCheckInset : frame.maxX - Self.trailingCheckInset
                // Both faces sit at the row's top-left corner, each at the size it was drawn at;
                // the frame between them clips whichever is the larger, and the one fades into
                // the other on the way.
                ZStack(alignment: .topLeading) {
                    Image(nsImage: glide.departure)
                        .resizable()
                        .frame(width: from.width, height: from.height, alignment: .topLeading)
                        .opacity(1 - Self.smoothstep(0.2, 0.65, p))
                    Image(nsImage: glide.arrival)
                        .resizable()
                        .frame(width: to.width, height: to.height, alignment: .topLeading)
                        .opacity(Self.smoothstep(0.35, 0.8, p))
                }
                .frame(width: frame.width, height: frame.height, alignment: .topLeading)
                .clipShape(shape)
                // The fill and the lift's shadow go behind the clip, so the shadow can spread.
                .background {
                    shape
                        .fill(Chrome.overlay(fill))
                        .shadow(color: .black.opacity(0.28 * lift), radius: 10, y: 4)
                }
                .position(x: frame.midX, y: frame.midY)
                .overlay(alignment: .topLeading) {
                    // The check crosses the row on its own: from beside the ellipsis to the
                    // front as the thread settles (easing back from its pop on the way), and
                    // back again as it reopens, fading once the row is open.
                    Image(systemName: settles ? "checkmark.circle.fill" : "checkmark.circle")
                        .font(Chrome.inlineIconFont)
                        .foregroundStyle(Chrome.success)
                        .scaleEffect(settles ? 1.35 - 0.35 * Self.smoothstep(0, 0.5, p) : 1)
                        .opacity(settles ? 1 - 0.2 * p : 1 - Self.smoothstep(0.55, 0.9, p))
                        .position(x: checkStart + (checkEnd - checkStart) * checkTravel, y: frame.midY)
                }
                // A ghost with no row to hand over to (headed for the list's edge or the
                // folded header) fades out on the way instead of vanishing on arrival.
                .opacity(glide.hasLanded ? 1 : 1 - Self.smoothstep(0.55, 0.92, p))
                .frame(width: canvas.width, height: canvas.height, alignment: .topLeading)
                .clipShape(FrameShape(frame: glide.bounds))
            } keyframes: { _ in
                SpringKeyframe(1.0, spring: RowGlideAnimator.spring)
            }
            .onAppear { hasStarted = true }
            .task {
                // The real row comes back just under the ghost's last frames, so the handover
                // never shows a gap; the ghost goes once the keyframes have run their course.
                let duration = RowGlideAnimator.spring.settlingDuration
                try? await Task.sleep(for: .seconds(max(duration - 0.12, 0)))
                RowGlideAnimator.shared.arrive(glide.id)
                try? await Task.sleep(for: .seconds(0.12))
                RowGlideAnimator.shared.finish(glide.id)
            }
    }

    private nonisolated static func smoothstep(_ edge0: Double, _ edge1: Double, _ x: Double) -> Double {
        let t = min(max((x - edge0) / (edge1 - edge0), 0), 1)
        return t * t * (3 - 2 * t)
    }
}

/// A fixed rectangle in the layer's space, for clipping a glide to the list it belongs to.
private struct FrameShape: Shape {
    let frame: CGRect

    func path(in rect: CGRect) -> Path {
        Path(frame)
    }
}
