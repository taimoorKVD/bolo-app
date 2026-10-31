//! Windows volume levels for the microphone Bolo records from and the speakers
//! it plays sounds on. Laymen can't find these in Windows Settings, so Bolo
//! reads them and offers a one-click fix in its own UI. Nothing is changed
//! unless the user presses that button.

use serde::Serialize;

/// Below this the mic is effectively muted for speech.
pub const QUIET_MIC: f32 = 0.25;
/// Below this the speakers are effectively silent.
pub const QUIET_OUT: f32 = 0.05;

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AudioHealth {
    pub mic_level: Option<f32>,
    pub mic_muted: bool,
    pub mic_quiet: bool,
    pub output_level: Option<f32>,
    pub output_muted: bool,
    pub output_quiet: bool,
}

pub fn health(mic_name: Option<&str>) -> AudioHealth {
    let mic = imp::endpoint(true, mic_name);
    let out = imp::endpoint(false, None);
    let mut health = AudioHealth::default();
    if let Some((level, muted)) = mic {
        health.mic_level = Some(level);
        health.mic_muted = muted;
        health.mic_quiet = muted || level < QUIET_MIC;
    }
    if let Some((level, muted)) = out {
        health.output_level = Some(level);
        health.output_muted = muted;
        health.output_quiet = muted || level < QUIET_OUT;
    }
    health
}

/// Raise a quiet mic to a sensible level and unmute it.
pub fn fix_mic(mic_name: Option<&str>) -> Result<(), String> {
    imp::raise(true, mic_name, 0.8).map_err(|e| format!("Couldn't change the microphone level ({e})."))
}

/// Raise silent speakers to a sensible level and unmute them.
pub fn fix_output() -> Result<(), String> {
    imp::raise(false, None, 0.4).map_err(|e| format!("Couldn't change the speaker volume ({e})."))
}

#[cfg(windows)]
mod imp {
    use crate::audio::normalize_mic_name;
    use windows::core::{GUID, Interface};
    use windows::Win32::Foundation::PROPERTYKEY;
    use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
    use windows::Win32::Media::Audio::{
        eCapture, eConsole, eRender, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator, DEVICE_STATE_ACTIVE,
    };
    use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED};

    const FRIENDLY_NAME: PROPERTYKEY = PROPERTYKEY {
        fmtid: GUID::from_u128(0xa45c254e_df1c_4efd_8020_67d146a850e0),
        pid: 14,
    };

    unsafe fn device(capture: bool, name: Option<&str>) -> windows::core::Result<IMMDevice> {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let flow = if capture { eCapture } else { eRender };
        if let Some(wanted) = name.map(normalize_mic_name).filter(|n| !n.is_empty()) {
            let all = enumerator.EnumAudioEndpoints(flow, DEVICE_STATE_ACTIVE)?;
            for i in 0..all.GetCount()? {
                let dev = all.Item(i)?;
                if let Some(found) = crate::audio::friendly_name(&dev, &FRIENDLY_NAME) {
                    if normalize_mic_name(&found) == wanted {
                        return Ok(dev);
                    }
                }
            }
        }
        enumerator.GetDefaultAudioEndpoint(flow, eConsole)
    }

    unsafe fn volume(capture: bool, name: Option<&str>) -> windows::core::Result<IAudioEndpointVolume> {
        let dev = device(capture, name)?;
        let raw = dev.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None)?;
        raw.cast()
    }

    pub fn endpoint(capture: bool, name: Option<&str>) -> Option<(f32, bool)> {
        unsafe {
            let vol = volume(capture, name).ok()?;
            let level = vol.GetMasterVolumeLevelScalar().ok()?;
            let muted = vol.GetMute().map(|b| b.as_bool()).unwrap_or(false);
            Some((level, muted))
        }
    }

    pub fn raise(capture: bool, name: Option<&str>, to: f32) -> windows::core::Result<()> {
        unsafe {
            let vol = volume(capture, name)?;
            let level = vol.GetMasterVolumeLevelScalar()?;
            if level < to {
                vol.SetMasterVolumeLevelScalar(to, std::ptr::null())?;
            }
            vol.SetMute(false, std::ptr::null())?;
            Ok(())
        }
    }
}

#[cfg(not(windows))]
mod imp {
    pub fn endpoint(_capture: bool, _name: Option<&str>) -> Option<(f32, bool)> {
        None
    }
    pub fn raise(_capture: bool, _name: Option<&str>, _to: f32) -> Result<(), String> {
        Err("not supported on this system".into())
    }
}
