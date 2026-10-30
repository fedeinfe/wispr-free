//! Spotting a meeting app that starts using the mic, from a fake process list.

use wf_meetings::{MeetingApp, MeetingDetector, MicClient, classify_client, parse_pw_dump};

fn client(name: &str) -> MicClient {
    MicClient {
        pid: 1,
        name: name.to_owned(),
    }
}

const ZOOM: &str = "us.zoom.xos";
const SLACK: &str = "com.tinyspeck.slackmacgap.helper";

/// `count` polls that all see the same `names`.
fn repeat(names: &'static [&'static str], count: usize) -> Vec<&'static [&'static str]> {
    vec![names; count]
}

/// Runs one poll per entry and returns the suggestion each poll made.
fn run(detector: &mut MeetingDetector, polls: &[&[&str]]) -> Vec<Option<MeetingApp>> {
    polls
        .iter()
        .map(|names| {
            let clients: Vec<MicClient> = names.iter().map(|n| client(n)).collect();
            detector.observe(&clients, false)
        })
        .collect()
}

#[test]
fn known_meeting_apps_are_recognised_on_every_platform() {
    let cases = [
        ("us.zoom.xos", MeetingApp::Zoom),
        ("Zoom.exe", MeetingApp::Zoom),
        ("zoom", MeetingApp::Zoom),
        ("com.microsoft.teams2", MeetingApp::Teams),
        ("com.microsoft.teams2.helper", MeetingApp::Teams),
        ("ms-teams.exe", MeetingApp::Teams),
        ("teams-for-linux", MeetingApp::Teams),
        ("Cisco-Systems.Spark", MeetingApp::Webex),
        ("CiscoCollabHost.exe", MeetingApp::Webex),
        ("webex", MeetingApp::Webex),
        ("com.apple.FaceTime", MeetingApp::FaceTime),
        ("com.apple.avconferenced", MeetingApp::FaceTime),
        ("com.tinyspeck.slackmacgap.helper", MeetingApp::Slack),
        ("slack.exe", MeetingApp::Slack),
        ("slack", MeetingApp::Slack),
        ("com.hnc.Discord.helper", MeetingApp::Discord),
        ("net.whatsapp.WhatsApp", MeetingApp::WhatsApp),
        ("ru.keepcoder.Telegram", MeetingApp::Telegram),
        ("com.google.Chrome.helper", MeetingApp::Browser),
        ("com.vivaldi.Vivaldi.helper", MeetingApp::Browser),
        ("com.apple.WebKit.GPU", MeetingApp::Browser),
        ("org.mozilla.firefox", MeetingApp::Browser),
        ("com.microsoft.edgemac.helper", MeetingApp::Browser),
        ("chrome.exe", MeetingApp::Browser),
        ("msedge.exe", MeetingApp::Browser),
        ("firefox", MeetingApp::Browser),
    ];
    for (name, app) in cases {
        assert_eq!(classify_client(name), Some(app), "{name}");
    }
}

#[test]
fn other_apps_using_the_mic_are_ignored() {
    for name in [
        "com.apple.VoiceMemos",
        "com.apple.dictationim",
        "audacity.exe",
        "obs",
        "wispr-free",
        "zoomit.exe",
        "com.example.slackbuddy",
    ] {
        assert_eq!(classify_client(name), None, "{name}");
    }
}

#[test]
fn nothing_is_suggested_without_the_opt_in() {
    let mut detector = MeetingDetector::new(false);

    let suggestions = run(&mut detector, &repeat(&[ZOOM], 20));

    assert!(suggestions.iter().all(Option::is_none));
}

#[test]
fn a_detector_stays_quiet_until_the_setting_turns_it_on() {
    assert!(!MeetingDetector::default().is_enabled());
}

#[test]
fn a_meeting_app_holding_the_mic_is_suggested_once() {
    let mut detector = MeetingDetector::new(true);

    let suggestions = run(&mut detector, &repeat(&[ZOOM], 20));

    let made: Vec<_> = suggestions.iter().flatten().collect();
    assert_eq!(made, [&MeetingApp::Zoom]);
}

#[test]
fn a_brief_mic_test_is_not_a_meeting() {
    let mut detector = MeetingDetector::new(true);

    let suggestions = run(&mut detector, &[&[ZOOM], &[], &[ZOOM], &[]]);

    assert!(suggestions.iter().all(Option::is_none));
}

