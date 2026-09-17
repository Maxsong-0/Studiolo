import SwiftUI

struct EditorAttachments: View {
    @Binding var attachments: [Attachment]

    /// Same shape as the chat strip: one preview panel for the strip, so every
    /// draft photo opens in a single tap.
    @State private var preview = AttachmentPreviewCoordinator()

    var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            // The delete badge straddles the thumbnail's far top-right corner.
            // The cell's own top/trailing padding reserves that overhang, so the
            // badge sits inside its own cell — never in the gap where a later
            // sibling could cover it — and every photo stays deletable. On top
            // only the 2 pt the badge's circle shows above the photo: the pill
            // already leaves 12 above the strip, and 12 + 2 puts the photo 14
            // from the pill's top edge, the same inset it has from the leading
            // edge, where it lines up with the text.
            HStack(spacing: 0) {
                ForEach(attachments) { attachment in
                    AttachmentThumbnail(attachment: attachment, size: 48, preview: preview)
                        .overlay(alignment: .topTrailing) {
                            Button {
                                attachments.removeAll { $0.id == attachment.id }
                            } label: {
                                Image(systemName: "xmark.circle.fill")
                                    .symbolRenderingMode(.palette)
                                    .foregroundStyle(.white, .black.opacity(0.6))
                                    .padding(4)
                            }
                            .buttonStyle(.plain)
                            .offset(x: 6, y: -6)
                            .accessibilityLabel(Text("Remove \(attachment.name)"))
                        }
                        .padding(.top, 2)
                        .padding(.trailing, 8)
                }
            }
            .background {
                AttachmentAnchorCapture { preview.setAnchor($0) }
            }
        }
        .onChange(of: attachments) {
            preview.retire(except: Set(attachments.map(\.id)))
        }
        .onDisappear { preview.close() }
    }
}

