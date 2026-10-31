import SwiftUI

/// The setup walk's window and its bottom bar. The steps come with their logic.
struct OnboardingView: View {
    static let shape = WindowShape(
        identifier: "Set up Wispr Free", title: "Set up Wispr Free",
        size: NSSize(width: 680, height: 560), minimum: NSSize(width: 680, height: 460))

    var body: some View {
        VStack(spacing: 0) {
            Form {}
                .formStyle(.grouped)
            Divider()
            HStack(spacing: 12) {
                Spacer()
                Button("Back") {}
                    .accessibilityIdentifier("Back")
                Button("Continue") {}
                    .buttonStyle(.borderedProminent)
                    .accessibilityIdentifier("Continue")
            }
            .padding(16)
        }
    }
}
