import SwiftUI
import WfCore

struct StylePage: View {
    let editor: SettingsEditor
    let pages: SettingsPages

    @State private var newWord = ""

    var body: some View {
        SettingsForm(page: .style, editor: editor, pages: pages) {
            styles
            dictionary
            replacements
            snippets
        }
    }

    private var styles: some View {
        Section {
            SettingPicker(
                title: "Personal messages", options: Style.options,
                selection: editor.binding(\.style.personalMessages))
            SettingPicker(
                title: "Work messages", options: Style.options,
                selection: editor.binding(\.style.workMessages))
            SettingPicker(
                title: "Email", options: Style.options, selection: editor.binding(\.style.email))
            SettingPicker(
                title: "Other", options: Style.options, selection: editor.binding(\.style.other))
        } header: {
            Text("Styles")
        } footer: {
            Text("Tone and punctuation for each kind of app.")
        }
    }

    private func addWord() {
        let word = newWord
        newWord = ""
        pages.ask { try $0.addWord(word: word) }
    }

    private var dictionary: some View {
        Section {
            HStack(spacing: 8) {
                TextField("New word", text: $newWord, prompt: Text("Add a word"))
                    .labelsHidden()
                    .onSubmit(addWord)
                    .accessibilityIdentifier("New word")
                Button("Add", action: addWord)
                    .disabled(newWord.isEmpty)
                    .accessibilityIdentifier("Add word")
            }
            ForEach(Array(pages.state.dictionary.enumerated()), id: \.offset) { index, word in
                LabeledContent(word) {
                    Button("Remove") { pages.ask { try $0.removeWord(index: Int32(index)) } }
                        .accessibilityIdentifier("Remove \(word)")
                }
            }
            SettingToggle(
                title: "Add corrected words automatically",
                hint:
                    "Fix one word within 30 seconds of dictating and the new spelling joins this list. Only the word is kept, you can remove it above, and password fields are never read. Needs context awareness.",
                isOn: editor.binding(\.style.autoAddCorrections))
            SettingToggle(
                title: "Say \"press enter\" to press Enter", hint: "At the end of a dictation.",
                isOn: editor.binding(\.style.pressEnterCommand))
        } header: {
            Text("Dictionary")
        } footer: {
            Text("Words dictation should recognize and spell your way.")
        }
    }

    private var replacements: some View {
        Section {
            ForEach(Array(pages.state.replacements.enumerated()), id: \.offset) { index, row in
                let at = UInt32(index)
                HStack(spacing: 8) {
                    RowField(title: "Wrong", value: row.wrong) { wrong in
                        let edited = Replacement(wrong: wrong, right: row.right)
                        pages.ask { try $0.editReplacement(index: at, row: edited) }
                    }
                    RowField(title: "Right", value: row.right) { right in
                        let edited = Replacement(wrong: row.wrong, right: right)
                        pages.ask { try $0.editReplacement(index: at, row: edited) }
                    }
                    Button("Remove") { pages.ask { try $0.removeReplacement(index: at) } }
                        .accessibilityIdentifier("Remove replacement \(row.wrong)")
                }
            }
            AddRowLine(title: "Add replacement") { pages.ask { try $0.addReplacement() } }
        } header: {
            Text("Replacements")
        } footer: {
            Text("Always swap a wrong word for the right one.")
        }
    }

    private var snippets: some View {
        Section {
            ForEach(Array(pages.state.snippets.enumerated()), id: \.offset) { index, row in
                let at = UInt32(index)
                VStack(alignment: .leading, spacing: 8) {
                    HStack(spacing: 8) {
                        RowField(title: "Trigger phrase", value: row.trigger) { trigger in
                            let edited = Snippet(trigger: trigger, text: row.text)
                            pages.ask { try $0.editSnippet(index: at, row: edited) }
                        }
                        Button("Remove") { pages.ask { try $0.removeSnippet(index: at) } }
                            .accessibilityIdentifier("Remove snippet \(row.trigger)")
                    }
                    RowField(title: "Expands to", value: row.text, multiline: true) { text in
                        let edited = Snippet(trigger: row.trigger, text: text)
                        pages.ask { try $0.editSnippet(index: at, row: edited) }
                    }
                }
            }
            AddRowLine(title: "Add snippet") { pages.ask { try $0.addSnippet() } }
        } header: {
            Text("Snippets")
        } footer: {
            Text("Say the trigger phrase and it expands to the text.")
        }
    }
}
