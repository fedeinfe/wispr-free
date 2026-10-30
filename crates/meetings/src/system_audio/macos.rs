//! macOS: a CoreAudio process tap, or ScreenCaptureKit where taps don't exist yet.
//!
//! This doesn't use cpal's loopback: cpal only taps a device that has no input streams and
//! otherwise opens its input, so a USB headset set as default output would put the user's own
//! mic on the Others channel.
//!
//! A tap needs the "System Audio Recording" permission. Without it the stream still runs and
//! delivers silence, so a denied permission can't be told from a quiet call. Bundled apps also
//! need `NSAudioCaptureUsageDescription` in their Info.plist.

use std::ffi::{CStr, c_void};
use std::mem::size_of;
use std::ptr::{self, NonNull};

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{AnyThread, available};
use objc2_core_audio::{
    AudioDeviceCreateIOProcID, AudioDeviceDestroyIOProcID, AudioDeviceIOProcID, AudioDeviceStart,
    AudioDeviceStop, AudioHardwareCreateAggregateDevice, AudioHardwareCreateProcessTap,
    AudioHardwareDestroyAggregateDevice, AudioHardwareDestroyProcessTap,
    AudioObjectGetPropertyData, AudioObjectGetPropertyDataSize, AudioObjectID,
    AudioObjectPropertyAddress, AudioObjectPropertySelector, CATapDescription, CATapMuteBehavior,
    kAudioAggregateDeviceIsPrivateKey, kAudioAggregateDeviceNameKey,
    kAudioAggregateDeviceTapAutoStartKey, kAudioAggregateDeviceTapListKey,
    kAudioAggregateDeviceUIDKey, kAudioHardwarePropertyDefaultOutputDevice,
    kAudioHardwarePropertyTranslatePIDToProcessObject, kAudioObjectPropertyElementMain,
    kAudioObjectPropertyScopeGlobal, kAudioObjectSystemObject, kAudioObjectUnknown,
    kAudioSubTapDriftCompensationKey, kAudioSubTapUIDKey, kAudioTapPropertyFormat,
};
use objc2_core_audio_types::{
    AudioBuffer, AudioBufferList, AudioStreamBasicDescription, AudioTimeStamp,
    kAudioFormatFlagIsFloat,
};
use objc2_core_foundation::{CFDictionary, CFRetained, CFString};
use objc2_foundation::{NSArray, NSDictionary, NSNumber, NSString, NSUUID};
use wf_audio::{AudioBackend, Error, InputDevice, StreamFormat, StreamSink};

use super::screen_capture::ScreenCaptureStream;

const SYSTEM_OBJECT: AudioObjectID = kAudioObjectSystemObject as AudioObjectID;
const FIRST_TAP_VERSION_NAME: &str = "macOS 14.4";

/// What the tap listens to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scope {
    Everything,
    /// One process, kept off the speakers.
    MutedProcess(u32),
}

#[derive(Debug)]
pub struct SystemAudioBackend {
    scope: Scope,
}

impl SystemAudioBackend {
    pub fn new() -> Self {
        Self {
            scope: Scope::Everything,
        }
    }

    /// Taps a single process and silences it, so `examples/meeting.rs tap-test` can prove the tap
    /// works from its own test tone without hearing anything else on the machine.
    pub fn muted_process(pid: u32) -> Self {
        Self {
            scope: Scope::MutedProcess(pid),
        }
    }
}

impl Default for SystemAudioBackend {
    fn default() -> Self {
        Self::new()
    }
}

/// Keeps audio flowing until dropped.
pub enum SystemStream {
    Tap(TapStream),
    ScreenCapture(ScreenCaptureStream),
}

impl AudioBackend for SystemAudioBackend {
    type Stream = SystemStream;

    fn devices(&self) -> Vec<InputDevice> {
        let output: Option<AudioObjectID> =
            object_property(SYSTEM_OBJECT, kAudioHardwarePropertyDefaultOutputDevice);
        output
            .into_iter()
            .map(|id| InputDevice {
                id: format!("system:{id}"),
                name: "System audio".to_owned(),
            })
            .collect()
    }

    fn default_device(&self) -> Option<String> {
        self.devices().pop().map(|device| device.id)
    }

