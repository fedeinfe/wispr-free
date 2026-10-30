//! Windows and Linux: cpal already speaks to the OS's loopback, so only finding the right device
//! differs. Windows records a render endpoint through WASAPI loopback, Linux records the default
//! sink's monitor source through PulseAudio or pipewire-pulse.

use cpal::traits::{DeviceTrait, HostTrait};
use wf_audio::{AudioBackend, Error, InputDevice, StreamFormat, StreamSink, open_stream};

pub struct SystemAudioBackend {
    host: Option<cpal::Host>,
}

impl SystemAudioBackend {
    pub fn new() -> Self {
        Self { host: host() }
    }

    fn output(&self) -> Option<(cpal::Device, cpal::SupportedStreamConfig)> {
        loopback_device(self.host.as_ref()?)
    }
}

impl Default for SystemAudioBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioBackend for SystemAudioBackend {
    type Stream = cpal::Stream;

    fn devices(&self) -> Vec<InputDevice> {
        self.output()
            .and_then(|(device, _)| {
                let id = device.id().ok()?.to_string();
                let name = device
                    .description()
                    .map_or_else(|_| id.clone(), |d| d.name().to_owned());
                Some(InputDevice { id, name })
            })
            .into_iter()
            .collect()
    }

    fn default_device(&self) -> Option<String> {
        self.devices().pop().map(|device| device.id)
    }

    fn open(&self, id: &str, sink: StreamSink) -> Result<(cpal::Stream, StreamFormat), Error> {
        let (device, config) = self
            .output()
            .ok_or_else(|| Error::DeviceUnavailable(id.to_owned()))?;
        open_stream(&device, &config, sink)
    }
}

#[cfg(target_os = "windows")]
fn host() -> Option<cpal::Host> {
    Some(cpal::default_host())
}

#[cfg(target_os = "windows")]
fn loopback_device(host: &cpal::Host) -> Option<(cpal::Device, cpal::SupportedStreamConfig)> {
    let device = host.default_output_device()?;
    let config = device.default_output_config().ok()?;
    Some((device, config))
}

#[cfg(target_os = "linux")]
fn host() -> Option<cpal::Host> {
    match cpal::host_from_id(cpal::HostId::PulseAudio) {
        Ok(host) => Some(host),
        Err(e) => {
            tracing::warn!("no PulseAudio or PipeWire server, so no system audio: {e}");
            None
        }
    }
}

#[cfg(target_os = "linux")]
fn loopback_device(host: &cpal::Host) -> Option<(cpal::Device, cpal::SupportedStreamConfig)> {
    let sink = host.default_output_device()?.id().ok()?;
    let monitor = format!("{sink}.monitor");
    let device = host
        .input_devices()
        .ok()?
        .find(|device| device.id().is_ok_and(|id| id.to_string() == monitor))?;
    let config = device.default_input_config().ok()?;
    Some((device, config))
}
