//! Native Window Video Viewer - Ultra low latency rendering
//! Bypasses JavaScript completely for realtime performance

use std::num::NonZeroU32;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use crossbeam_channel::{bounded, Receiver, Sender, TryRecvError};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalSize};
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

use super::decoder::H264Decoder;
use super::network::RtpReceiver;
use super::types::{BroadcastError, StreamConfig};

/// Frame data for rendering
pub struct FrameBuffer {
    pub data: Vec<u32>, // ARGB format for softbuffer
    pub width: u32,
    pub height: u32,
}

/// Native video viewer with direct rendering
pub struct NativeViewer {
    running: Arc<AtomicBool>,
    frame_tx: Option<Sender<FrameBuffer>>,
    receiver_thread: Option<thread::JoinHandle<()>>,
}

impl NativeViewer {
    pub fn new() -> Self {
        Self {
            running: Arc::new(AtomicBool::new(false)),
            frame_tx: None,
            receiver_thread: None,
        }
    }

    /// Start receiving and displaying video in a native window
    pub fn start(&mut self, config: StreamConfig) -> Result<(), BroadcastError> {
        if self.running.load(Ordering::SeqCst) {
            return Err(BroadcastError::NetworkError("Already running".into()));
        }

        self.running.store(true, Ordering::SeqCst);

        // Channel for frames: receiver thread -> render thread
        let (frame_tx, frame_rx) = bounded::<FrameBuffer>(2); // Small buffer for low latency
        self.frame_tx = Some(frame_tx.clone());

        let running = self.running.clone();

        // Start network receiver thread
        self.receiver_thread = Some(thread::spawn(move || {
            if let Err(e) = run_receiver(running, config, frame_tx) {
                log::error!("Receiver error: {}", e);
            }
        }));

        // Start window in main thread (required by winit)
        let running_window = self.running.clone();
        thread::spawn(move || {
            if let Err(e) = run_window(running_window, frame_rx) {
                log::error!("Window error: {:?}", e);
            }
        });

        Ok(())
    }

    pub fn stop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        if let Some(handle) = self.receiver_thread.take() {
            let _ = handle.join();
        }
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }
}

impl Drop for NativeViewer {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Network receiver thread - receives RTP and decodes H.264
fn run_receiver(
    running: Arc<AtomicBool>,
    config: StreamConfig,
    frame_tx: Sender<FrameBuffer>,
) -> Result<(), BroadcastError> {
    log::info!("Native viewer receiver starting: port {}", config.port);

    let mut receiver = RtpReceiver::new(config.port, config.network_mode)?;
    let mut decoder = H264Decoder::new()?;
    
    let mut waiting_for_keyframe = true;
    let mut frames_decoded = 0u64;

    while running.load(Ordering::SeqCst) {
        match receiver.receive_frame() {
            Ok(Some(h264_frame)) => {
                // Check for keyframe
                let is_keyframe = is_h264_keyframe(&h264_frame);
                
                if waiting_for_keyframe {
                    if is_keyframe {
                        log::info!("Got keyframe, starting decode");
                        waiting_for_keyframe = false;
                    } else {
                        continue;
                    }
                }

                // Decode H.264 to RGBA
                match decoder.decode(&h264_frame) {
                    Ok(Some(frame)) => {
                        frames_decoded += 1;
                        
                        // Convert RGBA to ARGB (softbuffer format)
                        let argb = rgba_to_argb(&frame.rgba_data, frame.width, frame.height);
                        
                        let buffer = FrameBuffer {
                            data: argb,
                            width: frame.width,
                            height: frame.height,
                        };

                        // Send to render thread (non-blocking, drop old frames)
                        let _ = frame_tx.try_send(buffer);
                        
                        if frames_decoded % 60 == 0 {
                            log::info!("Decoded {} frames", frames_decoded);
                        }
                    }
                    Ok(None) => {}
                    Err(e) => {
                        log::warn!("Decode error: {}", e);
                        waiting_for_keyframe = true;
                    }
                }
            }
            Ok(None) => {
                thread::sleep(Duration::from_micros(500));
            }
            Err(e) => {
                log::warn!("Receive error: {}", e);
                thread::sleep(Duration::from_millis(10));
            }
        }
    }

    log::info!("Receiver stopped, decoded {} frames", frames_decoded);
    Ok(())
}

/// Check if H.264 frame contains keyframe (IDR)
#[inline]
fn is_h264_keyframe(data: &[u8]) -> bool {
    for i in 0..data.len().saturating_sub(4) {
        if data[i] == 0 && data[i+1] == 0 {
            let (offset, found) = if data[i+2] == 1 {
                (i + 3, true)
            } else if data[i+2] == 0 && i + 3 < data.len() && data[i+3] == 1 {
                (i + 4, true)
            } else {
                (0, false)
            };
            
            if found && offset < data.len() {
                let nal_type = data[offset] & 0x1F;
                if nal_type == 5 || nal_type == 7 { // IDR or SPS
                    return true;
                }
            }
        }
    }
    false
}

/// Convert RGBA to ARGB (u32 array for softbuffer)
#[inline]
fn rgba_to_argb(rgba: &[u8], width: u32, height: u32) -> Vec<u32> {
    let pixel_count = (width * height) as usize;
    let mut argb = Vec::with_capacity(pixel_count);
    
    for i in 0..pixel_count {
        let idx = i * 4;
        if idx + 3 < rgba.len() {
            let r = rgba[idx] as u32;
            let g = rgba[idx + 1] as u32;
            let b = rgba[idx + 2] as u32;
            // ARGB format: 0xAARRGGBB
            argb.push(0xFF000000 | (r << 16) | (g << 8) | b);
        }
    }
    
    argb
}

/// Window application handler
struct VideoApp {
    running: Arc<AtomicBool>,
    frame_rx: Receiver<FrameBuffer>,
    window: Option<Arc<Window>>,
    surface: Option<softbuffer::Surface<Arc<Window>, Arc<Window>>>,
    current_size: (u32, u32),
}

impl VideoApp {
    fn new(running: Arc<AtomicBool>, frame_rx: Receiver<FrameBuffer>) -> Self {
        Self {
            running,
            frame_rx,
            window: None,
            surface: None,
            current_size: (1280, 720),
        }
    }

