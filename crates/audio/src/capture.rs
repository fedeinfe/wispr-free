//! The capture thread: keeps the best-ranked input device open, follows hot-plug, and turns its
//! audio into pipeline frames.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender, TrySendError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::{CpalBackend, Error, FrameConverter, InputDevice, rank_devices};

/// About 10 s of frames. If the consumer stalls longer than that, frames are dropped.
const EVENT_BUFFER: usize = 256;

/// Where the OS audio lives. [`CpalBackend`] is the real one; tests use fakes.
///
/// Streams are opened and dropped on the capture thread only, so they need not be `Send`.
pub trait AudioBackend {
    /// Audio keeps flowing into the sink while this is alive.
    type Stream;

    fn devices(&self) -> Vec<InputDevice>;
    fn default_device(&self) -> Option<String>;
    fn open(&self, id: &str, sink: StreamSink) -> Result<(Self::Stream, StreamFormat), Error>;
}

/// Native format of an opened stream, as delivered to [`StreamSink::push`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamFormat {
    pub sample_rate: u32,
    pub channels: u16,
}

/// Handed to [`AudioBackend::open`]; the stream's callbacks report through it. Cheap, never blocks.
#[derive(Debug, Clone)]
pub struct StreamSink {
    tx: Sender<Msg>,
    stream: u64,
}

impl StreamSink {
    /// Interleaved samples in the stream's [`StreamFormat`].
    pub fn push(&self, samples: &[f32]) {
        let _ = self.tx.send(Msg::Audio {
            stream: self.stream,
            samples: samples.to_vec(),
        });
    }

