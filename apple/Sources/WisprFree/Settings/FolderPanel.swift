import AppKit

/// The system's folder picker.
enum FolderPanel {
    static func make(from start: String) -> NSOpenPanel {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.allowsMultipleSelection = false
        panel.canCreateDirectories = true
        panel.directoryURL = URL(filePath: start, directoryHint: .isDirectory)
        return panel
    }

    /// `answer` runs only if a folder is picked.
    static func ask(from start: String, over window: NSWindow?, answer: @escaping (String) -> Void) {
        let panel = make(from: start)
        let done: (NSApplication.ModalResponse) -> Void = { response in
            if response == .OK, let folder = panel.url {
                answer(folder.path(percentEncoded: false))
            }
        }
        if let window {
            panel.beginSheetModal(for: window, completionHandler: done)
        } else {
            panel.begin(completionHandler: done)
        }
    }
}
