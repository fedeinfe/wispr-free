//! The windows of the Swift app, and what asking for one comes to. Swift makes them and does as
//! it's told, so the rule between windows stays here.

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum AppWindow {
    Settings,
    Meetings,
    Onboarding,
}

/// What to do about a request for a window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowOrder {
    Open(AppWindow),
    /// It's open already, so it only comes forward.
    Raise(AppWindow),
}

/// The windows that are open, and nothing about them but that.
#[derive(Debug, Default)]
pub struct OpenWindows(Vec<AppWindow>);

impl OpenWindows {
    pub fn is_open(&self, window: AppWindow) -> bool {
        self.0.contains(&window)
    }

    /// Onboarding shares the models folder with Settings, and downloads don't lock their files,
    /// so while it's open it comes forward in Settings' place.
    pub fn ask(&mut self, window: AppWindow) -> WindowOrder {
        let window = if window == AppWindow::Settings && self.is_open(AppWindow::Onboarding) {
            AppWindow::Onboarding
        } else {
            window
        };
        if self.is_open(window) {
            return WindowOrder::Raise(window);
        }
        self.0.push(window);
        WindowOrder::Open(window)
    }

    pub fn closed(&mut self, window: AppWindow) {
        self.0.retain(|open| *open != window);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use AppWindow::{Meetings, Onboarding, Settings};

    #[test]
    fn a_closed_window_opens_and_an_open_one_comes_forward() {
        let mut windows = OpenWindows::default();

        assert_eq!(windows.ask(Settings), WindowOrder::Open(Settings));
        assert_eq!(windows.ask(Settings), WindowOrder::Raise(Settings));
        assert_eq!(windows.ask(Meetings), WindowOrder::Open(Meetings));
    }

    #[test]
    fn a_window_that_closed_opens_again() {
        let mut windows = OpenWindows::default();
        windows.ask(Meetings);

        windows.closed(Meetings);

        assert!(!windows.is_open(Meetings));
        assert_eq!(windows.ask(Meetings), WindowOrder::Open(Meetings));
    }

    #[test]
    fn asking_for_settings_during_onboarding_raises_onboarding() {
        let mut windows = OpenWindows::default();
        windows.ask(Onboarding);

        assert_eq!(windows.ask(Settings), WindowOrder::Raise(Onboarding));
        assert!(!windows.is_open(Settings));
    }

    #[test]
    fn settings_opens_again_once_onboarding_closed() {
        let mut windows = OpenWindows::default();
        windows.ask(Onboarding);
        windows.closed(Onboarding);

        assert_eq!(windows.ask(Settings), WindowOrder::Open(Settings));
    }

    #[test]
    fn meetings_opens_beside_onboarding_and_onboarding_beside_settings() {
        let mut windows = OpenWindows::default();
        windows.ask(Settings);

        assert_eq!(windows.ask(Onboarding), WindowOrder::Open(Onboarding));
        assert_eq!(windows.ask(Meetings), WindowOrder::Open(Meetings));
    }
}
