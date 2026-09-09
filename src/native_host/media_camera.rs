//! Windows camera preview on the native window's UI thread. No encoding,
//! recording, microphone capture or network delivery. The caller must wait for
//! winit's actual window creation (which establishes the UI STA) before start.

use std::{
    marker::PhantomData,
    rc::Rc,
    time::{Duration, Instant},
};
use windows::{
    core::HSTRING,
    Devices::Enumeration::{DeviceClass, DeviceInformation, DeviceInformationCollection},
    Graphics::Imaging::{BitmapAlphaMode, BitmapPixelFormat, BitmapSize, SoftwareBitmap},
    Media::{
        Capture::{
            Frames::{
                MediaFrameReader, MediaFrameReaderAcquisitionMode, MediaFrameReaderStartStatus,
                MediaFrameSourceKind,
            },
            MediaCapture, MediaCaptureInitializationSettings, MediaCaptureMemoryPreference,
            MediaCaptureSharingMode, StreamingCaptureMode,
        },
        MediaProperties::MediaEncodingSubtypes,
    },
    Storage::Streams::{Buffer, DataReader},
};
use windows_future::{AsyncStatus, IAsyncAction, IAsyncOperation};

pub struct CameraDevice {
    pub id: String,
    pub name: String,
}
pub struct CameraScan(IAsyncOperation<DeviceInformationCollection>);
impl CameraScan {
    pub fn begin() -> Result<Self, String> {
        DeviceInformation::FindAllAsyncDeviceClass(DeviceClass::VideoCapture)
            .map(Self)
            .map_err(message)
    }
    pub fn poll(&self) -> Result<Option<Vec<CameraDevice>>, String> {
        if self.0.Status().map_err(message)? == AsyncStatus::Started {
            return Ok(None);
        }
        let devices = self.0.GetResults().map_err(message)?;
        let mut result = Vec::new();
        for index in 0..devices.Size().map_err(message)? {
            let device = devices.GetAt(index).map_err(message)?;
            result.push(CameraDevice {
                id: device.Id().map_err(message)?.to_string(),
                name: device.Name().map_err(message)?.to_string(),
            });
        }
        Ok(Some(result))
    }
}
impl Drop for CameraScan {
    fn drop(&mut self) {
        let _ = self.0.Cancel();
        let _ = self.0.Close();
    }
}

/// A latest-only, bounded CPU preview, consumed locally by the setup surface.
pub struct PreviewFrame {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}
enum Pending {
    Initialize(IAsyncAction),
    Reader(IAsyncOperation<MediaFrameReader>),
    Start(IAsyncOperation<MediaFrameReaderStartStatus>),
    Stop(IAsyncAction),
}

/// !Send pins all OS operations to the main native UI thread.
pub struct CameraPreview {
    capture: Option<MediaCapture>,
    reader: Option<MediaFrameReader>,
    pending: Option<Pending>,
    device_id: String,
    deadline: Instant,
    last_frame: Instant,
    pub status: String,
    pub failure: Option<String>,
    pub frame: Option<PreviewFrame>,
    _ui_thread: PhantomData<Rc<()>>,
}

impl Default for CameraPreview {
    fn default() -> Self {
        Self {
            capture: None,
            reader: None,
            pending: None,
            device_id: String::new(),
            deadline: Instant::now(),
            last_frame: Instant::now(),
            status: "idle".into(),
            failure: None,
            frame: None,
            _ui_thread: PhantomData,
        }
    }
}

impl CameraPreview {
    /// A preview must be explicitly stopped before replacing its device.
    pub fn start(&mut self, device_id: &str, native_window_created: bool) -> Result<(), String> {
        if !native_window_created {
            return Err("native window is not ready for camera consent".into());
        }
        if self.capture.is_some() || self.pending.is_some() {
            return Err("stop the active preview before selecting another camera".into());
        }
        let settings = MediaCaptureInitializationSettings::new().map_err(message)?;
        settings
            .SetVideoDeviceId(&HSTRING::from(device_id))
            .map_err(message)?;
        settings
            .SetStreamingCaptureMode(StreamingCaptureMode::Video)
            .map_err(message)?;
        settings
            .SetMemoryPreference(MediaCaptureMemoryPreference::Cpu)
            .map_err(message)?;
        settings
            .SetSharingMode(MediaCaptureSharingMode::SharedReadOnly)
            .map_err(message)?;
        let capture = MediaCapture::new().map_err(message)?;
        let initialize = match capture.InitializeWithSettingsAsync(&settings) {
            Ok(operation) => operation,
            Err(error) => {
                let _ = capture.Close();
                return Err(message(error));
            }
        };
        self.capture = Some(capture);
        self.failure = None;
        self.pending = Some(Pending::Initialize(initialize));
        self.device_id = device_id.into();
        self.deadline = Instant::now() + Duration::from_secs(20);
        self.status = "initializing".into();
        Ok(())
    }

