import SwiftUI
import WfCore

/// The pill's colours, the same in both appearances.
enum PillSkin {
    /// `#17161c`.
    static let ink = Color(red: 0x17 / 255, green: 0x16 / 255, blue: 0x1c / 255)
    static let hairline = Color.white.opacity(0.2)
    static let shadow = Color.black.opacity(0.25)

    /// 0.5 pt, or a whole pixel on a 1x screen, where half a pixel drops out of straight edges.
    static func hairlineWidth(scale: CGFloat) -> CGFloat {
        max(0.5, 1 / scale)
    }
}

/// What the pill draws from one frame. Kept apart from the rest of the frame, as `@Observable`
/// tells no one of an equal value, so a tick that changes none of it redraws nothing.
struct PillLook: Equatable {
    var shown: PillState = .hidden
    var capsule = WfCore.Rect(x: 0, y: 0, width: 0, height: 0)
    var card: PillCard?
    var tooltip: String?
    var bars: [Float] = []
    var barScale: Float = 1

    /// What changes the capsule's shape or skin, for the morph.
    var shapeKey: String { "\(capsule.width)x\(capsule.height) \(skinKey)" }

    private var skinKey: String {
        if case .error = shown { "error" } else { "plain" }
    }
}

/// What the pill's controls do.
struct PillActions {
    let press: () -> Void
    let release: () -> Void
    let menu: (NSEvent, NSView) -> Void
    let cancel: () -> Void
    let stop: () -> Void
    let answer: (Bool) -> Void
}

@Observable
final class PillModel {
    var look = PillLook()
}

struct PillView: View {
    let model: PillModel
    let size: CGSize
    let actions: PillActions
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        let look = model.look
        ZStack(alignment: .topLeading) {
            if look.shown != .hidden {
                PillCapsule(look: look, actions: actions, reduceMotion: reduceMotion)
            }
            if let tooltip = look.tooltip {
                TooltipLayout(capsule: look.capsule) {
                    Tooltip(text: tooltip)
                }
            }
            if let card = look.card {
                Card(card: card, answer: actions.answer)
            }
        }
        .frame(width: size.width, height: size.height, alignment: .topLeading)
        .environment(\.colorScheme, .dark)
        .allowsWindowActivationEvents(true)
    }
}

/// The capsule, which morphs between shapes. With Reduce Motion it crossfades.
private struct PillCapsule: View {
    let look: PillLook
    let actions: PillActions
    let reduceMotion: Bool
    @Environment(\.displayScale) private var displayScale

    var body: some View {
        Group {
            if reduceMotion {
                face.id(look.shapeKey).transition(.opacity)
            } else {
                face
            }
        }
        .animation(PillMotion.animation(reduceMotion: reduceMotion), value: look.shapeKey)
    }

    private var face: some View {
        ZStack {
            Capsule().fill(fill)
            Capsule().strokeBorder(
                PillSkin.hairline, lineWidth: PillSkin.hairlineWidth(scale: displayScale))
            PressArea(press: actions.press, release: actions.release, menu: actions.menu)
            if look.shown == .processing {
                ProcessingRing(reduceMotion: reduceMotion)
            }
            if !look.bars.isEmpty {
                Bars(
                    heights: look.bars, scale: look.barScale,
                    fullHeight: look.capsule.height - 8, tint: barTint)
            }
            if look.shown == .listeningHandsFree {
                HandsFreeButtons(cancel: actions.cancel, stop: actions.stop)
            }
        }
        .frame(width: look.capsule.width, height: look.capsule.height)
        .compositingGroup()
        .shadow(color: PillSkin.shadow, radius: 8, y: 3)
        .offset(x: look.capsule.x, y: look.capsule.y)
    }

    private var fill: Color {
        if case .error = look.shown { .red } else { PillSkin.ink }
    }

    private var barTint: Color {
        look.shown == .meeting ? .green : .white
    }
}

enum PillMotion {
    /// The morph, or with Reduce Motion the crossfade, which moves nothing.
    static func animation(reduceMotion: Bool) -> Animation {
        reduceMotion ? .smooth : .snappy
    }
}

/// A light running round the rim, once in 1.4 s. With Reduce Motion it stands still.
private struct ProcessingRing: View {
    let reduceMotion: Bool

    private static let turn: TimeInterval = 1.4
    private static let colors: [Color] = [
        .white.opacity(0.08), .white.opacity(0.82), .white.opacity(0.08),
    ]

