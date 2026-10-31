import SwiftUI
import Testing
import ViewInspector
import WfCore

@testable import WisprFree

private let titles = [
    "General", "Models", "Pipeline", "Style & dictionary", "Meetings", "Sounds & media",
    "Privacy & data", "About",
]

@MainActor
private func navigation(on page: SettingsPage = .general) async throws -> (
    SettingsNavigation, FakeCore
) {
    let core = FakeCore(settings: try await defaultSettings())
    core.page = page
    return (SettingsNavigation(core: core), core)
}

@MainActor
private func view(on model: SettingsNavigation, core: FakeCore) throws -> SettingsView {
    SettingsView(
        navigation: model, editor: try SettingsEditor(core: core), pages: SettingsPages(core: core))
}

@MainActor
@Suite struct SettingsSkeleton {
    @Test func theSidebarListsTheEightPagesWithTodaysLabels() async throws {
        let (model, core) = try await navigation()
        let view = try view(on: model, core: core).inspect()

        let labels = view.findAll(ViewType.Label.self)

        #expect(try labels.map { try $0.title().text().string() } == titles)
        #expect(try labels.map { try $0.accessibilityIdentifier() } == titles)
    }

    @Test func theLastPageIsWhereItOpens() async throws {
        let (model, _) = try await navigation(on: .privacy)

        #expect(model.page == .privacy)
        #expect(model.title == "Privacy & data")
    }

    @Test func theTitleFollowsThePageAndRustKeepsIt() async throws {
        let (model, core) = try await navigation()

        model.selection = .about

        #expect(model.title == "About")
        #expect(await eventually { core.savedPages == [.about] })
    }

    @Test func nothingSelectedStaysOnThePage() async throws {
        let (model, core) = try await navigation(on: .models)

        model.selection = nil

        #expect(model.page == .models)
        try await Task.sleep(for: .milliseconds(20))
        #expect(core.savedPages.isEmpty)
    }

    @Test func thePageOnTheRightIsTheOneSelected() async throws {
        let (model, core) = try await navigation(on: .sounds)
        let view = try view(on: model, core: core).inspect()

        let page = try view.find(SettingsPageView.self).actualView().page

        #expect(page == .sounds)
    }
}
