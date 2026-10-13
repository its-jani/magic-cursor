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
    CUIAutomation, IUIAutomation, IUIAutomationElement,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetWindowTextW, GetWindowThreadProcessId, WindowFromPoint,
};

#[derive(Debug, Clone)]
pub struct DetectedHotspot {
    pub id: usize,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub name: String,
    pub control_type: String,
    pub class_name: String,
    pub process_id: u32,
    pub process_name: String,
    pub is_interactive: bool,
    pub rea_context: Option<String>,
}

impl DetectedHotspot {
    pub fn contains_point(&self, px: i32, py: i32) -> bool {
        px >= self.x && px <= (self.x + self.width) && py >= self.y && py <= (self.y + self.height)
    }

    pub fn summary(&self) -> String {
        let mut s = format!("[{}] {}", self.control_type, self.name);
        if let Some(ref rea) = self.rea_context {
            s.push_str(&format!(" ({})", rea));
        }
        s
    }
}

// =========================================================================
// Process & Window Metadata Resolver
// =========================================================================

pub fn get_process_info_for_hwnd(hwnd: HWND) -> (u32, String, String, Option<PathBuf>) {
    unsafe {
        let mut process_id: u32 = 0;
        let _ = GetWindowThreadProcessId(hwnd, Some(&mut process_id));

        // Window Title
        let mut title_buf = [0u16; 512];
        let title_len = GetWindowTextW(hwnd, &mut title_buf);
        let title = if title_len > 0 {
            OsString::from_wide(&title_buf[..title_len as usize])
                .to_string_lossy()
                .to_string()
        } else {
            String::new()
        };

        // Window Class
        let mut class_buf = [0u16; 256];
        let class_len = GetClassNameW(hwnd, &mut class_buf);
        let class_name = if class_len > 0 {
            OsString::from_wide(&class_buf[..class_len as usize])
                .to_string_lossy()
                .to_string()
        } else {
            String::new()
        };

        // Executable Path
        let mut exe_path: Option<PathBuf> = None;
        let mut _proc_name = format!("Process_{}", process_id);

        if process_id != 0 {
            let proc_handle = OpenProcess(
                PROCESS_QUERY_INFORMATION | PROCESS_VM_READ,
                false,
                process_id,
            )
            .or_else(|_| {
                OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id)
            });

            if let Ok(handle) = proc_handle {
                let mut path_buf = [0u16; MAX_PATH as usize];
                let path_len = GetModuleFileNameExW(Some(handle), None, &mut path_buf);
                if path_len > 0 {
                    let pb = PathBuf::from(
                        OsString::from_wide(&path_buf[..path_len as usize]).to_string_lossy().to_string(),
                    );
                    if let Some(file_name) = pb.file_name() {
                        _proc_name = file_name.to_string_lossy().to_string();
                    }
                    exe_path = Some(pb);
                }
            }
        }

        (process_id, title, class_name, exe_path)
    }
}

// =========================================================================
// REA (Reverse Engineer Anything) Context Enrichment (MCP Bridge)
// =========================================================================

pub struct ReaContextEnricher;