    var body: some View {
        if reduceMotion {
            Capsule().strokeBorder(Color.white.opacity(0.5), lineWidth: 1.5)
                .allowsHitTesting(false)
        } else {
            TimelineView(.animation) { context in
                let turns = context.date.timeIntervalSinceReferenceDate / Self.turn
                Capsule()
                    .strokeBorder(
                        AngularGradient(
                            colors: Self.colors, center: .center,
                            angle: .degrees(turns.truncatingRemainder(dividingBy: 1) * 360)),
                        lineWidth: 1.5)
            }
            .allowsHitTesting(false)
        }
    }
}

private struct Bars: View {
    let heights: [Float]
    let scale: Float
    let fullHeight: CGFloat
    let tint: Color

    var body: some View {
        HStack(spacing: 2) {
            ForEach(heights.indices, id: \.self) { index in
                Capsule()
                    .fill(tint)
                    .frame(width: 2, height: height(of: heights[index]))
            }
        }
        .allowsHitTesting(false)
    }

    private func height(of bar: Float) -> CGFloat {
        max(2, CGFloat(bar * scale) * fullHeight)
    }
}

private struct HandsFreeButtons: View {
    let cancel: () -> Void
    let stop: () -> Void

    var body: some View {
        HStack(spacing: 0) {
            button("xmark", kind: .secondary, label: "Cancel", action: cancel)
            Spacer(minLength: 0)
            button("checkmark", kind: .primary, label: "Stop", action: stop)
        }
        .padding(.horizontal, 3)
    }

    private func button(
        _ symbol: String, kind: PillButtonStyle.Kind, label: String, action: @escaping () -> Void
    ) -> some View {
        Button(action: action) {
            Image(systemName: symbol)
                .font(.system(size: 8, weight: .bold))
                .frame(width: 18, height: 18)
        }
        .buttonStyle(PillButtonStyle(kind: kind, round: true))
        .accessibilityLabel(label)
    }
}

/// Above the capsule and centred on it, kept 4 pt inside the window.
private struct TooltipLayout: Layout {
    let capsule: WfCore.Rect

    private static let margin: CGFloat = 4
    private static let gap: CGFloat = 6

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        proposal.replacingUnspecifiedDimensions()
    }

    func placeSubviews(
        in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()
    ) {
        guard let tooltip = subviews.first else { return }
        let room = ProposedViewSize(width: bounds.width - 2 * Self.margin, height: nil)
        let size = tooltip.sizeThatFits(room)
        let centred = capsule.x + (capsule.width - size.width) / 2
        let x = max(Self.margin, min(bounds.width - size.width - Self.margin, centred))
        let y = capsule.y - size.height - Self.gap
        tooltip.place(
            at: CGPoint(x: bounds.minX + x, y: bounds.minY + y), proposal: room)
    }
}

private struct Tooltip: View {
    let text: String
    @Environment(\.displayScale) private var displayScale

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: 6, style: .continuous)
        Text(text)
            .font(.callout)
            .foregroundStyle(.white)
            .lineLimit(1)
            .truncationMode(.tail)
            .padding(.horizontal, 8)
            .frame(height: 24)
            .background(PillSkin.ink, in: shape)
            .overlay(
                shape.strokeBorder(
                    PillSkin.hairline, lineWidth: PillSkin.hairlineWidth(scale: displayScale))
            )
            .compositingGroup()
            .shadow(color: PillSkin.shadow, radius: 8, y: 3)
            .allowsHitTesting(false)
    }
}

private struct Card: View {
    let card: PillCard
    let answer: (Bool) -> Void
    @Environment(\.displayScale) private var displayScale

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: 14, style: .continuous)
        VStack(alignment: .leading, spacing: 0) {
            Text(card.text)
                .font(.body)
                .foregroundStyle(.white)
                .lineLimit(1)
                .minimumScaleFactor(0.8)
            Spacer(minLength: 0)
            HStack(spacing: 6) {
                Spacer(minLength: 0)
                button(card.decline, kind: .secondary, accepted: false)
                button(card.accept, kind: .primary, accepted: true)
            }
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .frame(width: card.place.width, height: card.place.height)
        .background(PillSkin.ink, in: shape)
        .overlay(
            shape.strokeBorder(
                PillSkin.hairline, lineWidth: PillSkin.hairlineWidth(scale: displayScale))
        )
        .compositingGroup()
        .shadow(color: PillSkin.shadow, radius: 8, y: 3)
        .offset(x: card.place.x, y: card.place.y)
    }

    private func button(
        _ title: String, kind: PillButtonStyle.Kind, accepted: Bool
    ) -> some View {
        Button {
            answer(accepted)
        } label: {
            Text(title)
                .font(.subheadline)
                .padding(.horizontal, 10)
                .frame(height: 22)
        }
        .buttonStyle(PillButtonStyle(kind: kind))
    }
}
