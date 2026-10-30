//! The real [`AudioBackend`]: the platform's default cpal host (CoreAudio, WASAPI, ALSA).
//!
//! Streams use the device's default input config and plain capture paths, so no OS echo
//! cancellation or AGC is requested (spec §3).

use std::str::FromStr;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{DeviceId, ErrorKind, FromSample, SampleFormat, SizedSample};

use crate::{AudioBackend, Error, InputDevice, StreamFormat, StreamSink};

pub struct CpalBackend {
    host: cpal::Host,
}

impl CpalBackend {
    pub fn new() -> Self {
        Self {
            host: cpal::default_host(),
        }
    }
}

impl Default for CpalBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioBackend for CpalBackend {
    type Stream = cpal::Stream;

    fn devices(&self) -> Vec<InputDevice> {
        let Ok(devices) = self.host.input_devices() else {
            return Vec::new();
        };
        devices.filter_map(|device| describe(&device)).collect()
    }

    fn default_device(&self) -> Option<String> {
        let device = self.host.default_input_device()?;
        Some(device.id().ok()?.to_string())
    }

    fn open(&self, id: &str, sink: StreamSink) -> Result<(cpal::Stream, StreamFormat), Error> {
        let unavailable = || Error::DeviceUnavailable(id.to_owned());
        let device_id = DeviceId::from_str(id).map_err(|_| unavailable())?;
        let device = self.host.device_by_id(&device_id).ok_or_else(unavailable)?;
        let supported = device.default_input_config().map_err(backend)?;
        open_stream(&device, &supported, sink)
    }
}

/// Starts capturing from `device` in `supported` format. Loopback uses this directly because an
/// output device has no default *input* config.
pub fn open_stream(
    device: &cpal::Device,
    supported: &cpal::SupportedStreamConfig,
    sink: StreamSink,
) -> Result<(cpal::Stream, StreamFormat), Error> {
    let format = StreamFormat {
        sample_rate: supported.sample_rate(),
        channels: supported.channels(),
    };
    let config = supported.config();
    let stream = match supported.sample_format() {
        SampleFormat::F32 => build::<f32>(device, &config, sink),
        SampleFormat::I16 => build::<i16>(device, &config, sink),
        SampleFormat::I32 => build::<i32>(device, &config, sink),
        SampleFormat::U16 => build::<u16>(device, &config, sink),
        SampleFormat::I8 => build::<i8>(device, &config, sink),
        SampleFormat::U8 => build::<u8>(device, &config, sink),
        SampleFormat::F64 => build::<f64>(device, &config, sink),
        other => {
            return Err(Error::Backend(format!(
                "unsupported sample format {other:?}"
            )));
        }
    }?;
    stream.play().map_err(backend)?;
    Ok((stream, format))
}

fn describe(device: &cpal::Device) -> Option<InputDevice> {
    let id = device.id().ok()?.to_string();
    let name = device
        .description()
        .map_or_else(|_| id.clone(), |d| d.name().to_owned());
    Some(InputDevice { id, name })
}

fn build<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    sink: StreamSink,
) -> Result<cpal::Stream, Error>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let errors = sink.clone();
    device
        .build_input_stream(
            *config,
            move |data: &[T], _: &cpal::InputCallbackInfo| {
                let samples: Vec<f32> = data
                    .iter()
                    .map(|&s| <f32 as FromSample<T>>::from_sample_(s))
                    .collect();
                sink.push(&samples);
            },
            move |err: cpal::Error| match err.kind() {
                ErrorKind::DeviceNotAvailable
                | ErrorKind::StreamInvalidated
                | ErrorKind::HostUnavailable => errors.device_lost(),
                ErrorKind::Xrun => tracing::debug!("input overrun"),
                _ => tracing::warn!("input stream error: {err}"),
            },
            None,
        )
        .map_err(backend)
}

fn backend(err: cpal::Error) -> Error {
    match err.kind() {
        ErrorKind::DeviceNotAvailable => Error::DeviceUnavailable(err.to_string()),
        _ => Error::Backend(err.to_string()),
    }
}
