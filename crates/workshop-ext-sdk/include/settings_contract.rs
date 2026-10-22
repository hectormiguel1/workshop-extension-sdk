//! Versioned, game-independent settings provider wire contract.
//! x86 layout compatible with the v1 control descriptors (`ff13_settings.h`).
//! Crux's own Rust definition of the text extension, vendored for the SDK's drift test.
//! All strings are bounded NUL-terminated UTF-8 resource keys (prefer ZTR keys).
//! Metadata and callback code must remain valid for the process lifetime.
pub type ReadValue = unsafe extern "C" fn(usize) -> f32;
pub type WriteValue = unsafe extern "C" fn(usize, f32);
pub type Activate = unsafe extern "C" fn(usize);
pub type ReadState = unsafe extern "C" fn(usize) -> u32;
pub type ReadText = unsafe extern "C" fn(usize, *mut u8, u32) -> u32;
pub type Query = unsafe extern "C" fn(*const Host) -> usize;

#[repr(C)]
pub struct Host {
    pub size: u32,
    pub version: u32,
    pub game: u32,
    pub game_name: usize,
    pub backend_version: usize,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Descriptor {
    pub size: u32,
    pub version: u32,
    pub id: usize,
    pub name: usize,
    pub count: u32,
    pub controls: usize,
    pub native_count: u32,
    pub native_stride: u32,
    pub native_menus: usize,
    pub stride: u32,
    pub author: usize,
    pub version_text: usize,
    pub description: usize,
    pub website: usize,
    pub color: [f32; 4],
    pub color_flags: u32,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Control {
    pub size: u32,
    pub kind: u32,
    pub id: usize,
    pub label: usize,
    pub description: usize,
    pub min: f32,
    pub max: f32,
    pub step: f32,
    pub read: usize,
    pub write: usize,
    pub user: usize,
    pub activate: usize,
    pub option_count: u32,
    pub options: usize,
    pub extensions: usize,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Extension {
    pub size: u32,
    pub kind: u32,
    pub version: u32,
    pub next: usize,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct CallbackExtension {
    pub header: Extension,
    pub callback: usize,
    pub user: usize,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct TextExtension {
    pub header: Extension,
    pub callback: usize,
    pub user: usize,
    pub refresh_ms: u32,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct PresentationExtension {
    pub header: Extension,
    pub flags: u32,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct DisplayExtension {
    pub header: Extension,
    pub prefix: usize,
    pub suffix: usize,
    pub decimals: u32,
}

/// Called outside the registry lock. Return nonzero to stop enumeration.
pub type Visitor = unsafe extern "C" fn(*const u8, usize, usize, Query, usize) -> i32;
pub type Register = unsafe extern "C" fn(*const u8, usize, Option<Query>) -> i32;
pub type Visit = unsafe extern "C" fn(Option<Visitor>, usize) -> i32;
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Api {
    pub version: u32,
    pub size: u32,
    pub register_provider: Register,
    pub visit_providers: Visit,
}
