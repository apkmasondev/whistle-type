//! Microphone capture with WASAPI (shared mode, event driven).
//!
//! The stream is opened only while recording (or while the microphone test runs), so the Windows microphone
//! privacy indicator is off and the process does no audio work when idle. Windows' own audio engine
//! converts any device format to 16 kHz mono float (`AUTOCONVERTPCM | SRC_DEFAULT_QUALITY`); if a driver
//! refuses that, we capture the device mix format and convert ourselves. Audio stays in memory only.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

use windows::core::{implement, Interface, HRESULT, PCWSTR, PWSTR};
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Foundation::{CloseHandle, E_ACCESSDENIED, E_INVALIDARG, HANDLE, PROPERTYKEY, WAIT_OBJECT_0};
use windows::Win32::Media::Audio::*;
use windows::Win32::Media::KernelStreaming::{KSDATAFORMAT_SUBTYPE_PCM, WAVE_FORMAT_EXTENSIBLE};
use windows::Win32::Media::Multimedia::{KSDATAFORMAT_SUBTYPE_IEEE_FLOAT, WAVE_FORMAT_IEEE_FLOAT};
use windows::Win32::System::Com::StructuredStorage::PropVariantToStringAlloc;
use windows::Win32::System::Com::*;
use windows::Win32::System::Threading::{CreateEventW, SetEvent, WaitForMultipleObjects};

use crate::{log_info, log_warn};

pub const SAMPLE_RATE: u32 = 16_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioError {
    /// No input device at all.
    NoDevice,
    /// The selected device is not connected.
    DeviceNotFound(String),
    /// Windows privacy settings block microphone access for desktop apps.
    AccessDenied,
    /// Another application holds the device in exclusive mode.
    InUse,
    /// The device was unplugged / disabled during capture.
    Disconnected,
    ServiceNotRunning,
    Other(String),
}

impl std::fmt::Display for AudioError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use crate::i18n::{fmt, t};
        let s = match self {
            AudioError::NoDevice => t().err_no_mic.to_string(),
            AudioError::DeviceNotFound(n) => fmt(t().err_mic_not_connected, &[("name", n)]),
            AudioError::AccessDenied => t().err_mic_blocked.to_string(),
            AudioError::InUse => t().err_mic_in_use.to_string(),
            AudioError::Disconnected => t().err_mic_disconnected.to_string(),
            AudioError::ServiceNotRunning => t().err_audio_service.to_string(),
            AudioError::Other(m) => fmt(t().err_mic_other, &[("m", m)]),
        };
        f.write_str(&s)
    }
}

fn map_err(e: &windows::core::Error) -> AudioError {
    let code = e.code();
    if code == E_ACCESSDENIED {
        AudioError::AccessDenied
    } else if code == AUDCLNT_E_DEVICE_IN_USE {
        AudioError::InUse
    } else if code == AUDCLNT_E_DEVICE_INVALIDATED {
        AudioError::Disconnected
    } else if code == AUDCLNT_E_SERVICE_NOT_RUNNING {
        AudioError::ServiceNotRunning
    } else if code == HRESULT(0x80070490u32 as i32) {
        AudioError::NoDevice
    } else {
        AudioError::Other(format!("{} (0x{:08X})", e.message(), code.0 as u32))
    }
}

/// COM must be initialised on the calling thread.
fn enumerator() -> Result<IMMDeviceEnumerator, AudioError> {
    unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }.map_err(|e| map_err(&e))
}

fn take_pwstr(p: PWSTR) -> String {
    if p.is_null() {
        return String::new();
    }
    let s = unsafe { p.to_string().unwrap_or_default() };
    unsafe { CoTaskMemFree(Some(p.0 as *const _)) };
    s
}

fn device_id(d: &IMMDevice) -> String {
    unsafe { d.GetId() }.map(take_pwstr).unwrap_or_default()
}

pub fn device_name(d: &IMMDevice) -> String {
    unsafe {
        let Ok(store) = d.OpenPropertyStore(STGM_READ) else { return "Microphone".into() };
        let Ok(v) = store.GetValue(&PKEY_Device_FriendlyName as *const PROPERTYKEY) else { return "Microphone".into() };
        PropVariantToStringAlloc(&v).map(take_pwstr).unwrap_or_else(|_| "Microphone".into())
    }
}

