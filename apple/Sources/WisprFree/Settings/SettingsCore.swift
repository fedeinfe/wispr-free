import WfCore

/// What Settings needs of Rust.
protocol SettingsCore: AnyObject {
    func settings() throws -> Versioned
    func save(settings: Settings, revision: UInt64) async throws -> SaveOutcome
    func settingsPage() throws -> SettingsPage
    func setSettingsPage(page: SettingsPage) async throws
}