    pub fn stop(&mut self) {
        self.frame = None;
        if matches!(self.pending, Some(Pending::Stop(_))) {
            return;
        }
        self.cancel_pending();
        if let Some(reader) = &self.reader {
            match reader.StopAsync() {
                Ok(operation) => {
                    self.pending = Some(Pending::Stop(operation));
                    self.deadline = Instant::now() + Duration::from_secs(5);
                    self.status = "stopping".into();
                    return;
                }
                Err(error) => {
                    self.fail(format!("camera stop failed: {error}"));
                    return;
                }
            }
        }
        self.close_handles();
        self.status = self.failure.clone().unwrap_or_else(|| "stopped".into());
    }

    /// Poll from Update while the native event loop continues pumping consent
    /// and frame delivery. Never join a WinRT async operation on the UI thread.
    pub fn poll(&mut self) {
        if self.pending.is_some() && Instant::now() > self.deadline {
            self.fail("camera operation timed out".into());
            return;
        }
        if let Err(error) = self.advance() {
            self.fail(error);
        }
    }

    fn advance(&mut self) -> Result<(), String> {
        if let Some(pending) = self.pending.take() {
            match pending {
                Pending::Initialize(operation) => {
                    if match operation.Status() {
                        Ok(status) => status,
                        Err(error) => {
                            let _ = operation.Cancel();
                            let _ = operation.Close();
                            return Err(message(error));
                        }
                    } == AsyncStatus::Started
                    {
                        self.pending = Some(Pending::Initialize(operation));
                        return Ok(());
                    }
                    let result = operation.GetResults();
                    let _ = operation.Close();
                    result.map_err(message)?;
                    let capture = self
                        .capture
                        .as_ref()
                        .ok_or("camera initialization lost its owner")?;
                    let sources = capture.FrameSources().map_err(message)?;
                    let iter = sources.First().map_err(message)?;
                    let mut selected = None;
                    while iter.HasCurrent().map_err(message)? {
                        let source = iter.Current().map_err(message)?.Value().map_err(message)?;
                        let info = source.Info().map_err(message)?;
                        if info.SourceKind().map_err(message)? == MediaFrameSourceKind::Color
                            && info
                                .DeviceInformation()
                                .and_then(|device| device.Id())
                                .is_ok_and(|id| id == self.device_id)
                        {
                            selected = Some(source);
                            break;
                        }
                        iter.MoveNext().map_err(message)?;
                    }
                    let source = selected.ok_or("selected camera has no readable color source")?;
                    let operation = capture
                        .CreateFrameReaderWithSubtypeAndSizeAsync(
                            &source,
                            &MediaEncodingSubtypes::Bgra8().map_err(message)?,
                            BitmapSize {
                                Width: 320,
                                Height: 240,
                            },
                        )
                        .map_err(message)?;
                    self.pending = Some(Pending::Reader(operation));
                    self.status = "opening preview".into();
                }
                Pending::Reader(operation) => {
                    if match operation.Status() {
                        Ok(status) => status,
                        Err(error) => {
                            let _ = operation.Cancel();
                            let _ = operation.Close();
                            return Err(message(error));
                        }
                    } == AsyncStatus::Started
                    {
                        self.pending = Some(Pending::Reader(operation));
                        return Ok(());
                    }
                    let result = operation.GetResults();
                    let _ = operation.Close();
                    let reader = result.map_err(message)?;
                    self.reader = Some(reader.clone());
                    reader
                        .SetAcquisitionMode(MediaFrameReaderAcquisitionMode::Realtime)
                        .map_err(message)?;
                    let operation = reader.StartAsync().map_err(message)?;
                    self.reader = Some(reader);
                    self.pending = Some(Pending::Start(operation));
                }
                Pending::Start(operation) => {
                    if match operation.Status() {
                        Ok(status) => status,
                        Err(error) => {
                            let _ = operation.Cancel();
                            let _ = operation.Close();
                            return Err(message(error));
                        }
                    } == AsyncStatus::Started
                    {
                        self.pending = Some(Pending::Start(operation));
                        return Ok(());
                    }
                    let result = operation.GetResults();
                    let _ = operation.Close();
                    let status = result.map_err(message)?;
                    if status != MediaFrameReaderStartStatus::Success {
                        return Err(format!("camera preview refused: {status:?}"));
                    }
                    self.status = "previewing".into();
                    self.last_frame = Instant::now();
                }
                Pending::Stop(operation) => {
                    if match operation.Status() {
                        Ok(status) => status,
                        Err(error) => {
                            let _ = operation.Cancel();
                            let _ = operation.Close();
                            return Err(message(error));
                        }
                    } == AsyncStatus::Started
                    {
                        self.pending = Some(Pending::Stop(operation));
                        return Ok(());
                    }
                    let result = operation.GetResults().map_err(message);
                    let _ = operation.Close();
                    self.close_handles();
                    self.status = "stopped".into();
                    result?;
                }
            }
        } else if let Some(reader) = &self.reader {
            if self.last_frame.elapsed() < Duration::from_millis(250) {
                return Ok(());
            }
            // A null latest-frame result is normal between arrivals. A bounded
            // absence becomes a visible failure rather than a frozen preview.
            match reader.TryAcquireLatestFrame() {
              Ok(frame) => {
                let copied = copy_frame(&frame);
                let _ = frame.Close();
                self.frame = Some(copied?);
                self.last_frame = Instant::now();
            }
              Err(error) if error.code() != windows::core::Error::empty().code() => return Err(message(error)),
              Err(_) if self.last_frame.elapsed() > Duration::from_secs(5) => return Err("camera stopped providing preview frames; it may be disconnected or unavailable".into()),
              Err(_) => {}
            }
        }
        Ok(())
    }

