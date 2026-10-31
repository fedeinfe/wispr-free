import SwiftUI

/// System buttons draw inactive in a never-key panel, so `.borderedProminent` loses its colour
/// there. Primary is white on dark, secondary white at 16%.
struct PillButtonStyle: ButtonStyle {
    enum Kind {
        case primary, secondary
    }

    let kind: Kind
    var round = false

    func makeBody(configuration: Configuration) -> some View {
        Face(configuration: configuration, kind: kind, round: round)
    }

    private struct Face: View {
        let configuration: Configuration
        let kind: Kind
        let round: Bool
        @State private var hovered = false

        var body: some View {
            configuration.label
                .foregroundStyle(kind == .primary ? PillSkin.ink : Color.white)
                .background(fill, in: shape)
                .contentShape(shape)
                .brightness(hovered ? 0.08 : 0)
                .opacity(configuration.isPressed ? 0.7 : 1)
                .onHover { hovered = $0 }
        }

        private var fill: Color {
            kind == .primary ? .white : .white.opacity(0.16)
        }

        private var shape: AnyShape {
            round
                ? AnyShape(Circle())
                : AnyShape(RoundedRectangle(cornerRadius: 6, style: .continuous))
        }
    }
}