    fn open(&self, _id: &str, sink: StreamSink) -> Result<(SystemStream, StreamFormat), Error> {
        if available!(macos = 14.4) {
            match TapStream::start(self.scope, sink.clone()) {
                Ok((tap, format)) => return Ok((SystemStream::Tap(tap), format)),
                Err(e) if self.scope == Scope::Everything => {
                    tracing::warn!("process tap failed, trying ScreenCaptureKit: {e}");
                }
                Err(e) => return Err(e),
            }
        } else {
            tracing::info!("before {FIRST_TAP_VERSION_NAME}, using ScreenCaptureKit");
        }
        let (stream, format) = ScreenCaptureStream::start(sink)?;
        Ok((SystemStream::ScreenCapture(stream), format))
    }
}

/// A process tap, the private aggregate device it needs, and the callback reading from it.
pub struct TapStream {
    tap: AudioObjectID,
    aggregate: AudioObjectID,
    proc_id: AudioDeviceIOProcID,
    _context: Box<Context>,
}

struct Context {
    sink: StreamSink,
    channels: usize,
}

impl TapStream {
    fn start(scope: Scope, sink: StreamSink) -> Result<(Self, StreamFormat), Error> {
        let description = tap_description(scope)?;
        let mut tap: AudioObjectID = kAudioObjectUnknown as AudioObjectID;
        // SAFETY: `description` is a valid object and `tap` a valid out pointer.
        check("create the process tap", unsafe {
            AudioHardwareCreateProcessTap(Some(&description), &mut tap)
        })?;
        // From here on `Drop` cleans up whatever has been created.
        let mut stream = Self {
            tap,
            aggregate: kAudioObjectUnknown as AudioObjectID,
            proc_id: None,
            _context: Box::new(Context { sink, channels: 0 }),
        };

        let asbd: AudioStreamBasicDescription = object_property(tap, kAudioTapPropertyFormat)
            .ok_or_else(|| Error::Backend("cannot read the tap's format".to_owned()))?;
        if asbd.mFormatFlags & kAudioFormatFlagIsFloat == 0 || asbd.mBitsPerChannel != 32 {
            return Err(Error::Backend(format!(
                "unexpected tap format: {} bit, flags {:#x}",
                asbd.mBitsPerChannel, asbd.mFormatFlags
            )));
        }
        stream._context.channels = asbd.mChannelsPerFrame as usize;

        let properties = aggregate_properties(&description);
        // SAFETY: `NSDictionary` is toll-free bridged to `CFDictionary`.
        let properties_cf: &CFDictionary =
            unsafe { &*Retained::as_ptr(&properties).cast::<CFDictionary>() };
        // SAFETY: valid dictionary and out pointer.
        check("create the aggregate device", unsafe {
            AudioHardwareCreateAggregateDevice(properties_cf, NonNull::from(&mut stream.aggregate))
        })?;

        let context = ptr::from_ref::<Context>(&stream._context).cast_mut().cast();
        // SAFETY: `context` stays valid until the IOProc is destroyed in `Drop`.
        check("add the audio callback", unsafe {
            AudioDeviceCreateIOProcID(
                stream.aggregate,
                Some(on_audio),
                context,
                NonNull::from(&mut stream.proc_id),
            )
        })?;
        // SAFETY: the device and IOProc were just created.
        check("start the aggregate device", unsafe {
            AudioDeviceStart(stream.aggregate, stream.proc_id)
        })?;

        let format = StreamFormat {
            sample_rate: asbd.mSampleRate as u32,
            channels: asbd.mChannelsPerFrame as u16,
        };
        Ok((stream, format))
    }
}

impl Drop for TapStream {
    fn drop(&mut self) {
        // SAFETY: each id was created by `start` and is only destroyed here. Stopping first
        // guarantees the callback has returned before the context goes away.
        unsafe {
            if self.proc_id.is_some() {
                AudioDeviceStop(self.aggregate, self.proc_id);
                AudioDeviceDestroyIOProcID(self.aggregate, self.proc_id);
            }
            if self.aggregate != kAudioObjectUnknown as AudioObjectID {
                AudioHardwareDestroyAggregateDevice(self.aggregate);
            }
            AudioHardwareDestroyProcessTap(self.tap);
        }
    }
}