    fn fail(&mut self, error: String) {
        self.cancel_pending();
        self.close_handles();
        self.frame = None;
        self.failure = Some(error.clone());
        self.status = error;
    }
    fn cancel_pending(&mut self) {
        match self.pending.take() {
            Some(Pending::Initialize(op) | Pending::Stop(op)) => {
                let _ = op.Cancel();
                let _ = op.Close();
            }
            Some(Pending::Reader(op)) => {
                let _ = op.Cancel();
                let _ = op.Close();
            }
            Some(Pending::Start(op)) => {
                let _ = op.Cancel();
                let _ = op.Close();
            }
            None => {}
        }
    }
    fn close_handles(&mut self) {
        if let Some(reader) = self.reader.take() {
            let _ = reader.Close();
        }
        if let Some(capture) = self.capture.take() {
            let _ = capture.Close();
        }
    }
}
impl Drop for CameraPreview {
    fn drop(&mut self) {
        self.cancel_pending();
        self.close_handles();
    }
}

fn copy_frame(
    frame: &windows::Media::Capture::Frames::MediaFrameReference,
) -> Result<PreviewFrame, String> {
    let bitmap = frame
        .VideoMediaFrame()
        .and_then(|video| video.SoftwareBitmap())
        .map_err(message)?;
    let converted = SoftwareBitmap::ConvertWithAlpha(
        &bitmap,
        BitmapPixelFormat::Bgra8,
        BitmapAlphaMode::Ignore,
    )
    .map_err(message);
    let _ = bitmap.Close();
    let converted = converted?;
    let result = (|| {
        let width = converted.PixelWidth().map_err(message)?;
        let height = converted.PixelHeight().map_err(message)?;
        if width <= 0 || height <= 0 || width > 640 || height > 480 {
            return Err("camera preview dimensions exceed the allocation limit".into());
        }
        let size = (width as u32)
            .checked_mul(height as u32)
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or("camera preview size overflow")?;
        let buffer = Buffer::Create(size).map_err(message)?;
        converted.CopyToBuffer(&buffer).map_err(message)?;
        if buffer.Length().map_err(message)? != size {
            return Err("camera preview buffer has an unexpected length".into());
        }
        let reader = DataReader::FromBuffer(&buffer).map_err(message)?;
        let mut bgra = vec![0; size as usize];
        let copied = reader.ReadBytes(&mut bgra).map_err(message);
        let _ = reader.Close();
        copied?;
        Ok(PreviewFrame {
            width: width as u32,
            height: height as u32,
            bgra,
        })
    })();
    let _ = converted.Close();
    result
}
fn message(error: windows::core::Error) -> String {
    error.to_string()
}

