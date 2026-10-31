//! Windows UI Automation element extraction and application-context enrichment.
//!
//! The extractor resolves the interactive elements around the cursor through
//! `IUIAutomation`, then tags the owning window with the framework detection in
//! [`inspect_application`] so the model knows whether it is looking at an
//! Electron, `WinUI`, WPF, or Qt surface.

use std::collections::HashSet;
use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::path::{Path, PathBuf};

use windows::Win32::Foundation::{HWND, MAX_PATH, POINT, RECT};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
};
use windows::Win32::System::ProcessStatus::GetModuleFileNameExW;
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_VM_READ,
};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationElement, IUIAutomationTreeWalker,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetWindowRect, GetWindowTextW, GetWindowThreadProcessId, WindowFromPoint,
};

/// Largest number of siblings walked from the parent of the element under the cursor.
const MAX_SIBLINGS: usize = 15;

/// Elements smaller than this in either axis are treated as noise.
const MIN_ELEMENT_PX: i32 = 12;

/// Elements covering more than this are container windows, not targets.
const MAX_ELEMENT_WIDTH_PX: i32 = 3840;
/// See [`MAX_ELEMENT_WIDTH_PX`].
const MAX_ELEMENT_HEIGHT_PX: i32 = 2160;

/// Lengths of the stack buffers used for window titles and class names.
const WINDOW_TITLE_CHARS: usize = 512;
const WINDOW_CLASS_CHARS: usize = 256;

/// UIA control type identifiers, as exposed by `UIA_ControlTypePropertyId`.
mod control_type {
    pub const BUTTON: i32 = 50000;
    pub const CALENDAR: i32 = 50001;
    pub const CHECK_BOX: i32 = 50002;
    pub const COMBO_BOX: i32 = 50003;
    pub const EDIT: i32 = 50004;
    pub const HYPERLINK: i32 = 50005;
    pub const IMAGE: i32 = 50006;
    pub const LIST_ITEM: i32 = 50007;
    pub const LIST: i32 = 50008;
    pub const MENU: i32 = 50009;
    pub const MENU_BAR: i32 = 50010;
    pub const MENU_ITEM: i32 = 50011;
    pub const PROGRESS_BAR: i32 = 50012;
    pub const RADIO_BUTTON: i32 = 50013;
    pub const SCROLL_BAR: i32 = 50014;
    pub const SLIDER: i32 = 50015;
    pub const SPINNER: i32 = 50016;
    pub const STATUS_BAR: i32 = 50017;
    pub const TAB: i32 = 50018;
    pub const TAB_ITEM: i32 = 50019;
    pub const TEXT: i32 = 50020;
    pub const TOOL_BAR: i32 = 50021;
    pub const TOOL_TIP: i32 = 50022;
    pub const TREE: i32 = 50023;
    pub const TREE_ITEM: i32 = 50024;
    pub const CUSTOM: i32 = 50025;
    pub const GROUP: i32 = 50026;
    pub const THUMB: i32 = 50027;
    pub const DATA_GRID: i32 = 50028;
    pub const DATA_ITEM: i32 = 50029;
    pub const DOCUMENT: i32 = 50030;
    pub const SPLIT_BUTTON: i32 = 50031;
    pub const WINDOW: i32 = 50032;
    pub const PANE: i32 = 50033;
    pub const HEADER: i32 = 50034;
    pub const HEADER_ITEM: i32 = 50035;
    pub const TABLE: i32 = 50036;
    pub const TITLE_BAR: i32 = 50037;
    pub const SEPARATOR: i32 = 50038;
}

/// An interactive element found under or near the cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectedHotspot {
    /// One-based index within the extraction result.
    pub id: usize,
    /// Left edge in virtual-screen coordinates.
    pub x: i32,
    /// Top edge in virtual-screen coordinates.
    pub y: i32,
    /// Width in pixels.
    pub width: i32,
    /// Height in pixels.
    pub height: i32,
    /// Accessible name, falling back to the control type.
    pub name: String,
    /// Human-readable UIA control type, such as `Button`.
    pub control_type: String,
    /// Window class of the element.
    pub class_name: String,
    /// Owning process identifier.
    pub process_id: u32,
    /// Owning executable name, or `PID:<id>` when unknown.
    pub process_name: String,
    /// Framework tags produced by [`inspect_application`].
    pub rea_context: Option<String>,
}