impl ReaContextEnricher {
    /// Inspects whether the target application is an Electron / WebView / Native framework app
    /// and enriches context metadata (ASAR packages, framework type, webview boundaries).
    pub fn inspect_application(
        class_name: &str,
        exe_path: Option<&Path>,
        control_type: &str,
    ) -> Option<String> {
        let mut tags = Vec::new();

        // 1. Electron & Chromium inspection
        let is_chromium = class_name.contains("Chrome_WidgetWin")
            || class_name.contains("Intermediate D3D Window")
            || class_name.contains("CefBrowserWindow");

        if is_chromium {
            tags.push("Chromium/Blink DOM".to_string());
        }

        if let Some(path) = exe_path {
            if let Some(parent) = path.parent() {
                // Check for Electron resources/app.asar or resources/app
                let asar = parent.join("resources").join("app.asar");
                let app_dir = parent.join("resources").join("app");
                if asar.exists() {
                    tags.push("Electron ASAR bundle detected".to_string());
                } else if app_dir.exists() {
                    tags.push("Electron unpacked bundle detected".to_string());
                }
            }

            let file_name_lower = path
                .file_name()
                .map(|s| s.to_string_lossy().to_lowercase())
                .unwrap_or_default();

            if file_name_lower.contains("code") || file_name_lower.contains("cursor") {
                tags.push("Monaco/VSCode Editor Element".to_string());
            } else if file_name_lower.contains("slack") {
                tags.push("Slack Electron Interface".to_string());
            } else if file_name_lower.contains("discord") {
                tags.push("Discord Electron Interface".to_string());
            }
        }

        // 2. Native Windows Frameworks
        if class_name.contains("Windows.UI.Core") || class_name.contains("ApplicationFrameWindow") {
            tags.push("WinUI 3 / XAML Surface".to_string());
        } else if class_name.contains("HwndWrapper") {
            tags.push("WPF (.NET) Window".to_string());
        } else if class_name.contains("Qt5QWindowIcon") || class_name.contains("Qt6QWindowIcon") {
            tags.push("Qt Native Widget".to_string());
        }

        if tags.is_empty() {
            if !control_type.is_empty() && control_type != "Custom" {
                return Some(format!("Native Win32 {}", control_type));
            }
            None
        } else {
            Some(tags.join(" | "))
        }
    }
}

// =========================================================================
// Windows UI Automation Element Extractor
// =========================================================================

pub struct UIAutomationExtractor {
    automation: Option<IUIAutomation>,
}

impl UIAutomationExtractor {
    pub fn new() -> Self {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let automation: Option<IUIAutomation> =
                CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok();

            Self { automation }
        }
    }

    /// Queries UI Automation elements around the cursor (x, y) to construct hotspots.
    pub fn extract_hotspots_near_cursor(&self, cursor_x: i32, cursor_y: i32) -> Vec<DetectedHotspot> {
        let mut hotspots = Vec::new();
        let automation = match self.automation.as_ref() {
            Some(a) => a,
            None => return hotspots,
        };

        unsafe {
            // 1. Get HWND under cursor to get base process details
            let hwnd = WindowFromPoint(POINT { x: cursor_x, y: cursor_y });
            let (proc_id, _title, win_class, exe_path) = get_process_info_for_hwnd(hwnd);

            // 2. Query element directly under cursor via IUIAutomation::ElementFromPoint
            let pt = POINT { x: cursor_x, y: cursor_y };
            let direct_elem = automation.ElementFromPoint(pt).ok();

            let mut visited_ids = std::collections::HashSet::new();

            if let Some(elem) = direct_elem {
                self.add_element_if_valid(
                    &elem,
                    &mut hotspots,
                    &mut visited_ids,
                    &win_class,
                    exe_path.as_deref(),
                    proc_id,
                );

                // 3. Walk tree to discover parent/siblings near the cursor
                if let Ok(walker) = automation.ControlViewWalker() {
                    // Try getting parent
                    if let Ok(parent) = walker.GetParentElement(&elem) {
                        self.add_element_if_valid(
                            &parent,
                            &mut hotspots,
                            &mut visited_ids,
                            &win_class,
                            exe_path.as_deref(),
                            proc_id,
                        );

                        // Try getting adjacent siblings
                        if let Ok(first_child) = walker.GetFirstChildElement(&parent) {
                            let mut curr = Some(first_child);
                            let mut count = 0;
                            while let Some(child) = curr {
                                if count > 15 {
                                    break;
                                }
                                self.add_element_if_valid(
                                    &child,
                                    &mut hotspots,
                                    &mut visited_ids,
                                    &win_class,
                                    exe_path.as_deref(),
                                    proc_id,
                                );
                                curr = walker.GetNextSiblingElement(&child).ok();
                                count += 1;
                            }
                        }
                    }
                }
            }

            // Fallback: If no UIA elements detected, create a synthetic window-bounds hotspot
            if hotspots.is_empty() && !hwnd.is_invalid() {
                let mut rect = RECT::default();
                let _ = windows::Win32::UI::WindowsAndMessaging::GetWindowRect(hwnd, &mut rect);
                let w = rect.right - rect.left;
                let h = rect.bottom - rect.top;
                if w > 20 && h > 20 {
                    let rea_tag = ReaContextEnricher::inspect_application(
                        &win_class,
                        exe_path.as_deref(),
                        "Window",
                    );
                    hotspots.push(DetectedHotspot {
                        id: 1,
                        x: rect.left,
                        y: rect.top,
                        width: w,
                        height: h,
                        name: if _title.is_empty() { win_class.clone() } else { _title },
                        control_type: "Window".to_string(),
                        class_name: win_class,
                        process_id: proc_id,
                        process_name: exe_path
                            .as_ref()
                            .and_then(|p| p.file_name())
                            .map(|s| s.to_string_lossy().to_string())
                            .unwrap_or_else(|| format!("PID:{}", proc_id)),
                        is_interactive: true,
                        rea_context: rea_tag,
                    });
                }
            }
        }

        hotspots
    }

    unsafe fn add_element_if_valid(
        &self,
        elem: &IUIAutomationElement,
        hotspots: &mut Vec<DetectedHotspot>,
        visited: &mut std::collections::HashSet<String>,
        win_class: &str,
        exe_path: Option<&Path>,
        default_pid: u32,
    ) {
        let rect: RECT = match elem.CurrentBoundingRectangle() {
            Ok(r) => r,
            Err(_) => return,
        };

        let w = rect.right - rect.left;
        let h = rect.bottom - rect.top;

        // Skip degenerate, zero, or giant full-screen containers
        if w < 12 || h < 12 || (w > 3840 && h > 2160) {
            return;
        }

        let key = format!("{}:{}:{}:{}", rect.left, rect.top, w, h);
        if visited.contains(&key) {
            return;
        }
        visited.insert(key);

        let name: String = elem
            .CurrentName()
            .map(|bstr| bstr.to_string())
            .unwrap_or_default();

        let control_type_id = elem.CurrentControlType().map(|c| c.0).unwrap_or(0);
        let control_type = control_type_to_string(control_type_id);

        let class_name: String = elem
            .CurrentClassName()
            .map(|bstr| bstr.to_string())
            .unwrap_or_else(|_| win_class.to_string());

        let proc_id = elem.CurrentProcessId().unwrap_or(default_pid as i32) as u32;

        let rea_tag = ReaContextEnricher::inspect_application(&class_name, exe_path, &control_type);

        let id = hotspots.len() + 1;
        hotspots.push(DetectedHotspot {
            id,
            x: rect.left,
            y: rect.top,
            width: w,
            height: h,
            name: if name.is_empty() { control_type.clone() } else { name },
            control_type,
            class_name,
            process_id: proc_id,
            process_name: exe_path
                .and_then(|p| p.file_name())
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| format!("PID:{}", proc_id)),
            is_interactive: true,
            rea_context: rea_tag,
        });
    }
}