/// Active capture devices. Requires COM on the calling thread.
pub fn list_devices() -> Result<Vec<Device>, AudioError> {
    let en = enumerator()?;
    let col = unsafe { en.EnumAudioEndpoints(eCapture, DEVICE_STATE_ACTIVE) }.map_err(|e| map_err(&e))?;
    let n = unsafe { col.GetCount() }.map_err(|e| map_err(&e))?;
    let mut out = Vec::new();
    for i in 0..n {
        if let Ok(d) = unsafe { col.Item(i) } {
            out.push(Device { id: device_id(&d), name: device_name(&d) });
        }
    }
    Ok(out)
}

pub fn default_device() -> Option<Device> {
    let en = enumerator().ok()?;
    let d = unsafe { en.GetDefaultAudioEndpoint(eCapture, eConsole) }.ok()?;
    Some(Device { id: device_id(&d), name: device_name(&d) })
}

fn open_device(en: &IMMDeviceEnumerator, id: Option<&str>) -> Result<IMMDevice, AudioError> {
    match id {
        Some(id) if !id.is_empty() => {
            let w = crate::util::wide(id);
            let d = unsafe { en.GetDevice(PCWSTR(w.as_ptr())) }.map_err(|_| AudioError::DeviceNotFound(id.into()))?;
            let state = unsafe { d.GetState() }.unwrap_or(DEVICE_STATE(0));
            if state != DEVICE_STATE_ACTIVE {
                return Err(AudioError::DeviceNotFound(device_name(&d)));
            }
            Ok(d)
        }
        _ => unsafe { en.GetDefaultAudioEndpoint(eCapture, eConsole) }.map_err(|e| match map_err(&e) {
            AudioError::Other(_) => AudioError::NoDevice,
            other => other,
        }),
    }
}

// ------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    /// Keep samples (at most `max_samples` at 16 kHz).
    Dictation { max_samples: usize },
    /// Only update the level meter (microphone test).
    Monitor,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AudioEvent {
    Started { device: String, converted_by_windows: bool },
    Failed(AudioError),
    LimitReached,
}

pub struct Captured {
    /// 16 kHz mono float in [-1, 1].
    pub samples: Vec<f32>,
    pub error: Option<AudioError>,
    pub device: String,
}

struct Shared {
    level: AtomicU32,
    started: AtomicBool,
}

pub struct Recorder {
    stop: HANDLE,
    shared: Arc<Shared>,
    thread: Option<JoinHandle<Captured>>,
}

unsafe impl Send for Recorder {}

impl Recorder {
    /// Opens the device and starts capturing on a new thread. Never blocks on the device.
    pub fn start(
        device_id: Option<String>,
        purpose: Purpose,
        notify: Arc<dyn Fn(AudioEvent) + Send + Sync>,
    ) -> Result<Recorder, AudioError> {
        let stop = unsafe { CreateEventW(None, true, false, None) }.map_err(|e| map_err(&e))?;
        let shared = Arc::new(Shared { level: AtomicU32::new(0), started: AtomicBool::new(false) });
        let sh = shared.clone();
        let stop_raw = stop.0 as isize;
        let thread = std::thread::Builder::new()
            .name("audio-capture".into())
            .spawn(move || {
                let stop = HANDLE(stop_raw as *mut _);
                unsafe {
                    let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
                }
                #[cfg(feature = "test-hooks")]
                if let Some(wav) = std::env::var_os("WHISTLETYPE_TEST_WAV") {
                    let c = test_source(std::path::Path::new(&wav), stop, &sh, purpose, &*notify);
                    unsafe { CoUninitialize() };
                    return c;
                }
                let mut samples = Vec::new();
                let mut device = String::new();
                let res = capture(device_id.as_deref(), purpose, stop, &sh, &mut samples, &mut device, &*notify);
                let error = res.err();
                if let Some(e) = &error {
                    notify(AudioEvent::Failed(e.clone()));
                }
                sh.level.store(0, Ordering::Relaxed);
                unsafe { CoUninitialize() };
                Captured { samples, error, device }
            })
            .map_err(|e| AudioError::Other(e.to_string()))?;
        Ok(Recorder { stop, shared, thread: Some(thread) })
    }

    /// Current input level, 0..1 (log scale, -60 dBFS .. 0 dBFS).
    pub fn level(&self) -> f32 {
        f32::from_bits(self.shared.level.load(Ordering::Relaxed))
    }

    /// A cheap, cloneable reader of the live level (for the overlay animation).
    pub fn level_source(&self) -> Box<dyn Fn() -> f32> {
        let sh = self.shared.clone();
        Box::new(move || f32::from_bits(sh.level.load(Ordering::Relaxed)))
    }

    pub fn is_started(&self) -> bool {
        self.shared.started.load(Ordering::Relaxed)
    }

