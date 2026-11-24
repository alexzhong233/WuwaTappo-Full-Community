//! Windows 原生右键菜单（TrackPopupMenuEx），由本机服务持有。
//!
//! 为什么不用 WebView 里的 HTML 菜单：桌宠窗口很小，右键菜单要压在其他窗口之上，
//! 且不能因为页面失焦而消失。这里和 Tauri 的 UI 线程完全无关，菜单由本机服务弹。
//!
//! 后台进程拿不到前台权限，`TrackPopupMenuEx` 在点击别处时不一定会自己收起，
//! 所以另起一个观察线程，只读物理按键状态，发现点到菜单矩形之外就取消「自己的」
//! 这个菜单。全程不消费、不重放、不注入输入。
use serde_json::Value;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

const WM_CANCELMODE: u32 = 0x001f;
const MENU_WINDOW_CLASS: &str = "#32768";
const MOUSE_BUTTONS: [i32; 3] = [1, 2, 4];
const MENU_KEYS: [&str; 13] = [
    "bag", "atlas", "exchange", "room", "console", "setting", "drag", "game", "hide",
    "switch-display", "open-logs", "restart", "quit",
];

static BUSY: AtomicBool = AtomicBool::new(false);

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Point {
    x: i32,
    y: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Rect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

#[link(name = "user32")]
unsafe extern "system" {
    fn CreatePopupMenu() -> isize;
    fn AppendMenuW(menu: isize, flags: u32, id: usize, item: *const u16) -> i32;
    fn DestroyMenu(menu: isize) -> i32;
    fn CreateWindowExW(
        ex_style: u32,
        class: *const u16,
        name: *const u16,
        style: u32,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        parent: isize,
        menu: isize,
        instance: isize,
        param: *mut c_void,
    ) -> isize;
    fn DestroyWindow(window: isize) -> i32;
    fn ShowWindow(window: isize, command: i32) -> i32;
    fn GetForegroundWindow() -> isize;
    fn SetForegroundWindow(window: isize) -> i32;
    fn GetCursorPos(point: *mut Point) -> i32;
    fn TrackPopupMenuEx(
        menu: isize,
        flags: u32,
        x: i32,
        y: i32,
        window: isize,
        params: *const c_void,
    ) -> i32;
    fn PostMessageW(window: isize, message: u32, wparam: usize, lparam: isize) -> i32;
    fn GetAsyncKeyState(key: i32) -> i16;
    fn GetWindowThreadProcessId(window: isize, pid: *mut u32) -> u32;
    fn EnumThreadWindows(
        thread: u32,
        callback: Option<unsafe extern "system" fn(isize, isize) -> i32>,
        param: isize,
    ) -> i32;
    fn GetClassNameW(window: isize, buffer: *mut u16, max: i32) -> i32;
    fn GetWindowRect(window: isize, rect: *mut Rect) -> i32;
    fn IsWindowVisible(window: isize) -> i32;
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

fn allowed_key(key: &str) -> bool {
    if MENU_KEYS.contains(&key) {
        return true;
    }
    if let Some(rest) = key.strip_prefix("opacity-") {
        return matches!(rest, "0" | "25" | "50" | "75" | "100");
    }
    if let Some(rest) = key.strip_prefix("scale-") {
        return matches!(rest, "50" | "75" | "100" | "125" | "150" | "175" | "200");
    }
    false
}

/// 菜单内容完全由页面提供，所以这里只放行已知动作，文本也限制长度。
fn validate(items: &Value, depth: u32) -> Result<(), String> {
    let list = items.as_array().ok_or("Invalid menu")?;
    if list.is_empty() || list.len() > 32 || depth > 1 {
        return Err("Invalid menu".into());
    }
    for item in list {
        let object = item.as_object().ok_or("Invalid menu item")?;
        if object.get("separator") == Some(&Value::Bool(true)) {
            continue;
        }
        let text = object
            .get("text")
            .and_then(Value::as_str)
            .ok_or("Invalid menu text")?;
        let length = text.chars().count();
        if length == 0 || length > 120 || text.chars().any(|ch| (ch as u32) < 32) {
            return Err("Invalid menu text".into());
        }
        match object.get("items") {
            Some(children) => validate(children, depth + 1)?,
            None => {
                let key = object
                    .get("key")
                    .and_then(Value::as_str)
                    .ok_or("Unknown menu action")?;
                if !allowed_key(key) {
                    return Err("Unknown menu action".into());
                }
            }
        }
    }
    Ok(())
}

struct MenuTree {
    next_id: usize,
    commands: Vec<(usize, String)>,
    roots: Vec<isize>,
}

impl Drop for MenuTree {
    fn drop(&mut self) {
        for handle in self.roots.drain(..) {
            unsafe { DestroyMenu(handle) };
        }
    }
}

impl MenuTree {
    fn build(&mut self, items: &Value) -> Result<isize, String> {
        // validate() 已经把结构、长度和动作键都验过，这里只做句柄拼装。
        let list = items.as_array().ok_or("Invalid menu")?;
        let menu = unsafe { CreatePopupMenu() };
        if menu == 0 {
            return Err("无法创建菜单".into());
        }
        self.roots.push(menu);
        for item in list {
            let object = item.as_object().ok_or("Invalid menu item")?;
            let appended = if object.get("separator") == Some(&Value::Bool(true)) {
                unsafe { AppendMenuW(menu, 0x800, 0, std::ptr::null()) }
            } else {
                let label = wide(object.get("text").and_then(Value::as_str).unwrap_or_default());
                match object.get("items") {
                    Some(children) => {
                        let child = self.build(children)?;
                        let appended = unsafe { AppendMenuW(menu, 0x10, child as usize, label.as_ptr()) };
                        if appended != 0 {
                            // 子菜单跟着父菜单一起销毁，不能重复销毁
                            self.roots.retain(|handle| *handle != child);
                        }
                        appended
                    }
                    None => {
                        let key = object
                            .get("key")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned();
                        let identifier = self.next_id;
                        self.next_id += 1;
                        self.commands.push((identifier, key));
                        let flags = if object.get("checked") == Some(&Value::Bool(true)) {
                            0x8
                        } else {
                            0
                        };
                        unsafe { AppendMenuW(menu, flags, identifier, label.as_ptr()) }
                    }
                }
            };
            if appended == 0 {
                return Err("无法添加菜单项".into());
            }
        }
        Ok(menu)
    }
}

fn menu_rectangles(thread_id: u32) -> Vec<Rect> {
    unsafe extern "system" fn visit(window: isize, param: isize) -> i32 {
        // SAFETY: param 是 menu_rectangles 里活着的 Vec 指针，只在本次枚举期间使用。
        let collected = unsafe { &mut *(param as *mut Vec<Rect>) };
        let mut buffer = [0u16; 64];
        let length = unsafe { GetClassNameW(window, buffer.as_mut_ptr(), buffer.len() as i32) };
        if length > 0
            && String::from_utf16_lossy(&buffer[..length as usize]) == MENU_WINDOW_CLASS
            && unsafe { IsWindowVisible(window) } != 0
        {
            let mut rect = Rect::default();
            if unsafe { GetWindowRect(window, &mut rect) } != 0 {
                collected.push(rect);
            }
        }
        1
    }
    let mut collected: Vec<Rect> = Vec::new();
    unsafe { EnumThreadWindows(thread_id, Some(visit), &mut collected as *mut Vec<Rect> as isize) };
    collected
}

fn watch_dismissal(owner: isize, thread_id: u32, previous: isize, stop: Arc<AtomicBool>) {
    let pressed = |key: i32| unsafe { GetAsyncKeyState(key) & i16::MIN != 0 };
    let mut buttons: Vec<bool> = MOUSE_BUTTONS.iter().map(|key| pressed(*key)).collect();
    let mut escape = pressed(0x1b);
    while !stop.load(Ordering::SeqCst) {
        thread::sleep(Duration::from_millis(15));
        let foreground = unsafe { GetForegroundWindow() };
        let mut cancel = foreground != 0 && foreground != previous && foreground != owner;
        let escape_state = unsafe { GetAsyncKeyState(0x1b) };
        let escape_now = escape_state & i16::MIN != 0;
        cancel |= (escape_now && !escape) || (escape_state & 1 != 0);
        escape = escape_now;
        let mut clicked = false;
        for (index, key) in MOUSE_BUTTONS.iter().enumerate() {
            let state = unsafe { GetAsyncKeyState(*key) };
            let now = state & i16::MIN != 0;
            clicked |= (now && !buttons[index]) || (state & 1 != 0);
            buttons[index] = now;
        }
        if clicked {
            let rectangles = menu_rectangles(thread_id);
            let mut point = Point::default();
            if !rectangles.is_empty() && unsafe { GetCursorPos(&mut point) } != 0 {
                cancel |= !rectangles.iter().any(|rect| {
                    rect.left <= point.x
                        && point.x < rect.right
                        && rect.top <= point.y
                        && point.y < rect.bottom
                });
            }
        }
        if cancel {
            unsafe { PostMessageW(owner, WM_CANCELMODE, 0, 0) };
            return;
        }
    }
}

/// 析构时统一收尾：停观察线程、还回前台窗口、销毁宿主窗口与菜单句柄。
struct Popup {
    owner: isize,
    previous: isize,
    roots: Vec<isize>,
    stop: Arc<AtomicBool>,
    watcher: Option<thread::JoinHandle<()>>,
}

impl Drop for Popup {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.watcher.take() {
            let _ = handle.join();
        }
        unsafe {
            if self.owner != 0 {
                if GetForegroundWindow() == self.owner && self.previous != 0 {
                    SetForegroundWindow(self.previous);
                }
                DestroyWindow(self.owner);
            }
            // 销毁根菜单会连带销毁挂上去的子菜单
            for handle in self.roots.drain(..) {
                DestroyMenu(handle);
            }
        }
    }
}

fn show(items: &Value) -> Result<Value, String> {
    let mut tree = MenuTree {
        next_id: 1,
        commands: Vec::new(),
        roots: Vec::new(),
    };
    let menu = tree.build(items)?;
    let previous = unsafe { GetForegroundWindow() };
    let class = wide("STATIC");
    let title = wide("WuwaTappo context menu");
    // WS_POPUP + WS_EX_TOOLWINDOW：只激活我们自己的零尺寸窗口，不进任务栏
    let owner = unsafe {
        CreateWindowExW(
            0x80,
            class.as_ptr(),
            title.as_ptr(),
            0x8000_0000,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            std::ptr::null_mut(),
        )
    };
    if owner == 0 {
        for handle in tree.roots.drain(..) {
            unsafe { DestroyMenu(handle) };
        }
        return Err("无法创建菜单宿主窗口".into());
    }
    let stop = Arc::new(AtomicBool::new(false));
    let mut popup = Popup {
        owner,
        previous,
        roots: std::mem::take(&mut tree.roots),
        stop: Arc::clone(&stop),
        watcher: None,
    };
    unsafe {
        ShowWindow(owner, 5);
        SetForegroundWindow(owner);
    }
    let mut point = Point::default();
    if unsafe { GetCursorPos(&mut point) } == 0 {
        return Err("无法取得鼠标位置".into());
    }
    let thread_id = unsafe { GetWindowThreadProcessId(owner, std::ptr::null_mut()) };
    popup.watcher = Some({
        let stop = Arc::clone(&stop);
        thread::spawn(move || watch_dismissal(owner, thread_id, previous, stop))
    });
    // TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON
    let selected = unsafe { TrackPopupMenuEx(menu, 0x100 | 0x80 | 0x2, point.x, point.y, owner, std::ptr::null()) };
    stop.store(true, Ordering::SeqCst);
    drop(popup);
    let action = tree
        .commands
        .iter()
        .find(|(identifier, _)| *identifier as i32 == selected)
        .map(|(_, key)| Value::String(key.clone()))
        .unwrap_or(Value::Null);
    Ok(serde_json::json!({ "action": action }))
}

/// `items` 是页面给的菜单结构；返回 `{"action": key}`，点空白处收起时 `action` 为 null。
pub fn popup(items: &Value) -> Result<Value, String> {
    validate(items, 0)?;
    if BUSY.swap(true, Ordering::SeqCst) {
        return Ok(serde_json::json!({ "action": Value::Null, "busy": true }));
    }
    struct BusyGuard;
    impl Drop for BusyGuard {
        fn drop(&mut self) {
            BUSY.store(false, Ordering::SeqCst);
        }
    }
    let _busy_guard = BusyGuard;
    show(items)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn accepts_the_menu_the_injected_page_sends() {
        let items = json!([
            {"text":"图鉴","key":"atlas"},
            {"separator":true},
            {"text":"控制台","key":"console"},
            {"text":"设置","items":[{"text":"25%","key":"opacity-25"},{"text":"100%","key":"opacity-100"}]},
            {"text":"退出","key":"quit","checked":true}
        ]);
        assert!(validate(&items, 0).is_ok());
    }

    #[test]
    fn rejects_unknown_actions_and_oversized_text() {
        assert!(validate(&json!([{"text":"未知","key":"rm-rf"}]), 0).is_err());
        assert!(validate(&json!([{"text":"x".repeat(121),"key":"quit"}]), 0).is_err());
        assert!(validate(&json!([{"text":"","key":"quit"}]), 0).is_err());
        assert!(validate(&json!([{"text":"a\tb","key":"quit"}]), 0).is_err());
        assert!(validate(&json!([]), 0).is_err());
        assert!(validate(&json!("atlas"), 0).is_err());
    }

    #[test]
    fn rejects_more_than_two_levels() {
        let deep = json!([{"text":"a","items":[{"text":"b","items":[{"text":"c","key":"quit"}]}]}]);
        assert!(validate(&deep, 0).is_err());
    }

    #[test]
    fn only_known_opacity_and_scale_steps_are_allowed() {
        for key in ["opacity-25", "opacity-50", "opacity-75", "opacity-100"] {
            assert!(allowed_key(key), "{key}");
        }
        for key in ["scale-50", "scale-200"] {
            assert!(allowed_key(key), "{key}");
        }
        assert!(!allowed_key("opacity-30"));
        assert!(!allowed_key("scale-999"));
        assert!(!allowed_key("opacity-"));
        assert!(allowed_key("open-logs"));
        assert!(allowed_key("switch-display"));
        assert!(allowed_key("opacity-0"));
    }
}
