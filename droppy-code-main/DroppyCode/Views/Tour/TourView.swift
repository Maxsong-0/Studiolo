import AppKit
import SwiftUI

// The welcome tour is a port of TourKit by Ram Patra (https://github.com/rampatra/TourKit,
// MIT License): the slideshow card, its page indicator and its controls, carried over to
// the app's glass and its asset catalog. See THIRD_PARTY_NOTICES.md.

/// One page of the tour: its artwork from the asset catalog, a title and a line under it.
struct TourPage: Identifiable, Hashable {
    let id: UUID
    let imageName: String
    let title: String
    let description: String

    init(imageName: String, title: String, description: String) {
        self.id = UUID()
        self.imageName = imageName
        self.title = title
        self.description = description
    }
}

struct TourView: View {
    let pages: [TourPage]
    let width: CGFloat
    /// The card's height, the same on every page: the window measures the tallest page's
    /// text and passes it, so the card never grows or shrinks between pages. Nil sizes to
    /// the current page (previews and the empty state).
    let height: CGFloat?
    let continueButtonTitle: String
    let finishButtonTitle: String
    let onFinish: (() -> Void)?
    let onClose: (() -> Void)?
    @Environment(\.dismiss) private var dismiss
    @Environment(\.colorScheme) private var colorScheme
    @State var currentIndex: Int

    init(
        pages: [TourPage],
        width: CGFloat = 660,
        height: CGFloat? = nil,
        initialPageIndex: Int = 0,
        continueButtonTitle: String = "Continue",
        finishButtonTitle: String = "Done",
        onFinish: (() -> Void)? = nil,
        onClose: (() -> Void)? = nil
    ) {
        self.pages = pages
        self.width = width
        self.height = height
        self.continueButtonTitle = continueButtonTitle
        self.finishButtonTitle = finishButtonTitle
        self.onFinish = onFinish
        self.onClose = onClose
        _currentIndex = State(initialValue: Self.clamped(initialPageIndex, pageCount: pages.count))
    }

