import WfCore

/// What the setup walk needs of Rust, beyond the Settings pages it shares.
protocol OnboardingCore: AnyObject {
    func setupBack() throws
    func setupContinue() throws
    func askPermission(index: Int32) throws
    func openPermissionPane(index: Int32) throws
    func reopenApp() throws
    func startTrial() throws
    func stopTrial() throws
}
