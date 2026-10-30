//! ScreenCaptureKit's audio output, for macOS 13 up to the first version with process taps. It
//! needs the Screen Recording permission, and sends nothing at all while nothing plays.

use std::ptr::{self, NonNull};
use std::sync::mpsc;
use std::time::Duration;

use block2::RcBlock;
use dispatch2::{DispatchQueue, DispatchRetained};
use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{AnyThread, DefinedClass, define_class, msg_send};
use objc2_core_audio_types::{AudioBuffer, AudioBufferList};
use objc2_core_media::{
    CMBlockBuffer, CMSampleBuffer, CMTime,
    kCMSampleBufferFlag_AudioBufferList_Assure16ByteAlignment,
};
use objc2_foundation::{NSArray, NSError};
use objc2_screen_capture_kit::{
    SCContentFilter, SCShareableContent, SCStream, SCStreamConfiguration, SCStreamOutput,
    SCStreamOutputType,
};
use wf_audio::{Error, StreamFormat, StreamSink};

use super::macos::push_buffer_list;

const SAMPLE_RATE_HZ: u32 = 48_000;
const CHANNELS: u16 = 2;
/// The video can't be turned off, so make it as cheap as it gets.
const VIDEO_SIZE: usize = 2;
const REPLY_TIMEOUT: Duration = Duration::from_secs(5);

struct Ivars {
    sink: StreamSink,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[ivars = Ivars]
    struct AudioOutput;

    unsafe impl NSObjectProtocol for AudioOutput {}

    unsafe impl SCStreamOutput for AudioOutput {
        #[unsafe(method(stream:didOutputSampleBuffer:ofType:))]
        unsafe fn on_sample(
            &self,
            _stream: &SCStream,
            buffer: &CMSampleBuffer,
            kind: SCStreamOutputType,
        ) {
            if kind == SCStreamOutputType::Audio {
                push_sample_buffer(buffer, &self.ivars().sink);
            }
        }
    }
);

impl AudioOutput {
    fn new(sink: StreamSink) -> Retained<Self> {
        let this = Self::alloc().set_ivars(Ivars { sink });
        // SAFETY: NSObject's init takes no arguments.
        unsafe { msg_send![super(this), init] }
    }
}

/// Holds a stereo buffer list without a heap allocation per callback.
#[repr(C)]
struct StereoBufferList {
    list: AudioBufferList,
    second: AudioBuffer,
}

fn push_sample_buffer(buffer: &CMSampleBuffer, sink: &StreamSink) {
    let mut storage = std::mem::MaybeUninit::<StereoBufferList>::zeroed();
    let list = storage.as_mut_ptr().cast::<AudioBufferList>();
    let mut block: *mut CMBlockBuffer = ptr::null_mut();
    // SAFETY: `list` has room for the size given, and `block` is a valid out pointer.
    let status = unsafe {
        buffer.audio_buffer_list_with_retained_block_buffer(
            ptr::null_mut(),
            list,
            size_of::<StereoBufferList>(),
            None,
            None,
            kCMSampleBufferFlag_AudioBufferList_Assure16ByteAlignment,
            &mut block,
        )
    };
    let Some(block) = NonNull::new(block) else {
        return;
    };
    // SAFETY: the function above returned this block at +1, and it backs the list's data.
    let _block = unsafe { objc2_core_foundation::CFRetained::from_raw(block) };
    if status == 0
        && let Some(list) = NonNull::new(list)
    {
        // SAFETY: ScreenCaptureKit delivers 32-bit float audio, and the list was just filled in.
        unsafe { push_buffer_list(list, usize::from(CHANNELS), sink) };
    }
}

/// A running ScreenCaptureKit audio stream. Lives on the capture thread.
pub struct ScreenCaptureStream {
    stream: Retained<SCStream>,
    output: Retained<AudioOutput>,
    _queue: DispatchRetained<DispatchQueue>,
}

