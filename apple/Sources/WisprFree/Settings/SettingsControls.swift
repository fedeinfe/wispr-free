import SwiftUI
import WfCore

extension SettingsEditor {
    /// A control on a setting no rule depends on.
    func binding<Value>(_ path: WritableKeyPath<WfCore.Settings, Value>) -> Binding<Value> {
        Binding(
            get: { self.settings[keyPath: path] },
            set: { value in self.edit { $0[keyPath: path] = value } })
    }
}

/// A page: a grouped form, with the notice on top when this is the page that raised it.
struct SettingsForm<Content: View>: View {
    let page: SettingsPage
    let editor: SettingsEditor
    let pages: SettingsPages
    @ViewBuilder let content: Content

    private var notice: String? {
        if let notice = editor.notice { return notice }
        return pages.state.noticePage == page ? pages.state.notice : nil
    }

    var body: some View {
        Form {
            if let notice {
                Section {
                    Label(notice, systemImage: "exclamationmark.triangle")
                        .foregroundStyle(.red)
                        .accessibilityIdentifier("Notice")
                }
            }
            content
        }
        .formStyle(.grouped)
    }
}

/// A line that says something went wrong.
struct ErrorLine: View {
    let text: String

    var body: some View {
        if !text.isEmpty {
            Text(text).foregroundStyle(.red)
        }
    }
}

/// Today's label on a switch, with its hint under it.
struct SettingToggle: View {
    let title: String
    var hint: String?
    @Binding var isOn: Bool

    var body: some View {
        Toggle(isOn: $isOn) {
            Text(title)
            if let hint { Text(hint) }
        }
        .accessibilityIdentifier(title)
    }
}

/// Today's label on a menu of choices, with its hint under it.
struct SettingPicker<Value: Hashable>: View {
    let title: String
    var hint: String?
    let options: [(value: Value, title: String)]
    @Binding var selection: Value

    var body: some View {
        Picker(selection: $selection) {
            ForEach(options, id: \.value) { option in
                Text(option.title).tag(option.value)
            }
        } label: {
            Text(title)
            if let hint { Text(hint) }
        }
        .accessibilityIdentifier(title)
    }
}

/// A folder's path, which may be long, with what can be done to the folder beside it.
struct FolderLine<Buttons: View>: View {
    let path: String
    @ViewBuilder let buttons: Buttons

    var body: some View {
        HStack(spacing: 8) {
            Text(path)
                .foregroundStyle(.secondary)
                .textSelection(.enabled)
                .frame(maxWidth: .infinity, alignment: .leading)
            buttons
        }
    }
}

/// A button that adds a row, above the list it adds to. Below it, a `Form` leaves the row out of
/// the section's card and takes the footer in.
struct AddRowLine: View {
    let label: String
    let title: String
    let action: () -> Void

    var body: some View {
        LabeledContent(label) {
            Button("Add", action: action)
                .accessibilityIdentifier(title)
                .accessibilityLabel(title)
        }
    }
}

/// A number with a unit, on a stepper. The range is the control's, as on any stepper.
struct SettingStepper: View {
    let title: String
    let unit: String
    let range: ClosedRange<UInt32>
    var step = 1
    @Binding var value: UInt32

    var body: some View {
        LabeledContent(title) {
            HStack(spacing: 8) {
                Text("\(value) \(unit)")
                Stepper(title, value: $value, in: range, step: step)
                    .labelsHidden()
                    .accessibilityIdentifier(title)
            }
        }
    }
}

/// A field whose text comes back from Rust a moment after it's typed. While it has the focus the
/// text is its own, or a late answer would undo the last key.
struct RowField: View {
    let title: String
    let value: String
    var multiline = false
    let commit: (String) -> Void

    @State private var draft = ""
    @FocusState private var focused: Bool

    var body: some View {
        TextField(
            title, text: $draft, prompt: Text(title), axis: multiline ? .vertical : .horizontal
        )
        .labelsHidden()
        .focused($focused)
        .onAppear { draft = value }
        .onChange(of: draft) { _, typed in
            if typed != value { commit(typed) }
        }
        .onChange(of: value) { _, current in
            if !focused { draft = current }
        }
        .accessibilityIdentifier(title)
    }
}

extension Style {
    static let options: [(value: Style, title: String)] = [
        (.formal, "Formal"), (.casual, "Casual"), (.veryCasual, "Very casual"),
        (.excited, "Excited"),
    ]
}

extension CleanupStrength {
    static let options: [(value: CleanupStrength, title: String)] = [
        (.none, "None"), (.light, "Light"), (.medium, "Medium"), (.high, "High"),
    ]
}

extension KeepModels {
    static let options: [(value: KeepModels, title: String)] = [
        (.always, "Always"), (.fiveMinutes, "For 5 minutes"),
        (.unloadAfterUse, "Unload after use"),
    ]
}

extension MediaWhileDictating {
    static let options: [(value: MediaWhileDictating, title: String)] = [
        (.nothing, "Leave media alone"), (.mute, "Mute system output"), (.pause, "Pause media"),
    ]
}

extension HistoryRetention {
    static let options: [(value: HistoryRetention, title: String)] = [
        (.keep, "Keep"), (.delete24h, "Delete after 24 hours"), (.never, "Never store"),
    ]
}
