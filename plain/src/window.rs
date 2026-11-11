//! The window: one shell, one view, one GPU, driven by winit. What can be decided without a
//! window lives in `input.rs`; this file only wires it to winit, wgpu and the clipboard.

use std::collections::VecDeque;
use std::fmt::Display;
use std::mem;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::sync::Arc;
use std::time::{Duration, Instant};

use term_engine::{
    Button, CellPoint, FONT_PX_DEFAULT, Gpu, MouseAction, Rect, Side, ViewDraw, ViewEvent, cell_at,
    clear_color,
};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalSize};
use winit::event::{ElementState, KeyEvent, Modifiers, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::window::{Window, WindowId};

use crate::backlog::{Backlog, CloseOnDrop, QUEUE_LIMIT, counted};
use crate::input::{self, Clicks, Settle, WheelLines, WindowKey};
use crate::session::{self, Session};
use crate::shell::{Shell, ShellEvent};

const TITLE: &str = "Snowflake";
/// The most output fed between two frames, so a flood never starves the screen or the keyboard.
const FEED_SLICE: Duration = Duration::from_millis(8);
/// Lines per wheel notch.
const NOTCH_LINES: f64 = 3.0;
/// The exit code when the command cannot start, as shells use.
const CANNOT_START: i32 = 127;

pub struct Options {
    pub record: Option<PathBuf>,
    pub cwd: Option<PathBuf>,
    pub command: Option<Vec<String>>,
}

/// Returns the process exit code: the command's own, or 0 when the window was closed.
pub fn run(options: Options) -> i32 {
    // Before any window opens, as in the windowless mode.
    if !session::can_record(options.record.as_deref()) {
        return 1;
    }
    let event_loop = match EventLoop::<ShellEvent>::with_user_event().build() {
        Ok(event_loop) => event_loop,
        Err(e) => return fail("cannot open a window", e),
    };
    let mut app = App {
        start: Some((options, event_loop.create_proxy())),
        term: None,
        code: 0,
    };
    if let Err(e) = event_loop.run_app(&mut app) {
        return fail("the window failed", e);
    }
    app.code
}

fn fail(what: &str, e: impl Display) -> i32 {
    eprintln!("snowflake: {what}: {e}");
    1
}

struct App {
    /// What opening the window needs; taken when winit is ready for it.
    start: Option<(Options, EventLoopProxy<ShellEvent>)>,
    term: Option<Term>,
    code: i32,
}

impl App {
    /// Dropping the terminal drops the session first, which hangs up the shell, then the window.
    fn close(&mut self, event_loop: &ActiveEventLoop, code: i32) {
        self.term = None;
        self.code = code;
        event_loop.exit();
    }
}

impl ApplicationHandler<ShellEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let Some((options, proxy)) = self.start.take() else {
            return;
        };
        match Term::open(event_loop, options, proxy) {
            Ok(term) => self.term = Some(term),
            Err(code) => self.close(event_loop, code),
        }
    }

    /// Only queues: the output is fed in `about_to_wait`, a slice at a time.
    fn user_event(&mut self, _: &ActiveEventLoop, event: ShellEvent) {
        let Some(term) = &mut self.term else {
            return;
        };
        match event {
            ShellEvent::Output(bytes) => term.pending.push_back(bytes),
            // Unix keeps the low 8 bits; Windows codes are full 32-bit values.
            ShellEvent::Exited(code) => term.exited = Some(code as i32),
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        let Some(term) = &mut self.term else {
            return;
        };
        if let Some(code) = term.window_event(event) {
            self.close(event_loop, code);
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let Some(term) = &mut self.term else {
            return;
        };
        match term.about_to_wait() {
            Ok(flow) => event_loop.set_control_flow(flow),
            Err(code) => self.close(event_loop, code),
        }
    }

    /// The menu's Quit ends the process from inside AppKit; hang the shell up before that.
    fn exiting(&mut self, _: &ActiveEventLoop) {
        self.term = None;
    }
}

/// Everything that exists while the window is open. Fields drop in order: the backlog closes
/// first, so a reader waiting on it is let go, then the session (and the shell with it) goes.
struct Term {
    /// Output handed to the loop and not yet fed, in `pending` or still in winit's queue.
    backlog: CloseOnDrop,
    session: Session,
    draw: ViewDraw,
    gpu: Gpu,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    /// The size `config` was last applied with.
    configured: Option<(u32, u32)>,
    window: Arc<Window>,
    /// The GPU's largest texture side; a bigger window shows its top left part.
    max_side: u32,
    /// The surface size in physical pixels.
    size: (u32, u32),
    /// False while the window is fully hidden or minimised.
    visible: bool,
    scale: f64,
    font_px: f32,
    mods: Modifiers,
    pointer: (f64, f64),
    /// The cell and side the last pointer event named, so a move inside them sends nothing.
    hover: Option<(CellPoint, Side)>,
    held: Option<Button>,
    clicks: Clicks,
    wheel: WheelLines,
    settle: Settle,
    /// A display scale change waits until its resize is in, then fits at once.
    refit_now: bool,
    pending: VecDeque<Vec<u8>>,
    /// Time spent feeding output since the last frame.
    fed_for: Duration,
    /// The command's exit code, acted on once its output is all fed.
    exited: Option<i32>,
    /// Made on first use and kept, since on some systems the copied text lives only as long as it.
    clipboard: Option<arboard::Clipboard>,
    /// `open` processes not yet reaped.
    openers: Vec<Child>,
    stats: Option<PathBuf>,
    frames: u64,
}

impl Term {
    /// The window and the GPU come first, because the grid depends on their size and scale; the
    /// shell starts with that grid. The error is the exit code, already explained on stderr.
    fn open(
        event_loop: &ActiveEventLoop,
        options: Options,
        proxy: EventLoopProxy<ShellEvent>,
    ) -> Result<Term, i32> {
        let attributes = Window::default_attributes()
            .with_title(TITLE)
            .with_inner_size(LogicalSize::new(1280.0, 800.0));
        #[cfg(target_os = "macos")]
        let attributes = {
            use winit::platform::macos::{OptionAsAlt, WindowAttributesExtMacOS};
            attributes.with_option_as_alt(OptionAsAlt::OnlyLeft)
        };
        let window = Arc::new(
            event_loop
                .create_window(attributes)
                .map_err(|e| fail("cannot open a window", e))?,
        );
        // No composing pop-up: keys go straight to the terminal.
        window.set_ime_allowed(false);

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::default());
        let surface = instance
            .create_surface(window.clone())
            .map_err(|e| fail("cannot draw in the window", e))?;
        // The GPU needs the surface's format before it exists, so ask the same adapter it will
        // get what the surface supports.
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            force_fallback_adapter: false,
            compatible_surface: Some(&surface),
        }))
        .ok_or_else(|| fail("cannot draw in the window", "no graphics adapter"))?;
        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(wgpu::TextureFormat::is_srgb)
            .ok_or_else(|| fail("cannot draw in the window", "no sRGB surface format"))?;
        let mut gpu = pollster::block_on(Gpu::new(&instance, Some(&surface), format))
            .map_err(|e| fail("cannot start the GPU", format!("{e:?}")))?;
        let scale = window.scale_factor();
        gpu.set_text_size(FONT_PX_DEFAULT, scale);
        let max_side = gpu.device().limits().max_texture_dimension_2d;
        let size = surface_size(window.inner_size(), max_side);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.0,
            height: size.1,
            present_mode: present_mode(&caps),
            desired_maximum_frame_latency: 1,
            alpha_mode: caps.alpha_modes[0],
            view_formats: Vec::new(),
        };

        let cell = gpu.cell_size();
        let grid = input::grid_for(size, cell);
        // Made before the shell, so every way out of here, and later out of `Term`, closes it.
        let backlog = CloseOnDrop(Arc::new(Backlog::new(QUEUE_LIMIT)));
        // Runs on the shell's threads: hand the event over, then wait while the backlog is full.
        let on_event = counted(backlog.0.clone(), move |event| {
            // The loop is gone only after the window closed, and then nobody listens.
            proxy.send_event(event).is_ok()
        });
        let shell = Shell::spawn(options.command, options.cwd, grid, on_event).map_err(|e| {
            eprintln!("snowflake: cannot start the command: {e}");
            CANNOT_START
        })?;
        let mut session = Session::new(shell, grid, options.record)
            .map_err(|e| fail("cannot write the recording", e))?;
        session.set_grid(grid, cell);

        let mut term = Term {
            backlog,
            session,
            draw: ViewDraw::new(&gpu),
            gpu,
            surface,
            config,
            configured: None,
            window,
            max_side,
            size,
            visible: true,
            scale,
            font_px: FONT_PX_DEFAULT,
            mods: Modifiers::default(),
            pointer: (0.0, 0.0),
            hover: None,
            held: None,
            clicks: Clicks::new(),
            wheel: WheelLines::new(),
            settle: Settle::new(),
            refit_now: false,
            pending: VecDeque::new(),
            fed_for: Duration::ZERO,
            exited: None,
            clipboard: None,
            openers: Vec::new(),
            stats: std::env::var_os("TERM_PLAIN_STATS").map(PathBuf::from),
            frames: 0,
        };
        term.configure();
        term.window.request_redraw();
        Ok(term)
    }

    /// `Some` is the exit code when the window should close.
    fn window_event(&mut self, event: WindowEvent) -> Option<i32> {
        match event {
            WindowEvent::CloseRequested => return Some(0),
            WindowEvent::RedrawRequested => return self.redraw(),
            WindowEvent::Resized(size) => {
                // The surface follows at once; the grid waits for the size to settle and the
                // engine pads or clips meanwhile.
                self.size = surface_size(size, self.max_side);
                self.configure();
                self.settle.changed(Instant::now());
                self.window.request_redraw();
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                self.scale = scale_factor;
                self.gpu.set_text_size(self.font_px, scale_factor);
                self.refit_now = true;
                self.window.request_redraw();
            }
            WindowEvent::Occluded(hidden) => {
                self.visible = !hidden;
                if self.visible {
                    self.configure();
                    self.window.request_redraw();
                }
            }
            WindowEvent::Focused(focused) => {
                self.session.view.set_focused(focused);
                self.session.flush();
            }
            WindowEvent::ModifiersChanged(mods) => self.mods = mods,
            WindowEvent::KeyboardInput {
                event,
                is_synthetic: false,
                ..
            } => return self.key(&event),
            WindowEvent::CursorMoved { position, .. } => {
                self.pointer = (position.x, position.y);
                let (at, side) = self.cell();
                if self.hover != Some((at, side)) {
                    self.hover = Some((at, side));
                    self.pointer_action(MouseAction::Move(self.held), at, side, 1);
                }
            }
            WindowEvent::MouseInput { state, button, .. } => self.button(state, button),
            WindowEvent::MouseWheel { delta, .. } => self.scroll(delta),
            _ => {}
        }
        None
    }

    /// Feeds a slice of waiting output, refits, asks for a frame when the view changed, and
    /// returns how long the loop may sleep. `Err` is the exit code once the command has ended.
    fn about_to_wait(&mut self) -> Result<ControlFlow, i32> {
        while self.fed_for < FEED_SLICE
            && let Some(bytes) = self.pending.pop_front()
        {
            let start = Instant::now();
            self.session.output(&bytes);
            self.backlog.fed(bytes.len());
            self.fed_for += start.elapsed();
        }
        self.view_events();
        self.openers
            .retain_mut(|child| matches!(child.try_wait(), Ok(None)));
        if self.pending.is_empty()
            && let Some(code) = self.exited
        {
            return Err(code);
        }

        let now = Instant::now();
        // A minimised window can report no size at all, which would fit the shell to one cell.
        // The refit stays pending, unconsumed, until the window has a size again.
        let sized = input::may_present(true, self.size);
        if sized && (self.settle.due(now) || mem::take(&mut self.refit_now)) {
            self.refit();
        }
        let wake = self.session.view.tick(now);
        let frame = input::may_present(self.visible, self.size) && self.session.view.needs_draw();
        if frame {
            self.window.request_redraw();
        }
        if !self.pending.is_empty() && !frame {
            // No frame will start the next slice, so start it here and come straight back once
            // the loop has handled what else is waiting.
            self.fed_for = Duration::ZERO;
            return Ok(ControlFlow::Poll);
        }
        // A settle deadline already past would wake the loop at once, again and again, while the
        // refit waits for a size; the next resize wakes it instead.
        let settle = self.settle.deadline().filter(|_| sized);
        Ok(match wake.into_iter().chain(settle).min() {
            Some(at) => ControlFlow::WaitUntil(at),
            None => ControlFlow::Wait,
        })
    }

    /// The only place that presents. `Some` is the exit code when drawing cannot go on.
    fn redraw(&mut self) -> Option<i32> {
        // Drawn or not, the output that waited for this frame has had its chance.
        self.fed_for = Duration::ZERO;
        if !input::may_present(self.visible, self.size) {
            return None;
        }
        self.configure();
        let frame = match self.surface.get_current_texture() {
            Ok(frame) => frame,
            Err(wgpu::SurfaceError::Outdated | wgpu::SurfaceError::Lost) => {
                self.configured = None;
                self.configure();
                self.window.request_redraw();
                return None;
            }
            // The view still needs drawing, so the next loop turn asks again.
            Err(wgpu::SurfaceError::Timeout) => return None,
            Err(e) => return Some(fail("cannot draw", e)),
        };
        let (w, h) = self.size;
        let rect = Rect { x: 0, y: 0, w, h };
        self.draw
            .prepare(&mut self.gpu, &mut self.session.view, rect, self.size);
        let target = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let device = self.gpu.device();
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("frame"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("frame"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        // Past the last whole cell the window shows the same colour as the cells.
                        load: wgpu::LoadOp::Clear(clear_color(
                            self.session.view.theme().palette.bg,
                        )),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            self.draw.draw(&self.gpu, &mut pass);
        }
        self.gpu.queue().submit([encoder.finish()]);
        self.window.pre_present_notify();
        frame.present();
        self.frames += 1;
        if let Some(path) = &self.stats
            && let Err(e) = std::fs::write(path, format!("frames={}\n", self.frames))
        {
            eprintln!("snowflake: stats stopped: {e}");
            self.stats = None;
        }
        None
    }

    /// Never with a zero side, and not while hidden: the next visible frame does it.
    fn configure(&mut self) {
        if !input::may_present(self.visible, self.size) || self.configured == Some(self.size) {
            return;
        }
        (self.config.width, self.config.height) = self.size;
        self.surface.configure(self.gpu.device(), &self.config);
        self.configured = Some(self.size);
    }

    fn refit(&mut self) {
        let cell = self.gpu.cell_size();
        self.session
            .set_grid(input::grid_for(self.size, cell), cell);
    }

    fn key(&mut self, event: &KeyEvent) -> Option<i32> {
        let state = self.mods.state();
        #[cfg(target_os = "macos")]
        let state = input::option_mods(
            &event.logical_key,
            state,
            self.mods.lalt_state() == winit::keyboard::ModifiersKeyState::Pressed,
        );
        let key = input::key_input(
            &event.logical_key,
            event.text.as_deref(),
            state,
            event.state == ElementState::Pressed,
            event.repeat,
        )?;
        match input::window_key(&key) {
            Some(WindowKey::Quit | WindowKey::Close) => return Some(0),
            Some(WindowKey::Copy) => {
                if let Some(text) = self.session.view.copy() {
                    self.set_clipboard(text);
                }
            }
            Some(WindowKey::Paste) => {
                if let Some(text) = self.clipboard_text() {
                    self.session.view.paste(&text);
                    self.session.flush();
                }
            }
            Some(zoom) => {
                self.font_px = input::zoomed(self.font_px, zoom);
                self.gpu.set_text_size(self.font_px, self.scale);
                self.refit();
                // The cells changed size even when the grid did not.
                self.window.request_redraw();
            }
            None => {
                self.session.view.key(&key);
                self.session.flush();
            }
        }
        None
    }

    fn cell(&self) -> (CellPoint, Side) {
        let (w, h) = self.size;
        let rect = Rect { x: 0, y: 0, w, h };
        let (x, y) = self.pointer;
        cell_at(rect, self.gpu.cell_size(), self.session.view.grid(), x, y)
    }

    fn button(&mut self, state: ElementState, button: MouseButton) {
        let button = match button {
            MouseButton::Left => Button::Left,
            MouseButton::Middle => Button::Middle,
            MouseButton::Right => Button::Right,
            _ => return,
        };
        let (at, side) = self.cell();
        match state {
            ElementState::Pressed => {
                let clicks = if button == Button::Left {
                    self.clicks.press(at, Instant::now())
                } else {
                    1
                };
                self.held = Some(button);
                self.pointer_action(MouseAction::Press(button), at, side, clicks);
            }
            ElementState::Released => {
                if self.held == Some(button) {
                    self.held = None;
                }
                self.pointer_action(MouseAction::Release(button), at, side, 1);
            }
        }
    }

    fn scroll(&mut self, delta: MouseScrollDelta) {
        let cell_h = self.gpu.cell_size().h;
        let pixels = match delta {
            // A notch is a fraction of a line on some mice, so notches add up through the
            // same remainder as trackpad pixels.
            MouseScrollDelta::LineDelta(_, notches) => {
                f64::from(notches) * NOTCH_LINES * f64::from(cell_h)
            }
            MouseScrollDelta::PixelDelta(position) => position.y,
        };
        let lines = self.wheel.pixels(pixels, cell_h);
        if lines != 0 {
            let (at, side) = self.cell();
            self.pointer_action(MouseAction::Wheel(lines), at, side, 1);
        }
    }

    fn pointer_action(&mut self, action: MouseAction, at: CellPoint, side: Side, clicks: u8) {
        let mods = input::key_mods(self.mods.state());
        self.session.view.pointer(action, at, side, mods, clicks);
        self.session.flush();
    }

    fn view_events(&mut self) {
        for event in self.session.view.take_events() {
            match event {
                ViewEvent::Title(title) if title.is_empty() => self.window.set_title(TITLE),
                ViewEvent::Title(title) => self.window.set_title(&title),
                ViewEvent::Copy(text) => self.set_clipboard(text),
                ViewEvent::OpenLink(url) => match Command::new("/usr/bin/open").arg(&url).spawn() {
                    Ok(child) => self.openers.push(child),
                    Err(e) => eprintln!("snowflake: cannot open {url}: {e}"),
                },
                ViewEvent::Bell => {}
            }
        }
    }

    fn clipboard(&mut self) -> Option<&mut arboard::Clipboard> {
        if self.clipboard.is_none() {
            match arboard::Clipboard::new() {
                Ok(clipboard) => self.clipboard = Some(clipboard),
                Err(e) => eprintln!("snowflake: no clipboard: {e}"),
            }
        }
        self.clipboard.as_mut()
    }

    fn set_clipboard(&mut self, text: String) {
        if let Some(clipboard) = self.clipboard()
            && let Err(e) = clipboard.set_text(text)
        {
            eprintln!("snowflake: cannot copy: {e}");
        }
    }

    /// `None` when the clipboard holds no text.
    fn clipboard_text(&mut self) -> Option<String> {
        match self.clipboard()?.get_text() {
            Ok(text) => Some(text),
            Err(arboard::Error::ContentNotAvailable) => None,
            Err(e) => {
                eprintln!("snowflake: cannot paste: {e}");
                None
            }
        }
    }
}

/// The window's size cut to what the GPU can draw into.
fn surface_size(size: PhysicalSize<u32>, max_side: u32) -> (u32, u32) {
    (size.width.min(max_side), size.height.min(max_side))
}

/// `TERM_PLAIN_PRESENT` picks `fifo` (the default, waits for the display) or `immediate`.
fn present_mode(caps: &wgpu::SurfaceCapabilities) -> wgpu::PresentMode {
    let wanted = match std::env::var("TERM_PLAIN_PRESENT").as_deref() {
        Ok("immediate") => wgpu::PresentMode::Immediate,
        Ok("fifo") | Err(_) => wgpu::PresentMode::Fifo,
        Ok(other) => {
            eprintln!(
                "snowflake: TERM_PLAIN_PRESENT is fifo or immediate, not {other:?}; using fifo"
            );
            wgpu::PresentMode::Fifo
        }
    };
    if caps.present_modes.contains(&wanted) {
        wanted
    } else {
        // Every surface supports fifo.
        eprintln!("snowflake: this display cannot present {wanted:?}; using fifo");
        wgpu::PresentMode::Fifo
    }
}