    /// Stops the stream and returns everything captured.
    pub fn stop(mut self) -> Captured {
        self.finish()
    }

    fn finish(&mut self) -> Captured {
        unsafe {
            let _ = SetEvent(self.stop);
        }
        let c = self
            .thread
            .take()
            .and_then(|t| t.join().ok())
            .unwrap_or(Captured { samples: Vec::new(), error: Some(AudioError::Other("capture thread panicked".into())), device: String::new() });
        unsafe {
            let _ = CloseHandle(self.stop);
        }
        c
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        if self.thread.is_some() {
            let _ = self.finish();
        }
    }
}

fn meter(rms: f32) -> f32 {
    let db = 20.0 * rms.max(1e-6).log10();
    ((db + 60.0) / 60.0).clamp(0.0, 1.0)
}

#[derive(Clone, Copy)]
enum SampleFormat {
    F32,
    I16,
    I24,
    I32,
}

fn capture(
    device_id: Option<&str>,
    purpose: Purpose,
    stop: HANDLE,
    shared: &Shared,
    out: &mut Vec<f32>,
    device_name_out: &mut String,
    notify: &(dyn Fn(AudioEvent) + Send + Sync),
) -> Result<(), AudioError> {
    let en = enumerator()?;
    let dev = open_device(&en, device_id)?;
    *device_name_out = device_name(&dev);
    let client: IAudioClient = unsafe { dev.Activate(CLSCTX_ALL, None) }.map_err(|e| map_err(&e))?;

    let want = WAVEFORMATEX {
        wFormatTag: WAVE_FORMAT_IEEE_FLOAT as u16,
        nChannels: 1,
        nSamplesPerSec: SAMPLE_RATE,
        nAvgBytesPerSec: SAMPLE_RATE * 4,
        nBlockAlign: 4,
        wBitsPerSample: 32,
        cbSize: 0,
    };
    let buffer_hns: i64 = 2_000_000; // 200 ms
    let flags = AUDCLNT_STREAMFLAGS_EVENTCALLBACK | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY;
    let mut converted_by_windows = true;
    let mut fmt_rate = SAMPLE_RATE;
    let mut fmt_channels = 1usize;
    let mut fmt_kind = SampleFormat::F32;
    let mut client = client;
    let init = unsafe { client.Initialize(AUDCLNT_SHAREMODE_SHARED, flags, buffer_hns, 0, &want, None) };
    if let Err(e) = init {
        let code = e.code();
        if code == E_ACCESSDENIED || code == AUDCLNT_E_DEVICE_IN_USE || code == AUDCLNT_E_DEVICE_INVALIDATED {
            return Err(map_err(&e));
        }
        log_warn!("audio: 16 kHz auto-conversion refused ({}), using the device mix format", e.message());
        // A failed Initialize leaves the client unusable: activate a fresh one.
        client = unsafe { dev.Activate(CLSCTX_ALL, None) }.map_err(|e| map_err(&e))?;
        let mix = unsafe { client.GetMixFormat() }.map_err(|e| map_err(&e))?;
        let r = unsafe {
            let m: WAVEFORMATEX = std::ptr::read_unaligned(mix);
            fmt_rate = m.nSamplesPerSec;
            fmt_channels = m.nChannels.max(1) as usize;
            let tag = m.wFormatTag as u32;
            let bits = m.wBitsPerSample;
            let sub = if tag == WAVE_FORMAT_EXTENSIBLE {
                let ext: WAVEFORMATEXTENSIBLE = std::ptr::read_unaligned(mix as *const WAVEFORMATEXTENSIBLE);
                Some(ext.SubFormat)
            } else {
                None
            };
            fmt_kind = match (tag, sub, bits) {
                (WAVE_FORMAT_IEEE_FLOAT, _, 32) => SampleFormat::F32,
                (_, Some(s), 32) if s == KSDATAFORMAT_SUBTYPE_IEEE_FLOAT => SampleFormat::F32,
                (1, _, 16) => SampleFormat::I16,
                (_, Some(s), 16) if s == KSDATAFORMAT_SUBTYPE_PCM => SampleFormat::I16,
                (_, Some(s), 24) if s == KSDATAFORMAT_SUBTYPE_PCM => SampleFormat::I24,
                (_, Some(s), 32) if s == KSDATAFORMAT_SUBTYPE_PCM => SampleFormat::I32,
                _ => {
                    CoTaskMemFree(Some(mix as *const _));
                    return Err(AudioError::Other(format!("unsupported device format (tag {tag}, {bits} bit)")));
                }
            };
            let r = client.Initialize(AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_EVENTCALLBACK, buffer_hns, 0, mix, None);
            CoTaskMemFree(Some(mix as *const _));
            r
        };
        r.map_err(|e| map_err(&e))?;
        converted_by_windows = false;
    }

    let event = unsafe { CreateEventW(None, false, false, None) }.map_err(|e| map_err(&e))?;
    struct EventGuard(HANDLE);
    impl Drop for EventGuard {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
    let _guard = EventGuard(event);
    unsafe { client.SetEventHandle(event) }.map_err(|e| map_err(&e))?;
    let cap: IAudioCaptureClient = unsafe { client.GetService() }.map_err(|e| map_err(&e))?;
    unsafe { client.Start() }.map_err(|e| map_err(&e))?;
    shared.started.store(true, Ordering::SeqCst);
    log_info!(
        "audio: capture started on \"{}\" ({} Hz, {} ch, windows conversion: {})",
        device_name_out, fmt_rate, fmt_channels, converted_by_windows
    );
    notify(AudioEvent::Started { device: device_name_out.clone(), converted_by_windows });

    // max samples in device units for the fallback path
    let max_native = match purpose {
        Purpose::Dictation { max_samples } => (max_samples as u64 * fmt_rate as u64 / SAMPLE_RATE as u64) as usize,
        Purpose::Monitor => 0,
    };
    let mut native: Vec<f32> = Vec::new();
    let mut limit_hit = false;
    let mut result = Ok(());
    let handles = [stop, event];
    'outer: loop {
        let w = unsafe { WaitForMultipleObjects(&handles, false, 1000) };
        if w == WAIT_OBJECT_0 {
            break;
        }
        loop {
            let packet = match unsafe { cap.GetNextPacketSize() } {
                Ok(n) => n,
                Err(e) => {
                    result = Err(map_err(&e));
                    break 'outer;
                }
            };
            if packet == 0 {
                break;
            }
            let mut data: *mut u8 = std::ptr::null_mut();
            let mut frames = 0u32;
            let mut flags = 0u32;
            if let Err(e) = unsafe { cap.GetBuffer(&mut data, &mut frames, &mut flags, None, None) } {
                result = Err(map_err(&e));
                break 'outer;
            }
            let silent = flags & (AUDCLNT_BUFFERFLAGS_SILENT.0 as u32) != 0 || data.is_null();
            let mut sum_sq = 0.0f32;
            let mut mono_count = 0usize;
            let keep = matches!(purpose, Purpose::Dictation { .. }) && !limit_hit;
            for f in 0..frames as usize {
                let mut acc = 0.0f32;
                if !silent {
                    for c in 0..fmt_channels {
                        let i = f * fmt_channels + c;
                        acc += unsafe {
                            match fmt_kind {
                                SampleFormat::F32 => *(data as *const f32).add(i),
                                SampleFormat::I16 => *(data as *const i16).add(i) as f32 / 32768.0,
                                SampleFormat::I24 => {
                                    let p = data.add(i * 3);
                                    (i32::from_le_bytes([0, *p, *p.add(1), *p.add(2)]) >> 8) as f32 / 8_388_608.0
                                }
                                SampleFormat::I32 => *(data as *const i32).add(i) as f32 / 2_147_483_648.0,
                            }
                        };
                    }
                    acc /= fmt_channels as f32;
                }
                let acc = if acc.is_finite() { acc.clamp(-1.0, 1.0) } else { 0.0 };
                sum_sq += acc * acc;
                mono_count += 1;
                if keep {
                    if converted_by_windows {
                        out.push(acc);
                    } else {
                        native.push(acc);
                    }
                }
            }
            unsafe {
                let _ = cap.ReleaseBuffer(frames);
            }
            if mono_count > 0 {
                let rms = (sum_sq / mono_count as f32).sqrt();
                shared.level.store(meter(rms).to_bits(), Ordering::Relaxed);
            }
            if let Purpose::Dictation { max_samples } = purpose {
                let len = if converted_by_windows { out.len() } else { native.len() };
                let max = if converted_by_windows { max_samples } else { max_native };
                if !limit_hit && len >= max {
                    limit_hit = true;
                    notify(AudioEvent::LimitReached);
                }
            }
        }
    }
    unsafe {
        let _ = client.Stop();
    }
    if !converted_by_windows && !native.is_empty() {
        *out = crate::resample::resample(&native, fmt_rate, SAMPLE_RATE);
    }
    if let Purpose::Dictation { max_samples } = purpose {
        out.truncate(max_samples);
    }
    log_info!("audio: capture stopped, {:.2} s", out.len() as f32 / SAMPLE_RATE as f32);
    result
}