impl DetectedHotspot {
    /// Returns whether the point lies inside the element's bounds, edges included.
    pub fn contains_point(&self, px: i32, py: i32) -> bool {
        px >= self.x && px <= self.x + self.width && py >= self.y && py <= self.y + self.height
    }

    /// Returns a single-line description used in prompts and clipboard copies.
    pub fn summary(&self) -> String {
        match &self.rea_context {
            Some(context) => format!("[{}] {} ({})", self.control_type, self.name, context),
            None => format!("[{}] {}", self.control_type, self.name),
        }
    }

    /// Returns the label shown on the tray badge for this element.
    pub fn tray_title(&self) -> String {
        format!("[{}] {}", self.control_type, self.name)
    }
}

/// Identity of the process and window that own an element.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProcessInfo {
    /// Process identifier, or `0` when the window belongs to no process.
    pub process_id: u32,
    /// Window title, empty when the window has none.
    pub title: String,
    /// Window class name, empty when unavailable.
    pub class_name: String,
    /// Full path of the owning executable, when it can be read.
    pub executable: Option<PathBuf>,
}

impl ProcessInfo {
    /// Returns the executable's file name, or `PID:<id>` as a fallback.
    pub fn display_name(&self) -> String {
        self.executable
            .as_ref()
            .and_then(|path| path.file_name())
            .map_or_else(
                || format!("PID:{}", self.process_id),
                |name| name.to_string_lossy().into_owned(),
            )
    }
}

/// Reads the process id, title, class, and executable path of `hwnd`.
pub fn process_info_for_window(hwnd: HWND) -> ProcessInfo {
    let mut process_id: u32 = 0;

    // SAFETY: `hwnd` may be any handle; the API writes to a stack local only.
    unsafe {
        let _ = GetWindowThreadProcessId(hwnd, Some(&mut process_id));
    }

    ProcessInfo {
        process_id,
        title: window_text(hwnd),
        class_name: window_class(hwnd),
        executable: executable_path(process_id),
    }
}

/// Reads a window's title, returning an empty string when it has none.
fn window_text(hwnd: HWND) -> String {
    let mut buffer = [0u16; WINDOW_TITLE_CHARS];

    // SAFETY: the buffer is owned by this frame and its length is passed implicitly
    // through the fixed-size array type.
    let length = unsafe { GetWindowTextW(hwnd, &mut buffer) };

    wide_to_string(&buffer, length)
}

/// Reads a window's class name, returning an empty string when unavailable.
fn window_class(hwnd: HWND) -> String {
    let mut buffer = [0u16; WINDOW_CLASS_CHARS];

    // SAFETY: as in `window_text`.
    let length = unsafe { GetClassNameW(hwnd, &mut buffer) };

    wide_to_string(&buffer, length)
}

/// Converts the first `length` UTF-16 units of `buffer` into a `String`.
fn wide_to_string(buffer: &[u16], length: i32) -> String {
    let length = usize::try_from(length).unwrap_or(0).min(buffer.len());
    if length == 0 {
        return String::new();
    }
    OsString::from_wide(&buffer[..length])
        .to_string_lossy()
        .into_owned()
}

/// Resolves the executable behind a process id.
fn executable_path(process_id: u32) -> Option<PathBuf> {
    if process_id == 0 {
        return None;
    }

    // The full-access query exposes the module path; the limited query is enough
    // for processes this user may not inspect in depth, which still yields a name.
    // SAFETY: OpenProcess takes a pid and returns an owned handle; dropping it
    // closes the handle.
    let handle = unsafe {
        OpenProcess(
            PROCESS_QUERY_INFORMATION | PROCESS_VM_READ,
            false,
            process_id,
        )
        .or_else(|_| OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id))
        .ok()?
    };

    let mut buffer = [0u16; MAX_PATH as usize];

    // SAFETY: the handle came from OpenProcess and the buffer is a stack local.
    let length = unsafe { GetModuleFileNameExW(Some(handle), None, &mut buffer) };

    let length = usize::try_from(length).unwrap_or(0).min(buffer.len());
    if length == 0 {
        return None;
    }

    Some(PathBuf::from(
        OsString::from_wide(&buffer[..length])
            .to_string_lossy()
            .into_owned(),
    ))
}

