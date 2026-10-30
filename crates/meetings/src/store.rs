//! The on-disk meeting record, the stable interface between the stages and the Meetings UI.
//! One folder per meeting, one file per stage, so a stage that fails leaves the others usable:
//!
//! ```text
//! <root>/<id>/meeting.json          title, parts, speaker names, stage states
//!             part-000.ogg …        the audio, one file per recording (ticket 15's format)
//!             live.jsonl            live transcript, appended as chunks close
//!             turns.json            who speaks when on the Others channel
//!             transcript.json       the refined transcript
//!             summary-notes.json    map step's notes, one entry per transcript chunk
//!             summary.json          the finished summary
//!             notes.md              the user's own notes
//! ```
//!
//! Every file but the audio and the live log is replaced whole through a temporary file, so a
//! crash never leaves half of one. Calls block on disk, so make them from a worker thread.

use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use wf_core::audio::AudioClip;

use crate::diarize::Turn;
use crate::reader::{MeetingAudio, read_meeting, repair_meeting};
use crate::summary::{Summary, apply_speaker_names};
use crate::transcript::{Gap, GapReason, Segment, SpeakerId, SpeakerNames, Transcript};
use crate::{Channel, Error};

/// Bump on a breaking change to any file above. A record from a newer build is refused, never
/// rewritten with fields missing.
const SCHEMA: u32 = 1;
const RECORD: &str = "meeting.json";
const LIVE_LOG: &str = "live.jsonl";
const TURNS: &str = "turns.json";
const REFINED: &str = "transcript.json";
const SUMMARY: &str = "summary.json";
const NOTES: &str = "notes.md";
pub(crate) const SUMMARY_NOTES: &str = "summary-notes.json";
/// A crash loses less than this of live text without it counting as a hole.
const MIN_INTERRUPTED_MS: u64 = 1_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MeetingRecord {
    pub schema: u32,
    /// The folder's name. Sorts by creation time.
    pub id: String,
    pub created_ms: u64,
    /// The user's title. `None` falls back to the summary's.
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub parts: Vec<Part>,
    #[serde(default)]
    pub speakers: SpeakerNames,
    #[serde(default)]
    pub audio_deleted: bool,
    #[serde(default)]
    pub stages: Stages,
}

/// One recording. A meeting has more than one after it was resumed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Part {
    pub file: String,
    /// Where it starts on the meeting's timeline.
    pub offset_ms: u64,
    /// `None` while it records, and after a crash until [`MeetingStore::recover`] runs.
    pub duration_ms: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stages {
    #[serde(default)]
    pub diarization: StageState,
    #[serde(default)]
    pub refine: StageState,
    #[serde(default)]
    pub summary: StageState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Diarization,
    Refine,
    Summary,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum StageState {
    #[default]
    Pending,
    Done {
        at_ms: u64,
    },
    /// Turned off, or impossible here. Not an error.
    Skipped {
        reason: String,
    },
    Failed {
        reason: String,
    },
}

impl StageState {
    pub fn done() -> Self {
        Self::Done { at_ms: now_ms() }
    }

    pub fn is_done(&self) -> bool {
        matches!(self, Self::Done { .. })
    }
}

/// A transcript from the refine pass, covering the timeline up to `covers_ms`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefinedTranscript {
    pub covers_ms: u64,
    pub transcript: Transcript,
}

/// Where a new recording goes. Give `path` to [`MeetingConfig`](crate::MeetingConfig).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewPart {
    pub path: PathBuf,
    /// Where it starts on the meeting's timeline.
    pub offset: Duration,
}

#[derive(Debug, Clone)]
pub struct MeetingStore {
    root: PathBuf,
}