/// e2e tests: play a WAV file in real time instead of the microphone.
#[cfg(feature = "test-hooks")]
fn test_source(
    path: &std::path::Path,
    stop: HANDLE,
    shared: &Shared,
    purpose: Purpose,
    notify: &(dyn Fn(AudioEvent) + Send + Sync),
) -> Captured {
    use windows::Win32::System::Threading::WaitForSingleObject;
    // A .txt file holds the path of the WAV to play, so tests can switch clips without restarting the app.
    let path = if path.extension().is_some_and(|e| e == "txt") {
        std::path::PathBuf::from(std::fs::read_to_string(path).unwrap_or_default().trim())
    } else {
        path.to_path_buf()
    };
    let src = crate::wav::read(&path).map(|w| w.to_whistle_input()).unwrap_or_default();
    shared.started.store(true, Ordering::SeqCst);
    notify(AudioEvent::Started { device: "test WAV".into(), converted_by_windows: true });
    let mut out = Vec::new();
    let mut pos = 0;
    let mut produced = 0usize;
    let t0 = std::time::Instant::now();
    loop {
        if unsafe { WaitForSingleObject(stop, 10) } == WAIT_OBJECT_0 {
            break;
        }
        // real-time pacing from the clock (Sleep granularity is ~15.6 ms)
        let due = (t0.elapsed().as_secs_f64() * SAMPLE_RATE as f64) as usize;
        let n = due.saturating_sub(produced);
        if n == 0 {
            continue;
        }
        produced += n;
        let chunk: Vec<f32> = (0..n)
            .map(|_| {
                let v = src.get(pos).copied().unwrap_or(0.0);
                pos += 1;
                v
            })
            .collect();
        let rms = (chunk.iter().map(|x| x * x).sum::<f32>() / chunk.len() as f32).sqrt();
        shared.level.store(meter(rms).to_bits(), Ordering::Relaxed);
        if let Purpose::Dictation { max_samples } = purpose {
            if out.len() < max_samples {
                out.extend(chunk);
            }
        }
    }
    Captured { samples: out, error: None, device: "test WAV".into() }
}

