use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::c_void,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc, Arc, Mutex,
    },
    thread::{self, JoinHandle},
};
use tauri::{AppHandle, Emitter, Manager};

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct InputStatus {
    pub granted: bool,
    pub running: bool,
    pub error: Option<String>,
}

const HID_REQUEST_TYPE_LISTEN_EVENT: u32 = 1;
const HID_ACCESS_GRANTED: u32 = 0;
const HID_ACCESS_DENIED: u32 = 1;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InputAccessDiagnostics {
    pub input_monitoring_granted: bool,
    pub hid_access: &'static str,
    pub accessibility_granted: bool,
    pub bundle_identifier: String,
    pub executable_path: String,
    pub translocated: bool,
    pub hint: &'static str,
}

fn access_hint(input_monitoring_granted: bool, translocated: bool) -> &'static str {
    if translocated {
        // Gatekeeper runs an app opened straight from the DMG from a randomized
        // AppTranslocation path, so the granted row can never match the next launch.
        "move_app_to_applications"
    } else if !input_monitoring_granted {
        "grant_input_monitoring"
    } else {
        "ok"
    }
}

/// Re-issues the Input Monitoring request so the app keeps its row in System Settings.
///
/// `CGRequestListenEventAccess` is idempotent while access is already granted: it returns
/// true without prompting. Re-issuing it is what restores the row under
/// System Settings > Privacy & Security > Input Monitoring, because that list is rebuilt
/// from request records and a reinstall can leave the old TCC record authorizing the event
/// tap while the row disappears. Callers must only reach this with access already granted
/// or from an explicit user action, since it prompts otherwise.
fn register_listen_event() {
    unsafe {
        CGRequestListenEventAccess();
        // IOKit keeps a parallel record; some macOS releases only refresh the panel row
        // when the HID request is made.
        IOHIDRequestAccess(HID_REQUEST_TYPE_LISTEN_EVENT);
    }
}

fn ensure_input_monitoring_listed() {
    if !unsafe { CGPreflightListenEventAccess() } {
        return;
    }
    unsafe {
        CGRequestListenEventAccess();
    }
    // Only re-issue the HID request when it cannot prompt, so a silent startup path
    // never turns into a permission dialog.
    if unsafe { IOHIDCheckAccess(HID_REQUEST_TYPE_LISTEN_EVENT) } != HID_ACCESS_DENIED {
        unsafe {
            IOHIDRequestAccess(HID_REQUEST_TYPE_LISTEN_EVENT);
        }
    }
}