impl MeetingStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// `<data folder>/meetings`.
    pub fn default_root() -> Option<PathBuf> {
        Some(wf_core::settings::data_dir()?.join("meetings"))
    }

    /// Makes a new, empty meeting.
    pub fn create(&self) -> Result<Meeting, Error> {
        fs::create_dir_all(&self.root)?;
        let created_ms = now_ms();
        let dir = self.claim_folder(created_ms)?;
        let id = dir
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let meeting = Meeting {
            dir,
            record: MeetingRecord {
                schema: SCHEMA,
                id,
                created_ms,
                title: None,
                parts: Vec::new(),
                speakers: SpeakerNames::new(),
                audio_deleted: false,
                stages: Stages::default(),
            },
        };
        meeting.save_record()?;
        Ok(meeting)
    }

    /// `create_dir` fails on an existing folder, so two meetings never share one.
    fn claim_folder(&self, created_ms: u64) -> Result<PathBuf, Error> {
        for attempt in 0.. {
            let dir = self.root.join(format!("{created_ms:016}-{attempt:02}"));
            match fs::create_dir(&dir) {
                Ok(()) => return Ok(dir),
                Err(e) if e.kind() == ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e.into()),
            }
        }
        unreachable!("the loop only ends by returning")
    }

    pub fn open(&self, id: &str) -> Result<Meeting, Error> {
        let dir = self.folder(id)?;
        let path = dir.join(RECORD);
        let record: MeetingRecord = match read_json(&path)? {
            Some(record) => record,
            None => return Err(Error::NoSuchMeeting(id.to_owned())),
        };
        Ok(Meeting { dir, record })
    }

    /// Every meeting, newest first. Folders that aren't meetings are skipped.
    pub fn list(&self) -> Result<Vec<MeetingRecord>, Error> {
        let entries = match fs::read_dir(&self.root) {
            Ok(entries) => entries,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        };
        let mut records = Vec::new();
        for entry in entries {
            let id = entry?.file_name().to_string_lossy().into_owned();
            if let Ok(meeting) = self.open(&id) {
                records.push(meeting.record);
            }
        }
        records.sort_by(|a, b| b.id.cmp(&a.id));
        Ok(records)
    }

    pub fn delete(&self, id: &str) -> Result<(), Error> {
        let dir = self.folder(id)?;
        match fs::remove_dir_all(dir) {
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
            other => Ok(other?),
        }
    }

    /// Finishes recordings a crash left open. Run it on launch, before anything is recording.
    /// Returns the meetings it touched.
    pub fn recover(&self) -> Result<Vec<String>, Error> {
        let mut recovered = Vec::new();
        for record in self.list()? {
            let mut meeting = self.open(&record.id)?;
            if meeting.is_recording() {
                meeting.recover_part()?;
                recovered.push(record.id);
            }
        }
        Ok(recovered)
    }

    fn folder(&self, id: &str) -> Result<PathBuf, Error> {
        let plain = !id.is_empty() && id.chars().all(|c| c.is_ascii_digit() || c == '-');
        if plain {
            Ok(self.root.join(id))
        } else {
            Err(Error::NoSuchMeeting(id.to_owned()))
        }
    }
}

/// One meeting's folder, opened. There should be one per meeting at a time: changes go through
/// this copy of the record.
#[derive(Debug)]
pub struct Meeting {
    dir: PathBuf,
    record: MeetingRecord,
}

impl Meeting {
    pub fn record(&self) -> &MeetingRecord {
        &self.record
    }

    /// Whether a recording is running, or crashed and hasn't been recovered.
    pub fn is_recording(&self) -> bool {
        self.record.parts.iter().any(|p| p.duration_ms.is_none())
    }

    /// The timeline so far. A part still recording doesn't count yet.
    pub fn length(&self) -> Duration {
        let end = self.record.parts.last().and_then(|part| {
            let duration = part.duration_ms?;
            Some(part.offset_ms + duration)
        });
        Duration::from_millis(end.unwrap_or(0))
    }

    /// Starts a recording, the first or a resumed one. The audio goes in a new file: the writer
    /// never touches a finished one. Stages the new audio outdates go back to pending.
    pub fn begin_part(&mut self) -> Result<NewPart, Error> {
        if self.is_recording() {
            return Err(Error::StillRecording);
        }
        let offset = self.length();
        let file = format!("part-{:03}.ogg", self.record.parts.len());
        self.record.parts.push(Part {
            file: file.clone(),
            offset_ms: offset.as_millis() as u64,
            duration_ms: None,
        });
        self.record.stages = Stages::default();
        self.save_record()?;
        Ok(NewPart {
            path: self.dir.join(file),
            offset,
        })
    }

    /// Ends the running recording with the length [`MeetingRecorder::stop`] returned.
    ///
    /// [`MeetingRecorder::stop`]: crate::MeetingRecorder::stop
    pub fn end_part(&mut self, length: Duration) -> Result<(), Error> {
        if let Some(part) = self.record.parts.last_mut() {
            part.duration_ms = Some(length.as_millis() as u64);
        }
        self.save_record()
    }

