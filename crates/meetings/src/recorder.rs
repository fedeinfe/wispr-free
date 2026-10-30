//! One meeting recording: both capture streams, aligned and written to disk as they arrive.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
use std::thread::JoinHandle;
use std::time::Duration;

use wf_audio::{AudioBackend, Capture, CaptureConfig, CaptureEvent, CpalBackend, InputDevice};

use crate::{Channel, Error, MeetingWriter, StereoAligner, SystemAudioBackend};

/// About 10 s of both channels. The writer outruns capture by far, so this only absorbs a
/// momentarily slow disk.
const WRITE_BUFFER: usize = 512;
/// [`MeetingEvent`]s beyond this are dropped rather than slowing the recording down.
const EVENT_BUFFER: usize = 256;
/// How quickly the capture threads notice [`MeetingRecorder::stop`].
const STOP_POLL: Duration = Duration::from_millis(50);

#[derive(Debug, Clone)]
pub struct MeetingConfig {
    /// Where the recording goes. Must not exist yet.
    pub path: PathBuf,
    /// Microphone device ids, most preferred first. Empty means the system default.
    pub mic_ranking: Vec<String>,
    /// How often to look for device changes.
    pub poll_interval: Duration,
}

impl MeetingConfig {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            mic_ranking: Vec::new(),
            poll_interval: CaptureConfig::default().poll_interval,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum MeetingEvent {
    /// 40 ms of one channel in the pipeline format, for the live transcript and the level meter.
    Frame { channel: Channel, samples: Vec<f32> },
    /// A channel switched device. `None` means it has no source right now and records silence;
    /// it resumes on its own when one appears.
    Device {
        channel: Channel,
        device: Option<InputDevice>,
    },
    /// Writing to disk failed and the recording stopped. What reached the disk stays readable.
    Failed(String),
}

/// A meeting being recorded. Only ever started by an explicit call: detection merely suggests.
///
/// Dropping it finishes the file like [`stop`](Self::stop) does.
pub struct MeetingRecorder {
    events: Receiver<MeetingEvent>,
    stop: Arc<AtomicBool>,
    pumps: Vec<JoinHandle<()>>,
    worker: Option<JoinHandle<Result<Duration, Error>>>,
}

impl MeetingRecorder {
    /// Records the microphone and system audio.
    pub fn start(config: MeetingConfig) -> Result<Self, Error> {
        Self::start_with(config, CpalBackend::new, SystemAudioBackend::new)
    }

    /// Records from the given backends, each made on its own capture thread.
    pub fn start_with<M, S, FM, FS>(
        config: MeetingConfig,
        make_mic: FM,
        make_system: FS,
    ) -> Result<Self, Error>
    where
        M: AudioBackend,
        S: AudioBackend,
        FM: FnOnce() -> M + Send + 'static,
        FS: FnOnce() -> S + Send + 'static,
    {
        let writer = MeetingWriter::create(&config.path)?;
        let mic = Capture::start_with(
            CaptureConfig {
                ranking: config.mic_ranking,
                poll_interval: config.poll_interval,
            },
            make_mic,
        )?;
        let system = Capture::start_with(
            CaptureConfig {
                ranking: Vec::new(),
                poll_interval: config.poll_interval,
            },
            make_system,
        )?;

        let (input, inbox) = mpsc::sync_channel(WRITE_BUFFER);
        let (events_tx, events) = mpsc::sync_channel(EVENT_BUFFER);
        let stop = Arc::new(AtomicBool::new(false));
        let pumps = vec![
            pump(Channel::You, mic, input.clone(), stop.clone())?,
            pump(Channel::Others, system, input, stop.clone())?,
        ];
        let worker = std::thread::Builder::new()
            .name("wf-meeting-writer".into())
            .spawn(move || write_meeting(writer, inbox, events_tx))?;
        Ok(Self {
            events,
            stop,
            pumps,
            worker: Some(worker),
        })
    }

    pub fn events(&self) -> &Receiver<MeetingEvent> {
        &self.events
    }

    /// Stops capturing, writes what is left and returns the meeting's length.
    pub fn stop(mut self) -> Result<Duration, Error> {
        self.finish()
    }

    fn finish(&mut self) -> Result<Duration, Error> {
        self.stop.store(true, Ordering::Relaxed);
        for pump in self.pumps.drain(..) {
            let _ = pump.join();
        }
        match self.worker.take() {
            Some(worker) => worker.join().unwrap_or(Err(Error::WriterPanicked)),
            None => Err(Error::WriterPanicked),
        }
    }
}

impl Drop for MeetingRecorder {
    fn drop(&mut self) {
        if self.worker.is_some()
            && let Err(e) = self.finish()
        {
            tracing::error!("meeting did not finish cleanly: {e}");
        }
    }
}

/// Moves one capture's events to the writer. The capture lives on this thread and closes its
/// device when the thread ends.
fn pump(
    channel: Channel,
    capture: Capture,
    input: SyncSender<(Channel, CaptureEvent)>,
    stop: Arc<AtomicBool>,
) -> Result<JoinHandle<()>, Error> {
    let thread = std::thread::Builder::new()
        .name("wf-meeting-capture".into())
        .spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                match capture.events().recv_timeout(STOP_POLL) {
                    Ok(event) => {
                        if input.send((channel, event)).is_err() {
                            break;
                        }
                    }
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }
        })?;
    Ok(thread)
}

/// Runs until both captures are gone. After a write failure it keeps draining so capture never
/// blocks, and stops touching the file.
fn write_meeting(
    writer: MeetingWriter,
    inbox: Receiver<(Channel, CaptureEvent)>,
    events: SyncSender<MeetingEvent>,
) -> Result<Duration, Error> {
    let notify = |event| {
        // A full buffer means nobody is reading right now. The recording must not wait for them.
        let _ = events.try_send(event);
    };
    let mut writer = Some(writer);
    let mut failure = None;
    let mut aligner = StereoAligner::new();
    for (channel, event) in inbox {
        match event {
            CaptureEvent::Device(device) => notify(MeetingEvent::Device { channel, device }),
            CaptureEvent::Frame(samples) => {
                let frames = aligner.push(channel, samples.clone());
                notify(MeetingEvent::Frame { channel, samples });
                if let Some(active) = writer.as_mut()
                    && let Err(e) = frames
                        .iter()
                        .try_for_each(|frame| active.write_frame(&frame.you, &frame.others))
                {
                    tracing::error!("meeting recording stopped: {e}");
                    notify(MeetingEvent::Failed(e.to_string()));
                    writer = None;
                    failure = Some(e);
                }
            }
        }
    }

    let mut writer = match (writer, failure) {
        (Some(writer), _) => writer,
        (None, Some(e)) => return Err(e),
        (None, None) => unreachable!("the writer only goes away on failure"),
    };
    for frame in aligner.drain() {
        writer.write_frame(&frame.you, &frame.others)?;
    }
    writer.finish()
}