/// Tags a window with the UI framework that renders it.
///
/// The tags are hints for the model, not ground truth: they come from the window
/// class, the executable name, and whether an Electron bundle sits next to it.
pub fn inspect_application(
    class_name: &str,
    exe_path: Option<&Path>,
    control_type: &str,
) -> Option<String> {
    let mut tags = Vec::new();

    if class_name.contains("Chrome_WidgetWin")
        || class_name.contains("Intermediate D3D Window")
        || class_name.contains("CefBrowserWindow")
    {
        tags.push("Chromium/Blink DOM".to_string());
    }

    if let Some(path) = exe_path {
        if let Some(parent) = path.parent() {
            let resources = parent.join("resources");
            if resources.join("app.asar").exists() {
                tags.push("Electron ASAR bundle detected".to_string());
            } else if resources.join("app").exists() {
                tags.push("Electron unpacked bundle detected".to_string());
            }
        }

        if let Some(tag) = shell_tag(path) {
            tags.push(tag.to_string());
        }
    }

    if class_name.contains("Windows.UI.Core") || class_name.contains("ApplicationFrameWindow") {
        tags.push("WinUI 3 / XAML Surface".to_string());
    } else if class_name.contains("HwndWrapper") {
        tags.push("WPF (.NET) Window".to_string());
    } else if class_name.contains("QWindowIcon") {
        tags.push("Qt Native Widget".to_string());
    }

    if tags.is_empty() {
        if control_type.is_empty() || control_type == "Custom" {
            return None;
        }
        return Some(format!("Native Win32 {control_type}"));
    }

    Some(tags.join(" | "))
}

/// Recognises well-known Electron applications by executable name.
fn shell_tag(path: &Path) -> Option<&'static str> {
    let file_name = path.file_name()?.to_string_lossy().to_lowercase();

    if file_name.contains("code") || file_name.contains("cursor") {
        Some("Monaco/VSCode Editor Element")
    } else if file_name.contains("slack") {
        Some("Slack Electron Interface")
    } else if file_name.contains("discord") {
        Some("Discord Electron Interface")
    } else {
        None
    }
}

/// Maps a UIA control type identifier to its readable name.
pub fn control_type_name(id: i32) -> &'static str {
    match id {
        control_type::BUTTON => "Button",
        control_type::CALENDAR => "Calendar",
        control_type::CHECK_BOX => "CheckBox",
        control_type::COMBO_BOX => "ComboBox",
        control_type::EDIT => "Edit",
        control_type::HYPERLINK => "Hyperlink",
        control_type::IMAGE => "Image",
        control_type::LIST_ITEM => "ListItem",
        control_type::LIST => "List",
        control_type::MENU => "Menu",
        control_type::MENU_BAR => "MenuBar",
        control_type::MENU_ITEM => "MenuItem",
        control_type::PROGRESS_BAR => "ProgressBar",
        control_type::RADIO_BUTTON => "RadioButton",
        control_type::SCROLL_BAR => "ScrollBar",
        control_type::SLIDER => "Slider",
        control_type::SPINNER => "Spinner",
        control_type::STATUS_BAR => "StatusBar",
        control_type::TAB => "Tab",
        control_type::TAB_ITEM => "TabItem",
        control_type::TEXT => "Text",
        control_type::TOOL_BAR => "ToolBar",
        control_type::TOOL_TIP => "ToolTip",
        control_type::TREE => "Tree",
        control_type::TREE_ITEM => "TreeItem",
        control_type::CUSTOM => "Custom",
        control_type::GROUP => "Group",
        control_type::THUMB => "Thumb",
        control_type::DATA_GRID => "DataGrid",
        control_type::DATA_ITEM => "DataItem",
        control_type::DOCUMENT => "Document",
        control_type::SPLIT_BUTTON => "SplitButton",
        control_type::WINDOW => "Window",
        control_type::PANE => "Pane",
        control_type::HEADER => "Header",
        control_type::HEADER_ITEM => "HeaderItem",
        control_type::TABLE => "Table",
        control_type::TITLE_BAR => "TitleBar",
        control_type::SEPARATOR => "Separator",
        _ => "Element",
    }
}

/// Queries UI Automation for the elements around the cursor.
pub struct UIAutomationExtractor {
    automation: Option<IUIAutomation>,
    com_initialized: bool,
}

impl std::fmt::Debug for UIAutomationExtractor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UIAutomationExtractor")
            .field("available", &self.automation.is_some())
            .field("com_initialized", &self.com_initialized)
            .finish()
    }
}

impl Default for UIAutomationExtractor {
    fn default() -> Self {
        Self::new()
    }
}