    var imageHeight: CGFloat {
        (width / Self.imageAspectRatio).rounded()
    }

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: 20, style: .continuous)
        let isDark = colorScheme == .dark
        return Group {
            if pages.isEmpty {
                emptyState
            } else {
                VStack(spacing: 0) {
                    imageSection
                    bottomPanel
                }
                .frame(width: width, height: height)
            }
        }
        .frame(width: width, height: height)
        // The floating-panel recipe from SubagentPanel: one shared rounded shape for
        // either state, one glass/tint surface, one clip, and one inset hairline.
        .background {
            shape
                .fill(.clear)
                .glassEffect(.regular, in: shape)
                .overlay {
                    shape.fill(Chrome.glassTint.opacity(isDark ? 0.22 : 0.16))
                }
        }
        .clipShape(shape)
        .overlay {
            shape.strokeBorder(Chrome.overlay(0.14), lineWidth: 1)
        }
        // The scrim sits under the glass as a plain filled shape; AppKit owns the shadow.
        .background {
            shape
                .fill((isDark ? Color.black : Color.white).opacity(isDark ? 0.3 : 0.34))
        }
        .animation(.easeInOut(duration: 0.25), value: currentIndex)
        .presentedChrome()
    }

    private var emptyState: some View {
        VStack(spacing: 12) {
            Image(systemName: "rectangle.stack")
                .font(.title)
                .foregroundStyle(Chrome.secondaryText)
            Text("No tour pages")
                .font(.headline)
                .foregroundStyle(Chrome.primaryText)
        }
        .frame(maxWidth: .infinity)
        .frame(height: Self.emptyStateHeight)
    }

    static let imageAspectRatio: CGFloat = 16.0 / 10.0
    static let emptyStateHeight: CGFloat = 220

    private var imageSection: some View {
        ZStack(alignment: .top) {
            artwork(for: pages[currentIndex])
                .frame(width: width, height: imageHeight)
                .clipped()
                .id(currentIndex)
                .transition(.opacity)

            // A short seat for the page indicator only: the picture is the point of the
            // page, so the scrim never reaches into it.
            LinearGradient(
                stops: [
                    .init(color: .clear, location: 0),
                    .init(color: Color(white: 0.08).opacity(0.28), location: 0.55),
                    .init(color: Color(white: 0.08).opacity(0.55), location: 1.0)
                ],
                startPoint: .top,
                endPoint: .bottom
            )
            .frame(height: 96)
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .bottom)
            .allowsHitTesting(false)

            TourPageIndicator(totalPages: pages.count, currentIndex: currentIndex)
                .padding(.bottom, 14)
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .bottom)
                .allowsHitTesting(false)

            topControls
        }
        .frame(width: width, height: imageHeight)
    }

    private var bottomPanel: some View {
        let currentPage = pages[currentIndex]

        return VStack(spacing: 0) {
            VStack(spacing: 0) {
                Spacer(minLength: 12)

                Text(verbatim: currentPage.title)
                    .font(.system(size: 28, weight: .bold))
                    .multilineTextAlignment(.center)
                    .foregroundStyle(Chrome.primaryText)
                    .fixedSize(horizontal: false, vertical: true)

                Text(verbatim: currentPage.description)
                    .font(.body)
                    .multilineTextAlignment(.center)
                    .foregroundStyle(Chrome.secondaryText)
                    .fixedSize(horizontal: false, vertical: true)
                    .padding(.top, 6)

                Spacer(minLength: 12)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)

            primaryActionButton
        }
        .padding(.horizontal, 32)
        .padding(.top, 12)
        .padding(.bottom, 24)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .id(currentIndex)
        .transition(.opacity)
    }

    private var topControls: some View {
        HStack {
            iconButton(systemName: "chevron.left") {
                goBack()
            }
            .opacity(currentIndex > 0 ? 1 : 0)

            Spacer()

            iconButton(systemName: "checkmark") {
                if let onClose {
                    onClose()
                } else {
                    dismiss()
                }
            }
        }
        .padding(.horizontal, 14)
        .padding(.top, 12)
    }

    private var primaryActionButton: some View {
        Button(action: advance) {
            Text(verbatim: isLastPage ? finishButtonTitle : continueButtonTitle)
                .font(.system(size: 15, weight: .semibold))
                .frame(width: 220, height: 42)
        }
        .buttonStyle(.glassProminent)
        .keyboardShortcut(.defaultAction)
    }

    private func iconButton(systemName: String, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Image(systemName: systemName)
                .font(.system(size: 13, weight: .semibold))
                .frame(width: 32, height: 32)
                .contentShape(Circle())
        }
        .buttonStyle(.plain)
        .chromeGlassCircle()
    }

    var isLastPage: Bool {
        currentIndex == pages.count - 1
    }

    static func clamped(_ index: Int, pageCount: Int) -> Int {
        guard pageCount > 0 else { return 0 }
        return max(0, min(index, pageCount - 1))
    }

    func advance() {
        if isLastPage {
            if let onFinish {
                onFinish()
            } else if let onClose {
                onClose()
            } else {
                dismiss()
            }
        } else {
            currentIndex += 1
        }
    }

    func goBack() {
        let newIndex = max(0, currentIndex - 1)
        guard newIndex != currentIndex else { return }
        currentIndex = newIndex
    }

    private func artwork(for page: TourPage) -> some View {
        Group {
            if NSImage(named: page.imageName) != nil {
                Image(page.imageName)
                    .resizable()
                    .scaledToFill()
            } else {
                LinearGradient(
                    colors: [
                        Color(red: 0.16, green: 0.12, blue: 0.40),
                        Color(red: 0.05, green: 0.05, blue: 0.10)
                    ],
                    startPoint: .top,
                    endPoint: .bottom
                )
                .overlay {
                    Image(systemName: "photo")
                        .font(.system(size: 40))
                        .foregroundStyle(Color.white.opacity(0.25))
                }
            }
        }
    }
}

struct TourPageIndicator: View {
    let totalPages: Int
    let currentIndex: Int

    init(totalPages: Int, currentIndex: Int) {
        self.totalPages = totalPages
        self.currentIndex = currentIndex
    }

    var body: some View {
        HStack(spacing: 7) {
            ForEach(0..<totalPages, id: \.self) { index in
                Capsule(style: .continuous)
                    .fill(index == currentIndex ? Color.white.opacity(0.95) : Color.white.opacity(0.32))
                    .frame(width: index == currentIndex ? 24 : 8, height: 8)
            }
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("Page \(currentIndex + 1) of \(max(totalPages, 1))")
    }
}
