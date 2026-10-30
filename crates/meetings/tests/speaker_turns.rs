//! Keeping speakers stable and placing them on the transcript.

use std::time::Duration;

use wf_meetings::{
    Channel, Segment, SpeakerId, Turn, assign_speakers, split_at_turns, stable_turns,
};
use wf_pipeline::SpeakerTurn;

fn found(start_ms: u64, end_ms: u64, speaker: u32) -> SpeakerTurn {
    SpeakerTurn {
        start: Duration::from_millis(start_ms),
        end: Duration::from_millis(end_ms),
        speaker,
    }
}

fn turn(start_ms: u64, end_ms: u64, speaker: u32) -> Turn {
    Turn {
        start_ms,
        end_ms,
        speaker: SpeakerId(speaker),
    }
}

fn others(start_ms: u64, end_ms: u64) -> Segment {
    Segment {
        channel: Channel::Others,
        speaker: None,
        start_ms,
        end_ms,
        text: "words".to_owned(),
    }
}

#[test]
fn speakers_are_numbered_from_one_in_order_of_first_appearance() {
    let turns = stable_turns(
        &[],
        &[
            found(0, 1_000, 7),
            found(2_000, 3_000, 3),
            found(4_000, 5_000, 7),
        ],
    );

    assert_eq!(
        turns,
        [
            turn(0, 1_000, 1),
            turn(2_000, 3_000, 2),
            turn(4_000, 5_000, 1)
        ]
    );
}

/// The engine's numbers change between runs, and speaker names hang on ours.
#[test]
fn running_again_keeps_each_voice_on_its_id() {
    let first = stable_turns(&[], &[found(0, 1_000, 0), found(2_000, 3_000, 1)]);

    let again = stable_turns(&first, &[found(0, 1_000, 1), found(2_000, 3_000, 0)]);

    assert_eq!(again, first);
}

#[test]
fn a_new_voice_after_a_resume_gets_a_new_id() {
    let first = stable_turns(&[], &[found(0, 1_000, 0), found(2_000, 3_000, 1)]);

    let again = stable_turns(
        &first,
        &[
            found(0, 1_000, 0),
            found(2_000, 3_000, 1),
            found(9_000, 10_000, 2),
        ],
    );

    assert_eq!(again[0].speaker, SpeakerId(1));
    assert_eq!(again[1].speaker, SpeakerId(2));
    assert_eq!(again[2].speaker, SpeakerId(3));
}

/// Two voices the first run told apart may become one cluster. The other id must not get
/// reused for somebody else, or a name would jump to a different person.
#[test]
fn a_new_voice_never_takes_over_the_id_of_a_voice_that_went_quiet() {
    let first = stable_turns(&[], &[found(0, 1_000, 0), found(2_000, 3_000, 1)]);

    let again = stable_turns(&first, &[found(0, 3_000, 0), found(9_000, 10_000, 1)]);

    assert_eq!(again[0].speaker, SpeakerId(1));
    assert_eq!(again[1].speaker, SpeakerId(3));
}

/// Two new clusters both overlap one old voice: only the closer match may keep its id.
#[test]
fn an_old_id_goes_to_one_voice_only() {
    let first = stable_turns(&[], &[found(0, 10_000, 0)]);

    let again = stable_turns(&first, &[found(0, 3_000, 0), found(3_000, 10_000, 1)]);

    assert_eq!(again[0].speaker, SpeakerId(2));
    assert_eq!(again[1].speaker, SpeakerId(1));
}

#[test]
fn a_chunk_inside_one_turn_stays_whole() {
    let turns = [turn(0, 10_000, 1)];

    assert_eq!(
        split_at_turns(2_000, 6_000, &turns),
        [(2_000, 6_000, Some(SpeakerId(1)))]
    );
}

#[test]
fn a_chunk_no_turn_touches_has_no_speaker() {
    let turns = [turn(0, 1_000, 1)];

    assert_eq!(split_at_turns(5_000, 8_000, &turns), [(5_000, 8_000, None)]);
}

#[test]
fn a_chunk_splits_where_the_speaker_changes() {
    let turns = [turn(0, 5_000, 1), turn(5_000, 12_000, 2)];

    assert_eq!(
        split_at_turns(2_000, 9_000, &turns),
        [
            (2_000, 5_000, Some(SpeakerId(1))),
            (5_000, 9_000, Some(SpeakerId(2)))
        ]
    );
}

#[test]
fn a_pause_between_turns_is_shared_so_no_audio_falls_out() {
    let turns = [turn(0, 4_000, 1), turn(5_000, 12_000, 2)];

    assert_eq!(
        split_at_turns(0, 9_000, &turns),
        [
            (0, 4_500, Some(SpeakerId(1))),
            (4_500, 9_000, Some(SpeakerId(2)))
        ]
    );
}

/// A word cut out on its own is no use to ASR, and it was probably misheard as a speaker change.
#[test]
fn a_sliver_of_another_voice_joins_its_neighbour() {
    let turns = [
        turn(0, 4_000, 1),
        turn(4_000, 4_300, 2),
        turn(4_300, 9_000, 1),
    ];

    assert_eq!(
        split_at_turns(0, 9_000, &turns),
        [(0, 9_000, Some(SpeakerId(1)))]
    );
}

#[test]
fn over_talk_keeps_the_voice_already_speaking() {
    let turns = [turn(0, 6_000, 1), turn(4_000, 10_000, 2)];

    assert_eq!(
        split_at_turns(0, 10_000, &turns),
        [
            (0, 6_000, Some(SpeakerId(1))),
            (6_000, 10_000, Some(SpeakerId(2)))
        ]
    );
}

#[test]
fn a_voice_buried_inside_another_turn_is_not_a_split() {
    let turns = [turn(0, 10_000, 1), turn(2_000, 4_000, 2)];

    assert_eq!(
        split_at_turns(0, 10_000, &turns),
        [(0, 10_000, Some(SpeakerId(1)))]
    );
}

#[test]
fn live_segments_take_the_speaker_who_spoke_most_of_them() {
    let turns = [turn(0, 2_000, 1), turn(2_000, 9_000, 2)];
    let mut segments = vec![
        others(1_000, 6_000),
        others(20_000, 21_000),
        Segment {
            channel: Channel::You,
            ..others(0, 2_000)
        },
    ];

    assign_speakers(&mut segments, &turns);

    assert_eq!(segments[0].speaker, Some(SpeakerId(2)));
    assert_eq!(segments[1].speaker, None);
    assert_eq!(segments[2].speaker, None, "You is never diarized");
}