    fn recover_part(&mut self) -> Result<(), Error> {
        let Some(part) = self.record.parts.last() else {
            return Ok(());
        };
        let offset_ms = part.offset_ms;
        let length = repair_meeting(&self.dir.join(&part.file))?;
        let end_ms = offset_ms + length.as_millis() as u64;
        let mut log = LiveLog::open(self)?;
        let transcript = self.read_live()?;
        for channel in [Channel::You, Channel::Others] {
            let heard = transcript
                .segments
                .iter()
                .filter(|s| s.channel == channel)
                .map(|s| s.end_ms)
                .fold(offset_ms, u64::max);
            if end_ms.saturating_sub(heard) >= MIN_INTERRUPTED_MS {
                log.gap(&Gap {
                    channel,
                    start_ms: heard,
                    end_ms,
                    reason: GapReason::Interrupted,
                })?;
            }
        }
        self.end_part(length)
    }

    /// Both channels of every recording, on one timeline.
    pub fn audio(&self) -> Result<MeetingAudio, Error> {
        if self.record.audio_deleted {
            return Err(Error::AudioDeleted);
        }
        let (mut you, mut others) = (Vec::new(), Vec::new());
        for part in &self.record.parts {
            let audio = read_meeting(&self.dir.join(&part.file))?;
            you.extend_from_slice(audio.you.samples());
            others.extend_from_slice(audio.others.samples());
        }
        Ok(MeetingAudio {
            you: AudioClip::new(you),
            others: AudioClip::new(others),
        })
    }

    /// The best transcript so far: refined where the refine pass has run, live after that.
    pub fn transcript(&self) -> Result<Transcript, Error> {
        let mut transcript = self.read_live()?;
        if let Some(refined) = read_json::<RefinedTranscript>(&self.dir.join(REFINED))? {
            let covers = refined.covers_ms;
            transcript.segments.retain(|s| s.start_ms >= covers);
            transcript.gaps.retain(|g| g.start_ms >= covers);
            transcript.segments.extend(refined.transcript.segments);
            transcript.gaps.extend(refined.transcript.gaps);
        }
        transcript.sort();
        Ok(transcript)
    }

    /// The transcript exactly as the live pass heard it.
    pub fn read_live(&self) -> Result<Transcript, Error> {
        let mut transcript = Transcript::default();
        let file = match File::open(self.dir.join(LIVE_LOG)) {
            Ok(file) => file,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(transcript),
            Err(e) => return Err(e.into()),
        };
        for line in BufReader::new(file).lines() {
            // A crash can tear the line it was writing. The rest are good.
            match serde_json::from_str(&line?) {
                Ok(LiveEntry::Segment(segment)) => transcript.segments.push(segment),
                Ok(LiveEntry::Gap(gap)) => transcript.gaps.push(gap),
                Err(_) => {}
            }
        }
        transcript.sort();
        Ok(transcript)
    }

    /// Replaces the refined transcript. Clears any gap it covers.
    pub fn save_refined(&mut self, refined: &RefinedTranscript) -> Result<(), Error> {
        write_json(&self.dir.join(REFINED), refined)
    }

    /// The speaker turns on the Others channel.
    pub fn turns(&self) -> Result<Vec<Turn>, Error> {
        Ok(read_json(&self.dir.join(TURNS))?.unwrap_or_default())
    }

    pub fn save_turns(&mut self, turns: &[Turn]) -> Result<(), Error> {
        write_json(&self.dir.join(TURNS), &turns)
    }

    /// The summary, with the current speaker names. `None` until one was generated.
    pub fn summary(&self) -> Result<Option<Summary>, Error> {
        let Some(summary) = read_json::<Summary>(&self.dir.join(SUMMARY))? else {
            return Ok(None);
        };
        Ok(Some(apply_speaker_names(summary, &self.record.speakers)))
    }

    pub fn save_summary(&mut self, summary: &Summary) -> Result<(), Error> {
        write_json(&self.dir.join(SUMMARY), summary)
    }

    /// The user's own notes. Empty until they write some.
    pub fn notes(&self) -> Result<String, Error> {
        match fs::read_to_string(self.dir.join(NOTES)) {
            Ok(notes) => Ok(notes),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(String::new()),
            Err(e) => Err(e.into()),
        }
    }

    pub fn set_notes(&mut self, notes: &str) -> Result<(), Error> {
        Ok(write_atomic(&self.dir.join(NOTES), notes.as_bytes())?)
    }

