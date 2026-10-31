import WfCore

/// What Settings needs of Rust.
protocol SettingsCore: AnyObject {
    func settings() throws -> Versioned
    func save(settings: Settings, revision: UInt64) async throws -> SaveOutcome
    func settingsPage() throws -> SettingsPage
    func setSettingsPage(page: SettingsPage) async throws
    func changeSetting(change: RuledChange) throws
    func recordShortcut(index: Int32) throws
    func stopRecordingShortcut() throws
    func resetShortcut(index: Int32) throws
    func setMicrophoneAuto(on: Bool) throws
    func moveMicrophone(index: Int32, by: Int32) throws
    func toggleMicrophoneTest() throws
    func setLanguagesAuto(on: Bool) throws
    func toggleLanguage(index: Int32) throws
    func addWord(word: String) throws
    func removeWord(index: Int32) throws
    func addReplacement() throws
    func editReplacement(index: UInt32, row: Replacement) throws
    func removeReplacement(index: UInt32) throws
    func addSnippet() throws
    func editSnippet(index: UInt32, row: Snippet) throws
    func removeSnippet(index: UInt32) throws
    func download(index: Int32) throws
    func downloadModelsInUse() throws
    func cancelDownload() throws
    func verify(index: Int32) throws
    func deleteModel(index: Int32) throws
    func chooseModelsFolder() throws
    func folderPicked(folder: String) throws
    func useDefaultModelsFolder() throws
    func openModelsFolder() throws
    func recheckModelsFolder() throws
    func moveFolder() throws
    func switchFolder() throws
    func cancelPendingFolder() throws
    func cancelMove() throws
    func openDataFolder() throws
    func exportAll() throws
    func deleteAll() throws
    func checkForUpdates() throws
}