fn tap_description(scope: Scope) -> Result<Retained<CATapDescription>, Error> {
    let allocated = CATapDescription::alloc();
    // SAFETY: the initialisers take retained arrays of process object ids.
    let description = match scope {
        Scope::Everything => unsafe {
            CATapDescription::initStereoGlobalTapButExcludeProcesses(allocated, &NSArray::new())
        },
        Scope::MutedProcess(pid) => {
            let process = process_object(pid)?;
            let processes = NSArray::from_retained_slice(&[NSNumber::new_u32(process)]);
            let description =
                unsafe { CATapDescription::initStereoMixdownOfProcesses(allocated, &processes) };
            // SAFETY: a plain property setter.
            unsafe { description.setMuteBehavior(CATapMuteBehavior::Muted) };
            description
        }
    };
    // SAFETY: plain property setters.
    unsafe {
        description.setName(&NSString::from_str("wispr-free meeting audio"));
        description.setPrivate(true);
    }
    Ok(description)
}

fn process_object(pid: u32) -> Result<AudioObjectID, Error> {
    let pid = pid as i32;
    let address = global_address(kAudioHardwarePropertyTranslatePIDToProcessObject);
    let mut object = kAudioObjectUnknown as AudioObjectID;
    let mut size = size_of::<AudioObjectID>() as u32;
    // SAFETY: the qualifier is a pid and the output an object id, as this property defines.
    let status = unsafe {
        AudioObjectGetPropertyData(
            SYSTEM_OBJECT,
            NonNull::from(&address),
            size_of::<i32>() as u32,
            ptr::from_ref(&pid).cast(),
            NonNull::from(&mut size),
            NonNull::from(&mut object).cast(),
        )
    };
    if status != 0 || object == kAudioObjectUnknown as AudioObjectID {
        return Err(Error::DeviceUnavailable(format!(
            "process {pid} has no audio object"
        )));
    }
    Ok(object)
}

fn aggregate_properties(
    description: &CATapDescription,
) -> Retained<NSDictionary<NSString, AnyObject>> {
    // SAFETY: a plain property getter.
    let tap_uid = unsafe { description.UUID() }.UUIDString();
    let yes = NSNumber::new_bool(true);
    let tap = NSDictionary::<NSString, AnyObject>::from_slices(
        &[
            &*key(kAudioSubTapUIDKey),
            &*key(kAudioSubTapDriftCompensationKey),
        ],
        &[tap_uid.as_ref(), yes.as_ref()],
    );
    let taps = NSArray::from_retained_slice(&[tap]);
    let aggregate_uid = NSUUID::UUID().UUIDString();
    let name = NSString::from_str("wispr-free meeting tap");
    NSDictionary::from_slices(
        &[
            &*key(kAudioAggregateDeviceNameKey),
            &*key(kAudioAggregateDeviceUIDKey),
            &*key(kAudioAggregateDeviceIsPrivateKey),
            &*key(kAudioAggregateDeviceTapAutoStartKey),
            &*key(kAudioAggregateDeviceTapListKey),
        ],
        &[
            name.as_ref(),
            aggregate_uid.as_ref(),
            yes.as_ref(),
            yes.as_ref(),
            taps.as_ref(),
        ],
    )
}

fn key(name: &CStr) -> Retained<NSString> {
    NSString::from_str(&name.to_string_lossy())
}

/// Runs on CoreAudio's I/O thread.
unsafe extern "C-unwind" fn on_audio(
    _device: AudioObjectID,
    _now: NonNull<AudioTimeStamp>,
    input: NonNull<AudioBufferList>,
    _input_time: NonNull<AudioTimeStamp>,
    _output: NonNull<AudioBufferList>,
    _output_time: NonNull<AudioTimeStamp>,
    context: *mut c_void,
) -> i32 {
    // SAFETY: `context` is the `Context` the IOProc was created with, alive until it's destroyed.
    let context = unsafe { &*context.cast::<Context>() };
    // SAFETY: CoreAudio passes a valid list for the duration of the call.
    unsafe { push_buffer_list(input, context.channels, &context.sink) };
    0
}