    /// Sets the user's title. `None` goes back to the summary's.
    pub fn set_title(&mut self, title: Option<&str>) -> Result<(), Error> {
        self.record.title = title
            .map(str::trim)
            .filter(|title| !title.is_empty())
            .map(str::to_owned);
        self.save_record()
    }

    /// Names a speaker. A blank name goes back to "Speaker N". Stored apart from the transcript
    /// and the summary, so it holds when either is generated again.
    pub fn rename_speaker(&mut self, speaker: SpeakerId, name: &str) -> Result<(), Error> {
        let name = name.trim();
        if name.is_empty() {
            self.record.speakers.remove(&speaker);
        } else {
            self.record.speakers.insert(speaker, name.to_owned());
        }
        self.save_record()
    }

    pub fn set_stage(&mut self, stage: Stage, state: StageState) -> Result<(), Error> {
        let stages = &mut self.record.stages;
        *match stage {
            Stage::Diarization => &mut stages.diarization,
            Stage::Refine => &mut stages.refine,
            Stage::Summary => &mut stages.summary,
        } = state;
        self.save_record()
    }

    /// Whether the transcript can't get better: nothing recording, no holes, and every stage
    /// that needs the audio has run, or is off.
    pub fn transcript_is_final(&self) -> Result<bool, Error> {
        let settled = |state: &StageState| {
            matches!(state, StageState::Done { .. } | StageState::Skipped { .. })
        };
        let stages = &self.record.stages;
        Ok(!self.is_recording()
            && settled(&stages.diarization)
            && settled(&stages.refine)
            && self.transcript()?.gaps.is_empty())
    }

    /// Removes the audio once the transcript is final. After this nothing can be refined.
    pub fn delete_audio(&mut self) -> Result<(), Error> {
        if !self.transcript_is_final()? {
            return Err(Error::TranscriptNotFinal);
        }
        for part in &self.record.parts {
            match fs::remove_file(self.dir.join(&part.file)) {
                Err(e) if e.kind() != ErrorKind::NotFound => return Err(e.into()),
                _ => {}
            }
        }
        self.record.audio_deleted = true;
        self.save_record()
    }

    pub(crate) fn path(&self, file: &str) -> PathBuf {
        self.dir.join(file)
    }

    fn save_record(&self) -> Result<(), Error> {
        write_json(&self.dir.join(RECORD), &self.record)
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum LiveEntry {
    Segment(Segment),
    Gap(Gap),
}

/// Appends the live transcript as it arrives. Each line reaches the disk before the next one
/// starts, so a crash loses only what was still being transcribed.
#[derive(Debug)]
pub struct LiveLog {
    file: File,
}

impl LiveLog {
    pub fn open(meeting: &Meeting) -> Result<Self, Error> {
        let path = meeting.dir.join(LIVE_LOG);
        let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
        // A torn last line would swallow the next one.
        if fs::read(&path)?.last().is_some_and(|&last| last != b'\n') {
            file.write_all(b"\n")?;
        }
        Ok(Self { file })
    }

    pub fn segment(&mut self, segment: &Segment) -> Result<(), Error> {
        self.append(&LiveEntry::Segment(segment.clone()))
    }

    pub fn gap(&mut self, gap: &Gap) -> Result<(), Error> {
        self.append(&LiveEntry::Gap(gap.clone()))
    }

    fn append(&mut self, entry: &LiveEntry) -> Result<(), Error> {
        let mut line = serde_json::to_vec(entry)?;
        line.push(b'\n');
        self.file.write_all(&line)?;
        self.file.sync_data()?;
        Ok(())
    }
}

pub(crate) fn read_json<T: DeserializeOwned>(path: &Path) -> Result<Option<T>, Error> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let header: serde_json::Value = serde_json::from_str(&text)?;
    if header.get("schema").and_then(|schema| schema.as_u64()) > Some(u64::from(SCHEMA)) {
        return Err(Error::NewerRecord(path.to_owned()));
    }
    Ok(Some(serde_json::from_value(header)?))
}

pub(crate) fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), Error> {
    let text = serde_json::to_vec_pretty(value)?;
    Ok(write_atomic(path, &text)?)
}

/// Writes through a temporary file, so a crash never leaves half of one.
fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".tmp");
    let mut file = File::create(&temporary)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    fs::rename(&temporary, path)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u64)
}