impl UIAutomationExtractor {
    /// Initialises COM on the calling thread and creates the automation object.
    ///
    /// Call this on the thread that will run the queries; COM is apartment bound.
    pub fn new() -> Self {
        // SAFETY: CoInitializeEx only affects the current thread, and the result is
        // paired with CoUninitialize in Drop when it reports success.
        let (com_initialized, automation) = unsafe {
            let initialized = CoInitializeEx(None, COINIT_MULTITHREADED).is_ok();
            let automation: Option<IUIAutomation> =
                CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok();
            (initialized, automation)
        };

        Self {
            automation,
            com_initialized,
        }
    }

    /// Returns the elements under and around `(cursor_x, cursor_y)`, nearest first.
    pub fn extract_hotspots_near_cursor(
        &self,
        cursor_x: i32,
        cursor_y: i32,
    ) -> Vec<DetectedHotspot> {
        let Some(automation) = self.automation.as_ref() else {
            return Vec::new();
        };

        let point = POINT {
            x: cursor_x,
            y: cursor_y,
        };

        // SAFETY: WindowFromPoint takes a point by value and returns a borrowed,
        // non-owning window handle.
        let hwnd = unsafe { WindowFromPoint(point) };
        let process = process_info_for_window(hwnd);

        // SAFETY: the automation object is alive and the point is a plain value.
        let Some(element) = (unsafe { automation.ElementFromPoint(point).ok() }) else {
            return window_fallback_hotspot(hwnd, &process);
        };

        let mut hotspots = Vec::new();
        let mut visited = HashSet::new();

        collect(&element, &mut hotspots, &mut visited, &process);

        // SAFETY: the walker and elements are COM objects owned by this scope.
        if let Ok(walker) = unsafe { automation.ControlViewWalker() } {
            // SAFETY: the walker and element are live and owned by this scope.
            if let Ok(parent) = unsafe { walker.GetParentElement(&element) } {
                collect(&parent, &mut hotspots, &mut visited, &process);
                collect_children(&walker, &parent, &mut hotspots, &mut visited, &process);
            }
        }

        if hotspots.is_empty() {
            return window_fallback_hotspot(hwnd, &process);
        }

        hotspots
    }
}

/// Collects the siblings reachable from `parent`, up to [`MAX_SIBLINGS`].
fn collect_children(
    walker: &IUIAutomationTreeWalker,
    parent: &IUIAutomationElement,
    hotspots: &mut Vec<DetectedHotspot>,
    visited: &mut HashSet<String>,
    process: &ProcessInfo,
) {
    // SAFETY: both objects are live COM interfaces owned by the caller.
    let Ok(mut current) = (unsafe { walker.GetFirstChildElement(parent) }) else {
        return;
    };

    for _ in 0..MAX_SIBLINGS {
        collect(&current, hotspots, visited, process);

        // SAFETY: as above; a missing sibling simply ends the walk.
        let Ok(next) = (unsafe { walker.GetNextSiblingElement(&current) }) else {
            return;
        };
        current = next;
    }
}

/// Appends `element` to `hotspots` when it is a usable target.
fn collect(
    element: &IUIAutomationElement,
    hotspots: &mut Vec<DetectedHotspot>,
    visited: &mut HashSet<String>,
    process: &ProcessInfo,
) {
    // SAFETY: `element` is a live COM interface for the duration of the call.
    let Ok(rect) = (unsafe { element.CurrentBoundingRectangle() }) else {
        return;
    };

    let (width, height) = (rect.right - rect.left, rect.bottom - rect.top);
    if width < MIN_ELEMENT_PX
        || height < MIN_ELEMENT_PX
        || (width > MAX_ELEMENT_WIDTH_PX && height > MAX_ELEMENT_HEIGHT_PX)
    {
        return;
    }

    if !visited.insert(format!("{},{},{width},{height}", rect.left, rect.top)) {
        return;
    }

    // SAFETY: property reads on a live element; each returns an owned BSTR.
    let (name, control_type_id, class_name, process_id) = unsafe {
        (
            element
                .CurrentName()
                .map(|name| name.to_string())
                .unwrap_or_default(),
            element.CurrentControlType().map_or(0, |kind| kind.0),
            element
                .CurrentClassName()
                .map_or_else(|_| process.class_name.clone(), |name| name.to_string()),
            element
                .CurrentProcessId()
                .unwrap_or(process.process_id as i32) as u32,
        )
    };

    let control_type = control_type_name(control_type_id).to_string();
    let rea_context =
        inspect_application(&class_name, process.executable.as_deref(), &control_type);

    hotspots.push(DetectedHotspot {
        id: hotspots.len() + 1,
        x: rect.left,
        y: rect.top,
        width,
        height,
        name: if name.is_empty() {
            control_type.clone()
        } else {
            name
        },
        control_type,
        class_name,
        process_id,
        process_name: process.display_name(),
        rea_context,
    });
}