#[test]
fn an_app_that_only_flickers_off_the_mic_is_not_suggested_again() {
    let mut detector = MeetingDetector::new(true);
    run(&mut detector, &repeat(&[ZOOM], 3));

    let suggestions = run(&mut detector, &[&[], &[ZOOM], &[ZOOM], &[ZOOM]]);

    assert!(suggestions.iter().all(Option::is_none));
}

#[test]
fn a_new_call_after_the_app_let_go_of_the_mic_is_suggested_again() {
    let mut detector = MeetingDetector::new(true);
    run(&mut detector, &repeat(&[ZOOM], 3));
    run(&mut detector, &repeat(&[], 30)); // a minute with the mic free

    let suggestions = run(&mut detector, &repeat(&[ZOOM], 3));

    assert_eq!(
        suggestions.iter().flatten().collect::<Vec<_>>(),
        [&MeetingApp::Zoom]
    );
}

#[test]
fn nothing_is_suggested_while_already_recording() {
    let mut detector = MeetingDetector::new(true);

    for _ in 0..10 {
        assert_eq!(detector.observe(&[client(ZOOM)], true), None);
    }
    // The same call goes on after the user stops recording: don't nag about it.
    for _ in 0..10 {
        assert_eq!(detector.observe(&[client(ZOOM)], false), None);
    }
}

#[test]
fn two_apps_on_the_mic_are_suggested_separately() {
    let mut detector = MeetingDetector::new(true);

    let suggestions = run(&mut detector, &repeat(&[ZOOM, SLACK], 6));

    let made: Vec<_> = suggestions.iter().flatten().collect();
    assert_eq!(made, [&MeetingApp::Zoom, &MeetingApp::Slack]);
}

#[test]
fn several_helper_processes_of_one_app_count_once() {
    let mut detector = MeetingDetector::new(true);

    let suggestions = run(
        &mut detector,
        &repeat(&[SLACK, "com.tinyspeck.slackmacgap"], 6),
    );

    assert_eq!(suggestions.iter().flatten().count(), 1);
}

#[test]
fn turning_the_opt_in_off_stops_suggestions_at_once() {
    let mut detector = MeetingDetector::new(true);
    run(&mut detector, &[&[ZOOM]]);

    detector.set_enabled(false);

    assert!(
        run(&mut detector, &repeat(&[ZOOM], 5))
            .iter()
            .all(Option::is_none)
    );
}

#[test]
fn pipewire_capture_streams_are_listed_with_their_binary() {
    let dump = r#"[
      { "id": 30, "type": "PipeWire:Interface:Node", "info": { "state": "running", "props": {
          "media.class": "Stream/Input/Audio",
          "application.process.binary": "zoom",
          "application.process.id": 4242,
          "application.name": "ZOOM VoiceEngine" } } },
      { "id": 31, "type": "PipeWire:Interface:Node", "info": { "state": "running", "props": {
          "media.class": "Stream/Output/Audio",
          "application.process.binary": "spotify" } } },
      { "id": 32, "type": "PipeWire:Interface:Node", "info": { "state": "suspended", "props": {
          "media.class": "Stream/Input/Audio",
          "application.process.binary": "firefox" } } },
      { "id": 33, "type": "PipeWire:Interface:Node", "info": { "state": "running", "props": {
          "media.class": "Audio/Source" } } },
      { "id": 34, "type": "PipeWire:Interface:Node", "info": { "state": "running", "props": {
          "media.class": "Stream/Input/Audio",
          "application.name": "Google Chrome" } } },
      { "id": 35, "type": "PipeWire:Interface:Client", "info": { "props": { "pipewire.sec.pid": 1 } } }
    ]"#;

    let clients = parse_pw_dump(dump);

    let expected = [
        MicClient {
            pid: 4242,
            name: "zoom".to_owned(),
        },
        MicClient {
            pid: 0,
            name: "Google Chrome".to_owned(),
        },
    ];
    assert_eq!(clients, expected);
}

#[test]
fn a_pw_dump_that_cannot_be_parsed_lists_nobody() {
    assert!(parse_pw_dump("pw-dump: not running").is_empty());
}