// ------------------------------------------------------------------------------------------------
// Device change notifications
// ------------------------------------------------------------------------------------------------

#[implement(IMMNotificationClient)]
struct DeviceWatcher;

impl IMMNotificationClient_Impl for DeviceWatcher_Impl {
    fn OnDeviceStateChanged(&self, _id: &PCWSTR, _state: DEVICE_STATE) -> windows::core::Result<()> {
        crate::msg::post(crate::msg::WM_WT_DEVICES, 0, 0);
        Ok(())
    }
    fn OnDeviceAdded(&self, _id: &PCWSTR) -> windows::core::Result<()> {
        crate::msg::post(crate::msg::WM_WT_DEVICES, 0, 0);
        Ok(())
    }
    fn OnDeviceRemoved(&self, _id: &PCWSTR) -> windows::core::Result<()> {
        crate::msg::post(crate::msg::WM_WT_DEVICES, 0, 0);
        Ok(())
    }
    fn OnDefaultDeviceChanged(&self, flow: EDataFlow, role: ERole, _id: &PCWSTR) -> windows::core::Result<()> {
        if flow == eCapture && role == eConsole {
            crate::msg::post(crate::msg::WM_WT_DEVICES, 1, 0);
        }
        Ok(())
    }
    fn OnPropertyValueChanged(&self, _id: &PCWSTR, _key: &PROPERTYKEY) -> windows::core::Result<()> {
        Ok(())
    }
}

/// Keeps the notification registration alive; unregisters on drop.
pub struct DeviceNotifications {
    enumerator: IMMDeviceEnumerator,
    client: IMMNotificationClient,
}

impl DeviceNotifications {
    pub fn register() -> Result<DeviceNotifications, AudioError> {
        let enumerator = enumerator()?;
        let client: IMMNotificationClient = DeviceWatcher.into();
        unsafe { enumerator.RegisterEndpointNotificationCallback(&client) }.map_err(|e| map_err(&e))?;
        Ok(DeviceNotifications { enumerator, client })
    }
}

impl Drop for DeviceNotifications {
    fn drop(&mut self) {
        unsafe {
            let _ = self.enumerator.UnregisterEndpointNotificationCallback(&self.client);
        }
    }
}

#[allow(dead_code)]
fn _assert_interface() {
    let _ = IMMNotificationClient::IID;
    let _ = E_INVALIDARG;
}