/// Camera interface ids remain part of the key even when names are unique,
/// so unplugging a same-named neighbour cannot change the selected endpoint.
pub fn discovered_cameras(
    devices: &[CameraDevice],
) -> Vec<super::bridge_media::DiscoveredMediaDevice> {
    use super::bridge_media::{
        DeviceAvailability, DiscoveredMediaDevice, MediaDeviceIdentity, MediaKind,
    };
    devices
        .iter()
        .map(|device| DiscoveredMediaDevice {
            identity: MediaDeviceIdentity::new(format!("camera:{}#{}", device.name, device.id)),
            kind: MediaKind::Camera,
            name: Some(device.name.clone()),
            default: false,
            availability: DeviceAvailability::Available,
        })
        .collect()
}

/// Explicit setup preview. No simulation, listener, Station identity or audio
/// input is installed. Close/Escape requests teardown before exiting the loop.
pub fn run_preview(profile: &super::bridge_profile::BridgeProfile, surface: &str) -> i32 {
    use bevy::prelude::*;
    let result = super::bridge_media::validate_media(&profile.media)
        .map_err(|error| error.to_string())
        .and_then(|media| {
            media
                .surfaces
                .iter()
                .find(|entry| entry.surface == surface)
                .and_then(|entry| entry.camera.as_ref())
                .map(|id| id.as_str().to_string())
                .ok_or_else(|| format!("Surface {surface:?} has no assigned camera"))
        });
    let selected = match result {
        Ok(id) => id,
        Err(error) => {
            eprintln!("{error}");
            return 1;
        }
    };
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: format!("Project Phoenix — camera {surface} — Escape to stop"),
                    resolution: (640, 480).into(),
                    ..default()
                }),
                close_when_requested: false,
                ..default()
            })
            .disable::<bevy::audio::AudioPlugin>()
            .set(bevy::log::LogPlugin {
                filter: "warn".into(),
                ..default()
            }),
    );
    app.insert_non_send_resource(PreviewWindow {
        selected,
        surface: surface.into(),
        scan: None,
        preview: CameraPreview::default(),
        image: None,
        received_frame: false,
        started: Instant::now(),
        opening: true,
        closing: false,
        failed: false,
    });
    app.add_systems(Startup, preview_scene)
        .add_systems(Update, update_preview_window);
    match app.run() {
        AppExit::Success => 0,
        AppExit::Error(code) => i32::from(code.get()),
    }
}

struct PreviewWindow {
    selected: String,
    surface: String,
    scan: Option<CameraScan>,
    preview: CameraPreview,
    image: Option<bevy::prelude::Handle<bevy::prelude::Image>>,
    received_frame: bool,
    started: Instant,
    opening: bool,
    closing: bool,
    failed: bool,
}