impl Drop for UIAutomationExtractor {
    fn drop(&mut self) {
        self.automation = None;
        unsafe {
            CoUninitialize();
        }
    }
}

fn control_type_to_string(id: i32) -> String {
    match id {
        50000 => "Button",
        50001 => "Calendar",
        50002 => "CheckBox",
        50003 => "ComboBox",
        50004 => "Edit",
        50005 => "Hyperlink",
        50006 => "Image",
        50007 => "ListItem",
        50008 => "List",
        50009 => "Menu",
        50010 => "MenuBar",
        50011 => "MenuItem",
        50012 => "ProgressBar",
        50013 => "RadioButton",
        50014 => "ScrollBar",
        50015 => "Slider",
        50016 => "Spinner",
        50017 => "StatusBar",
        50018 => "Tab",
        50019 => "TabItem",
        50020 => "Text",
        50021 => "ToolBar",
        50022 => "ToolTip",
        50023 => "Tree",
        50024 => "TreeItem",
        50025 => "Custom",
        50026 => "Group",
        50027 => "Thumb",
        50028 => "DataGrid",
        50029 => "DataItem",
        50030 => "Document",
        50031 => "SplitButton",
        50032 => "Window",
        50033 => "Pane",
        50034 => "Header",
        50035 => "HeaderItem",
        50036 => "Table",
        50037 => "TitleBar",
        50038 => "Separator",
        _ => "Element",
    }
    .to_string()
}
