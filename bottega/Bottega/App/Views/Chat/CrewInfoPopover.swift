// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import SwiftUI

/// One line of a Crew explainer: a glyph, a title, and a sentence under it.
private func crewInfoStep(glyph: AnyView, title: String, line: String) -> some View {
    HStack(alignment: .top, spacing: 12) {
        glyph.frame(width: 30, height: 30)
        VStack(alignment: .leading, spacing: 2) {
            Text(verbatim: title)
                .font(.system(size: 13, weight: .semibold))
            Text(verbatim: line)
                .font(.system(size: 12))
                .foregroundStyle(Chrome.secondaryText)
        }
    }
}

/// What Crew and its hands are, behind the info buttons on Settings › Crew.
struct CrewInfoPopover: View {
    enum Topic { case crew, hands }
    let topic: Topic

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack(spacing: 10) {
                HStack(spacing: -6) {
                    ForEach(0..<5, id: \.self) { i in
                        CrewGlyph(persona: CrewRoster.persona(at: i), size: 24)
                    }
                }
                VStack(alignment: .leading, spacing: 2) {
                    Text(topic == .crew ? "Crew" : "Hands")
                        .font(.system(size: 15, weight: .semibold))
                    Text(topic == .crew ? "One chat leads a team of hands on big jobs." : "The helpers a chat sends out, one brief each.")
                        .font(.system(size: 12))
                        .foregroundStyle(Chrome.secondaryText)
                }
            }
            VStack(alignment: .leading, spacing: 12) {
                if topic == .crew {
                    crewInfoStep(
                        glyph: AnyView(CrewMarkImage().foregroundStyle(Chrome.primaryText).frame(width: 18, height: 18)),
                        title: "You ask, the chat leads",
                        line: "Ask as you always do. Small things it does itself; anything bigger it splits into briefs."
                    )
                    crewInfoStep(
                        glyph: AnyView(HStack(spacing: -8) {
                            CrewGlyph(persona: CrewRoster.persona(at: 0), size: 24)
                            CrewGlyph(persona: CrewRoster.persona(at: 1), size: 24)
                        }),
                        title: "Hands go out in parallel",
                        line: "Each hand gets one brief and a panel of its own beside the chat, all working at the same time."
                    )
                    crewInfoStep(
                        glyph: AnyView(Image(systemName: "checkmark.circle").font(.system(size: 20)).foregroundStyle(Chrome.success)),
                        title: "Reports come back, the lead finishes",
                        line: "The lead checks their work, fixes what needs fixing and answers you. Their changes land as one merge."
                    )
                } else {
                    crewInfoStep(
                        glyph: AnyView(CrewGlyph(persona: CrewRoster.persona(at: 0), size: 24)),
                        title: "One brief, one panel",
                        line: "A hand gets a self-contained brief and its own panel beside the chat, on the model its pair names or the chat's own."
                    )
                    crewInfoStep(
                        glyph: AnyView(Image(systemName: "square.on.square").font(.system(size: 18)).foregroundStyle(Chrome.primaryText)),
                        title: "Its own copy of the checkout",
                        line: "With the switch on, each hand works in a copy of the project so hands never trip over each other; its changes land in the chat's checkout when it reports."
                    )
                    crewInfoStep(
                        glyph: AnyView(Image(systemName: "arrow.uturn.backward.circle").font(.system(size: 20)).foregroundStyle(Chrome.success)),
                        title: "Reports back to the lead",
                        line: "A finished hand hands its report to the lead, which checks the work and answers you. Cleared hands move from the panel to the sidebar."
                    )
                }
            }
            if topic == .crew {
                Divider().opacity(0.5)
                Text("Right-click the Crew mark in a chat to switch it off or on for that chat.")
                    .font(.system(size: 12))
                    .foregroundStyle(Chrome.secondaryText)
            }
        }
        .padding(18)
    }
}