/// Drops a TCC record that authorizes an older build of this app.
///
/// Grants are keyed on the code signature (cdhash / designated requirement). Ad-hoc
/// signed builds get a new cdhash on every release, so after an update the system still
/// reports "granted" through `CGPreflightListenEventAccess` while the stale record cannot
/// match the new binary. That is why `CGRequestListenEventAccess` returns silently — no
/// prompt, no error — and the event tap never comes up. `tccutil` needs no privileges and
/// is the only way to rebuild the record from inside the app: without it the user has to
/// delete the app row in System Settings by hand before the button does anything.
fn reset_listen_event(bundle_identifier: &str) -> bool {
    std::process::Command::new("/usr/bin/tccutil")
        .args(["reset", "ListenEvent", bundle_identifier])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// Granted on paper but unusable in practice: the tap could not be created or enabled.
fn stale_grant(status: &InputStatus) -> bool {
    status.granted
        && !status.running
        && matches!(
            status.error.as_deref(),
            Some(
                "input_event_tap_create_failed"
                    | "input_event_tap_enable_failed"
                    | "input_event_tap_disabled"
            )
        )
}

pub fn diagnostics(app: &AppHandle) -> InputAccessDiagnostics {
    let input_monitoring_granted = unsafe { CGPreflightListenEventAccess() };
    let hid = unsafe { IOHIDCheckAccess(HID_REQUEST_TYPE_LISTEN_EVENT) };
    let executable_path = std::env::current_exe()
        .map(|path| path.display().to_string())
        .unwrap_or_default();
    let translocated = executable_path.contains("/AppTranslocation/");
    InputAccessDiagnostics {
        input_monitoring_granted,
        hid_access: match hid {
            HID_ACCESS_GRANTED => "granted",
            HID_ACCESS_DENIED => "denied",
            _ => "unknown",
        },
        accessibility_granted: unsafe { AXIsProcessTrusted() },
        bundle_identifier: app.config().identifier.clone(),
        executable_path,
        translocated,
        hint: access_hint(input_monitoring_granted, translocated),
    }
}

pub fn describe_access(app: &AppHandle) -> String {
    let access = diagnostics(app);
    format!(
        "input-monitoring={} hid={} accessibility={} translocated={} bundle={} hint={} path={}",
        access.input_monitoring_granted,
        access.hid_access,
        access.accessibility_granted,
        access.translocated,
        access.bundle_identifier,
        access.hint,
        access.executable_path
    )
}

pub fn physical_input_quiet() -> bool {
    if unsafe { CGEventSourceSecondsSinceLastEventType(0, u32::MAX) } < 0.25 {
        return false;
    }
    !(0u16..=255).any(|key| unsafe { CGEventSourceKeyState(0, key) })
        && !(0..32).any(|button| unsafe { CGEventSourceButtonState(0, button) })
}

fn input_status_snapshot(
    granted: bool,
    worker_running: bool,
    tap_enabled: bool,
    recorded_error: Option<String>,
) -> InputStatus {
    let running = granted && worker_running && tap_enabled;
    let error = if !granted {
        Some(
            if worker_running {
                "input_monitoring_permission_revoked"
            } else {
                "input_monitoring_permission_required"
            }
            .into(),
        )
    } else if running {
        None
    } else if worker_running && !tap_enabled {
        Some("input_event_tap_disabled".into())
    } else {
        match recorded_error.as_deref() {
            Some(
                "input_monitoring_permission_required" | "input_monitoring_permission_revoked",
            ) => Some("input_access_granted_waiting_for_resume".into()),
            Some(_) => recorded_error,
            None => Some("input_monitor_not_running".into()),
        }
    };
    InputStatus {
        granted,
        running,
        error,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct MousePosition {
    pub x: f64,
    pub y: f64,
    #[serde(rename = "memberId", skip_serializing_if = "Option::is_none")]
    pub member_id: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct MouseButton {
    button: &'static str,
    action: &'static str,
    #[serde(rename = "memberId", skip_serializing_if = "Option::is_none")]
    member_id: Option<u64>,
}

type MemberResolver = Arc<dyn Fn(f64, f64) -> Option<u64> + Send + Sync>;
type AuxiliaryHitTest = Arc<dyn Fn(f64, f64) -> bool + Send + Sync>;

struct MonitorState {
    running: AtomicBool,
    stop: AtomicBool,
    scale: AtomicU64,
    window_listener_installed: AtomicBool,
    run_loop: Mutex<usize>,
    event_tap: Mutex<usize>,
    error: Mutex<Option<String>>,
    member_resolver: Mutex<Option<MemberResolver>>,
    auxiliary_hit_test: Mutex<Option<AuxiliaryHitTest>>,
}

impl MonitorState {
    fn error(&self, message: String) {
        if let Ok(mut error) = self.error.lock() {
            *error = Some(message);
        }
    }

    fn status(&self) -> InputStatus {
        let granted = unsafe { CGPreflightListenEventAccess() };
        let worker_running = self.running.load(Ordering::Acquire);
        let mut recorded_error = match self.error.lock() {
            Ok(error) => error.clone(),
            Err(_) => Some("input_status_lock_poisoned".into()),
        };
        let tap_enabled = match self.event_tap.lock() {
            Ok(tap) => *tap != 0 && unsafe { CGEventTapIsEnabled(*tap as *mut c_void) },
            Err(_) => {
                recorded_error = Some("input_event_tap_lock_poisoned".into());
                false
            }
        };
        input_status_snapshot(granted, worker_running, tap_enabled, recorded_error)
    }

    fn wake_and_stop(&self) {
        self.stop.store(true, Ordering::Release);
        if let Ok(run_loop) = self.run_loop.lock() {
            if *run_loop != 0 {
                // The worker retains this run loop while its pointer is in the mutex.
                unsafe {
                    CFRunLoopStop(*run_loop as *mut c_void);
                    CFRunLoopWakeUp(*run_loop as *mut c_void);
                }
            }
        }
    }
}

pub struct InputMonitor {
    state: Arc<MonitorState>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl Default for InputMonitor {
    fn default() -> Self {
        Self::new()
    }
}

impl InputMonitor {
    pub fn new() -> Self {
        Self {
            state: Arc::new(MonitorState {
                running: AtomicBool::new(false),
                stop: AtomicBool::new(false),
                scale: AtomicU64::new(1.0f64.to_bits()),
                window_listener_installed: AtomicBool::new(false),
                run_loop: Mutex::new(0),
                event_tap: Mutex::new(0),
                error: Mutex::new(None),
                member_resolver: Mutex::new(None),
                auxiliary_hit_test: Mutex::new(None),
            }),
            worker: Mutex::new(None),
        }
    }

    pub fn set_member_resolver<F>(&self, resolver: F) -> Result<(), String>
    where
        F: Fn(f64, f64) -> Option<u64> + Send + Sync + 'static,
    {
        *self
            .state
            .member_resolver
            .lock()
            .map_err(|_| "input_router_lock_poisoned")? = Some(Arc::new(resolver));
        Ok(())
    }

    pub fn set_auxiliary_hit_test<F>(&self, hit_test: F) -> Result<(), String>
    where
        F: Fn(f64, f64) -> bool + Send + Sync + 'static,
    {
        *self
            .state
            .auxiliary_hit_test
            .lock()
            .map_err(|_| "input_auxiliary_hit_test_lock_poisoned")? = Some(Arc::new(hit_test));
        Ok(())
    }

    pub fn status(&self) -> InputStatus {
        self.state.status()
    }

    pub fn start(&self, app: AppHandle) -> Result<InputStatus, String> {
        let result = self.start_inner(app.clone());
        if let Err(error) = &result {
            self.state.error(error.clone());
        }
        let status = self.emit_status(&app)?;
        result.map(|_| status)
    }

    fn start_inner(&self, app: AppHandle) -> Result<InputStatus, String> {
        // Restore the Settings row before the worker starts: a reinstall can leave the tap
        // authorized by a stale TCC record while the Input Monitoring entry is gone.
        ensure_input_monitoring_listed();
        let mut worker = self
            .worker
            .lock()
            .map_err(|_| "input_worker_lock_poisoned")?;
        if self.status().running {
            return Ok(self.status());
        }
        if let Some(previous) = worker.take() {
            self.state.wake_and_stop();
            previous.join().map_err(|_| "input_worker_panicked")?;
        }
        if !unsafe { CGPreflightListenEventAccess() } {
            self.state
                .error("input_monitoring_permission_required".into());
            return Ok(self.status());
        }
        let window = app
            .get_webview_window("main")
            .ok_or("main_window_missing")?;
        let scale = window
            .scale_factor()
            .map_err(|error| format!("input_window_scale_failed: {error}"))?;
        self.state.scale.store(scale.to_bits(), Ordering::Release);
        if !self
            .state
            .window_listener_installed
            .swap(true, Ordering::AcqRel)
        {
            let state = self.state.clone();
            window.on_window_event(move |event| {
                if let tauri::WindowEvent::ScaleFactorChanged { scale_factor, .. } = event {
                    state.scale.store(scale_factor.to_bits(), Ordering::Release);
                }
            });
        }
        self.state.stop.store(false, Ordering::Release);
        *self
            .state
            .error
            .lock()
            .map_err(|_| "input_status_lock_poisoned")? = None;
        let state = self.state.clone();
        let worker_app = app.clone();
        let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
        *worker = Some(
            thread::Builder::new()
                .name("wuwatappo-input".into())
                .spawn(move || {
                    run_monitor(worker_app, state, ready_sender);
                })
                .map_err(|error| format!("input_worker_spawn_failed: {error}"))?,
        );
        let initialized = ready_receiver
            .recv()
            .map_err(|_| "input_worker_initialization_failed")?;
        initialized.map(|()| self.status())
    }

    fn emit_status(&self, app: &AppHandle) -> Result<InputStatus, String> {
        let status = self.status();
        app.emit("mac-input-status", &status)
            .map_err(|error| format!("input_status_delivery_failed: {error}"))?;
        Ok(status)
    }

    pub fn stop(&self) -> Result<(), String> {
        let mut worker = self
            .worker
            .lock()
            .map_err(|_| "input_worker_lock_poisoned")?;
        self.state.wake_and_stop();
        if let Some(worker) = worker.take() {
            worker.join().map_err(|_| "input_worker_panicked")?;
        }
        Ok(())
    }
}

impl Drop for InputMonitor {
    fn drop(&mut self) {
        if let Err(error) = self.stop() {
            self.state.error(error);
        }
    }
}

pub fn start(app: AppHandle) -> Result<InputStatus, String> {
    let monitor = app.state::<InputMonitor>();
    monitor.start(app.clone())
}

#[tauri::command]
pub fn get_input_status(app: AppHandle) -> InputStatus {
    app.state::<InputMonitor>().status()
}

#[tauri::command]
pub fn get_input_access_diagnostics(app: AppHandle) -> InputAccessDiagnostics {
    diagnostics(&app)
}

#[tauri::command]
pub fn resume_input_monitor(app: AppHandle) -> Result<InputStatus, String> {
    // Recheck existing permission only; this path never prompts for access. Restoring the
    // Settings row while access is granted is silent, so it stays safe to reach from here.
    start(app)
}

#[tauri::command]
pub fn request_input_access(app: AppHandle) -> Result<InputStatus, String> {
    // This is the only permission-requesting path, invoked by an explicit app action.
    // It deliberately skips the preflight short-circuit: when a reinstall leaves access
    // granted but the Settings row missing, a preflight check would report success and
    // never re-register the app, which is how the row goes missing in the first place.
    let monitor = app.state::<InputMonitor>();
    let before = monitor.status();
    if stale_grant(&before) {
        // The record points at a previous build. Keeping it means the request below is a
        // no-op and the user sees a menu item that never responds, so clear it first and
        // let the next request write a record for the binary that is running now.
        let bundle = app.config().identifier.clone();
        if reset_listen_event(&bundle) {
            eprintln!("Cleared stale input monitoring record for {bundle}");
        } else {
            eprintln!("Could not clear stale input monitoring record for {bundle}");
        }
    }
    register_listen_event();
    start(app)
}

#[derive(Debug)]
enum NativeInput {
    Shortcut {
        key: String,
        flags: u64,
    },
    Key {
        key: String,
        pressed: bool,
    },
    Move(Point),
    Button {
        point: Point,
        button: &'static str,
        pressed: bool,
    },
}

#[derive(Clone, Copy, Debug)]
#[repr(C)]
struct Point {
    x: f64,
    y: f64,
}

struct TapContext {
    sender: mpsc::Sender<NativeInput>,
    state: Arc<MonitorState>,
    tap: *mut c_void,
    pressed: BTreeSet<u16>,
}

impl TapContext {
    fn send(&self, event: NativeInput) {
        if self.sender.send(event).is_err() {
            self.state.error("input_dispatch_disconnected".into());
            self.state.wake_and_stop();
        }
    }

    fn release_keys(&mut self) {
        for code in std::mem::take(&mut self.pressed) {
            self.send(NativeInput::Key {
                key: key_label(code),
                pressed: false,
            });
        }
    }
}

unsafe extern "C" fn observe_event(
    _proxy: *mut c_void,
    event_type: u32,
    event: *mut c_void,
    context: *mut c_void,
) -> *mut c_void {
    let context = unsafe { &mut *(context as *mut TapContext) };
    if event_type == 0xffff_fffe || event_type == 0xffff_ffff {
        context.release_keys();
        if unsafe { CGPreflightListenEventAccess() } {
            unsafe {
                CGEventTapEnable(context.tap, true);
            }
        } else {
            context
                .state
                .error("input_monitoring_permission_revoked".into());
            context.state.wake_and_stop();
        }
        return event;
    }
    if event.is_null() || context.state.stop.load(Ordering::Acquire) {
        return event;
    }
    match event_type {
        10 | 11 | 12 => {
            let code = unsafe { CGEventGetIntegerValueField(event, 9) } as u16;
            if event_type == 12 && code == 57 {
                // Caps Lock sends a flag transition, not a normal down/up pair.
                context.send(NativeInput::Key {
                    key: key_label(code),
                    pressed: true,
                });
                context.send(NativeInput::Key {
                    key: key_label(code),
                    pressed: false,
                });
                return event;
            }
            let pressed = if event_type == 12 {
                unsafe { CGEventSourceKeyState(0, code) }
            } else {
                event_type == 10
            };
            if pressed {
                if context.pressed.insert(code) {
                    context.send(NativeInput::Shortcut {
                        key: key_label(code),
                        flags: unsafe { CGEventGetFlags(event) },
                    });
                    context.send(NativeInput::Key {
                        key: key_label(code),
                        pressed: true,
                    });
                }
            } else if context.pressed.remove(&code) {
                context.send(NativeInput::Key {
                    key: key_label(code),
                    pressed: false,
                });
            }
        }
        1 | 2 | 3 | 4 | 25 | 26 => {
            let button = match event_type {
                1 | 2 => "left",
                3 | 4 => "right",
                _ => "middle",
            };
            // OtherMouse events can also represent extra side buttons; preserve only the supported middle button.
            if matches!(event_type, 25 | 26)
                && unsafe { CGEventGetIntegerValueField(event, 3) } != 2
            {
                return event;
            }
            context.send(NativeInput::Button {
                point: unsafe { CGEventGetLocation(event) },
                button,
                pressed: matches!(event_type, 1 | 3 | 25),
            });
        }
        5 | 6 | 7 | 27 => context.send(NativeInput::Move(unsafe { CGEventGetLocation(event) })),
        _ => {}
    }
    event
}

fn run_monitor(
    app: AppHandle,
    state: Arc<MonitorState>,
    ready: mpsc::SyncSender<Result<(), String>>,
) {
    let (sender, receiver) = mpsc::channel();
    let mut context = Box::new(TapContext {
        sender,
        state: state.clone(),
        tap: std::ptr::null_mut(),
        pressed: BTreeSet::new(),
    });
    let mask = [1u32, 2, 3, 4, 5, 6, 7, 10, 11, 12, 25, 26, 27]
        .into_iter()
        .fold(0u64, |mask, event| mask | (1u64 << event));
    let tap = unsafe {
        CGEventTapCreate(
            1,
            0,
            1,
            mask,
            observe_event,
            (&mut *context as *mut TapContext).cast(),
        )
    };
    if tap.is_null() {
        let error = "input_event_tap_create_failed".to_owned();
        state.error(error.clone());
        let _ = ready.send(Err(error));
        return;
    }
    context.tap = tap;
    let source = unsafe { CFMachPortCreateRunLoopSource(std::ptr::null_mut(), tap, 0) };
    if source.is_null() {
        unsafe {
            CFMachPortInvalidate(tap);
            CFRelease(tap);
        }
        let error = "input_run_loop_source_create_failed".to_owned();
        state.error(error.clone());
        let _ = ready.send(Err(error));
        return;
    }
    let dispatch_state = state.clone();
    let dispatch_app = app.clone();
    let dispatch = match thread::Builder::new()
        .name("wuwatappo-input-events".into())
        .spawn(move || dispatch_events(dispatch_app, dispatch_state, receiver))
    {
        Ok(thread) => thread,
        Err(error) => {
            unsafe {
                CFMachPortInvalidate(tap);
                CFRelease(source);
                CFRelease(tap);
            }
            let error = format!("input_dispatch_spawn_failed: {error}");
            state.error(error.clone());
            let _ = ready.send(Err(error));
            return;
        }
    };
    let run_loop = unsafe { CFRunLoopGetCurrent() };
    unsafe {
        CFRetain(run_loop);
        CFRunLoopAddSource(run_loop, source, kCFRunLoopCommonModes);
        CGEventTapEnable(tap, true);
    }
    let initialization = (|| -> Result<(), String> {
        *state
            .run_loop
            .lock()
            .map_err(|_| "input_run_loop_lock_poisoned")? = run_loop as usize;
        *state
            .event_tap
            .lock()
            .map_err(|_| "input_event_tap_lock_poisoned")? = tap as usize;
        if !unsafe { CGEventTapIsEnabled(tap) } {
            return Err("input_event_tap_enable_failed".into());
        }
        state.running.store(true, Ordering::Release);
        Ok(())
    })();
    if let Err(error) = &initialization {
        state.error(error.clone());
        state.stop.store(true, Ordering::Release);
    }
    if ready.send(initialization).is_err() {
        state.stop.store(true, Ordering::Release);
    }
    // Bounded native run-loop iterations also observe a stop that races entry.
    while !state.stop.load(Ordering::Acquire) {
        unsafe {
            CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.25, 0);
        }
    }
    context.release_keys();
    state.running.store(false, Ordering::Release);
    let status = state.status();
    if let Err(error) = app.emit("mac-input-status", status) {
        state.error(format!("input_status_delivery_failed: {error}"));
    }
    unsafe {
        CGEventTapEnable(tap, false);
        CFRunLoopRemoveSource(run_loop, source, kCFRunLoopCommonModes);
        CFMachPortInvalidate(tap);
        CFRelease(source);
    }
    {
        // A status read holds this mutex while querying the native tap, so it
        // cannot race destruction of the CFMachPort.
        let mut stored = match state.event_tap.lock() {
            Ok(stored) => stored,
            Err(poisoned) => poisoned.into_inner(),
        };
        *stored = 0;
        unsafe {
            CFRelease(tap);
        }
    }
    {
        let mut stored = match state.run_loop.lock() {
            Ok(stored) => stored,
            Err(poisoned) => poisoned.into_inner(),
        };
        *stored = 0;
        unsafe {
            CFRelease(run_loop);
        }
    }
    drop(context);
    if dispatch.join().is_err() {
        state.error("input_dispatch_panicked".into());
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MouseTarget {
    Main,
    Auxiliary,
}

#[derive(Default)]
struct MouseRouting {
    pressed: BTreeMap<&'static str, MouseTarget>,
}

impl MouseRouting {
    fn button(&mut self, button: &'static str, pressed: bool, over_auxiliary: bool) -> bool {
        if pressed {
            if self.pressed.contains_key(button) {
                return false;
            }
            let target = if over_auxiliary {
                MouseTarget::Auxiliary
            } else {
                MouseTarget::Main
            };
            self.pressed.insert(button, target);
            target == MouseTarget::Main
        } else {
            self.pressed.remove(button) == Some(MouseTarget::Main)
        }
    }

    fn movement(&self, over_auxiliary: bool) -> bool {
        self.pressed
            .values()
            .any(|target| *target == MouseTarget::Main)
            || (self.pressed.is_empty() && !over_auxiliary)
    }
}

#[derive(Debug)]
enum RoutedInput {
    Key {
        key: String,
        pressed: bool,
    },
    Move(MousePosition),
    Button {
        position: MousePosition,
        button: MouseButton,
    },
}

fn route_event(
    event: NativeInput,
    routing: &mut MouseRouting,
    state: &MonitorState,
) -> Result<Option<RoutedInput>, String> {
    match event {
        NativeInput::Shortcut { .. } => Ok(None),
        NativeInput::Key { key, pressed } => Ok(Some(RoutedInput::Key { key, pressed })),
        NativeInput::Move(point) => {
            let point = physical_mouse_point(point, state);
            let over_auxiliary = over_auxiliary_window(point, state)?;
            if !routing.movement(over_auxiliary) {
                return Ok(None);
            }
            Ok(Some(RoutedInput::Move(resolve_mouse_position(
                point, state,
            )?)))
        }
        NativeInput::Button {
            point,
            button,
            pressed,
        } => {
            let point = physical_mouse_point(point, state);
            // The first down owns the complete gesture. A main-origin drag keeps its
            // up event over an auxiliary window; an auxiliary-origin gesture never
            // reaches the main frontend or its synthetic DOM routing.
            let over_auxiliary = if pressed {
                over_auxiliary_window(point, state)?
            } else {
                false
            };
            if !routing.button(button, pressed, over_auxiliary) {
                return Ok(None);
            }
            let position = resolve_mouse_position(point, state)?;
            Ok(Some(RoutedInput::Button {
                position,
                button: MouseButton {
                    button,
                    action: if pressed { "press" } else { "release" },
                    member_id: position.member_id,
                },
            }))
        }
    }
}

fn dispatch_events(
    app: AppHandle,
    state: Arc<MonitorState>,
    receiver: mpsc::Receiver<NativeInput>,
) {
    let mut routing = MouseRouting::default();
    for event in receiver {
        if matches!(event, NativeInput::Shortcut { .. }) {
            continue;
        }
        let event = match route_event(event, &mut routing, &state) {
            Ok(Some(event)) => event,
            Ok(None) => continue,
            Err(error) => {
                state.error(format!("input_event_route_failed: {error}"));
                state.wake_and_stop();
                break;
            }
        };
        let result = match event {
            RoutedInput::Key { key, pressed } => {
                app.emit(if pressed { "key-press" } else { "key-release" }, key)
            }
            RoutedInput::Move(position) => app.emit("mouse-move", position),
            RoutedInput::Button { position, button } => {
                // Keep global cursor state current before the main-owned button event.
                app.emit("mouse-move", position)
                    .and_then(|_| app.emit("mouse-button", button))
            }
        };
        if let Err(error) = result {
            state.error(format!("input_event_delivery_failed: {error}"));
            state.wake_and_stop();
            break;
        }
    }
}

fn physical_mouse_point(point: Point, state: &MonitorState) -> Point {
    let scale = f64::from_bits(state.scale.load(Ordering::Acquire));
    let (x, y) = target_window_physical_point(point.x, point.y, scale);
    Point { x, y }
}

fn over_auxiliary_window(point: Point, state: &MonitorState) -> Result<bool, String> {
    let hit_test = state
        .auxiliary_hit_test
        .lock()
        .map_err(|_| "input_auxiliary_hit_test_lock_poisoned")?
        .clone();
    Ok(hit_test.is_some_and(|hit_test| hit_test(point.x, point.y)))
}

fn resolve_mouse_position(point: Point, state: &MonitorState) -> Result<MousePosition, String> {
    let resolver = state
        .member_resolver
        .lock()
        .map_err(|_| "input_router_lock_poisoned")?
        .clone();
    Ok(MousePosition {
        x: point.x,
        y: point.y,
        member_id: resolver.and_then(|resolve| resolve(point.x, point.y)),
    })
}

fn target_window_physical_point(x: f64, y: f64, scale: f64) -> (f64, f64) {
    // Tao's macOS inner_position is the entire logical global origin multiplied by
    // the target window's backing scale. The frontend subtracts that position and
    // divides by the same scale, including when the window is on a secondary display.
    (x * scale, y * scale)
}

fn key_label(code: u16) -> String {
    match code {
        0 => "a",
        1 => "s",
        2 => "d",
        3 => "f",
        4 => "h",
        5 => "g",
        6 => "z",
        7 => "x",
        8 => "c",
        9 => "v",
        10 => "§",
        11 => "b",
        12 => "q",
        13 => "w",
        14 => "e",
        15 => "r",
        16 => "y",
        17 => "t",
        18 => "1",
        19 => "2",
        20 => "3",
        21 => "4",
        22 => "6",
        23 => "5",
        24 => "=",
        25 => "9",
        26 => "7",
        27 => "-",
        28 => "8",
        29 => "0",
        30 => "]",
        31 => "o",
        32 => "u",
        33 => "[",
        34 => "i",
        35 => "p",
        36 => "Enter",
        37 => "l",
        38 => "j",
        39 => "'",
        40 => "k",
        41 => ";",
        42 => "\\",
        43 => ",",
        44 => "/",
        45 => "n",
        46 => "m",
        47 => ".",
        48 => "Tab",
        49 => " ",
        50 => "`",
        51 => "Backspace",
        53 => "Escape",
        54 => "MetaRight",
        55 => "MetaLeft",
        56 => "ShiftLeft",
        57 => "CapsLock",
        58 => "AltLeft",
        59 => "ControlLeft",
        60 => "ShiftRight",
        61 => "AltRight",
        62 => "ControlRight",
        63 => "Fn",
        64 => "F17",
        65 => "NumpadDecimal",
        67 => "NumpadMultiply",
        69 => "NumpadAdd",
        71 => "NumLock",
        72 => "AudioVolumeUp",
        73 => "AudioVolumeDown",
        74 => "AudioVolumeMute",
        75 => "NumpadDivide",
        76 => "NumpadEnter",
        78 => "NumpadSubtract",
        79 => "F18",
        80 => "F19",
        81 => "NumpadEqual",
        82 => "Numpad0",
        83 => "Numpad1",
        84 => "Numpad2",
        85 => "Numpad3",
        86 => "Numpad4",
        87 => "Numpad5",
        88 => "Numpad6",
        89 => "Numpad7",
        90 => "F20",
        91 => "Numpad8",
        92 => "Numpad9",
        93 => "¥",
        94 => "_",
        95 => "NumpadComma",
        96 => "F5",
        97 => "F6",
        98 => "F7",
        99 => "F3",
        100 => "F8",
        101 => "F9",
        102 => "Eisu",
        103 => "F11",
        104 => "Kana",
        105 => "F13",
        106 => "F16",
        107 => "F14",
        109 => "F10",
        111 => "F12",
        113 => "F15",
        114 => "Help",
        115 => "Home",
        116 => "PageUp",
        117 => "Delete",
        118 => "F4",
        119 => "End",
        120 => "F2",
        121 => "PageDown",
        122 => "F1",
        123 => "ArrowLeft",
        124 => "ArrowRight",
        125 => "ArrowDown",
        126 => "ArrowUp",
        _ => return format!("Key{code}"),
    }
    .to_owned()
}

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOHIDCheckAccess(request_type: u32) -> u32;
    fn IOHIDRequestAccess(request_type: u32) -> bool;
}

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> bool;
    fn AXUIElementCreateSystemWide() -> *mut c_void;
    fn AXUIElementCopyAttributeValue(
        element: *mut c_void,
        attribute: *const c_void,
        value: *mut *mut c_void,
    ) -> i32;
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGPreflightListenEventAccess() -> bool;
    fn CGRequestListenEventAccess() -> bool;
    fn CGEventTapCreate(
        location: u32,
        placement: u32,
        options: u32,
        mask: u64,
        callback: unsafe extern "C" fn(*mut c_void, u32, *mut c_void, *mut c_void) -> *mut c_void,
        user_info: *mut c_void,
    ) -> *mut c_void;
    fn CGEventTapEnable(tap: *mut c_void, enabled: bool);
    fn CGEventTapIsEnabled(tap: *mut c_void) -> bool;
    fn CGEventGetIntegerValueField(event: *mut c_void, field: u32) -> i64;
    fn CGEventGetLocation(event: *mut c_void) -> Point;
    fn CGEventGetFlags(event: *mut c_void) -> u64;
    fn CGEventSourceKeyState(state: i32, key: u16) -> bool;
    fn CGEventSourceSecondsSinceLastEventType(state: i32, event_type: u32) -> f64;
    fn CGEventSourceButtonState(state: i32, button: u32) -> bool;
    #[cfg(test)]
    fn CGEventCreateKeyboardEvent(source: *mut c_void, key: u16, down: bool) -> *mut c_void;
    #[cfg(test)]
    fn CGEventCreateMouseEvent(
        source: *mut c_void,
        event_type: u32,
        point: Point,
        button: u32,
    ) -> *mut c_void;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFRunLoopCommonModes: *mut c_void;
    static kCFRunLoopDefaultMode: *mut c_void;
    fn CFRetain(object: *mut c_void) -> *mut c_void;
    fn CFRelease(object: *mut c_void);
    fn CFMachPortCreateRunLoopSource(
        allocator: *mut c_void,
        tap: *mut c_void,
        order: isize,
    ) -> *mut c_void;
    fn CFMachPortInvalidate(tap: *mut c_void);
    fn CFRunLoopGetCurrent() -> *mut c_void;
    fn CFRunLoopAddSource(run_loop: *mut c_void, source: *mut c_void, mode: *mut c_void);
    fn CFRunLoopRemoveSource(run_loop: *mut c_void, source: *mut c_void, mode: *mut c_void);
    fn CFRunLoopRunInMode(mode: *mut c_void, seconds: f64, return_after_source_handled: u8) -> i32;
    fn CFRunLoopStop(run_loop: *mut c_void);
    fn CFRunLoopWakeUp(run_loop: *mut c_void);
    fn CFStringCreateWithBytes(
        allocator: *mut c_void,
        bytes: *const u8,
        length: isize,
        encoding: u32,
        is_external: bool,
    ) -> *mut c_void;
    fn CFStringGetLength(text: *mut c_void) -> isize;
    fn CFStringGetMaximumSizeForEncoding(length: isize, encoding: u32) -> isize;
    fn CFStringGetCString(text: *mut c_void, buffer: *mut i8, size: isize, encoding: u32) -> bool;
}

const CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;
/// kAXErrorAPIDisabled：没有「辅助功能」授权时 AX 调用一律返回它。
const AX_ERROR_API_DISABLED: i32 = -25211;

fn cf_string(value: &str) -> *mut c_void {
    unsafe {
        CFStringCreateWithBytes(
            std::ptr::null_mut(),
            value.as_ptr(),
            value.len() as isize,
            CF_STRING_ENCODING_UTF8,
            false,
        )
    }
}

fn cf_to_string(value: *mut c_void) -> String {
    if value.is_null() {
        return String::new();
    }
    unsafe {
        let length = CFStringGetLength(value);
        if length <= 0 {
            return String::new();
        }
        let size = CFStringGetMaximumSizeForEncoding(length, CF_STRING_ENCODING_UTF8) + 1;
        if size <= 1 {
            return String::new();
        }
        let mut buffer = vec![0i8; size as usize];
        if !CFStringGetCString(value, buffer.as_mut_ptr(), size, CF_STRING_ENCODING_UTF8) {
            return String::new();
        }
        std::ffi::CStr::from_ptr(buffer.as_ptr())
            .to_string_lossy()
            .into_owned()
    }
}

/// 系统当前焦点是不是落在「能输入文字」的控件上。
///
/// 打字自动表情的全部判据就在这里：只在真的往输入框里打字时才弹，
/// 用 WASD 走路、按快捷键、点按钮都不算。AX 查询失败（没授权 / 没有焦点）
/// 一律返回不可编辑——宁可少弹一次，也不要在用户操作角色时乱弹。
pub fn focused_text_context() -> serde_json::Value {
    use serde_json::json;
    if !unsafe { AXIsProcessTrusted() } {
        return json!({"editable": false, "reason": "accessibility-denied"});
    }
    unsafe {
        let system = AXUIElementCreateSystemWide();
        if system.is_null() {
            return json!({"editable": false, "reason": "no-system-element"});
        }
        let focused_attribute = cf_string("AXFocusedUIElement");
        let mut focused: *mut c_void = std::ptr::null_mut();
        let status = AXUIElementCopyAttributeValue(system, focused_attribute, &mut focused);
        if !focused_attribute.is_null() {
            CFRelease(focused_attribute);
        }
        CFRelease(system);
        if status != 0 || focused.is_null() {
            return json!({
                "editable": false,
                "reason": if status == AX_ERROR_API_DISABLED { "accessibility-denied" } else { "no-focus" }
            });
        }
        let role_attribute = cf_string("AXRole");
        let mut role_value: *mut c_void = std::ptr::null_mut();
        let role_status = AXUIElementCopyAttributeValue(focused, role_attribute, &mut role_value);
        if !role_attribute.is_null() {
            CFRelease(role_attribute);
        }
        CFRelease(focused);
        let role = if role_status == 0 {
            let text = cf_to_string(role_value);
            if !role_value.is_null() {
                CFRelease(role_value);
            }
            text
        } else {
            String::new()
        };
        let editable = matches!(
            role.as_str(),
            "AXTextField" | "AXTextArea" | "AXComboBox" | "AXSearchField" | "AXEditableText"
        );
        json!({"editable": editable, "role": role, "reason": if editable { "editable" } else { "not-editable" }})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestEvent(*mut c_void);
    impl Drop for TestEvent {
        fn drop(&mut self) {
            unsafe {
                CFRelease(self.0);
            }
        }
    }

    fn test_context() -> (TapContext, mpsc::Receiver<NativeInput>) {
        let state = Arc::new(MonitorState {
            running: AtomicBool::new(false),
            stop: AtomicBool::new(false),
            scale: AtomicU64::new(1.0f64.to_bits()),
            window_listener_installed: AtomicBool::new(false),
            run_loop: Mutex::new(0),
            event_tap: Mutex::new(0),
            error: Mutex::new(None),
            member_resolver: Mutex::new(None),
            auxiliary_hit_test: Mutex::new(None),
        });
        let (sender, receiver) = mpsc::channel();
        (
            TapContext {
                sender,
                state,
                tap: std::ptr::null_mut(),
                pressed: BTreeSet::new(),
            },
            receiver,
        )
    }

    fn send_test_event(context: &mut TapContext, kind: u32, event: &TestEvent) {
        unsafe {
            observe_event(
                std::ptr::null_mut(),
                kind,
                event.0,
                (context as *mut TapContext).cast(),
            );
        }
    }

    #[test]
    fn native_keyboard_repeat_produces_one_press_release_pair() {
        let (mut context, receiver) = test_context();
        let down = TestEvent(unsafe { CGEventCreateKeyboardEvent(std::ptr::null_mut(), 49, true) });
        let up = TestEvent(unsafe { CGEventCreateKeyboardEvent(std::ptr::null_mut(), 49, false) });
        assert!(!down.0.is_null() && !up.0.is_null());
        send_test_event(&mut context, 10, &down);
        send_test_event(&mut context, 10, &down);
        send_test_event(&mut context, 11, &up);
        let events: Vec<_> = receiver.try_iter().collect();
        assert_eq!(events.len(), 3);
        assert!(matches!(&events[0], NativeInput::Shortcut { key, .. } if key == " "));
        assert!(matches!(&events[1], NativeInput::Key { key, pressed: true } if key == " "));
        assert!(matches!(&events[2], NativeInput::Key { key, pressed: false } if key == " "));
    }

    #[test]
    fn native_mouse_has_no_keyboard_or_clap_duplicate() {
        let (mut context, receiver) = test_context();
        let point = Point { x: -20.0, y: 30.0 };
        let down = TestEvent(unsafe { CGEventCreateMouseEvent(std::ptr::null_mut(), 1, point, 0) });
        let up = TestEvent(unsafe { CGEventCreateMouseEvent(std::ptr::null_mut(), 2, point, 0) });
        assert!(!down.0.is_null() && !up.0.is_null());
        send_test_event(&mut context, 1, &down);
        send_test_event(&mut context, 2, &up);
        let events: Vec<_> = receiver.try_iter().collect();
        assert_eq!(events.len(), 2);
        assert!(
            matches!(&events[0], NativeInput::Button { button: "left", pressed: true, point } if point.x == -20.0 && point.y == 30.0)
        );
        assert!(matches!(
            &events[1],
            NativeInput::Button {
                button: "left",
                pressed: false,
                ..
            }
        ));
    }

    #[test]
    fn native_stream_shutdown_releases_held_keys() {
        let (mut context, receiver) = test_context();
        let down =
            TestEvent(unsafe { CGEventCreateKeyboardEvent(std::ptr::null_mut(), 100, true) });
        assert!(!down.0.is_null());
        send_test_event(&mut context, 10, &down);
        context.release_keys();
        assert!(context.pressed.is_empty());
        let events: Vec<_> = receiver.try_iter().collect();
        assert_eq!(events.len(), 3);
        assert!(matches!(&events[0], NativeInput::Shortcut { key, .. } if key == "F8"));
        assert!(matches!(&events[1], NativeInput::Key { key, pressed: true } if key == "F8"));
        assert!(matches!(&events[2], NativeInput::Key { key, pressed: false } if key == "F8"));
    }

    #[test]
    fn hotkeys_and_space_keep_original_payload_contract() {
        assert_eq!(key_label(49), " ");
        assert_eq!(key_label(100), "F8");
        assert_eq!(key_label(101), "F9");
        assert_eq!(key_label(118), "F4");
        assert_eq!(key_label(96), "F5");
        assert_ne!(key_label(56), key_label(60));
    }

    fn route_button(
        routing: &mut MouseRouting,
        state: &MonitorState,
        x: f64,
        button: &'static str,
        pressed: bool,
    ) -> Option<RoutedInput> {
        route_event(
            NativeInput::Button {
                point: Point { x, y: 50.0 },
                button,
                pressed,
            },
            routing,
            state,
        )
        .unwrap()
    }

    fn auxiliary_test_state() -> Arc<MonitorState> {
        let (context, _) = test_context();
        let state = context.state;
        *state.auxiliary_hit_test.lock().unwrap() = Some(Arc::new(|x, _| x >= 100.0 && x < 200.0));
        *state.member_resolver.lock().unwrap() = Some(Arc::new(|_, _| Some(7)));
        state
    }

    #[test]
    fn panel_dom_clap_is_the_only_count_for_an_auxiliary_click() {
        let state = auxiliary_test_state();
        let mut routing = MouseRouting::default();
        let main_events = [
            route_button(&mut routing, &state, 150.0, "left", true),
            route_button(&mut routing, &state, 150.0, "left", false),
        ];
        let main_press_count = main_events
            .into_iter()
            .flatten()
            .filter(|event| {
                matches!(
                    event,
                    RoutedInput::Button {
                        button: MouseButton {
                            action: "press",
                            ..
                        },
                        ..
                    }
                )
            })
            .count();
        let panel_dom_pet_clap_count = 1; // Original panel onMousedownCapture emits pet-clap once.
        assert_eq!(main_press_count + panel_dom_pet_clap_count, 1);
        assert!(routing.pressed.is_empty());
    }

    #[test]
    fn auxiliary_origin_release_stays_suppressed_outside_auxiliary_window() {
        let state = auxiliary_test_state();
        let mut routing = MouseRouting::default();
        assert!(route_button(&mut routing, &state, 150.0, "left", true).is_none());
        assert!(route_event(
            NativeInput::Move(Point { x: 50.0, y: 50.0 }),
            &mut routing,
            &state
        )
        .unwrap()
        .is_none());
        assert!(route_button(&mut routing, &state, 50.0, "left", false).is_none());
        assert!(routing.pressed.is_empty());
        assert!(matches!(
            route_event(
                NativeInput::Move(Point { x: 50.0, y: 50.0 }),
                &mut routing,
                &state
            )
            .unwrap(),
            Some(RoutedInput::Move(_))
        ));
    }

    #[test]
    fn main_origin_drag_keeps_move_and_release_over_auxiliary_window() {
        let state = auxiliary_test_state();
        let mut routing = MouseRouting::default();
        assert!(matches!(
            route_button(&mut routing, &state, 50.0, "left", true),
            Some(RoutedInput::Button {
                button: MouseButton {
                    action: "press",
                    ..
                },
                ..
            })
        ));
        assert!(matches!(
            route_event(
                NativeInput::Move(Point { x: 150.0, y: 50.0 }),
                &mut routing,
                &state
            )
            .unwrap(),
            Some(RoutedInput::Move(_))
        ));
        assert!(matches!(
            route_button(&mut routing, &state, 150.0, "left", false),
            Some(RoutedInput::Button {
                button: MouseButton {
                    action: "release",
                    ..
                },
                ..
            })
        ));
        assert!(routing.pressed.is_empty());
        assert!(route_event(
            NativeInput::Move(Point { x: 150.0, y: 50.0 }),
            &mut routing,
            &state
        )
        .unwrap()
        .is_none());
    }

    #[test]
    fn covered_pet_hover_does_not_resolve_members_or_dispatch_main_dom_input() {
        let state = auxiliary_test_state();
        *state.member_resolver.lock().unwrap() =
            Some(Arc::new(|_, _| panic!("covered pet must not be resolved")));
        let mut routing = MouseRouting::default();
        assert!(route_event(
            NativeInput::Move(Point { x: 150.0, y: 50.0 }),
            &mut routing,
            &state
        )
        .unwrap()
        .is_none());
        assert!(route_button(&mut routing, &state, 150.0, "left", true).is_none());
        assert!(route_button(&mut routing, &state, 50.0, "left", false).is_none());
    }

    #[test]
    fn auxiliary_filter_receives_main_physical_coordinates_and_preserves_keyboard_stream() {
        let state = auxiliary_test_state();
        state.scale.store(2.0f64.to_bits(), Ordering::Release);
        let mut routing = MouseRouting::default();
        assert!(route_button(&mut routing, &state, 75.0, "left", true).is_none());
        assert!(
            matches!(route_event(NativeInput::Key { key: "a".into(), pressed: true }, &mut routing, &state).unwrap(), Some(RoutedInput::Key { key, pressed: true }) if key == "a")
        );
        assert!(
            matches!(route_event(NativeInput::Key { key: "a".into(), pressed: false }, &mut routing, &state).unwrap(), Some(RoutedInput::Key { key, pressed: false }) if key == "a")
        );
        assert!(route_button(&mut routing, &state, 25.0, "left", false).is_none());
    }

    #[test]
    fn simultaneous_buttons_keep_separate_press_owners() {
        let state = auxiliary_test_state();
        let mut routing = MouseRouting::default();
        assert!(route_button(&mut routing, &state, 150.0, "left", true).is_none());
        assert!(route_button(&mut routing, &state, 50.0, "right", true).is_some());
        assert!(route_event(
            NativeInput::Move(Point { x: 150.0, y: 50.0 }),
            &mut routing,
            &state
        )
        .unwrap()
        .is_some());
        assert!(route_button(&mut routing, &state, 150.0, "right", false).is_some());
        assert!(route_button(&mut routing, &state, 50.0, "left", false).is_none());
        assert!(routing.pressed.is_empty());
    }

    #[test]
    fn unobserved_releases_and_duplicate_presses_cannot_create_main_clicks() {
        let state = auxiliary_test_state();
        let mut routing = MouseRouting::default();
        assert!(route_button(&mut routing, &state, 50.0, "left", false).is_none());
        assert!(route_button(&mut routing, &state, 50.0, "left", true).is_some());
        assert!(route_button(&mut routing, &state, 50.0, "left", true).is_none());
        assert!(route_button(&mut routing, &state, 150.0, "left", false).is_some());
    }

    #[test]
    fn physical_coordinates_follow_target_window_scale_on_mixed_dpi() {
        let pointer = (1590.0, 100.0);
        let window_logical_origin = (1512.0, 20.0);
        for scale in [1.0, 2.0] {
            let physical = target_window_physical_point(pointer.0, pointer.1, scale);
            let relative = (
                (physical.0 - window_logical_origin.0 * scale) / scale,
                (physical.1 - window_logical_origin.1 * scale) / scale,
            );
            assert_eq!(relative, (78.0, 80.0));
        }
        assert_eq!(
            target_window_physical_point(-200.0, -20.0, 2.0),
            (-400.0, -40.0)
        );
    }

    #[test]
    fn mouse_payloads_preserve_numeric_members_and_no_extra_coordinate_fields() {
        let movement = serde_json::to_value(MousePosition {
            x: 10.0,
            y: 20.0,
            member_id: Some(5),
        })
        .unwrap();
        assert_eq!(movement["memberId"].as_u64(), Some(5));
        assert_eq!(movement["x"].as_f64(), Some(10.0));
        let button = serde_json::to_value(MouseButton {
            button: "left",
            action: "press",
            member_id: None,
        })
        .unwrap();
        assert_eq!(
            button,
            serde_json::json!({"button":"left","action":"press"})
        );
    }

    #[test]
    fn granted_permission_without_an_enabled_tap_is_not_running() {
        let status = input_status_snapshot(true, true, false, None);
        assert!(status.granted);
        assert!(!status.running);
        assert_eq!(status.error.as_deref(), Some("input_event_tap_disabled"));
    }

    #[test]
    fn revoked_permission_stops_reported_activity_even_if_worker_is_alive() {
        let status = input_status_snapshot(false, true, true, None);
        assert!(!status.granted);
        assert!(!status.running);
        assert_eq!(
            status.error.as_deref(),
            Some("input_monitoring_permission_revoked")
        );
    }

    #[test]
    fn newly_granted_permission_reports_pending_resume_not_missing_permission() {
        let status = input_status_snapshot(
            true,
            false,
            false,
            Some("input_monitoring_permission_required".into()),
        );
        assert!(status.granted);
        assert!(!status.running);
        assert_eq!(
            status.error.as_deref(),
            Some("input_access_granted_waiting_for_resume")
        );
    }

    #[test]
    fn enabled_monitor_clears_stale_permission_failure_from_its_status() {
        let status = input_status_snapshot(
            true,
            true,
            true,
            Some("input_monitoring_permission_required".into()),
        );
        assert_eq!(
            status,
            InputStatus {
                granted: true,
                running: true,
                error: None
            }
        );
    }

    #[test]
    fn failed_monitor_start_keeps_the_actual_failure_visible() {
        let status = input_status_snapshot(
            true,
            false,
            false,
            Some("input_event_tap_enable_failed".into()),
        );
        assert!(!status.running);
        assert_eq!(
            status.error.as_deref(),
            Some("input_event_tap_enable_failed")
        );
    }

    #[test]
    fn translocated_builds_point_at_the_applications_folder() {
        // A relocated app can hold a stale grant while the Settings row points at a
        // randomized path, so this hint must win over the granted state.
        assert_eq!(access_hint(true, true), "move_app_to_applications");
        assert_eq!(access_hint(false, true), "move_app_to_applications");
    }

    #[test]
    fn granted_input_monitoring_is_reported_as_healthy() {
        assert_eq!(access_hint(true, false), "ok");
    }

    #[test]
    fn missing_input_monitoring_asks_for_the_grant() {
        assert_eq!(access_hint(false, false), "grant_input_monitoring");
    }

    #[test]
    fn status_serializes_permission_failure_without_changing_other_capabilities() {
        let status = InputStatus {
            granted: false,
            running: false,
            error: Some("input_monitoring_permission_required".into()),
        };
        assert_eq!(serde_json::to_value(status).unwrap()["granted"], false);
    }
}