    fn render_frame(&mut self, frame: &FrameBuffer) {
        let Some(surface) = &mut self.surface else { return };
        let Some(window) = &self.window else { return };

        // Resize surface if needed
        if self.current_size != (frame.width, frame.height) {
            self.current_size = (frame.width, frame.height);
            let _ = window.request_inner_size(PhysicalSize::new(frame.width, frame.height));
        }

        let size = window.inner_size();
        if size.width == 0 || size.height == 0 {
            return;
        }

        // Resize surface buffer
        if let (Some(w), Some(h)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) {
            let _ = surface.resize(w, h);
        }

        // Get buffer and copy frame data
        if let Ok(mut buffer) = surface.buffer_mut() {
            let buf_len = buffer.len();
            let frame_len = frame.data.len();

            if buf_len == frame_len {
                // Direct copy - fastest
                buffer.copy_from_slice(&frame.data);
            } else {
                // Scale to fit (simple nearest neighbor)
                let src_w = frame.width as usize;
                let src_h = frame.height as usize;
                let dst_w = size.width as usize;
                let dst_h = size.height as usize;

                for y in 0..dst_h {
                    for x in 0..dst_w {
                        let src_x = x * src_w / dst_w;
                        let src_y = y * src_h / dst_h;
                        let src_idx = src_y * src_w + src_x;
                        let dst_idx = y * dst_w + x;
                        
                        if src_idx < frame_len && dst_idx < buf_len {
                            buffer[dst_idx] = frame.data[src_idx];
                        }
                    }
                }
            }

            let _ = buffer.present();
        }
    }
}

impl ApplicationHandler for VideoApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let attrs = Window::default_attributes()
            .with_title("Screen Broadcast - Student View")
            .with_inner_size(LogicalSize::new(1280, 720));

        match event_loop.create_window(attrs) {
            Ok(window) => {
                let window = Arc::new(window);
                
                // Create softbuffer surface
                let context = softbuffer::Context::new(window.clone()).unwrap();
                let surface = softbuffer::Surface::new(&context, window.clone()).unwrap();
                
                self.window = Some(window);
                self.surface = Some(surface);
                
                log::info!("Native window created");
            }
            Err(e) => {
                log::error!("Failed to create window: {}", e);
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                self.running.store(false, Ordering::SeqCst);
                event_loop.exit();
            }
            WindowEvent::RedrawRequested => {
                // Try to get latest frame
                let mut latest_frame = None;
                loop {
                    match self.frame_rx.try_recv() {
                        Ok(frame) => latest_frame = Some(frame),
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Disconnected) => {
                            event_loop.exit();
                            return;
                        }
                    }
                }

                if let Some(frame) = latest_frame {
                    self.render_frame(&frame);
                }

                // Request next frame
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if !self.running.load(Ordering::SeqCst) {
            _event_loop.exit();
            return;
        }

        // Continuous redraw for video
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}

/// Run the native window event loop
fn run_window(
    running: Arc<AtomicBool>,
    frame_rx: Receiver<FrameBuffer>,
) -> Result<(), Box<dyn std::error::Error>> {
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);

    let mut app = VideoApp::new(running, frame_rx);
    event_loop.run_app(&mut app)?;

    Ok(())
}

impl Default for NativeViewer {
    fn default() -> Self {
        Self::new()
    }
}
