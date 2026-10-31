//! The pages of the Settings sidebar, and how the one last shown is remembered.

use std::str::FromStr;

/// A page of Settings, in sidebar order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, uniffi::Enum)]
pub enum SettingsPage {
    #[default]
    General,
    Models,
    Pipeline,
    Style,
    Meetings,
    Sounds,
    Privacy,
    About,
}

/// One row of the sidebar.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SettingsPageRow {
    pub page: SettingsPage,
    pub title: String,
}

impl SettingsPage {
    pub const ALL: [SettingsPage; 8] = [
        SettingsPage::General,
        SettingsPage::Models,
        SettingsPage::Pipeline,
        SettingsPage::Style,
        SettingsPage::Meetings,
        SettingsPage::Sounds,
        SettingsPage::Privacy,
        SettingsPage::About,
    ];

    pub fn title(self) -> &'static str {
        match self {
            SettingsPage::General => "General",
            SettingsPage::Models => "Models",
            SettingsPage::Pipeline => "Pipeline",
            SettingsPage::Style => "Style & dictionary",
            SettingsPage::Meetings => "Meetings",
            SettingsPage::Sounds => "Sounds & media",
            SettingsPage::Privacy => "Privacy & data",
            SettingsPage::About => "About",
        }
    }

    /// The name it's stored under, which a new title doesn't change.
    pub fn as_str(self) -> &'static str {
        match self {
            SettingsPage::General => "general",
            SettingsPage::Models => "models",
            SettingsPage::Pipeline => "pipeline",
            SettingsPage::Style => "style",
            SettingsPage::Meetings => "meetings",
            SettingsPage::Sounds => "sounds",
            SettingsPage::Privacy => "privacy",
            SettingsPage::About => "about",
        }
    }
}

impl FromStr for SettingsPage {
    type Err = ();

    fn from_str(name: &str) -> Result<Self, ()> {
        Self::ALL
            .into_iter()
            .find(|page| page.as_str() == name)
            .ok_or(())
    }
}

/// The sidebar, top to bottom.
#[uniffi::export]
pub fn settings_pages() -> Vec<SettingsPageRow> {
    SettingsPage::ALL
        .into_iter()
        .map(|page| SettingsPageRow {
            page,
            title: page.title().to_owned(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sidebar_keeps_todays_eight_labels_in_their_order() {
        let titles: Vec<_> = settings_pages().into_iter().map(|row| row.title).collect();

        assert_eq!(
            titles,
            [
                "General",
                "Models",
                "Pipeline",
                "Style & dictionary",
                "Meetings",
                "Sounds & media",
                "Privacy & data",
                "About",
            ]
        );
    }

    #[test]
    fn every_page_comes_back_from_the_name_it_is_stored_under() {
        for page in SettingsPage::ALL {
            assert_eq!(page.as_str().parse(), Ok(page));
        }
        assert_eq!("nonsense".parse::<SettingsPage>(), Err(()));
    }
}
