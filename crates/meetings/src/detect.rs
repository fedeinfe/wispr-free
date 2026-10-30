//! Spots a known meeting app that starts using the mic, so the app can offer "Take notes for this
//! meeting?". This module only ever produces that suggestion: nothing here can start a recording,
//! and with the opt-in off it stays quiet.

/// Polls an app must hold the mic before it counts, which skips quick mic tests. At the intended
/// 2 s poll this is about 4 s.
const HOLD_POLLS: u32 = 2;
/// Polls without the mic before a session counts as over. Apps let go of the mic briefly when
/// muting or switching devices, and that must not bring the card back.
const RELEASE_POLLS: u32 = 15;

/// A process currently capturing from a microphone, as the OS reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MicClient {
    /// 0 when the OS doesn't say.
    pub pid: u32,
    /// Bundle id on macOS, executable name on Windows and Linux.
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeetingApp {
    Zoom,
    Teams,
    /// A browser using the mic, most likely Google Meet. The tab isn't visible from here.
    Browser,
    Webex,
    FaceTime,
    Slack,
}

impl MeetingApp {
    /// How the suggestion card names the app.
    pub fn label(self) -> &'static str {
        match self {
            Self::Zoom => "Zoom",
            Self::Teams => "Microsoft Teams",
            Self::Browser => "your browser",
            Self::Webex => "Webex",
            Self::FaceTime => "FaceTime",
            Self::Slack => "Slack",
        }
    }
}

/// Names are matched whole, ignoring case, after dropping `.exe` and any `.helper…` suffix.
const KNOWN_APPS: &[(&str, MeetingApp)] = &[
    ("us.zoom.xos", MeetingApp::Zoom),
    ("zoom", MeetingApp::Zoom),
    ("zoom voiceengine", MeetingApp::Zoom),
    ("com.microsoft.teams", MeetingApp::Teams),
    ("com.microsoft.teams2", MeetingApp::Teams),
    ("ms-teams", MeetingApp::Teams),
    ("teams", MeetingApp::Teams),
    ("teams-for-linux", MeetingApp::Teams),
    ("cisco-systems.spark", MeetingApp::Webex),
    ("com.webex.meetingmanager", MeetingApp::Webex),
    ("ciscocollabhost", MeetingApp::Webex),
    ("webexmta", MeetingApp::Webex),
    ("webex", MeetingApp::Webex),
    ("com.apple.facetime", MeetingApp::FaceTime),
    ("com.apple.avconferenced", MeetingApp::FaceTime),
    ("com.tinyspeck.slackmacgap", MeetingApp::Slack),
    ("slack", MeetingApp::Slack),
    ("com.google.chrome", MeetingApp::Browser),
    ("com.apple.safari", MeetingApp::Browser),
    ("com.apple.webkit.gpu", MeetingApp::Browser),
    ("org.mozilla.firefox", MeetingApp::Browser),
    ("com.microsoft.edgemac", MeetingApp::Browser),
    ("com.brave.browser", MeetingApp::Browser),
    ("company.thebrowser.browser", MeetingApp::Browser),
    ("chrome", MeetingApp::Browser),
    ("google chrome", MeetingApp::Browser),
    ("chromium", MeetingApp::Browser),
    ("msedge", MeetingApp::Browser),
    ("firefox", MeetingApp::Browser),
    ("brave", MeetingApp::Browser),
];

/// The meeting app a [`MicClient::name`] belongs to, if any.
pub fn classify_client(name: &str) -> Option<MeetingApp> {
    let name = name.to_lowercase();
    let name = name.strip_suffix(".exe").unwrap_or(&name);
    let name = name.split(".helper").next().unwrap_or(name);
    KNOWN_APPS
        .iter()
        .find(|(known, _)| *known == name)
        .map(|&(_, app)| app)
}

/// Feed it the mic clients every poll; it returns an app when the card should appear. Each mic
/// session of an app gets at most one suggestion, whatever the user answers.
#[derive(Debug, Default)]
pub struct MeetingDetector {
    enabled: bool,
    sessions: Vec<Session>,
}

#[derive(Debug)]
struct Session {
    app: MeetingApp,
    held_polls: u32,
    absent_polls: u32,
    settled: bool,
}

impl Session {
    fn new(app: MeetingApp) -> Self {
        Self {
            app,
            held_polls: 0,
            absent_polls: 0,
            settled: false,
        }
    }
}

impl MeetingDetector {
    /// `enabled` is the user's auto-detect setting, which is off by default.
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            sessions: Vec::new(),
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// `recording` is whether a meeting is being recorded right now: there's nothing to suggest
    /// then, and the sessions it covers stay settled afterwards.
    pub fn observe(&mut self, clients: &[MicClient], recording: bool) -> Option<MeetingApp> {
        if !self.enabled {
            return None;
        }
        let active: Vec<MeetingApp> = clients
            .iter()
            .filter_map(|client| classify_client(&client.name))
            .collect();

        self.sessions.retain_mut(|session| {
            if active.contains(&session.app) {
                session.absent_polls = 0;
                return true;
            }
            session.absent_polls += 1;
            // A session that never settled was only a mic test, so it is forgotten right away.
            session.settled && session.absent_polls < RELEASE_POLLS
        });
        for &app in &active {
            if !self.sessions.iter().any(|session| session.app == app) {
                self.sessions.push(Session::new(app));
            }
        }

        let mut suggestion = None;
        for session in &mut self.sessions {
            if session.settled || !active.contains(&session.app) {
                continue;
            }
            session.held_polls += 1;
            if recording {
                session.settled = true;
            } else if session.held_polls >= HOLD_POLLS && suggestion.is_none() {
                session.settled = true;
                suggestion = Some(session.app);
            }
        }
        suggestion
    }
}

/// Reads `pw-dump` JSON (Linux) for capture streams that are running.
pub fn parse_pw_dump(json: &str) -> Vec<MicClient> {
    let Ok(serde_json::Value::Array(objects)) = serde_json::from_str(json) else {
        return Vec::new();
    };
    objects
        .iter()
        .filter_map(|object| {
            let info = object.get("info")?;
            let props = info.get("props")?;
            let text = |key: &str| props.get(key).and_then(|value| value.as_str());
            if text("media.class") != Some("Stream/Input/Audio")
                || info.get("state").and_then(|state| state.as_str()) != Some("running")
            {
                return None;
            }
            let name = text("application.process.binary").or(text("application.name"))?;
            let pid = props
                .get("application.process.id")
                .and_then(|pid| pid.as_u64())
                .unwrap_or(0);
            Some(MicClient {
                pid: pid as u32,
                name: name.to_owned(),
            })
        })
        .collect()
}