/// Forwards 32-bit float audio as interleaved samples, whether it arrives interleaved in one
/// buffer or with a buffer per channel.
///
/// # Safety
///
/// `list` must be a valid `AudioBufferList` whose buffers hold 32-bit floats.
pub(super) unsafe fn push_buffer_list(
    list: NonNull<AudioBufferList>,
    channels: usize,
    sink: &StreamSink,
) {
    // SAFETY: the list is valid per this function's contract, and `mBuffers` is declared as a
    // one-element array but really holds `mNumberBuffers` entries.
    let buffers: &[AudioBuffer] = unsafe {
        let count = (*list.as_ptr()).mNumberBuffers as usize;
        std::slice::from_raw_parts(ptr::addr_of!((*list.as_ptr()).mBuffers).cast(), count)
    };
    let samples = |buffer: &AudioBuffer| -> &[f32] {
        if buffer.mData.is_null() {
            return &[];
        }
        let len = buffer.mDataByteSize as usize / size_of::<f32>();
        // SAFETY: valid per this function's contract.
        unsafe { std::slice::from_raw_parts(buffer.mData.cast::<f32>(), len) }
    };
    match buffers {
        [] => {}
        [single] if single.mNumberChannels as usize == channels => sink.push(samples(single)),
        planar => {
            let frames = planar.iter().map(|b| samples(b).len()).min().unwrap_or(0);
            let interleaved: Vec<f32> = (0..frames)
                .flat_map(|frame| planar.iter().map(move |b| samples(b)[frame]))
                .collect();
            sink.push(&interleaved);
        }
    }
}

fn global_address(selector: AudioObjectPropertySelector) -> AudioObjectPropertyAddress {
    AudioObjectPropertyAddress {
        mSelector: selector,
        mScope: kAudioObjectPropertyScopeGlobal,
        mElement: kAudioObjectPropertyElementMain,
    }
}

/// A fixed-size property of an audio object.
pub(crate) fn object_property<T: Copy>(
    object: AudioObjectID,
    selector: AudioObjectPropertySelector,
) -> Option<T> {
    let address = global_address(selector);
    let mut value = std::mem::MaybeUninit::<T>::uninit();
    let mut size = size_of::<T>() as u32;
    // SAFETY: `value` has room for `size` bytes, and only an exact-size answer is read back.
    let status = unsafe {
        AudioObjectGetPropertyData(
            object,
            NonNull::from(&address),
            0,
            ptr::null(),
            NonNull::from(&mut size),
            NonNull::new_unchecked(value.as_mut_ptr()).cast(),
        )
    };
    // SAFETY: CoreAudio filled in all of it.
    (status == 0 && size as usize == size_of::<T>()).then(|| unsafe { value.assume_init() })
}

/// A list-of-objects property. Empty on any error.
pub(crate) fn object_list(
    object: AudioObjectID,
    selector: AudioObjectPropertySelector,
) -> Vec<AudioObjectID> {
    let address = global_address(selector);
    let mut size = 0u32;
    // SAFETY: valid address and out pointer.
    let status = unsafe {
        AudioObjectGetPropertyDataSize(
            object,
            NonNull::from(&address),
            0,
            ptr::null(),
            NonNull::from(&mut size),
        )
    };
    if status != 0 {
        return Vec::new();
    }
    let mut objects = vec![0 as AudioObjectID; size as usize / size_of::<AudioObjectID>()];
    if objects.is_empty() {
        return objects;
    }
    // SAFETY: the buffer holds `size` bytes.
    let status = unsafe {
        AudioObjectGetPropertyData(
            object,
            NonNull::from(&address),
            0,
            ptr::null(),
            NonNull::from(&mut size),
            NonNull::new_unchecked(objects.as_mut_ptr()).cast(),
        )
    };
    if status != 0 {
        return Vec::new();
    }
    objects.truncate(size as usize / size_of::<AudioObjectID>());
    objects
}

/// A CFString property, which the caller owns once returned.
pub(crate) fn object_string(
    object: AudioObjectID,
    selector: AudioObjectPropertySelector,
) -> Option<String> {
    let string: *const CFString = object_property(object, selector)?;
    let string = NonNull::new(string.cast_mut())?;
    // SAFETY: CoreAudio returns CFString properties at +1.
    let string = unsafe { CFRetained::from_raw(string) };
    Some(string.to_string()).filter(|s| !s.is_empty())
}

fn check(action: &str, status: i32) -> Result<(), Error> {
    if status == 0 {
        Ok(())
    } else {
        Err(Error::Backend(format!(
            "cannot {action}: OSStatus {status}"
        )))
    }
}
