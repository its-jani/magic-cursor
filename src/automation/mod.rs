//! UI Automation element extraction and framework detection.

pub mod accessibility;

pub use accessibility::{
    control_type_name, inspect_application, process_info_for_window, DetectedHotspot, ProcessInfo,
    UIAutomationExtractor,
};