fn preview_scene(
    mut commands: bevy::prelude::Commands,
    mut images: bevy::prelude::ResMut<bevy::prelude::Assets<bevy::prelude::Image>>,
    mut state: bevy::prelude::NonSendMut<PreviewWindow>,
) {
    use bevy::{
        asset::RenderAssetUsages,
        prelude::*,
        render::render_resource::{Extent3d, TextureDimension, TextureFormat},
    };
    let image = images.add(Image::new_fill(
        Extent3d {
            width: 320,
            height: 240,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        &[0, 0, 0, 255],
        TextureFormat::Bgra8UnormSrgb,
        RenderAssetUsages::default(),
    ));
    commands.spawn(Camera2d);
    commands.spawn((
        Sprite::from_image(image.clone()),
        Transform::from_scale(Vec3::splat(2.0)),
    ));
    state.image = Some(image);
}

fn update_preview_window(
    mut state: bevy::prelude::NonSendMut<PreviewWindow>,
    mut windows: bevy::prelude::Query<
        (bevy::prelude::Entity, &mut bevy::prelude::Window),
        bevy::prelude::With<bevy::window::PrimaryWindow>,
    >,
    mut images: bevy::prelude::ResMut<bevy::prelude::Assets<bevy::prelude::Image>>,
    input: bevy::prelude::Res<bevy::prelude::ButtonInput<bevy::prelude::KeyCode>>,
    mut close: bevy::prelude::MessageReader<bevy::window::WindowCloseRequested>,
    mut exit: bevy::prelude::MessageWriter<bevy::prelude::AppExit>,
) {
    use bevy::{
        asset::RenderAssetUsages,
        prelude::*,
        render::render_resource::{Extent3d, TextureDimension, TextureFormat},
    };
    let Ok((entity, mut window)) = windows.single_mut() else {
        return;
    };
    // Bevy 0.18 keeps this registry on the winit thread, not in World.
    // NonSendMut<PreviewWindow> keeps this system on that same owner thread.
    if !bevy::winit::WINIT_WINDOWS.with(|windows| windows.borrow().get_window(entity).is_some()) {
        return;
    }
    if state.opening && state.started.elapsed() > Duration::from_secs(10) {
        state.failed = true;
        state.closing = true;
        state.scan = None;
        state.preview.fail("camera enumeration timed out".into());
    }
    if input.just_pressed(KeyCode::Escape)
        || close.read().any(|event| event.window == entity)
        || state.started.elapsed() > Duration::from_secs(30)
    {
        if state.started.elapsed() > Duration::from_secs(30) && !state.received_frame {
            state.failed = true;
            state
                .preview
                .fail("camera preview expired without a frame".into());
        }
        state.closing = true;
        state.scan = None;
        state.preview.stop();
    }
    if state.opening && !state.closing {
        if state.scan.is_none() {
            match CameraScan::begin() {
                Ok(scan) => state.scan = Some(scan),
                Err(error) => {
                    state.preview.status = error;
                    state.failed = true;
                    state.closing = true;
                }
            }
        }
        if let Some(scan) = &state.scan {
            match scan.poll() {
                Ok(Some(devices)) => {
                    let discovered = discovered_cameras(&devices);
                    let id = discovered
                        .iter()
                        .position(|device| device.identity.as_str() == state.selected)
                        .map(|index| devices[index].id.clone());
                    state.scan = None;
                    state.opening = false;
                    match id
                        .ok_or_else(|| {
                            format!("camera {} is missing; nothing substituted", state.selected)
                        })
                        .and_then(|id| state.preview.start(&id, true))
                    {
                        Ok(()) => {}
                        Err(error) => {
                            state.preview.status = error;
                            state.failed = true;
                            state.closing = true;
                        }
                    }
                }
                Ok(None) => {}
                Err(error) => {
                    state.scan = None;
                    state.preview.status = error;
                    state.failed = true;
                    state.closing = true;
                }
            }
        }
    }
    state.preview.poll();
    if state.preview.failure.is_some() {
        state.failed = true;
        state.closing = true;
    }
    if !state.opening
        && !state.closing
        && state.preview.capture.is_none()
        && state.preview.pending.is_none()
    {
        state.failed = true;
        state.closing = true;
    }
    window.title = format!(
        "Project Phoenix — camera {} — {} — Escape to stop",
        state.surface, state.preview.status
    );
    if let Some(frame) = state.preview.frame.take() {
        state.received_frame = true;
        if let Some(image) = state
            .image
            .as_ref()
            .and_then(|handle| images.get_mut(handle))
        {
            *image = Image::new(
                Extent3d {
                    width: frame.width,
                    height: frame.height,
                    depth_or_array_layers: 1,
                },
                TextureDimension::D2,
                frame.bgra,
                TextureFormat::Bgra8UnormSrgb,
                RenderAssetUsages::default(),
            );
        }
    }
    if state.closing && state.preview.capture.is_none() && state.preview.pending.is_none() {
        println!(
            "Surface {:?}: camera preview closed: {}",
            state.surface, state.preview.status
        );
        exit.write(if state.failed {
            AppExit::error()
        } else {
            AppExit::Success
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn camera_identity_survives_same_named_neighbour_disconnection() {
        let first = CameraDevice {
            id: "interface-a".into(),
            name: "Camera".into(),
        };
        let alone = discovered_cameras(std::slice::from_ref(&first));
        let pair = discovered_cameras(&[
            first,
            CameraDevice {
                id: "interface-b".into(),
                name: "Camera".into(),
            },
        ]);
        assert_eq!(alone[0].identity, pair[0].identity);
        assert_ne!(pair[0].identity, pair[1].identity);
    }
    #[test]
    fn preview_refuses_capture_before_a_native_window_exists() {
        let mut preview = CameraPreview::default();
        assert!(preview
            .start("endpoint", false)
            .unwrap_err()
            .contains("not ready"));
        assert!(preview.capture.is_none());
        assert!(preview.pending.is_none());
    }
    #[test]
    fn terminal_failure_survives_explicit_teardown() {
        let mut preview = CameraPreview::default();
        preview.fail("stop operation failed".into());
        preview.stop();
        assert_eq!(preview.failure.as_deref(), Some("stop operation failed"));
        assert!(preview.frame.is_none());
        assert!(preview.capture.is_none());
    }
}