impl ScreenCaptureStream {
    pub fn start(sink: StreamSink) -> Result<(Self, StreamFormat), Error> {
        let content = shareable_content()?;
        // SAFETY: a plain property getter.
        let displays = unsafe { content.displays() };
        let display = displays
            .firstObject()
            .ok_or_else(|| backend("no display to attach the stream to"))?;
        // SAFETY: `display` came from the shareable content above.
        let filter = unsafe {
            SCContentFilter::initWithDisplay_excludingWindows(
                SCContentFilter::alloc(),
                &display,
                &NSArray::new(),
            )
        };
        // SAFETY: plain property setters on a new object.
        let config = unsafe {
            let config = SCStreamConfiguration::new();
            config.setCapturesAudio(true);
            config.setSampleRate(SAMPLE_RATE_HZ as isize);
            config.setChannelCount(CHANNELS as isize);
            config.setExcludesCurrentProcessAudio(true);
            config.setWidth(VIDEO_SIZE);
            config.setHeight(VIDEO_SIZE);
            config.setMinimumFrameInterval(CMTime::new(1, 1));
            config
        };
        // SAFETY: filter and configuration are valid and no delegate is needed.
        let stream = unsafe {
            SCStream::initWithFilter_configuration_delegate(
                SCStream::alloc(),
                &filter,
                &config,
                None,
            )
        };

        let output = AudioOutput::new(sink);
        let queue = DispatchQueue::new("wispr-free.system-audio", None);
        // SAFETY: `output` and `queue` are kept alive as long as the stream.
        unsafe {
            stream.addStreamOutput_type_sampleHandlerQueue_error(
                ProtocolObject::from_ref(&*output),
                SCStreamOutputType::Audio,
                Some(&queue),
            )
        }
        .map_err(|e| backend(&e.localizedDescription().to_string()))?;

        let (reply, started) = mpsc::channel();
        let completion = RcBlock::new(move |error: *mut NSError| {
            // SAFETY: the error is null or valid for the duration of the block.
            let message = unsafe { error.as_ref() }.map(|e| e.localizedDescription().to_string());
            let _ = reply.send(message);
        });
        // SAFETY: the block is copied by the callee.
        unsafe { stream.startCaptureWithCompletionHandler(Some(&completion)) };
        match started.recv_timeout(REPLY_TIMEOUT) {
            Ok(None) => {}
            Ok(Some(message)) => return Err(backend(&message)),
            Err(_) => return Err(backend("ScreenCaptureKit did not start")),
        }

        let format = StreamFormat {
            sample_rate: SAMPLE_RATE_HZ,
            channels: CHANNELS,
        };
        Ok((
            Self {
                stream,
                output,
                _queue: queue,
            },
            format,
        ))
    }
}

impl Drop for ScreenCaptureStream {
    fn drop(&mut self) {
        // SAFETY: the output was added in `start`; stopping needs no completion handler.
        unsafe {
            self.stream.stopCaptureWithCompletionHandler(None);
            let _ = self.stream.removeStreamOutput_type_error(
                ProtocolObject::from_ref(&*self.output),
                SCStreamOutputType::Audio,
            );
        }
    }
}

/// `Retained` isn't `Send`, but the answer only moves once, from the completion handler to the
/// thread that asked.
struct Reply(Result<Retained<SCShareableContent>, String>);
// SAFETY: see above, and SCShareableContent is an immutable snapshot.
unsafe impl Send for Reply {}

fn shareable_content() -> Result<Retained<SCShareableContent>, Error> {
    let (reply, answer) = mpsc::channel();
    let completion = RcBlock::new(
        move |content: *mut SCShareableContent, error: *mut NSError| {
            // SAFETY: both are null or valid for the duration of the block.
            let result = match unsafe { Retained::retain(content) } {
                Some(content) => Ok(content),
                None => Err(unsafe { error.as_ref() }.map_or_else(
                    || "no shareable content".to_owned(),
                    |e| e.localizedDescription().to_string(),
                )),
            };
            let _ = reply.send(Reply(result));
        },
    );
    // SAFETY: the block is copied by the callee.
    unsafe { SCShareableContent::getShareableContentWithCompletionHandler(&completion) };
    match answer.recv_timeout(REPLY_TIMEOUT) {
        Ok(Reply(result)) => result.map_err(|message| backend(&message)),
        Err(_) => Err(backend("ScreenCaptureKit did not answer")),
    }
}

fn backend(message: &str) -> Error {
    Error::Backend(format!("ScreenCaptureKit: {message}"))
}