    /// The device went away (unplugged, disabled). Capture moves to the next ranked device.
    pub fn device_lost(&self) {
        let _ = self.tx.send(Msg::Lost {
            stream: self.stream,
        });
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum CaptureEvent {
    /// Capture switched device. `None` means there is no usable input device right now;
    /// capture resumes on its own when one appears.
    Device(Option<InputDevice>),
    /// 40 ms of pipeline audio ([`wf_core::audio::FRAME_SAMPLES`] samples, 16 kHz mono).
    Frame(Vec<f32>),
}

#[derive(Debug, Clone)]
pub struct CaptureConfig {
    /// Device ids, most preferred first. Empty means "follow the system default".
    pub ranking: Vec<String>,
    /// How often to look for plugged/unplugged devices and default-device changes.
    pub poll_interval: Duration,
}

impl Default for CaptureConfig {
    fn default() -> Self {
        Self {
            ranking: Vec::new(),
            poll_interval: Duration::from_secs(2),
        }
    }
}

/// A running capture thread. Read [`events`](Self::events) continuously; dropping the handle
/// closes the device.
pub struct Capture {
    events: Receiver<CaptureEvent>,
    control: Sender<Msg>,
    thread: Option<JoinHandle<()>>,
}

impl Capture {
    /// Starts capturing from the OS's microphones.
    pub fn start(config: CaptureConfig) -> Result<Self, Error> {
        Self::start_with(config, CpalBackend::new)
    }

    /// Starts capturing with `make_backend()`, which is called on the capture thread.
    pub fn start_with<B, F>(config: CaptureConfig, make_backend: F) -> Result<Self, Error>
    where
        B: AudioBackend,
        F: FnOnce() -> B + Send + 'static,
    {
        let (events_tx, events) = mpsc::sync_channel(EVENT_BUFFER);
        let (control, inbox) = mpsc::channel();
        let sink_tx = control.clone();
        let thread = std::thread::Builder::new()
            .name("wf-audio-capture".into())
            .spawn(move || {
                let worker = Worker {
                    backend: make_backend(),
                    config,
                    events: events_tx,
                    sink_tx,
                    current: None,
                    next_stream: 0,
                    last_device: None,
                };
                worker.run(inbox);
            })
            .map_err(|e| Error::Backend(e.to_string()))?;
        Ok(Self {
            events,
            control,
            thread: Some(thread),
        })
    }

    pub fn events(&self) -> &Receiver<CaptureEvent> {
        &self.events
    }

    /// Replaces the device ranking (settings changed). Takes effect immediately.
    pub fn set_ranking(&self, ranking: Vec<String>) {
        let _ = self.control.send(Msg::SetRanking(ranking));
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        let _ = self.control.send(Msg::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

enum Msg {
    Audio { stream: u64, samples: Vec<f32> },
    Lost { stream: u64 },
    SetRanking(Vec<String>),
    Shutdown,
}

struct Open<S> {
    device: InputDevice,
    id: u64,
    converter: FrameConverter,
    _stream: S,
}

struct Worker<B: AudioBackend> {
    backend: B,
    config: CaptureConfig,
    events: SyncSender<CaptureEvent>,
    sink_tx: Sender<Msg>,
    current: Option<Open<B::Stream>>,
    next_stream: u64,
    /// Last device reported, so `Device` events fire only on change.
    last_device: Option<Option<InputDevice>>,
}

impl<B: AudioBackend> Worker<B> {
    fn run(mut self, inbox: Receiver<Msg>) {
        let mut next_poll = Instant::now();
        loop {
            if Instant::now() >= next_poll {
                self.choose_device();
                next_poll = Instant::now() + self.config.poll_interval;
            }
            let timeout = next_poll.saturating_duration_since(Instant::now());
            match inbox.recv_timeout(timeout) {
                Ok(Msg::Audio { stream, samples }) => self.on_audio(stream, &samples),
                Ok(Msg::Lost { stream }) => {
                    if self.current.as_ref().is_some_and(|open| open.id == stream) {
                        tracing::info!("input device lost");
                        self.current = None;
                        next_poll = Instant::now();
                    }
                }
                Ok(Msg::SetRanking(ranking)) => {
                    self.config.ranking = ranking;
                    next_poll = Instant::now();
                }
                Ok(Msg::Shutdown) | Err(RecvTimeoutError::Disconnected) => break,
                Err(RecvTimeoutError::Timeout) => {}
            }
        }
    }

    /// Opens the best-ranked device that is better than the current one, if any.
    fn choose_device(&mut self) {
        let ranked = rank_devices(
            &self.backend.devices(),
            &self.config.ranking,
            self.backend.default_device().as_deref(),
        );
        if let Some(open) = &self.current
            && !ranked.iter().any(|d| d.id == open.device.id)
        {
            tracing::info!(device = %open.device.name, "input device disappeared");
            self.current = None;
        }
        let current_id = self.current.as_ref().map(|open| open.device.id.clone());
        for device in ranked {
            if Some(&device.id) == current_id.as_ref() {
                return; // nothing better is available
            }
            match self.open(&device) {
                Ok(open) => {
                    tracing::info!(device = %device.name, "capturing from input device");
                    self.current = Some(open);
                    self.report(Some(device));
                    return;
                }
                Err(e) => tracing::warn!(device = %device.name, "cannot open input device: {e}"),
            }
        }
        if self.current.is_none() {
            self.report(None);
        }
    }

    fn open(&mut self, device: &InputDevice) -> Result<Open<B::Stream>, Error> {
        self.next_stream += 1;
        let id = self.next_stream;
        let sink = StreamSink {
            tx: self.sink_tx.clone(),
            stream: id,
        };
        let (stream, format) = self.backend.open(&device.id, sink)?;
        let converter = FrameConverter::new(format.sample_rate, usize::from(format.channels))?;
        Ok(Open {
            device: device.clone(),
            id,
            converter,
            _stream: stream,
        })
    }

    fn on_audio(&mut self, stream: u64, samples: &[f32]) {
        let Some(open) = self.current.as_mut().filter(|open| open.id == stream) else {
            return; // late audio from a stream already replaced
        };
        for frame in open.converter.push(samples) {
            self.send(CaptureEvent::Frame(frame));
        }
    }

    fn report(&mut self, device: Option<InputDevice>) {
        if self.last_device.as_ref() != Some(&device) {
            self.last_device = Some(device.clone());
            self.send(CaptureEvent::Device(device));
        }
    }

    fn send(&self, event: CaptureEvent) {
        if let Err(TrySendError::Full(_)) = self.events.try_send(event) {
            tracing::warn!("capture consumer is not keeping up; dropping audio");
        }
    }
}