impl Drop for UIAutomationExtractor {
    fn drop(&mut self) {
        // Release the COM object before tearing down the apartment.
        self.automation = None;

        if self.com_initialized {
            // SAFETY: every successful CoInitializeEx on this thread is matched by
            // exactly one CoUninitialize on the same thread.
            unsafe { CoUninitialize() };
        }
    }
}

/// Builds a hotspot covering the whole window when UIA exposes no elements.
fn window_fallback_hotspot(hwnd: HWND, process: &ProcessInfo) -> Vec<DetectedHotspot> {
    if hwnd.is_invalid() {
        return Vec::new();
    }

    let mut rect = RECT::default();
    // SAFETY: `hwnd` was returned by WindowFromPoint and `rect` is a stack local.
    if unsafe { GetWindowRect(hwnd, &mut rect) }.is_err() {
        return Vec::new();
    }

    let width = rect.right - rect.left;
    let height = rect.bottom - rect.top;
    if width <= MIN_ELEMENT_PX || height <= MIN_ELEMENT_PX {
        return Vec::new();
    }

    let rea_context =
        inspect_application(&process.class_name, process.executable.as_deref(), "Window");

    vec![DetectedHotspot {
        id: 1,
        x: rect.left,
        y: rect.top,
        width,
        height,
        name: if process.title.is_empty() {
            process.class_name.clone()
        } else {
            process.title.clone()
        },
        control_type: "Window".to_string(),
        class_name: process.class_name.clone(),
        process_id: process.process_id,
        process_name: process.display_name(),
        rea_context,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hotspot() -> DetectedHotspot {
        DetectedHotspot {
            id: 1,
            x: 100,
            y: 200,
            width: 50,
            height: 20,
            name: "Save".to_string(),
            control_type: "Button".to_string(),
            class_name: "Button".to_string(),
            process_id: 42,
            process_name: "notepad.exe".to_string(),
            rea_context: None,
        }
    }

    #[test]
    fn control_types_map_to_readable_names() {
        assert_eq!(control_type_name(control_type::BUTTON), "Button");
        assert_eq!(control_type_name(control_type::TAB_ITEM), "TabItem");
        assert_eq!(control_type_name(-1), "Element");
    }

    #[test]
    fn contains_point_includes_the_edges() {
        let hotspot = hotspot();

        assert!(hotspot.contains_point(100, 200));
        assert!(hotspot.contains_point(150, 220));
        assert!(!hotspot.contains_point(99, 200));
        assert!(!hotspot.contains_point(151, 200));
    }

    #[test]
    fn summary_appends_the_framework_tag() {
        let mut hotspot = hotspot();
        hotspot.rea_context = Some("WPF (.NET) Window".to_string());

        assert_eq!(hotspot.summary(), "[Button] Save (WPF (.NET) Window)");
    }

    #[test]
    fn chromium_classes_are_tagged() {
        let tag = inspect_application("Chrome_WidgetWin_1", None, "Button");

        assert_eq!(tag.as_deref(), Some("Chromium/Blink DOM"));
    }

    #[test]
    fn unknown_custom_controls_produce_no_tag() {
        assert!(inspect_application("SomeClass", None, "Custom").is_none());
        assert_eq!(
            inspect_application("SomeClass", None, "Button").as_deref(),
            Some("Native Win32 Button")
        );
    }

    #[test]
    fn display_name_falls_back_to_the_pid() {
        let process = ProcessInfo {
            process_id: 17,
            ..ProcessInfo::default()
        };

        assert_eq!(process.display_name(), "PID:17");
    }

    #[test]
    fn wide_buffers_are_trimmed_to_their_length() {
        let buffer: Vec<u16> = "Title\0garbage".encode_utf16().collect();

        assert_eq!(wide_to_string(&buffer, 5), "Title");
        assert_eq!(wide_to_string(&buffer, 0), "");
        assert_eq!(wide_to_string(&buffer, -4), "");
    }
}
